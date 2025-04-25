use alloc::sync::Arc;
use core::ops::AddAssign;
use std::collections::btree_map::BTreeMap;
use std::vec::Vec;

use axerrno::{AxError, AxResult, ax_err, ax_err_type};
use memory_addr::{
    AddrRange, MemoryAddr, PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, PageIter4K, is_aligned_4k,
};
use page_table_multiarch::{
    GenericPTE, MappingFlags, PageSize, PagingError, PagingHandler, PagingMetaData, PagingResult,
};

use axaddrspace::npt::EPTMetadata;
use axaddrspace::{AddrSpace, GuestPhysAddr, GuestVirtAddr, HostPhysAddr, HostVirtAddr};

use super::def::{GUEST_MEM_REGION_BASE, GUEST_PT_ROOT_GPA};
use super::gpt::{ENTRY_COUNT, MoreGenericPTE, p1_index, p2_index, p3_index, p4_index, p5_index};

// Copy from `axmm`.
fn paging_err_to_ax_err(err: PagingError) -> AxError {
    warn!("Paging error: {:?}", err);
    match err {
        PagingError::NoMemory => AxError::NoMemory,
        PagingError::NotAligned => AxError::InvalidInput,
        PagingError::NotMapped => AxError::NotFound,
        PagingError::AlreadyMapped => AxError::AlreadyExists,
        PagingError::MappedToHugePage => AxError::InvalidInput,
    }
}

#[allow(unused)]
#[repr(usize)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum GuestMappingType {
    One2OneMapping,
    /// Size of 2 megabytes (2<sup>21</sup> bytes).
    CoarseGrainedSegmentation2M = 0x20_0000,
    /// Size of 1 gigabytes (2<sup>30</sup> bytes).
    CoarseGrainedSegmentation1G = 0x4000_0000,
}

impl From<u64> for GuestMappingType {
    fn from(value: u64) -> Self {
        match value {
            0 => GuestMappingType::One2OneMapping,
            1 => GuestMappingType::CoarseGrainedSegmentation2M,
            2 => GuestMappingType::CoarseGrainedSegmentation1G,
            _ => {
                error!(
                    "Invalid guest mapping type: {}, downgrading to One2OneMapping",
                    value
                );
                GuestMappingType::One2OneMapping
            }
        }
    }
}

enum GuestMapping<H: PagingHandler> {
    /// One-to-one mapping.
    One2OneMapping {
        page_pos: usize, // Incremented from 0.
    },
    /// Coarse-grained segmentation (2M/1G).
    CoarseGrainedSegmentation {
        /// Manage LibOS's memory addrspace at 2MB/1GB granularity.
        mm_region_granularity: usize,
        /// Current normal memory region base address in GPA.
        mm_region_base: GuestPhysAddr,
        /// Memory page index incremented from 0.
        mm_page_idx: usize,
        /// Stores the host physical address of allocated regions for normal memory.
        mm_regions: BTreeMap<GuestPhysAddr, HostPhysicalRegionRef<H>>,
        /// Current page table region base address in GPA.
        pt_region_base: GuestPhysAddr,
        /// Page table page index incremented from 1 (the first is used for page table root).
        pt_page_idx: usize,
        /// Stores the host physical address of allocated regions for page table memory.
        pt_regions: Vec<HostPhysicalRegion<H>>,
    },
}

struct HostPhysicalRegion<H: PagingHandler> {
    base: HostPhysAddr,
    size: usize,
    phontom: core::marker::PhantomData<H>,
}

type HostPhysicalRegionRef<H> = Arc<HostPhysicalRegion<H>>;

impl<H: PagingHandler> HostPhysicalRegion<H> {
    fn allocate(granularity: usize) -> AxResult<Self> {
        let hpa = H::alloc_frames(granularity / PAGE_SIZE_4K, granularity).ok_or_else(|| {
            ax_err_type!(NoMemory, "Failed to allocate memory for HostPhysicalRegion")
        })?;

        // Clear the memory region.
        unsafe {
            core::ptr::write_bytes(H::phys_to_virt(hpa).as_mut_ptr(), 0, granularity);
        }

        Ok(Self {
            base: hpa,
            size: granularity,
            phontom: core::marker::PhantomData,
        })
    }

    fn allocate_ref(granularity: usize) -> AxResult<HostPhysicalRegionRef<H>> {
        Ok(Arc::new(Self::allocate(granularity)?))
    }

