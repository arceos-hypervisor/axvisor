use axhal::mem::phys_to_virt;
use std::{os::arceos::modules::axhal, vec::Vec};

use axerrno::{AxResult, ax_err, ax_err_type};
use memory_addr::{MemoryAddr, PAGE_SIZE_4K};

use axaddrspace::npt::EPTPointer;
use axaddrspace::{GuestPhysAddr, GuestVirtAddr, HostVirtAddr};
use equation_defs::*;

use crate::vmm::{VCpuRef, VMRef};

pub use equation_defs::gate::region::{EPTP_LIST_REGION_SIZE, PERCPU_REGION_SIZE, PerCPURegion};
pub use equation_defs::{INSTANCE_REGION_SIZE, PROCESS_INNER_REGION_SIZE, ProcessInnerRegion};

pub const GUEST_MEM_REGION_BASE_GPA: GuestPhysAddr =
    GuestPhysAddr::from_usize(GUEST_MEM_REGION_BASE_PA);
pub const SHIM_BASE_GPA: GuestPhysAddr = GuestPhysAddr::from_usize(SHIM_BASE_PA);
pub const GUEST_PT_ROOT_GPA: GuestPhysAddr = GuestPhysAddr::from_usize(GUEST_PT_ROOT_PA);

pub const INSTANCE_REGION_BASE_GPA: GuestPhysAddr =
    GuestPhysAddr::from_usize(INSTANCE_REGION_BASE_PA);
pub const PROCESS_INNER_REGION_BASE_GPA: GuestPhysAddr =
    GuestPhysAddr::from_usize(PROCESS_INNER_REGION_BASE_PA);
pub const PERCPU_EPTP_LIST_REGION_GPA: GuestPhysAddr =
    GuestPhysAddr::from_usize(PERCPU_EPTP_LIST_REGION_PA);

pub const GP_ALL_EPTP_LIST_REGIN_GPA: GuestPhysAddr =
    GuestPhysAddr::from_usize(GP_ALL_EPTP_LIST_REGION_PA);
pub const GP_ALL_INSTANCE_PERCPU_REGION_GPA: GuestPhysAddr =
    GuestPhysAddr::from_usize(GP_ALL_INSTANCE_PERCPU_REGION_PA);

pub const PERCPU_REGION_BASE_GPA: GuestPhysAddr = GuestPhysAddr::from_usize(PERCPU_REGION_BASE_PA);

pub const GUEST_MEMORY_REGION_BASE_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(GUEST_MEMORY_REGION_BASE_VA);

pub const GUEST_PT_BASE_GVA: GuestVirtAddr = GuestVirtAddr::from_usize(GUEST_PT_BASE_VA as usize);
pub const PROCESS_INNER_REGION_BASE_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(PROCESS_INNER_REGION_BASE_VA as usize);
pub const INSTANCE_REGION_BASE_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(INSTANCE_REGION_BASE_VA as usize);
pub const PERCPU_REGION_BASE_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(PERCPU_REGION_BASE_VA as usize);

pub const GP_ALL_EPTP_LIST_REGION_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(GP_ALL_EPTP_LIST_REGION_VA as usize);
pub const GP_PERCPU_EPT_LIST_REGION_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(GP_PERCPU_EPTP_LIST_REGION_VA as usize);
pub const GP_ALL_INSTANCE_PERCPU_REGION_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(GP_ALL_INSTANCE_PERCPU_REGION_VA as usize);

/// Guest Process stack size.
pub const USER_STACK_SIZE: usize = 4096 * 4; // 16K
/// Guest Process stack base address.
pub const USER_STACK_BASE: GuestVirtAddr = GuestVirtAddr::from_usize(0x400_000 - USER_STACK_SIZE);

pub const EPTP_LIST_LENGTH: usize = 512;

/// The EPTP list structure,
/// which size is strictly 4K.
pub struct EPTPList {
    eptp_list: [EPTPointer; EPTP_LIST_LENGTH],
}

static_assertions::const_assert_eq!(core::mem::size_of::<EPTPList>(), PAGE_SIZE_4K,);

impl EPTPList {
    /// Construct EPTP list from the given EPTP region in `HostVirtAddr`.
    /// The address must be aligned to 4K.
    /// The caller must ensure that the address is valid.
    pub fn construct<'a>(eptp_list_base: HostVirtAddr) -> Option<&'a mut Self> {
        assert!(eptp_list_base.is_aligned(PAGE_SIZE_4K));

        unsafe { eptp_list_base.as_mut_ptr_of::<Self>().as_mut() }
    }
}

impl EPTPList {
    pub fn dump(&self) {
        info!("Dumping EPTP list @ {:?}", self as *const _);
        for (i, eptp) in self.eptp_list.iter().enumerate() {
            if eptp.bits() != 0 {
                info!("EPTP[{}]: {:x?}", i, eptp);
            }
        }
    }

    #[allow(unused)]
    pub unsafe fn dump_region(region: HostVirtAddr) {
        assert!(region.is_aligned(PAGE_SIZE_4K));

        let eptp_list = Self::construct(region).expect("Failed to construct EPTP list");
        eptp_list.dump();
    }

