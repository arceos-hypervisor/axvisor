use core::fmt::{self, Debug};

use page_table_entry::x86_64::PTF;
use page_table_entry::MappingFlags;
use memory_addr::MemoryAddr;

use axaddrspace::GuestPhysAddr;

pub const ENTRY_COUNT: usize = 512;

pub(crate) const fn p5_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 36)) & (ENTRY_COUNT - 1)
}

pub(crate) const fn p4_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 27)) & (ENTRY_COUNT - 1)
}

pub(crate) const fn p3_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 18)) & (ENTRY_COUNT - 1)
}

pub(crate) const fn p2_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 9)) & (ENTRY_COUNT - 1)
}

pub(crate) const fn p1_index(vaddr: usize) -> usize {
    (vaddr >> 12) & (ENTRY_COUNT - 1)
}

/// A more generic page table entry.、
///
/// All architecture-specific page table entry types implement this trait.
pub trait MoreGenericPTE: Debug + Clone + Copy + Sync + Send + Sized {
    /// The physical address type of this entry.
    type PhysAddr: MemoryAddr;

    /// Creates a page table entry point to a terminate page or block.
    fn new_page(paddr: Self::PhysAddr, flags: MappingFlags, is_huge: bool) -> Self;
    /// Creates a page table entry point to a next level page table.
    fn new_table(paddr: Self::PhysAddr) -> Self;

    /// Returns the physical address mapped by this entry.
    fn paddr(&self) -> Self::PhysAddr;
    /// Returns the flags of this entry.
    fn flags(&self) -> MappingFlags;

    /// Set mapped physical address of the entry.
    fn set_paddr(&mut self, paddr: Self::PhysAddr);
    /// Set flags of the entry.
    fn set_flags(&mut self, flags: MappingFlags, is_huge: bool);

    /// Returns the raw bits of this entry.
    fn bits(self) -> usize;
    /// Returns whether this entry is zero.
    fn is_unused(&self) -> bool;
    /// Returns whether this entry flag indicates present.
    fn is_present(&self) -> bool;
    /// For non-last level translation, returns whether this entry maps to a
    /// huge frame.
    fn is_huge(&self) -> bool;
    /// Set this entry to zero.
    fn clear(&mut self);
}

/// An x86_64 guest page table entry.
/// Note: The [GuestEntry] can be moved to the independent crate `page_table_entry`.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct GuestEntry(u64);

impl GuestEntry {
    const PHYS_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000; // bits 12..52
}

impl MoreGenericPTE for GuestEntry {
    type PhysAddr = GuestPhysAddr;

    fn new_page(paddr: Self::PhysAddr, flags: MappingFlags, is_huge: bool) -> Self {
        let mut flags = PTF::from(flags);
        if is_huge {
            flags |= PTF::HUGE_PAGE;
        }
        Self(flags.bits() | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK))
    }
    fn new_table(paddr: Self::PhysAddr) -> Self {
        let flags = PTF::PRESENT | PTF::WRITABLE | PTF::USER_ACCESSIBLE;
        Self(flags.bits() | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK))
    }
    fn paddr(&self) -> Self::PhysAddr {
        Self::PhysAddr::from((self.0 & Self::PHYS_ADDR_MASK) as usize)
    }
    fn flags(&self) -> MappingFlags {
        PTF::from_bits_truncate(self.0).into()
    }
    fn set_paddr(&mut self, paddr: Self::PhysAddr) {
        self.0 = (self.0 & !Self::PHYS_ADDR_MASK) | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK)
    }
    fn set_flags(&mut self, flags: MappingFlags, is_huge: bool) {
        let mut flags = PTF::from(flags);
        if is_huge {
            flags |= PTF::HUGE_PAGE;
        }
        self.0 = (self.0 & Self::PHYS_ADDR_MASK) | flags.bits()
    }

    fn bits(self) -> usize {
        self.0 as usize
    }
    fn is_unused(&self) -> bool {
        self.0 == 0
    }
    fn is_present(&self) -> bool {
        PTF::from_bits_truncate(self.0).contains(PTF::PRESENT)
    }
    fn is_huge(&self) -> bool {
        PTF::from_bits_truncate(self.0).contains(PTF::HUGE_PAGE)
    }
    fn clear(&mut self) {
        self.0 = 0
    }
}

impl fmt::Debug for GuestEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("X64GuestEntry");
        f.field("raw", &self.0)
            .field("gpa", &self.paddr())
            .field("flags", &self.flags())
            .finish()
    }
}