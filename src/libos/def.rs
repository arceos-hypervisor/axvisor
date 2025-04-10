use alloc::string::{String, ToString};
use core::ffi::CStr;

use memory_addr::{MemoryAddr, PAGE_SIZE_4K};
use page_table_multiarch::PageSize;

use axhal::mem::phys_to_virt;
use std::{os::arceos::modules::axhal, vec::Vec};

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, HostPhysAddr, MappingFlags};

use crate::vmm::{VCpuRef, VMRef};

/// Metadata of VMX shadow page tables.
pub struct ShadowPageTableMetadata;

impl page_table_multiarch::PagingMetaData for ShadowPageTableMetadata {
    const LEVELS: usize = 4;
    const PA_MAX_BITS: usize = 52;
    const VA_MAX_BITS: usize = 48;

    type VirtAddr = axaddrspace::GuestVirtAddr;

    fn flush_tlb(_vaddr: Option<GuestVirtAddr>) {
        todo!()
    }
}

/// The structure of the memory region.
#[repr(C, packed)]
struct CMemoryRegion {
    /// Start address of the memory region (8 bytes).
    start: u64,
    /// End address of the memory region (8 bytes).
    end: u64,
    /// Access permissions (e.g., read/write/execute) and flags (e.g., private/shared),
    /// stored as a fixed-size array of 8 bytes.
    permissions: [i8; 8],
    /// Offset in the mapped file (8 bytes).
    offset: u64,
    /// Device number (major:minor) for special files, stored as a fixed-size array of 8 bytes.
    device: [i8; 8],
    /// Inode number of the mapped file (8 bytes).
    inode: u64,
    /// Fixed-size buffer for the path: Mapped file path or region name (e.g., "[heap]"),
    /// stored as a 256-byte array.
    pathname: [i8; 256],
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
    pub offset: u64,
    pub device: [i8; 8],
    pub inode: u64,
    pub pathname: String,
}

pub fn process_libos_memory_regions(
    total_count: usize,
    pages_start_gva: usize,
    pages_count: usize,
    vcpu: &VCpuRef,
    vm: &VMRef,
) -> Vec<ProcessMemoryRegion> {
    let mut page_index = 0;
    let mut remaining = total_count;

    let mut process_regons: Vec<ProcessMemoryRegion> = Vec::new();

    let pages_base_gpa = vcpu
        .get_arch_vcpu()
        .guest_page_table_query(GuestVirtAddr::from_usize(pages_start_gva))
        .unwrap()
        .0;
    let pages_base_hva = phys_to_virt(vm.guest_phys_to_host_phys(pages_base_gpa).unwrap());
    let pages_ptr = pages_base_hva.as_ptr() as *const usize;
    let pages_slice: &[usize] = unsafe { core::slice::from_raw_parts(pages_ptr, pages_count) };

    while remaining > 0 && page_index < pages_count {
        let page_base_gva = pages_slice[page_index];
        let (page_base_gpa, _flags, page_size) = vcpu
            .get_arch_vcpu()
            .guest_page_table_query(GuestVirtAddr::from_usize(page_base_gva))
            .unwrap();
        let page_base_hva = phys_to_virt(vm.guest_phys_to_host_phys(page_base_gpa).unwrap());
        let region_ptr = page_base_hva.as_ptr() as *const CMemoryRegion;

        let max_memory_region_in_page = page_size as usize / core::mem::size_of::<CMemoryRegion>();

        let regions_in_page = if remaining > max_memory_region_in_page {
            max_memory_region_in_page
        } else {
            remaining
        };

        let region_slice =
            unsafe { core::slice::from_raw_parts(region_ptr, regions_in_page as usize) };

        for region in region_slice {
            // Convert C strings
            let perms = unsafe { CStr::from_ptr(region.permissions.as_ptr()).to_string_lossy() };
            let path = unsafe { CStr::from_ptr(region.pathname.as_ptr()) }
                .to_string_lossy()
                .clone();

            let path = if path.is_empty() {
                "[No Path]".to_string()
            } else {
                path.trim_ascii().trim_end_matches('\0').to_string()
            };

            let region_start = GuestVirtAddr::from_usize(region.start as usize);
            let region_end = GuestVirtAddr::from_usize(region.end as usize);
            let inode = region.inode;

            let mut mappings = Vec::new();

            let mut start = region_start;

            while start < region_end {
                match vcpu.get_arch_vcpu().guest_page_table_query(start) {
                    Ok((gpa, _flags, page_size)) => {
                        if !start.is_aligned(page_size as usize) {
                            warn!(
                                "Process memory gva {:?} is {:?} mapped but not aligned to page size {:?}",
                                start, page_size, page_size
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

            // Parse flags
            let flags = MappingFlags::from_bits_truncate(region.flags as usize);

            info!(
                "[{}] {:016x}-{:016x} {} {:?} {} \"{}\"",
                total_count - remaining,
                region_start,
                region_end,
                perms,
                flags,
                inode,
                path
            );

            process_regons.push(ProcessMemoryRegion {
                gva: region_start,
                mappings,
                size: region_end.sub_addr(region_start) as usize,
                flags,
                offset: region.offset,
                device: region.device,
                inode,
                pathname: path,
            });

            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }

        page_index += 1;
    }

    process_regons
}
