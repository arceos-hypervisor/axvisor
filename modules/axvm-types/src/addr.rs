use memory_addr::{AddrRange, PhysAddr, VirtAddr, def_usize_addr, def_usize_addr_formatter};

/// Host virtual address.
pub type HostVirtAddr = VirtAddr;
/// Host physical address.
pub type HostPhysAddr = PhysAddr;

def_usize_addr! {
    /// Guest virtual address.
    pub type GuestVirtAddr;
    /// Guest physical address.
    pub type GuestPhysAddr;
}

def_usize_addr_formatter! {
    GuestVirtAddr = "GVA:{}";
    GuestPhysAddr = "GPA:{}";
}

/// Guest virtual address range.
pub type GuestVirtAddrRange = AddrRange<GuestVirtAddr>;
/// Guest physical address range.
pub type GuestPhysAddrRange = AddrRange<GuestPhysAddr>;