    fn base(&self) -> HostPhysAddr {
        self.base
    }

    fn copy_from(&self, src: &Self) {
        if self.size != src.size {
            warn!(
                "{:?} copying memory regions from {:?} with different sizes: {} vs {}",
                self.base,
                src.base(),
                self.size,
                src.size
            );
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                H::phys_to_virt(src.base()).as_ptr(),
                H::phys_to_virt(self.base).as_mut_ptr(),
                self.size,
            );
        }
    }
}

impl<H: PagingHandler> Drop for HostPhysicalRegion<H> {
    fn drop(&mut self) {
        debug!(
            "Dropping HostPhysicalRegion [{:?}-{:?}]",
            self.base,
            self.base.add(self.size)
        );
        H::dealloc_frames(self.base, self.size / PAGE_SIZE_4K);
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

    // Below are used for guest addrspace.
    gva_range: AddrRange<GuestVirtAddr>,

    /// Guest mapping type.
    guest_mapping: GuestMapping<H>,
    /// Guest virtual address areas in GVA.
    gva_areas: BTreeMap<GuestVirtAddr, (AddrRange<GuestVirtAddr>, MappingFlags)>,

    /// Guest Page Table levels.
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
    pub fn fork(&mut self) -> AxResult<Self> {
        let mut forked_addrspace = AddrSpace::new_empty(
            GuestPhysAddr::from_usize(0),
            1 << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
        )?;

        let forked_guest_mapping = match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos } => GuestMapping::One2OneMapping { page_pos },
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity,
                mm_region_base,
                mm_page_idx,
                ref mm_regions,
                pt_region_base,
                pt_page_idx,
                ref pt_regions,
            } => {
                let mut new_mm_regions = BTreeMap::new();
                let mut new_pt_region_base = GUEST_PT_ROOT_GPA;
                let mut new_pt_regions = Vec::new();

                for (ori_base, ori_region) in mm_regions {
                    warn!(
                        "Cloning mm [{:?}-{:?}], mapped to [{:?}-{:?}]",
                        ori_base,
                        ori_base.add(mm_region_granularity),
                        ori_region.base(),
                        ori_region.base().add(mm_region_granularity)
                    );

                    forked_addrspace.map_linear(
                        *ori_base,
                        ori_region.base(), // Map to the original region without copying.
                        mm_region_granularity,
                        MappingFlags::READ | MappingFlags::EXECUTE, // erase WRITE permission
                        true,
                    )?;

                    self.ept_addrspace.protect(
                        *ori_base,
                        mm_region_granularity,
                        MappingFlags::READ | MappingFlags::EXECUTE, // erase WRITE permission
                    )?;

                    new_mm_regions.insert(*ori_base, ori_region.clone());
                }

                // For page table regions, we need to copy the original page table regions.
                // Because the guest page table CAN NOT be queried by MMU without `WRITE` permission.
                // ref: Intel SDM 30.3.3.2 EPT Violations
                for ori_pt_region in pt_regions {
                    let new_pt_region = HostPhysicalRegion::allocate(PAGE_SIZE_2M)?;

                    // Copy the original region to the new region.
                    new_pt_region.copy_from(&ori_pt_region);

                    forked_addrspace.map_linear(
                        new_pt_region_base,
                        new_pt_region.base(),
                        PAGE_SIZE_2M,
                        MappingFlags::READ | MappingFlags::WRITE,
                        true,
                    )?;

                    new_pt_regions.push(new_pt_region);

                    new_pt_region_base.add_assign(PAGE_SIZE_2M);
                }

                if new_pt_region_base != pt_region_base.add(PAGE_SIZE_2M) {
                    error!(
                        "New page table region base address {:?} does not match original {:?}",
                        new_pt_region_base, pt_region_base
                    );
                }

                GuestMapping::CoarseGrainedSegmentation {
                    mm_region_granularity,
                    mm_region_base,
                    mm_page_idx,
                    mm_regions: new_mm_regions,
                    pt_region_base,
                    pt_page_idx,
                    pt_regions: new_pt_regions,
                }
            }
        };

        Ok(Self {
            ept_addrspace: forked_addrspace,
            gva_range: self.gva_range.clone(),
            guest_mapping: forked_guest_mapping,
            gva_areas: self.gva_areas.clone(),
            levels: self.levels,
            phontom: core::marker::PhantomData,
        })
    }

    pub fn handle_ept_page_fault(
        &mut self,
        addr: GuestPhysAddr,
        access_flags: MappingFlags,
    ) -> AxResult<bool> {
        debug!(
            "Handle EPT page fault at {:?}, flags {:?}",
            addr, access_flags
        );

        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                unimplemented!()
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity,
                mm_region_base: _,
                mm_page_idx: _,
                ref mut mm_regions,
                pt_region_base: _,
                pt_page_idx: _,
                pt_regions: _,
            } => {
                let fault_mm_region_base = addr.align_down(mm_region_granularity);

                let fault_mm_region = mm_regions
                    .get(&fault_mm_region_base)
                    .ok_or_else(|| ax_err_type!(NotFound, "Fault memory region not found"))?;

                if Arc::strong_count(fault_mm_region) > 1 {
                    // If the reference count is greater than 1, it means that there is still other GuestAddrSpace
                    // holding the reference to this region.
                    // So we need to allocate a new region for this GuestAddrSpace.
                    let new_pt_region = HostPhysicalRegion::allocate_ref(mm_region_granularity)?;

                    new_pt_region.copy_from(fault_mm_region);

                    self.ept_addrspace.map_linear(
                        fault_mm_region_base,
                        new_pt_region.base(),
                        mm_region_granularity,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                        true,
                    )?;
                    let fault_mm_region = mm_regions.insert(fault_mm_region_base, new_pt_region);
                    if fault_mm_region.is_none() {
                        error!(
                            "Ori memory region [{:?}-{:?}] not exist, check why",
                            fault_mm_region_base,
                            fault_mm_region_base.add(mm_region_granularity)
                        );
                    }
                    // The reference count of the original region will be decremented when it is dropped.
                } else {
                    // If the reference count is 1, it means that this is the only GuestAddrSpace holding the reference to this region.
                    // So we can just update the access flags of this region.
                    self.ept_addrspace.protect(
                        fault_mm_region_base,
                        mm_region_granularity,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                    )?;
                }
            }
        }
        Ok(true)
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
    pub fn new(gmt: GuestMappingType) -> AxResult<Self> {
        info!("Generate GuestAddrSpace with {:?}", gmt);

        let mut ept_addrspace = AddrSpace::new_empty(
            GuestPhysAddr::from_usize(0),
            0xffff << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
        )?;

        let guest_mapping = match gmt {
            GuestMappingType::One2OneMapping => {
                // If one to one mapping, map guest page table root to hpa.
                ept_addrspace.map_alloc(
                    GUEST_PT_ROOT_GPA,
                    PAGE_SIZE_4K,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;
                GuestMapping::One2OneMapping { page_pos: 1 }
            }
            GuestMappingType::CoarseGrainedSegmentation2M
            | GuestMappingType::CoarseGrainedSegmentation1G => {
                let mm_region_granularity = match gmt {
                    GuestMappingType::CoarseGrainedSegmentation2M => PAGE_SIZE_2M,
                    GuestMappingType::CoarseGrainedSegmentation1G => PAGE_SIZE_1G,
                    _ => unreachable!(),
                };

                // Map the first memory region.
                let mut mm_regions = BTreeMap::new();
                let first_mm_region = HostPhysicalRegion::allocate_ref(mm_region_granularity)?;

                ept_addrspace.map_linear(
                    GUEST_MEM_REGION_BASE,
                    first_mm_region.base(),
                    mm_region_granularity,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                    true,
                )?;
                mm_regions.insert(GUEST_MEM_REGION_BASE, first_mm_region);

                let first_pt_region = HostPhysicalRegion::allocate(PAGE_SIZE_2M)?;

                ept_addrspace.map_linear(
                    GUEST_PT_ROOT_GPA,
                    first_pt_region.base(),
                    PAGE_SIZE_2M,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;

                GuestMapping::CoarseGrainedSegmentation {
                    // Memory page index start from 0.
                    mm_page_idx: 0,
                    // Page table page index start from 1.
                    pt_page_idx: 1,
                    mm_region_granularity,
                    mm_region_base: GUEST_MEM_REGION_BASE,
                    mm_regions,
                    pt_region_base: GUEST_PT_ROOT_GPA,
                    pt_regions: vec![first_pt_region],
                }
            }
        };

        let mut guest_addrspace = Self {
            ept_addrspace,
            guest_mapping,
            gva_range: AddrRange::from_start_size(
                GuestVirtAddr::from_usize(0),
                1 << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
            ),
            gva_areas: BTreeMap::new(),
            levels: M::LEVELS,
            phontom: core::marker::PhantomData,
        };

        // If one-to-one mapping, map 512GB memory with 1GB huge page.
        if gmt == GuestMappingType::One2OneMapping {
            for gva in (0..PAGE_SIZE_1G * 512).step_by(PAGE_SIZE_1G) {
                guest_addrspace
                    .guest_map_region(
                        GuestVirtAddr::from_usize(gva),
                        |_| GuestPhysAddr::from_usize(gva),
                        PAGE_SIZE_1G,
                        MappingFlags::READ
                            | MappingFlags::WRITE
                            | MappingFlags::EXECUTE
                            | MappingFlags::USER,
                        true,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;
            }
        }

        Ok(guest_addrspace)
    }

    pub fn ept_root_hpa(&self) -> HostPhysAddr {
        self.ept_addrspace.page_table_root()
    }

    pub fn translate(&self, gpa: M::VirtAddr) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
        self.ept_addrspace.translate(gpa)
    }

    /// Add a new linear mapping in EPT.
    ///
    /// The `flags` parameter indicates the mapping permissions and attributes.
    pub fn ept_map_linear(
        &mut self,
        start_vaddr: M::VirtAddr,
        start_paddr: HostPhysAddr,
        size: usize,
        flags: MappingFlags,
        allow_huge: bool,
    ) -> AxResult {
        self.ept_addrspace
            .map_linear(start_vaddr, start_paddr, size, flags, allow_huge)
    }

    pub fn guest_map_alloc(
        &mut self,
        start: GuestVirtAddr,
        size: usize,
        flags: MappingFlags,
        populate: bool,
    ) -> AxResult {
        let mapped_gva_range = AddrRange::from_start_size(start, size);

        debug!(
            "guest_map_alloc [{:?}],({:#x} {:?}, {})",
            mapped_gva_range, size, flags, populate
        );

        if !self.gva_range.contains_range(mapped_gva_range) {
            return ax_err!(
                InvalidInput,
                alloc::format!("GVA [{:?}~{:?}] out of range", start, start.add(size)).as_str()
            );
        }
        if !start.is_aligned_4k() || !is_aligned_4k(size) {
            return ax_err!(InvalidInput, "GVA not aligned");
        }

        if mapped_gva_range.is_empty() {
            return ax_err!(InvalidInput, "GVA range is empty");
        }

        if self.gva_overlaps(mapped_gva_range) {
            // TODO: unmap overlapping area
            return ax_err!(AlreadyExists, "GVA range overlaps with existing area");
        }
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                self.ept_addrspace.map_alloc(
                    GuestPhysAddr::from_usize(start.as_usize()),
                    size,
                    flags,
                    populate,
                )?;
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_page_idx: _,
                pt_page_idx: _,
                mm_region_granularity: _,
                mm_region_base: _,
                mm_regions: _,
                pt_region_base: _,
                pt_regions: _,
            } => {
                if populate {
                    let start_addr = start;
                    let end_addr = start_addr.add(size);

                    for addr in PageIter4K::new(start_addr, end_addr).unwrap() {
                        self.alloc_memory_frame().and_then(|gpa_frame| {
                            self.map(addr, gpa_frame, PageSize::Size4K, flags)
                                .map_err(paging_err_to_ax_err)
                        })?;
                    }
                } else {
                    // Map to a empty entry for on-demand paging.
                    self.guest_map_region(
                        start,
                        |_gva| GuestPhysAddr::from(0),
                        size,
                        MappingFlags::empty(),
                        false,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;
                }
            }
        }

        assert!(
            self.gva_areas
                .insert(start, (mapped_gva_range, flags))
                .is_none(),
            "GVA range already exists, something is wrong!!!"
        );

        Ok(())
    }

    pub fn copy_from_guest(&self, src: GuestVirtAddr, dst: HostVirtAddr, size: usize) -> AxResult {
        // debug!(
        //     "copy_from_guest src: {:?} to dst: {:?} size: {:#x}",
        //     src, dst, size
        // );

        let start_addr = src;
        let end_addr = start_addr.add(size);

        if !self.gva_range.contains(start_addr) || !self.gva_range.contains(end_addr) {
            return ax_err!(
                InvalidInput,
                alloc::format!("GVA [{:?}~{:?}] out of range", start_addr, end_addr).as_str()
            );
        }

        if size == 0 {
            return ax_err!(InvalidInput, "GVA range is empty");
        }

        let start_addr_aligned = start_addr.align_down(PAGE_SIZE_4K);
        let end_addr_aligned = end_addr.align_up(PAGE_SIZE_4K);

        let mut remained_size = size;
        let mut dst_hva = dst;

        for gva in PageIter4K::new(start_addr_aligned, end_addr_aligned).unwrap() {
            let (gpa, _gflags, _gpgsize) = self.guest_query(gva).map_err(paging_err_to_ax_err)?;
            let (hpa, _hflags, _hpgsize) = self
                .ept_addrspace
                .translate(gpa)
                .ok_or_else(|| ax_err_type!(BadAddress, "GPA not mapped"))?;

            let hva = H::phys_to_virt(hpa);
            let src_hva = if gva == start_addr_aligned {
                hva.add(src.align_offset_4k())
            } else {
                hva
            };

            let copied_size = if gva == start_addr_aligned {
                (PAGE_SIZE_4K - src.align_offset_4k()).min(remained_size)
            } else if remained_size >= PAGE_SIZE_4K {
                PAGE_SIZE_4K
            } else {
                remained_size
            };

            unsafe {
                core::ptr::copy_nonoverlapping(src_hva.as_ptr(), dst_hva.as_mut_ptr(), copied_size);
            }

            remained_size -= copied_size;
            dst_hva = dst_hva.add(copied_size);
        }

        Ok(())
    }

    pub fn copy_into_guest(
        &mut self,
        src: HostVirtAddr,
        dst: GuestVirtAddr,
        size: usize,
    ) -> AxResult {
        debug!(
            "copy_into_guest src: {:?} to dst: {:?} size: {:#x}",
            src, dst, size
        );

        let start_addr = dst;
        let end_addr = start_addr.add(size);

        if !self.gva_range.contains(start_addr) || !self.gva_range.contains(end_addr) {
            return ax_err!(
                InvalidInput,
                alloc::format!("GVA [{:?}~{:?}] out of range", start_addr, end_addr).as_str()
            );
        }

        if size == 0 {
            return ax_err!(InvalidInput, "GVA range is empty");
        }

        let start_addr_aligned = start_addr.align_down(PAGE_SIZE_4K);
        let end_addr_aligned = end_addr.align_up(PAGE_SIZE_4K);

        let mut remained_size = size;
        let mut src_hva = src;

        for gva in PageIter4K::new(start_addr_aligned, end_addr_aligned).unwrap() {
            let (gpa, _gflags, _gpgsize) = self.guest_query(gva).map_err(paging_err_to_ax_err)?;
            let (hpa, _hflags, _hpgsize) = self
                .ept_addrspace
                .translate(gpa)
                .ok_or_else(|| ax_err_type!(BadAddress, "GPA not mapped"))?;

            let hva = H::phys_to_virt(hpa);
            let dst_hva = if gva == start_addr_aligned {
                hva.add(dst.align_offset_4k())
            } else {
                hva
            };

            let copied_size = if gva == start_addr_aligned {
                (PAGE_SIZE_4K - dst.align_offset_4k()).min(remained_size)
            } else if remained_size >= PAGE_SIZE_4K {
                PAGE_SIZE_4K
            } else {
                remained_size
            };

            unsafe {
                core::ptr::copy_nonoverlapping(src_hva.as_ptr(), dst_hva.as_mut_ptr(), copied_size);
            }

            remained_size -= copied_size;
            src_hva = src_hva.add(copied_size);
        }

        Ok(())
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    fn alloc_memory_frame(&mut self) -> AxResult<GuestPhysAddr> {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                warn!("Do not need to check memory region for one-to-one mapping");
                ax_err!(
                    BadState,
                    "Do not need to check memory region for one-to-one mapping"
                )
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity,
                mm_region_base,
                ref mut mm_page_idx,
                mm_regions: _,
                pt_region_base: _,
                pt_page_idx: _,
                pt_regions: _,
            } => {
                let allocated_frame_base = mm_region_base.add(*mm_page_idx * PAGE_SIZE_4K);
                *mm_page_idx += 1;

                assert!(
                    *mm_page_idx < mm_region_granularity / PAGE_SIZE_4K,
                    "mm_page_idx {} should be less than {}",
                    *mm_page_idx,
                    mm_region_granularity / PAGE_SIZE_4K
                );

                self.check_memory_region()?;
                Ok(allocated_frame_base)
            }
        }
    }

    fn alloc_page_frame(&mut self) -> AxResult<GuestPhysAddr> {
        let current_gpt_gpa = self.guest_page_table_root_gpa();

        let allocated_frame_base = match self.guest_mapping {
            GuestMapping::One2OneMapping { ref mut page_pos } => {
                if *page_pos == 2 {
                    warn!("When use one-to-one mapping, page_pos should be 0 or 1");
                    return ax_err!(BadState, "page_pos should be 0 or 1, 0 for pgd, 1 for pud");
                }

                let allocated_frame_base = current_gpt_gpa.add(*page_pos * PAGE_SIZE_4K);
                *page_pos += 1;

                self.ept_addrspace.map_alloc(
                    allocated_frame_base,
                    PAGE_SIZE_4K,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;
                allocated_frame_base
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity: _,
                mm_region_base: _,
                mm_page_idx: _,
                mm_regions: _,
                pt_region_base,
                ref mut pt_page_idx,
                pt_regions: _,
            } => {
                let allocated_frame_base = pt_region_base.add(*pt_page_idx * PAGE_SIZE_4K);
                assert!(
                    *pt_page_idx < PAGE_SIZE_2M / PAGE_SIZE_4K,
                    "pt_page_idx {} should be less than {}",
                    *pt_page_idx,
                    PAGE_SIZE_2M / PAGE_SIZE_4K,
                );
                *pt_page_idx += 1;
                self.check_pt_region()?;
                allocated_frame_base
            }
        };

        Ok(allocated_frame_base)
    }

    fn check_memory_region(&mut self) -> AxResult {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                error!("Do not need to check memory region for one-to-one mapping");
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity,
                ref mut mm_region_base,
                ref mut mm_page_idx,
                ref mut mm_regions,
                pt_region_base: _,
                pt_page_idx: _,
                pt_regions: _,
            } => {
                if *mm_page_idx < mm_region_granularity / PAGE_SIZE_4K - 1 {
                    return Ok(());
                }

                mm_region_base.add_assign(mm_region_granularity);

                warn!(
                    "Memory region exhausted, allocating new region at {:?}",
                    mm_region_base
                );

                // Allocate new region.
                let allocated_region = HostPhysicalRegion::allocate_ref(mm_region_granularity)?;

                self.ept_addrspace.map_linear(
                    *mm_region_base,
                    allocated_region.base(),
                    mm_region_granularity,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                    true,
                )?;

                mm_regions.insert(*mm_region_base, allocated_region.clone());
                *mm_page_idx = 0;
            }
        }

        Ok(())
    }

    fn check_pt_region(&mut self) -> AxResult {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                error!("Do not need to check memory region for one-to-one mapping");
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity: _,
                mm_region_base: _,
                mm_page_idx: _,
                mm_regions: _,
                ref mut pt_region_base,
                ref mut pt_page_idx,
                ref mut pt_regions,
            } => {
                if *pt_page_idx < PAGE_SIZE_2M / PAGE_SIZE_4K - 1 {
                    return Ok(());
                }

                pt_region_base.add_assign(PAGE_SIZE_2M);

                warn!(
                    "PT region exhausted, allocating new region at {:?}",
                    pt_region_base
                );

                // Allocate new region.
                let allocated_region = HostPhysicalRegion::allocate(PAGE_SIZE_2M)?;

                self.ept_addrspace.map_linear(
                    *pt_region_base,
                    allocated_region.base(),
                    PAGE_SIZE_2M,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;

                pt_regions.push(allocated_region);

                *pt_page_idx = 0;
            }
        }

        Ok(())
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    /// Returns whether the given address range overlaps with any existing area.
    pub fn gva_overlaps(&self, range: AddrRange<GuestVirtAddr>) -> bool {
        if let Some((_, (before, _flags))) = self.gva_areas.range(..range.start).last() {
            if before.overlaps(range) {
                return true;
            }
        }
        if let Some((_, (after, _flags))) = self.gva_areas.range(range.start..).next() {
            if after.overlaps(range) {
                return true;
            }
        }
        false
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
    fn guest_page_table_root_gpa(&self) -> GuestPhysAddr {
        GUEST_PT_ROOT_GPA
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
    fn map(
        &mut self,
        vaddr: GuestVirtAddr,
        target: GuestPhysAddr,
        page_size: PageSize,
        flags: MappingFlags,
    ) -> PagingResult {
        info!(
            "EPTP@[{:?}] GPT @[{:?}] mapping: {:?} -> {:?}, size {:?} {:?}",
            self.ept_addrspace.page_table_root(),
            self.guest_page_table_root_gpa(),
            vaddr,
            target,
            page_size,
            flags,
        );

        let entry = self.get_entry_mut_or_create(vaddr, page_size)?;
        if !entry.is_unused() {
            warn!("Entry used, {:#x?}", entry);
            return Err(PagingError::AlreadyMapped);
        }
        *entry = MoreGenericPTE::new_page(target.align_down(page_size), flags, page_size.is_huge());
        Ok(())
    }

    /// Unmaps the mapping starts with `vaddr`.
    ///
    /// Returns [`Err(PagingError::NotMapped)`](PagingError::NotMapped) if the
    /// mapping is not present.
    #[allow(unused)]
    fn unmap(&mut self, vaddr: GuestVirtAddr) -> PagingResult<(GuestPhysAddr, PageSize)> {
        let (entry, size) = self.get_entry_mut(vaddr)?;
        if !entry.is_present() {
            entry.clear();
            return Err(PagingError::NotMapped);
        }
        let paddr = entry.paddr();
        entry.clear();
        Ok((paddr, size))
    }

    /// Returns the physical address of the target frame, mapping flags, and
    /// the page size.
    ///
    /// Returns [`Err(PagingError::NotMapped)`](PagingError::NotMapped) if the
    /// mapping is not present.
    pub fn guest_query(
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

    /// Maps a contiguous virtual memory region to a contiguous physical memory
    /// region with the given mapping `flags`.
    ///
    /// The virtual and physical memory regions start with `vaddr` and `paddr`
    /// respectively. The region size is `size`. The addresses and `size` must
    /// be aligned to 4K, otherwise it will return [`Err(PagingError::NotAligned)`].
    ///
    /// When `allow_huge` is true, it will try to map the region with huge pages
    /// if possible. Otherwise, it will map the region with 4K pages.
    ///
    /// When `flush_tlb_by_page` is true, it will flush the TLB immediately after
    /// mapping each page. Otherwise, the TLB flush should by handled by the caller.
    ///
    /// [`Err(PagingError::NotAligned)`]: PagingError::NotAligned
    pub fn guest_map_region(
        &mut self,
        vaddr: GuestVirtAddr,
        get_paddr: impl Fn(GuestVirtAddr) -> GuestPhysAddr,
        size: usize,
        flags: MappingFlags,
        allow_huge: bool,
        flush_tlb_by_page: bool,
    ) -> PagingResult {
        let mut vaddr_usize: usize = vaddr.into();
        let mut size = size;
        if !PageSize::Size4K.is_aligned(vaddr_usize) || !PageSize::Size4K.is_aligned(size) {
            return Err(PagingError::NotAligned);
        }
        debug!(
            "(GPT@{:#x})guest_map_region: [{:#x}, {:#x}) {:?}",
            self.guest_page_table_root_gpa(),
            vaddr_usize,
            vaddr_usize + size,
            flags,
        );
        while size > 0 {
            let vaddr = vaddr_usize.into();
            let paddr = get_paddr(vaddr);
            let page_size = if allow_huge {
                if PageSize::Size1G.is_aligned(vaddr_usize)
                    && paddr.is_aligned(PageSize::Size1G)
                    && size >= PageSize::Size1G as usize
                {
                    PageSize::Size1G
                } else if PageSize::Size2M.is_aligned(vaddr_usize)
                    && paddr.is_aligned(PageSize::Size2M)
                    && size >= PageSize::Size2M as usize
                {
                    PageSize::Size2M
                } else {
                    PageSize::Size4K
                }
            } else {
                PageSize::Size4K
            };
            let _tlb = self.map(vaddr, paddr, page_size, flags).inspect_err(|e| {
                error!(
                    "failed to map page: {:#x?}({:?}) -> {:#x?}, {:?}",
                    vaddr_usize, page_size, paddr, e
                )
            })?;
            if flush_tlb_by_page {
                unimplemented!("flush_tlb_by_page");
            }

            vaddr_usize += page_size as usize;
            size -= page_size as usize;
        }
        Ok(())
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
    #[allow(unused)]
    pub fn walk<F>(&self, limit: usize, pre_func: Option<&F>, post_func: Option<&F>) -> PagingResult
    where
        F: Fn(usize, usize, GuestVirtAddr, &GPTE),
    {
        self.walk_recursive(
            self.table_of(self.guest_page_table_root_gpa())?,
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
    fn alloc_table(&mut self) -> PagingResult<GPTE::PhysAddr> {
        if let Ok(gpa) = self.alloc_page_frame() {
            let (hpa, _flags, _pgsize) = self.ept_addrspace.translate(gpa).ok_or_else(|| {
                warn!("Failed to translate GPA {:?}", gpa);
                PagingError::NotMapped
            })?;

            let ptr = H::phys_to_virt(hpa).as_mut_ptr();
            unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE_4K) };
            Ok(gpa)
        } else {
            Err(PagingError::NoMemory)
        }
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
            let paddr = self.alloc_table()?;
            *entry = MoreGenericPTE::new_table(paddr);
            self.table_of_mut(paddr)
        } else {
            self.next_table_mut(entry)
        }
    }

    fn get_entry(&self, gva: GuestVirtAddr) -> PagingResult<(&GPTE, PageSize)> {
        let vaddr: usize = gva.into();

        let p3 = if self.levels == 3 {
            self.table_of(self.guest_page_table_root_gpa())?
        } else if self.levels == 4 {
            let p4 = self.table_of(self.guest_page_table_root_gpa())?;
            let p4e = &p4[p4_index(vaddr)];
            self.next_table(p4e)?
        } else {
            // 5-level paging
            let p5 = self.table_of(self.guest_page_table_root_gpa())?;
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

    fn get_entry_mut(&mut self, vaddr: GuestVirtAddr) -> PagingResult<(&mut GPTE, PageSize)> {
        let vaddr: usize = vaddr.into();
        let p3 = if self.levels == 3 {
            self.table_of_mut(self.guest_page_table_root_gpa())?
        } else if self.levels == 4 {
            let p4 = self.table_of_mut(self.guest_page_table_root_gpa())?;
            let p4e = &mut p4[p4_index(vaddr)];
            self.next_table_mut(p4e)?
        } else {
            unreachable!()
        };
        let p3e = &mut p3[p3_index(vaddr)];
        if p3e.is_huge() {
            return Ok((p3e, PageSize::Size1G));
        }

        let p2 = self.next_table_mut(p3e)?;
        let p2e = &mut p2[p2_index(vaddr)];
        if p2e.is_huge() {
            return Ok((p2e, PageSize::Size2M));
        }

        let p1 = self.next_table_mut(p2e)?;
        let p1e = &mut p1[p1_index(vaddr)];
        Ok((p1e, PageSize::Size4K))
    }

    fn get_entry_mut_or_create(
        &mut self,
        vaddr: GuestVirtAddr,
        page_size: PageSize,
    ) -> PagingResult<&mut GPTE> {
        let vaddr: usize = vaddr.into();
        let p3 = if M::LEVELS == 3 {
            self.table_of_mut(self.guest_page_table_root_gpa())?
        } else if M::LEVELS == 4 {
            let p4 = self.table_of_mut(self.guest_page_table_root_gpa())?;
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

impl<M: PagingMetaData, EPTE: GenericPTE, GPTE: MoreGenericPTE, H: PagingHandler> Drop
    for GuestAddrSpace<M, EPTE, GPTE, H>
{
    fn drop(&mut self) {
        debug!("GuestAddrSpace drop");
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                // Do nothing
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_region_granularity: _,
                mm_region_base: _,
                mm_page_idx: _,
                mm_regions: _,
                pt_region_base: _,
                pt_page_idx: _,
                pt_regions: _,
            } => {
                warn!("CoarseGrainedSegmentation drop");
            }
        }
    }
}
