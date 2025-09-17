//! Inter-VM communication (IVC) module.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use memory_addr::{MemoryAddr, PAGE_SIZE_4K, align_up_4k};

use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::sync::Mutex;

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, GuestVirtAddrRange, HostPhysAddr, MappingFlags};
use axerrno::{AxResult, ax_err};
use bitflags::bitflags;
use page_table_multiarch::PagingHandler;

use crate::libos::npt_mapping::GuestNestedMapping;
use crate::region::HostPhysicalRegion;

// https://elixir.bootlin.com/linux/v6.8.10/source/include/uapi/asm-generic/hugetlb_encode.h#L26

pub const SHM_HUGE_SHIFT: usize = 26;
#[allow(unused)]
pub const SHM_HUGE_MASK: usize = 0x3f << SHM_HUGE_SHIFT;

bitflags! {
    /// System V Shared Memory Flags
    #[derive(Eq, PartialEq, Copy, Clone, Debug)]
    pub struct ShmFlags: usize {
        // --- Standard permission bits (same as open(2)) ---
        /// Read permission (same as 0o400 / S_IRUGO)
        const SHM_R         = 0o00000400;
        /// Write permission (same as 0o200 / S_IWUGO)
        const SHM_W         = 0o00000200;

        // --- IPC resource control flags ---
        /// Create if key does not exist
        const IPC_CREAT     = 0o00001000;
        /// Fail if key exists
        const IPC_EXCL      = 0o00002000;
        /// Don't block if not available
        const IPC_NOWAIT    = 0o00004000;

        // --- shmget() special flags ---
        /// Use HugeTLB pages
        const SHM_HUGETLB   = 0o00004000;
        /// Don't reserve swap space
        const SHM_NORESERVE = 0o00010000;

        // --- shmat() attach flags ---
        /// Read-only attach
        const SHM_RDONLY    = 0o00010000;
        /// Round attach address to SHMLBA
        const SHM_RND       = 0o00020000;
        /// Replace existing mapping
        const SHM_REMAP     = 0o00040000;
        /// Execution access
        const SHM_EXEC      = 0o00100000;

        // --- Optional: huge page size encode mask (needs constants from hugetlb_encode.h)
        const SHM_HUGE_2MB    = 21 << SHM_HUGE_SHIFT;
        const SHM_HUGE_1GB    = 30 << SHM_HUGE_SHIFT;
    }
}

/// A global btree map to store IVC channels,
/// indexed by (channel_key).
static IVC_CHANNELS: Mutex<BTreeMap<u32, IVCChannel<PagingHandlerImpl>>> =
    Mutex::new(BTreeMap::new());

/// Insert a new IVC channel into the global map.
/// If the channel already exists and `remove_collision` is false, it will return an error.
/// If `remove_collision` is true, it will remove the existing channel and insert the new one.
/// Returns `Ok(())` if the channel is inserted successfully, or an error if it already exists and `remove_collision` is false.
pub fn insert_channel(
    channel_key: u32,
    channel: IVCChannel<PagingHandlerImpl>,
    remove_collision: bool,
) -> AxResult<()> {
    let mut channels = IVC_CHANNELS.lock();

    if channels.contains_key(&channel_key) && !remove_collision {
        return ax_err!(
            AlreadyExists,
            format!("IVC channel key {:#x} already exists", channel_key)
        );
    }

    if channels.insert(channel_key, channel).is_some() && !remove_collision {
        // The channel will be removed peacefully if it already exists and `remove_collision` is true.
        Err(axerrno::ax_err_type!(
            BadState,
            "We have made sure that the channel key does not exist, but it does exist now."
        ))
    } else {
        Ok(())
    }
}

pub fn get_channel_region_size(channel_key: u32) -> Option<usize> {
    let channels = IVC_CHANNELS.lock();
    channels
        .get(&channel_key)
        .map(|ch| ch.backend_region().size())
}

#[allow(unused)]
pub fn contains_channel(channel_key: u32) -> bool {
    IVC_CHANNELS.lock().contains_key(&channel_key)
}

