use alloc::vec::Vec;
use core::fmt;
use core::fmt::Debug;

use axerrno::{AxError, AxResult, ax_err};
use memory_addr::{AddrRange, MemoryAddr, PAGE_SIZE_4K, PhysAddr, is_aligned_4k};
use page_table_entry::x86_64::PTF;
use page_table_multiarch::{
    GenericPTE, MappingFlags, PageSize, PageTable64, PagingError, PagingHandler, PagingMetaData,
    PagingResult,
};

use axaddrspace::{AddrSpace, GuestPhysAddr, GuestVirtAddr, HostPhysAddr};

use axaddrspace::npt::EPTMetadata;

const ENTRY_COUNT: usize = 512;

const fn p5_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 36)) & (ENTRY_COUNT - 1)
}

const fn p4_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 27)) & (ENTRY_COUNT - 1)
}

const fn p3_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 18)) & (ENTRY_COUNT - 1)
}

const fn p2_index(vaddr: usize) -> usize {
    (vaddr >> (12 + 9)) & (ENTRY_COUNT - 1)
}

const fn p1_index(vaddr: usize) -> usize {
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

/// The virtual memory address space.
pub struct GuestAddrSpace<
    M: PagingMetaData,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE,
    H: PagingHandler,
> {
    ept_addrspace: AddrSpace<M, EPTE, H>,
    /// Manage LibOS's memory addrspace at 2MB granularity.
    huge_page_num: usize,
    mem_pos: usize,  // Incremented from 0.
    page_pos: usize, // Decremented from 0x200.

    /// Guest Page Table
    root_paddr: GuestPhysAddr,
    levels: usize,

    phontom: core::marker::PhantomData<GPTE>,
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    pub fn clone(&self) -> AxResult<Self> {
        let cloned_aspace = Self::new()?;

        Ok(cloned_aspace)
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    /// Creates a new guest address space.
    pub fn new() -> AxResult<Self> {
        let mut ept_addrspace = AddrSpace::new_empty(
            GuestPhysAddr::from_usize(0),
            1 << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
        )?;

        Ok(Self {
            ept_addrspace,
            huge_page_num: 0,
            mem_pos: 0,
            page_pos: 0x200,
            root_paddr: GuestPhysAddr::from_usize(0),
            levels: 4,
            phontom: core::marker::PhantomData,
        })
    }

    pub fn page_table_root(&self) -> HostPhysAddr {
        self.ept_addrspace.page_table_root()
    }

    pub fn translate(&self, gpa: M::VirtAddr) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
        self.ept_addrspace.translate(gpa)
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    /// Get the root page table physical address.
    pub fn root_paddr(&self) -> GPTE::PhysAddr {
        self.root_paddr
    }

    /// Maps a virtual page to a physical frame with the given `page_size`
    /// and mapping `flags`.
    ///
    /// The virtual page starts with `vaddr`, amd the physical frame starts with
    /// `target`. If the addresses is not aligned to the page size, they will be
    /// aligned down automatically.
    ///
    /// Returns [`Err(PagingError::AlreadyMapped)`](PagingError::AlreadyMapped)
    /// if the mapping is already present.
    pub fn map(
        &mut self,
        vaddr: GuestVirtAddr,
        target: GuestPhysAddr,
        page_size: PageSize,
        flags: MappingFlags,
    ) -> PagingResult {
        let entry = self.get_entry_mut_or_create(vaddr, page_size)?;
        if !entry.is_unused() {
            return Err(PagingError::AlreadyMapped);
        }
        *entry = MoreGenericPTE::new_page(target.align_down(page_size), flags, page_size.is_huge());
        Ok(())
    }

    ///
    /// Returns the physical address of the target frame, mapping flags, and
    /// the page size.
    ///
    /// Returns [`Err(PagingError::NotMapped)`](PagingError::NotMapped) if the
    /// mapping is not present.
    pub fn query(
        &self,
        vaddr: GuestVirtAddr,
    ) -> PagingResult<(GuestPhysAddr, MappingFlags, PageSize)> {
        let (entry, size) = self.get_entry(vaddr)?;
        if entry.is_unused() {
            return Err(PagingError::NotMapped);
        }
        let off = size.align_offset(vaddr.into());
        Ok((entry.paddr().add(off).into(), entry.flags(), size))
    }

    /// Walk the page table recursively.
    ///
    /// When reaching a page table entry, call `pre_func` and `post_func` on the
    /// entry if they are provided. The max number of enumerations in one table
    /// is limited by `limit`. `pre_func` and `post_func` are called before and
    /// after recursively walking the page table.
    ///
    /// The arguments of `*_func` are:
    /// - Current level (starts with `0`): `usize`
    /// - The index of the entry in the current-level table: `usize`
    /// - The virtual address that is mapped to the entry: `M::VirtAddr`
    /// - The reference of the entry: [`&GPTE`](GenericPTE)
    pub fn walk<F>(&self, limit: usize, pre_func: Option<&F>, post_func: Option<&F>) -> PagingResult
    where
        F: Fn(usize, usize, GuestVirtAddr, &GPTE),
    {
        self.walk_recursive(
            self.table_of(self.root_paddr())?,
            0,
            0.into(),
            limit,
            pre_func,
            post_func,
        )
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    fn alloc_table() -> PagingResult<GPTE::PhysAddr> {
        // if let Some(paddr) = H::alloc_frame() {
        //     let ptr = H::phys_to_virt(paddr).as_mut_ptr();
        //     unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE_4K) };
        //     Ok(paddr)
        // } else {
        Err(PagingError::NoMemory)
        // }
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    fn table_of<'a>(&self, gpa: GuestPhysAddr) -> PagingResult<&'a [GPTE]> {
        let (hpa, _flags, _pgsize) = self.ept_addrspace.translate(gpa).ok_or_else(|| {
            warn!("Failed to translate GPA {:?}", gpa);
            PagingError::NotMapped
        })?;

        let ptr = H::phys_to_virt(hpa).as_ptr() as _;

        // debug!(
        //     "GuestPageTable64::table_of gpa: {:?} hpa: {:?} ptr: {:p}",
        //     gpa, hpa, ptr
        // );

        Ok(unsafe { core::slice::from_raw_parts(ptr, ENTRY_COUNT) })
    }

    fn table_of_mut<'a>(&mut self, gpa: GPTE::PhysAddr) -> PagingResult<&'a mut [GPTE]> {
        let (hpa, _flags, _pgsize) = self.ept_addrspace.translate(gpa).ok_or_else(|| {
            warn!("Failed to translate GPA {:?}", gpa);
            PagingError::NotMapped
        })?;

        let ptr = H::phys_to_virt(hpa).as_mut_ptr() as _;
        Ok(unsafe { core::slice::from_raw_parts_mut(ptr, ENTRY_COUNT) })
    }

    fn next_table<'a>(&self, entry: &GPTE) -> PagingResult<&'a [GPTE]> {
        if !entry.is_present() {
            Err(PagingError::NotMapped)
        } else if entry.is_huge() {
            Err(PagingError::MappedToHugePage)
        } else {
            self.table_of(entry.paddr())
        }
    }

    fn next_table_mut<'a>(&mut self, entry: &GPTE) -> PagingResult<&'a mut [GPTE]> {
        if entry.paddr().as_usize() == 0 {
            Err(PagingError::NotMapped)
        } else if entry.is_huge() {
            Err(PagingError::MappedToHugePage)
        } else {
            Ok(self.table_of_mut(entry.paddr())?)
        }
    }

    fn next_table_mut_or_create<'a>(&mut self, entry: &mut GPTE) -> PagingResult<&'a mut [GPTE]> {
        if entry.is_unused() {
            let paddr = Self::alloc_table()?;
            *entry = MoreGenericPTE::new_table(paddr);
            self.table_of_mut(paddr)
        } else {
            self.next_table_mut(entry)
        }
    }

    fn get_entry(&self, gva: GuestVirtAddr) -> PagingResult<(&GPTE, PageSize)> {
        let vaddr: usize = gva.into();

        let p3 = if self.levels == 3 {
            self.table_of(self.root_paddr())?
        } else if self.levels == 4 {
            let p4 = self.table_of(self.root_paddr())?;
            let p4e = &p4[p4_index(vaddr)];
            self.next_table(p4e)?
        } else {
            // 5-level paging
            let p5 = self.table_of(self.root_paddr())?;
            let p5e = &p5[p5_index(vaddr)];
            if p5e.is_huge() {
                return Err(PagingError::MappedToHugePage);
            }
            let p4 = self.next_table(p5e)?;
            let p4e = &p4[p4_index(vaddr)];

            if p4e.is_huge() {
                return Err(PagingError::MappedToHugePage);
            }

            self.next_table(p4e)?
        };

        let p3e = &p3[p3_index(vaddr)];
        if p3e.is_huge() {
            return Ok((p3e, PageSize::Size1G));
        }

        let p2 = self.next_table(p3e)?;
        let p2e = &p2[p2_index(vaddr)];
        if p2e.is_huge() {
            return Ok((p2e, PageSize::Size2M));
        }

        let p1 = self.next_table(p2e)?;
        let p1e = &p1[p1_index(vaddr)];
        Ok((p1e, PageSize::Size4K))
    }

    fn get_entry_mut_or_create(
        &mut self,
        vaddr: GuestVirtAddr,
        page_size: PageSize,
    ) -> PagingResult<&mut GPTE> {
        let vaddr: usize = vaddr.into();
        let p3 = if M::LEVELS == 3 {
            self.table_of_mut(self.root_paddr())?
        } else if M::LEVELS == 4 {
            let p4 = self.table_of_mut(self.root_paddr())?;
            let p4e = &mut p4[p4_index(vaddr)];
            self.next_table_mut_or_create(p4e)?
        } else {
            unreachable!()
        };
        let p3e = &mut p3[p3_index(vaddr)];
        if page_size == PageSize::Size1G {
            return Ok(p3e);
        }

        let p2 = self.next_table_mut_or_create(p3e)?;
        let p2e = &mut p2[p2_index(vaddr)];
        if page_size == PageSize::Size2M {
            return Ok(p2e);
        }

        let p1 = self.next_table_mut_or_create(p2e)?;
        let p1e = &mut p1[p1_index(vaddr)];
        Ok(p1e)
    }

    fn walk_recursive<F>(
        &self,
        table: &[GPTE],
        level: usize,
        start_vaddr: GuestVirtAddr,
        limit: usize,
        pre_func: Option<&F>,
        post_func: Option<&F>,
    ) -> PagingResult
    where
        F: Fn(usize, usize, GuestVirtAddr, &GPTE),
    {
        let start_vaddr_usize: usize = start_vaddr.into();
        let mut n = 0;
        for (i, entry) in table.iter().enumerate() {
            let vaddr_usize = start_vaddr_usize + (i << (12 + (self.levels - 1 - level) * 9));
            let vaddr = vaddr_usize.into();

            if entry.is_present() {
                if let Some(func) = pre_func {
                    func(level, i, vaddr, entry);
                }
                if level < self.levels - 1 && !entry.is_huge() {
                    let table_entry = self.next_table(entry)?;
                    self.walk_recursive(table_entry, level + 1, vaddr, limit, pre_func, post_func)?;
                }
                if let Some(func) = post_func {
                    func(level, i, vaddr, entry);
                }
                n += 1;
                if n >= limit {
                    break;
                }
            }
        }
        Ok(())
    }
}
