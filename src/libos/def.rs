use alloc::string::ToString;
use core::ffi::CStr;

use axaddrspace::{GuestVirtAddr, MappingFlags};

use axhal::mem::phys_to_virt;
use std::os::arceos::modules::axhal;

use crate::vmm::{VCpuRef, VMRef};

/// The structure of the memory region.
#[repr(C, packed)]
struct CMemoryRegion {
    start: u64,
    end: u64,
    permissions: [i8; 5],
    offset: u64,
    device: [i8; 6],
    inode: u64,
    pathname: [i8; 256],
    flags: u64,
}

pub fn process_libos_memory_regions(
    total_count: usize,
    pages_start_gva: usize,
    pages_count: usize,
    vcpu: &VCpuRef,
    vm: &VMRef,
) {
    let mut page_index = 0;
    let mut remaining = total_count;

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

            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }

        page_index += 1;
    }
}
