//! Inter-VM communication (IVC) module.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::sync::Mutex;

use axaddrspace::{GuestPhysAddr, HostPhysAddr};
use axerrno::AxResult;
use page_table_multiarch::PagingHandler;

/// A global btree map to store IVC channels,
/// indexed by (publisher_vm_id, channel_key).
static IVC_CHANNELS: Mutex<BTreeMap<(usize, usize), IVCChannel<PagingHandlerImpl>>> =
    Mutex::new(BTreeMap::new());

pub fn insert_channel(
    publisher_vm_id: usize,
    channel: IVCChannel<PagingHandlerImpl>,
) -> AxResult<()> {
    let mut channels = IVC_CHANNELS.lock();
    if channels
        .insert((publisher_vm_id, channel.key), channel)
        .is_some()
    {
        Err(axerrno::ax_err_type!(
            AlreadyExists,
            "IVC channel already exists"
        ))
    } else {
        Ok(())
    }
}

pub fn remove_channel(
    publisher_vm_id: usize,
    key: usize,
) -> AxResult<IVCChannel<PagingHandlerImpl>> {
    IVC_CHANNELS
        .lock()
        .remove(&(publisher_vm_id, key))
        .ok_or_else(|| {
            axerrno::ax_err_type!(
                NotFound,
                format!(
                    "IVC channel for publisher VM {} with key {} not found",
                    publisher_vm_id, key
                )
            )
        })
}

/// Subcribe to a channel of a publisher VM with the given key,
/// return the shared region base address and size.
pub fn subscribe_to_channel_of_publisher<'a>(
    publisher_vm_id: usize,
    key: usize,
    subscriber_vm_id: usize,
    subscriber_gpa: GuestPhysAddr,
) -> AxResult<(HostPhysAddr, usize)> {
    let mut channels = IVC_CHANNELS.lock();
    if let Some(channel) = channels.get_mut(&(publisher_vm_id, key)) {
        // Add the subscriber VM ID to the channel.
        channel.add_subscriber(subscriber_vm_id, subscriber_gpa);
        Ok((channel.base_hpa(), channel.size()))
    } else {
        Err(axerrno::ax_err_type!(
            NotFound,
            format!(
                "IVC channel for publisher VM [{}] key {:#x} not found",
                publisher_vm_id, key
            )
        ))
    }
}

/// Unsubscribe from a channel of a publisher VM with the given key,
/// return the shared region base address and size.
pub fn unsubscribe_from_channel_of_publisher(
    publisher_vm_id: usize,
    key: usize,
    subscriber_vm_id: usize,
) -> AxResult<(GuestPhysAddr, usize)> {
    let mut channels = IVC_CHANNELS.lock();
    if let Some(channel) = channels.get_mut(&(publisher_vm_id, key)) {
        // Remove the subscriber VM ID from the channel.
        if let Some(subscriber_gpa) = channel.remove_subscriber(subscriber_vm_id) {
            Ok((subscriber_gpa, channel.size()))
        } else {
            Err(axerrno::ax_err_type!(
                NotFound,
                format!(
                    "VM[{}] tries to subcriber non-existed channel publisher VM[{}] Key {:#x}",
                    subscriber_vm_id, publisher_vm_id, key
                )
            ))
        }
    } else {
        Err(axerrno::ax_err_type!(
            NotFound,
            format!("IVC channel for publisher VM {} not found", publisher_vm_id)
        ))
    }
}

pub struct IVCChannel<H: PagingHandler> {
    publisher_vm_id: usize,
    key: usize,
    /// A list of subscriber VM IDs that are subscribed to this channel.
    /// The key is the subscriber VM ID, and the value is the base address of the shared region in
    /// guest physical address of the subscriber VM.
    subscriber_vms: BTreeMap<usize, GuestPhysAddr>,
    shared_region_base: HostPhysAddr,
    shared_region_size: usize,
    /// The base address of the shared memory region in guest physical address of the publisher VM.
    base_gpa: GuestPhysAddr,
    _phatom: core::marker::PhantomData<H>,
}

