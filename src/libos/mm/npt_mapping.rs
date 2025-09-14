use axaddrspace::{GuestPhysAddr, GuestVirtAddr, HostPhysAddr};
use page_table_multiarch::PageSize;

/// Stores two-level address space mapping.
#[allow(unused)]
pub struct GuestNestedMapping {
    pub gva: GuestVirtAddr,
    pub gpa: GuestPhysAddr,
    pub gpgsize: PageSize,
    pub hpa: HostPhysAddr,
    pub hpgsize: PageSize,
}

impl GuestNestedMapping {
    pub fn new(
        gva: GuestVirtAddr,
        gpa: GuestPhysAddr,
        gpgsize: PageSize,
        hpa: HostPhysAddr,
        hpgsize: PageSize,
    ) -> Self {
        Self {
            gva,
            gpa,
            gpgsize,
            hpa,
            hpgsize,
        }
    }
}