    /// Copy the EPTP list into the given target region.
    /// The target region must be aligned to 4K.
    /// The caller must ensure that the target region is valid.
    ///
    /// The first entry (gate EPTP) WILL BE ingnored during the copy,
    /// because it is reserved for the gate process.
    pub unsafe fn copy_into_region(&self, target_region: HostVirtAddr) {
        assert!(target_region.is_aligned(PAGE_SIZE_4K));

        let eptp_list_ptr = target_region.as_mut_ptr_of::<Self>();
        unsafe {
            core::ptr::copy_nonoverlapping(
                self as *const _ as *const u8,
                eptp_list_ptr as *mut u8,
                core::mem::size_of::<EPTPList>(),
            );
        }
    }

    /// Get EPTP entry by the given index.
    /// If the entry is not set, return None.
    #[allow(unused)]
    pub fn get(&self, index: usize) -> Option<EPTPointer> {
        assert!(index < EPTP_LIST_LENGTH);

        let eptp = self.eptp_list[index];

        if eptp.bits() == 0 { None } else { Some(eptp) }
    }

    /// Set the gate EPTP entry.
    /// If the entry is already set, return false.
    ///
    /// Return true if the gate entry is initialized successfully.
    pub fn set_gate_entry(&mut self, eptp: EPTPointer) -> bool {
        if self.eptp_list[0].bits() != 0 {
            error!("Cannot set gate EPTP[0], it has already been set");
            return false;
        }

        self.eptp_list[0] = eptp;

        true
    }

    /// Set EPTP entry by the given index.
    /// If the entry index is 0, return false, because it is reserved for the gate.
    /// If the entry is already set, return false.
    /// Return true if the entry is updated successfully.
    pub fn set(&mut self, index: usize, eptp: EPTPointer) -> bool {
        assert!(index < EPTP_LIST_LENGTH);

        if index == 0 {
            error!("Cannot set EPTP[0], it is reserved for the gate process");
            return false;
        }

        let old_eptp = self.eptp_list[index];
        if old_eptp.bits() != 0 {
            false
        } else {
            self.eptp_list[index] = eptp;
            true
        }
    }

    /// Clear EPTP entry by the given index.
    /// If the entry is already cleared, return None.
    /// Return the removed EPTP entry in `HostPhysAddr` if it is cleared successfully.
    pub fn remove(&mut self, index: usize) -> Option<EPTPointer> {
        assert!(index < EPTP_LIST_LENGTH);

        if index == 0 {
            error!("Cannot remove EPTP[0], it is reserved for the gate process");
            return None;
        }

        let old_eptp = self.eptp_list[index];

        if old_eptp.bits() == 0 {
            return None;
        } else {
            let removed_eptp = self.eptp_list[index];
            self.eptp_list[index] = EPTPointer::empty();
            Some(removed_eptp)
        }
    }
}

pub fn get_instance_file_from_shared_pages(
    file_size: usize,
    pages_start_gva: usize,
    pages_count: usize,
    vcpu: &VCpuRef,
    vm: &VMRef,
) -> AxResult<Vec<u8>> {
    let pages_base_gpa = vcpu
        .get_arch_vcpu()
        .guest_page_table_query(GuestVirtAddr::from_usize(pages_start_gva))
        .map_err(|paging_err| {
            error!(
                "Failed to query guest page table: {:?}, gva {:#x}",
                paging_err, pages_start_gva
            );
            ax_err_type!(BadAddress, "GVA Not mapped to GPA")
        })?
        .0;

    let pages_base_hva = phys_to_virt(
        vm.guest_phys_to_host_phys(pages_base_gpa)
            .map(|(hpa, _flags, _pgsize)| hpa)
            .ok_or_else(|| {
                error!("GPA {:#x} is not mapped to HPA", pages_base_gpa);
                ax_err_type!(BadAddress, "GPA Not mapped to HPA")
            })?,
    );

    let pages_ptr = pages_base_hva.as_ptr() as *const usize;
    let pages_slice: &[usize] = unsafe { core::slice::from_raw_parts(pages_ptr, pages_count) };

    let mut instance_file: Vec<u8> = Vec::with_capacity(file_size);
    let mut page_index = 0;
    let mut remaining = file_size;

    while remaining > 0 && page_index < pages_count {
        let page_base_gva = pages_slice[page_index];
        let (page_base_gpa, _flags, page_size) = vcpu
            .get_arch_vcpu()
            .guest_page_table_query(GuestVirtAddr::from_usize(page_base_gva))
            .map_err(|paging_err| {
                error!(
                    "Failed to query guest page table: {:?}, gva {:#x}",
                    paging_err, page_base_gva
                );
                ax_err_type!(BadAddress, "GVA Not mapped to GPA")
            })?;

        let page_base_hva = phys_to_virt(
            vm.guest_phys_to_host_phys(page_base_gpa)
                .map(|(hpa, _flags, _pgsize)| hpa)
                .ok_or_else(|| {
                    error!("GPA {:#x} is not mapped to HPA", page_base_gpa);
                    ax_err_type!(BadAddress, "GPA Not mapped to HPA")
                })?,
        );

        let coping_size = usize::min(page_size.into(), remaining);

        instance_file.extend_from_slice(unsafe {
            core::slice::from_raw_parts(page_base_hva.as_ptr() as *const u8, coping_size)
        });

        page_index += 1;
        remaining -= coping_size;
    }

    if instance_file.len() != file_size {
        error!(
            "Failed to copy instance file, expected size: {} Bytes, actual copied size: {} Bytes",
            file_size,
            instance_file.len()
        );
        return ax_err!(InvalidInput);
    }

    Ok(instance_file)
}