#[repr(C)]
pub struct IVCChannelHeader {
    pub publisher_id: u64,
    pub key: u64,
    pub content_size: u64,
}

impl<H: PagingHandler> IVCChannel<H> {
    #[allow(unused)]
    pub fn header(&self) -> &IVCChannelHeader {
        unsafe {
            // Map the shared region base to the header structure.
            &*H::phys_to_virt(self.shared_region_base).as_mut_ptr_of::<IVCChannelHeader>()
        }
    }

    pub fn header_mut(&mut self) -> &mut IVCChannelHeader {
        unsafe {
            // Map the shared region base to the mutable header structure.
            &mut *H::phys_to_virt(self.shared_region_base).as_mut_ptr_of::<IVCChannelHeader>()
        }
    }

    #[allow(unused)]
    pub fn data_region(&self) -> *const u8 {
        unsafe {
            // Return a pointer to the data region, which starts after the header.
            H::phys_to_virt(self.shared_region_base)
                .as_mut_ptr()
                .add(core::mem::size_of::<IVCChannelHeader>())
        }
    }
}

impl<H: PagingHandler> core::fmt::Debug for IVCChannel<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "IVCChannel(publisher[{}], subscribers {:?}, base: {:?}, size: {:#x})",
            self.publisher_vm_id,
            self.subscriber_vms,
            self.shared_region_base,
            self.shared_region_size
        )
    }
}

impl<H: PagingHandler> Drop for IVCChannel<H> {
    fn drop(&mut self) {
        // Free the shared region frame when the channel is dropped.
        debug!(
            "Dropping IVCChannel for VM[{}], shared region base: {:?}",
            self.publisher_vm_id, self.shared_region_base
        );
        H::dealloc_frame(self.shared_region_base);
    }
}

impl<H: PagingHandler> IVCChannel<H> {
    pub fn alloc(
        publisher_vm_id: usize,
        key: usize,
        shared_region_size: usize,
        base_gpa: GuestPhysAddr,
    ) -> AxResult<Self> {
        // TODO: support larger shared region sizes with alloc_frames API.
        let shared_region_size = shared_region_size.min(4096);
        let shared_region_base = H::alloc_frame().ok_or_else(|| {
            axerrno::ax_err_type!(NoMemory, "Failed to allocate shared region frame")
        })?;

        let mut channel = IVCChannel {
            publisher_vm_id,
            key,
            subscriber_vms: BTreeMap::new(),
            shared_region_base,
            shared_region_size,
            base_gpa,
            _phatom: core::marker::PhantomData,
        };

        channel.header_mut().publisher_id = publisher_vm_id as u64;
        channel.header_mut().key = key as u64;
        channel.header_mut().content_size = 0;

        debug!("Allocated IVCChannel: {:?}", channel);

        Ok(channel)
    }

    pub fn base_hpa(&self) -> HostPhysAddr {
        self.shared_region_base
    }

    pub fn base_gpa_in_publisher(&self) -> GuestPhysAddr {
        self.base_gpa
    }

    pub fn size(&self) -> usize {
        self.shared_region_size
    }

    pub fn add_subscriber(&mut self, subscriber_vm_id: usize, subscriber_gpa: GuestPhysAddr) {
        if !self.subscriber_vms.contains_key(&subscriber_vm_id) {
            self.subscriber_vms.insert(subscriber_vm_id, subscriber_gpa);
        }
    }

    pub fn remove_subscriber(&mut self, subscriber_vm_id: usize) -> Option<GuestPhysAddr> {
        self.subscriber_vms.remove(&subscriber_vm_id)
    }

    pub fn subscribers(&self) -> Vec<(usize, GuestPhysAddr)> {
        self.subscriber_vms
            .iter()
            .map(|(vm_id, gpa)| (*vm_id, *gpa))
            .collect()
    }
}