pub fn sync_channel_mapping(
    key: u32,
    subscriber_vm_id: usize,
    subscriber_gpa: GuestPhysAddr,
    flags: MappingFlags,
    mapper: impl FnMut(GuestPhysAddr, HostPhysAddr, usize, MappingFlags, bool) -> AxResult,
) -> AxResult {
    let channels = IVC_CHANNELS.lock();
    if let Some(channel) = channels.get(&key) {
        if let Some((gpa, size)) = channel.subscribers().iter().find_map(|(vm_id, gpa, size)| {
            if *vm_id == subscriber_vm_id {
                Some((*gpa, *size))
            } else {
                None
            }
        }) {
            if gpa != subscriber_gpa {
                return ax_err!(
                    InvalidInput,
                    format!(
                        "IVC channel key [{:#x}] subscriber VM {} GPA mismatch: expected {:?}, got {:?}",
                        key, subscriber_vm_id, gpa, subscriber_gpa
                    )
                );
            }

            channel
                .backend_region()
                .ivc_map_linear(gpa, size, flags, mapper)?;

            Ok(())
        } else {
            ax_err!(
                NotFound,
                format!(
                    "IVC channel key [{:#x}] has no subscriber VM {}",
                    key, subscriber_vm_id
                )
            )
        }
    } else {
        Err(axerrno::ax_err_type!(
            NotFound,
            format!("IVC channel key [{:#x}] not found", key)
        ))
    }
}

/// Subcribe to a channel of the given key, this function will pass the subscriber VM ID and
/// the base address of the shared region in guest physical address of the subscriber VM.
/// If the channel does not exist, it will return an error.
/// If the channel exists, it will judge whether the given size is equal or smaller than the channel size,
/// if not, it will return an error.
/// If the channel is successfully subscribed, it will add the subscriber VM ID to the channel and
/// return the base address and size of the shared region in host physical address.
pub fn subscribe_to_channel(
    key: u32,
    subscriber_vm_id: usize,
    subscriber_gpa: GuestPhysAddr,
    subscriber_gpa_size: usize,
    flags: MappingFlags,
    mapper: impl FnMut(GuestPhysAddr, HostPhysAddr, usize, MappingFlags, bool) -> AxResult,
) -> AxResult {
    warn!(
        "Subscribing to IVC channel key {:#x} VM {}",
        key, subscriber_vm_id
    );

    let mut channels = IVC_CHANNELS.lock();
    if let Some(channel) = channels.get_mut(&key) {
        if channel.size() < subscriber_gpa_size {
            return ax_err!(
                InvalidInput,
                format!(
                    "IVC channel key [{:#x}] size {:#x} is smaller than subscriber size {:#x}",
                    key,
                    channel.size(),
                    subscriber_gpa_size
                )
            );
        }
        let actual_mapped_size = channel.size().min(subscriber_gpa_size);
        // Add the subscriber VM ID to the channel.
        channel.add_subscriber(subscriber_vm_id, subscriber_gpa, actual_mapped_size);

        channel.backend_region().ivc_map_linear(
            subscriber_gpa,
            actual_mapped_size,
            flags,
            mapper,
        )?;

        Ok(())
    } else {
        Err(axerrno::ax_err_type!(
            NotFound,
            format!("IVC channel key [{:#x}] not found", key)
        ))
    }
}

/// Unsubscribe from a channel according to the publisher VM ID and key.
/// If the channel does not exist, it will return an error.
/// If the channel exists, it will remove the subscriber VM ID from the channel and return the base address and size of the shared region in guest physical address of the subscriber VM.
/// If the channel has no subscribers, it will remove the channel from the global map.
#[allow(unused)]
pub fn unsubscribe_from_channel(key: u32, vm_id: usize) -> AxResult<(GuestPhysAddr, usize)> {
    let mut channels = IVC_CHANNELS.lock();
    if let Some(channel) = channels.get_mut(&key) {
        if let Some((base_gpa, size)) = channel.remove_subscriber(vm_id) {
            // If the channel has no subscribers, remove it from the global map.
            if channel.subscribers().is_empty() {
                channels.remove(&key);
            }
            Ok((base_gpa, size))
        } else {
            ax_err!(
                NotFound,
                format!("IVC channel key {:#x} for VM {} not found", key, vm_id)
            )
        }
    } else {
        ax_err!(NotFound, format!("IVC channel key {:#x} not found", key))
    }
}

pub enum IVCRegionType<H: PagingHandler> {
    /// IVC type inherited from host shared memory region.
    Shm {
        _shmkey: usize,
        host_gva_range: GuestVirtAddrRange,
        size: usize,
        nested_mappings: BTreeMap<GuestVirtAddr, GuestNestedMapping>,
    },
    /// IVC type for guest shared memory region.
    IVC { region: HostPhysicalRegion<H> },
}

impl<H: PagingHandler> IVCRegionType<H> {
    pub fn size(&self) -> usize {
        match self {
            IVCRegionType::Shm { size, .. } => *size,
            IVCRegionType::IVC { region } => region.size(),
        }
    }

