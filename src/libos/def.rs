use axerrno::{AxResult, ax_err_type};

use memory_addr::{MemoryAddr, PAGE_SIZE_4K};
use page_table_multiarch::PageSize;

use axhal::mem::phys_to_virt;
use std::{os::arceos::modules::axhal, vec::Vec};

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, HostPhysAddr, MappingFlags};

use crate::vmm::{VCpuRef, VMRef};

/* Guest Process Virtual Address Space Layout (in GVA).*/

/// Guest Process stack size.
pub const USER_STACK_SIZE: usize = 4096 * 4; // 16K
/// Guest Process stack base address.
pub const USER_STACK_BASE: GuestVirtAddr = GuestVirtAddr::from_usize(0x400_000 - USER_STACK_SIZE);
pub const INSTANCE_SHREGION_BASE_GVA: GuestVirtAddr =
    GuestVirtAddr::from_usize(0xffff_ff00_0000_0000);
pub const GP_EPT_LIST_REGION_GVA: GuestVirtAddr = GuestVirtAddr::from_usize(0xffff_ff00_0000_1000);

/*  Guest Process Physical Address Space Layout (in GPA).*/

/// Guest Process shared region base address (first page) in first segmentation mapping region.
pub const INSTANCE_SHARED_REGION_BASE: GuestPhysAddr = GuestPhysAddr::from_usize(0xf000_0000);
/// Guest Process's GPA view of the EPTP list region on current CPU, only mapped in gate processes.
pub const GP_EPTP_LIST_REGION_BASE: GuestPhysAddr = GuestPhysAddr::from_usize(0xf000_1000);

/// (Only used for coarse-grained segmentation mapping)
///
/// Guest Process first region base address.
pub const GUEST_MEM_REGION_BASE: GuestPhysAddr = GuestPhysAddr::from_usize(0);
/// (Only used for one2one mapping)
///
/// The base guest physical address for the guest page table.
/// GPT_ROOT will be set to the last page in the first region in coarse-grained segmentation mapping.
pub const GPT_ROOT_GPA: GuestPhysAddr = GuestPhysAddr::from_usize(0xc000_0000);

/// The structure of the memory region.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default)]
pub struct InstanceSharedRegion {
    pub instance_id: u64,
    pub process_id: u64,
}

/// The structure of the memory region.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct ELFMemoryRegion {
    /// Start address of the memory region (8 bytes).
    start: u64,
    /// End address of the memory region (8 bytes).
    end: u64,
    /// Flags associated with the memory region (8 bytes).
    flags: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessMemoryRegionMapping {
    pub gpa: GuestPhysAddr,
    pub page_size: PageSize,
    pub hpa: Option<HostPhysAddr>,
}

#[derive(Debug)]
pub struct ProcessMemoryRegion {
    pub gva: GuestVirtAddr,
    pub size: usize,
    pub flags: MappingFlags,
    // GVA to GPA mapping: may not be established by host Linux yet
    //      maybe we need to inject a page fault into Linux?
    // GPA to HPA mapping: may not be established by hypervisor yet.
    pub mappings: Vec<(GuestVirtAddr, Option<ProcessMemoryRegionMapping>)>,
}

pub fn process_elf_memory_regions(
    total_count: usize,
    pages_start_gva: usize,
    pages_count: usize,
    vcpu: &VCpuRef,
    vm: &VMRef,
) -> AxResult<Vec<ProcessMemoryRegion>> {
    let mut page_index = 0;
    let mut remaining = total_count;

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

    let pages_base_hva =
        phys_to_virt(vm.guest_phys_to_host_phys(pages_base_gpa).ok_or_else(|| {
            error!("GPA {:#x} is not mapped to HPA", pages_base_gpa);
            ax_err_type!(BadAddress, "GPA Not mapped to HPA")
        })?);
    let pages_ptr = pages_base_hva.as_ptr() as *const usize;
    let pages_slice: &[usize] = unsafe { core::slice::from_raw_parts(pages_ptr, pages_count) };

    let mut process_regions: Vec<ProcessMemoryRegion> = Vec::new();

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

        let page_base_hva =
            phys_to_virt(vm.guest_phys_to_host_phys(page_base_gpa).ok_or_else(|| {
                error!("GPA {:#x} is not mapped to HPA", page_base_gpa);
                ax_err_type!(BadAddress, "GPA Not mapped to HPA")
            })?);
        let region_ptr = page_base_hva.as_ptr() as *const ELFMemoryRegion;

        let max_memory_region_in_page =
            page_size as usize / core::mem::size_of::<ELFMemoryRegion>();

        let regions_in_page = if remaining > max_memory_region_in_page {
            max_memory_region_in_page
        } else {
            remaining
        };

        let region_slice =
            unsafe { core::slice::from_raw_parts(region_ptr, regions_in_page as usize) };

        for region in region_slice {
            let mapping_flags =
                MappingFlags::from_bits(region.flags as usize).ok_or_else(|| {
                    let flags = region.flags;
                    error!("Invalid mapping flags: {:#x}", flags);
                    ax_err_type!(InvalidInput, "Invalid mapping flags")
                })?;
            let region_start = GuestVirtAddr::from_usize(region.start as usize);
            let region_end = GuestVirtAddr::from_usize(region.end as usize);

            let mut mappings = Vec::new();

            let mut start = region_start;

            while start < region_end {
                match vcpu.get_arch_vcpu().guest_page_table_query(start) {
                    Ok((gpa, flags, page_size)) => {
                        if !start.is_aligned(page_size as usize) {
                            warn!(
                                "Process memory gva {:?} is {:?} mapped but not aligned to page size {:?}",
                                start, page_size, page_size
                            );
                        }

                        if flags != mapping_flags {
                            warn!(
                                "Process memory gva {:?} is mapped with flags {:?} but expected {:?}",
                                start, flags, mapping_flags
                            );
                        }

                        mappings.push((
                            start,
                            Some(ProcessMemoryRegionMapping {
                                gpa,
                                page_size,
                                hpa: vm.guest_phys_to_host_phys(gpa),
                            }),
                        ));
                        start = start.add(page_size as usize);
                    }
                    Err(_) => {
                        mappings.push((start, None));
                        start = start.add(PAGE_SIZE_4K);
                    }
                }
            }

            process_regions.push(ProcessMemoryRegion {
                gva: region_start,
                size: region_end.sub_addr(region_start),
                flags: mapping_flags,
                mappings,
            });

            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }

        page_index += 1;
    }
    Ok(process_regions)
}