    pub fn ivc_map_linear(
        &self,
        base_gpa: GuestPhysAddr,
        total_size: usize,
        flags: MappingFlags,
        mut ept_mapper: impl FnMut(GuestPhysAddr, HostPhysAddr, usize, MappingFlags, bool) -> AxResult,
    ) -> AxResult {
        match self {
            IVCRegionType::Shm {
                host_gva_range,
                size,
                nested_mappings,
                ..
            } => {
                if total_size != *size {
                    return ax_err!(
                        InvalidInput,
                        format!(
                            "IVC SHM size {:#x} does not match total size {:#x}",
                            size, total_size
                        )
                    );
                }

                let mut mapped_size = 0;
                for (gva, npt_mapping) in nested_mappings {
                    if gva != &npt_mapping.gva {
                        return ax_err!(
                            InvalidInput,
                            format!(
                                "IVC SHM nested mapping GVA {:?} does not match key {:?}",
                                npt_mapping.gva, gva
                            )
                        );
                    }

                    let gpa_offset = gva.as_usize() - host_gva_range.start.as_usize();

                    ept_mapper(
                        base_gpa.add(gpa_offset),
                        npt_mapping.hpa,
                        npt_mapping.gpgsize as usize,
                        flags,
                        true,
                    )?;

                    mapped_size += npt_mapping.gpgsize as usize;
                }

                if mapped_size != total_size {
                    error!(
                        "IVC SHM mapped size {:#x} does not match total size {:#x}",
                        mapped_size, total_size
                    );
                    return ax_err!(InvalidInput, "mapped_size mismatched in ivc_map_linear");
                }

                Ok(())
            }
            IVCRegionType::IVC { region } => {
                ept_mapper(base_gpa, region.base(), region.size(), flags, true)
            }
        }
    }
}

pub struct IVCChannel<H: PagingHandler> {
    key: u32,
    /// A list of subscriber VM IDs that are subscribed to this channel.
    /// The key is the subscriber VM ID, and the value is the base address of the shared region in
    /// guest physical address of the subscriber VM.
    subscriber_vms: BTreeMap<usize, (GuestPhysAddr, usize)>,
    region: IVCRegionType<H>,
    // region: HostPhysicalRegion<H>,
}

impl<H: PagingHandler> core::fmt::Debug for IVCChannel<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "IVCChannel[{:#x}], type {} size: {:#x}, subscribers {:x?}",
            self.key,
            match &self.region {
                IVCRegionType::Shm { .. } => "\"SYSV-SHM\"",
                IVCRegionType::IVC { .. } => "\"AXIVC\"",
            },
            self.region.size(),
            self.subscriber_vms
        )
    }
}

impl<H: PagingHandler> Drop for IVCChannel<H> {
    fn drop(&mut self) {
        // Free the shared region frame when the channel is dropped.
        debug!("Dropping IVCChannel {:#x}", self.key);
        match self.region {
            IVCRegionType::Shm { .. } => {}
            IVCRegionType::IVC { region: _ } => {
                // Deallocate the host physical region.
            }
        }
    }
}

impl<H: PagingHandler> IVCChannel<H> {
    #[allow(unused)]
    pub fn allocate(key: u32, size: usize) -> AxResult<Self> {
        let size = align_up_4k(size);
        let region = HostPhysicalRegion::allocate(size, Some(PAGE_SIZE_4K))?;
        Ok(Self {
            key,
            subscriber_vms: BTreeMap::new(),
            region: IVCRegionType::IVC { region },
        })
    }

    pub fn construct_from_shm(
        key: u32,
        host_gva_range: GuestVirtAddrRange,
        size: usize,
        nested_mappings: BTreeMap<GuestVirtAddr, GuestNestedMapping>,
    ) -> AxResult<Self> {
        Ok(Self {
            key,
            subscriber_vms: BTreeMap::new(),
            region: IVCRegionType::Shm {
                _shmkey: key as usize,
                host_gva_range,
                size,
                nested_mappings,
            },
        })
    }

    pub fn backend_region(&self) -> &IVCRegionType<H> {
        &self.region
    }

    pub fn size(&self) -> usize {
        self.region.size()
    }

    pub fn add_subscriber(
        &mut self,
        subscriber_vm_id: usize,
        subscriber_gpa: GuestPhysAddr,
        size: usize,
    ) {
        if !self.subscriber_vms.contains_key(&subscriber_vm_id) {
            self.subscriber_vms
                .insert(subscriber_vm_id, (subscriber_gpa, size));
        }
    }

    pub fn remove_subscriber(&mut self, subscriber_vm_id: usize) -> Option<(GuestPhysAddr, usize)> {
        self.subscriber_vms.remove(&subscriber_vm_id)
    }

    pub fn subscribers(&self) -> Vec<(usize, GuestPhysAddr, usize)> {
        self.subscriber_vms
            .iter()
            .map(|(vm_id, (gpa, size))| (*vm_id, *gpa, *size))
            .collect()
    }
}
