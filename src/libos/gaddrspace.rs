use alloc::sync::Arc;
use core::ops::AddAssign;
use std::collections::btree_map::BTreeMap;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::vec::Vec;

use axerrno::{AxError, AxResult, ax_err, ax_err_type};
use lazyinit::LazyInit;
use memory_addr::{
    AddrRange, MemoryAddr, PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, PageIter4K, align_up_4k,
    is_aligned_4k,
};
use page_table_multiarch::{
    GenericPTE, MappingFlags, PageSize, PagingError, PagingHandler, PagingMetaData, PagingResult,
};

use axaddrspace::npt::{EPTEntry, EPTMetadata};
use axaddrspace::{AddrSpace, GuestPhysAddr, GuestVirtAddr, HostPhysAddr, HostVirtAddr};
use equation_defs::bitmap_allocator::PageAllocator;
use equation_defs::{
    GuestMappingType, INSTANCE_REGION_SIZE, InstanceRegion, MMFrameAllocator, PTFrameAllocator,
};

use crate::libos::config::{
    SHIM_EKERNEL, SHIM_ERODATA, SHIM_ETEXT, SHIM_MMIO_REGIONS, SHIM_PHYS_VIRT_OFFSET, SHIM_SDATA,
    SHIM_SKERNEL, SHIM_SRODATA, SHIM_STEXT, SHIM_USER_ENTRY, get_shim_image,
};
use crate::libos::def::{
    GUEST_MEM_REGION_BASE_GPA, GUEST_PT_ROOT_GPA, INSTANCE_INNER_REGION_BASE_GPA,
    PROCESS_INNER_REGION_BASE_GPA, SHIM_BASE_GPA, USER_STACK_BASE, USER_STACK_SIZE,
};
use crate::libos::def::{
    GUEST_MEMORY_REGION_BASE_GVA, GUEST_PT_BASE_GVA, INSTANCE_INNER_REGION_BASE_GVA,
    PROCESS_INNER_REGION_BASE_GVA,
};
use crate::libos::def::{PROCESS_INNER_REGION_SIZE, ProcessInnerRegion};
use crate::libos::gpt::{
    ENTRY_COUNT, GuestEntry, MoreGenericPTE, p1_index, p2_index, p3_index, p4_index, p5_index,
};
use crate::libos::region::{HostPhysicalRegion, HostPhysicalRegionRef};

pub type EqAddrSpace<H> = GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H>;

// Copy from `axmm`.
pub(super) fn paging_err_to_ax_err(err: PagingError) -> AxError {
    warn!("Paging error: {:?}", err);
    match err {
        PagingError::NoMemory => AxError::NoMemory,
        PagingError::NotAligned => AxError::InvalidInput,
        PagingError::NotMapped => AxError::NotFound,
        PagingError::AlreadyMapped => AxError::AlreadyExists,
        PagingError::MappedToHugePage => AxError::InvalidInput,
    }
}

enum GuestMapping<H: PagingHandler> {
    /// One-to-one mapping.
    One2OneMapping {
        page_pos: usize, // Incremented from 0.
    },
    /// Coarse-grained segmentation (2M/1G).
    CoarseGrainedSegmentation {
        /// Stores the host physical address of allocated regions for normal memory.
        mm_regions: BTreeMap<GuestPhysAddr, HostPhysicalRegionRef<H>>,
        /// Stores the host physical address of allocated regions for page table memory.
        pt_regions: Vec<HostPhysicalRegion<H>>,
    },
}

/// Host physical region for shim kernel memory mapping,
/// according to our design, all instances share the same shim kernel.
static GLOBAL_SHIM_REGION: LazyInit<HostPhysicalRegion<PagingHandlerImpl>> = LazyInit::new();

pub(super) fn init_shim_kernel() -> AxResult {
    if GLOBAL_SHIM_REGION.is_inited() {
        return ax_err!(AlreadyExists, "Shim kernel region already initialized");
    }

    let shim_binary = get_shim_image();
    let shim_binary_size = align_up_4k(shim_binary.len());
    let shim_memory_size = if !is_aligned_4k(SHIM_EKERNEL - SHIM_SKERNEL) {
        warn!(
            "SHIM_MEM_SIZE {} is not aligned to 4K",
            SHIM_EKERNEL - SHIM_SKERNEL
        );
        align_up_4k(SHIM_EKERNEL - SHIM_SKERNEL)
    } else {
        SHIM_EKERNEL - SHIM_SKERNEL
    };

    let global_shim_region = HostPhysicalRegion::allocate(shim_memory_size, Some(PAGE_SIZE_4K))?;

    // Copy the shim binary to the guest address space.
    global_shim_region.copy_from_slice(shim_binary, 0, shim_binary_size)?;

    // Allocate a new shim memory region.
    GLOBAL_SHIM_REGION.init_once(global_shim_region);

    Ok(())
}

/// The virtual memory address space.
pub struct GuestAddrSpace<
    M: PagingMetaData,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE,
    H: PagingHandler,
> {
    /// Extended Page Table (EPT) address space.
    /// Responsible for second-stage address translation (GPA <-> HPA).
    ept_addrspace: AddrSpace<M, EPTE, H>,

    /// Process inner region, each process has its own inner region, storing process ID and
    /// memory allocation information, see `get_process_inner_region()`.
    process_inner_region: HostPhysicalRegion<H>,
    /// Instance inner region, shared by all processes in the same instance.
    /// This region is used to store instance information, perCPU run queue, etc.
    instance_region_base: HostPhysAddr,

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
    pub fn fork(&mut self, pid: usize) -> AxResult<Self> {
        let mut forked_addrspace = AddrSpace::new_empty(
            GuestPhysAddr::from_usize(0),
            1 << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
        )?;

        let shim_region = GLOBAL_SHIM_REGION.get().ok_or_else(|| {
            error!("Failed to get shim kernel region");
            ax_err_type!(BadState, "Failed to get shim kernel region")
        })?;

        // Map the shim memory region.
        // DO NOT allocate a new shim memory region,
        // since it is shared by all processes in the same instance.
        forked_addrspace.map_linear(
            SHIM_BASE_GPA,
            shim_region.base(),
            shim_region.size(),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        )?;

        // Allocate and map the process inner region.
        // The process inner region is used to store the process ID and memory allocation information, which is process-specific.
        let forked_process_inner_region =
            HostPhysicalRegion::allocate(self.process_inner_region.size(), Some(PAGE_SIZE_4K))?;
        forked_process_inner_region.copy_from(&self.process_inner_region);

        // Init the forked process inner region.
        let process_inner_region = unsafe {
            forked_process_inner_region
                .as_mut_ptr_of::<ProcessInnerRegion>()
                .as_mut()
        }
        .unwrap();

        if pid == self.process_id() {
            error!("Forked process ID is same as parent process ID");
            return ax_err!(
                InvalidInput,
                "Forked process ID is same as parent process ID"
            );
        }

        process_inner_region.is_primary = false;
        process_inner_region.process_id = pid;
        forked_addrspace.map_linear(
            PROCESS_INNER_REGION_BASE_GPA,
            forked_process_inner_region.base(),
            forked_process_inner_region.size(),
            MappingFlags::READ | MappingFlags::WRITE,
            false,
        )?;

        // Map the instance inner region.
        // Do not allocate a new instance inner region, since it is shared by all processes in the same instance.
        forked_addrspace.map_linear(
            INSTANCE_INNER_REGION_BASE_GPA,
            self.instance_region_base,
            INSTANCE_REGION_SIZE,
            MappingFlags::READ | MappingFlags::WRITE,
            false,
        )?;

        let forked_guest_mapping = match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos } => GuestMapping::One2OneMapping { page_pos },
            GuestMapping::CoarseGrainedSegmentation {
                ref mm_regions,
                ref pt_regions,
            } => {
                let mut new_mm_regions = BTreeMap::new();
                let mut new_pt_region_base = GUEST_PT_ROOT_GPA;
                let mut new_pt_regions = Vec::new();

                let mm_region_granularity = self.process_inner_region().mm_region_granularity;

                // Perform COW at coarse grained region granularity.
                for (ori_base, ori_region) in mm_regions {
                    debug!(
                        "GuestAddrSpace fork region [{:?}-{:?}], which is mapped to [{:?}-{:?}]",
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
                    let new_pt_region = HostPhysicalRegion::allocate(PAGE_SIZE_2M, None)?;

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

                GuestMapping::CoarseGrainedSegmentation {
                    mm_regions: new_mm_regions,
                    pt_regions: new_pt_regions,
                }
            }
        };

        Ok(Self {
            ept_addrspace: forked_addrspace,
            process_inner_region: forked_process_inner_region,
            instance_region_base: self.instance_region_base,
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
        let mm_region_granularity = self.process_inner_region().mm_region_granularity;

        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                unimplemented!()
            }
            GuestMapping::CoarseGrainedSegmentation {
                ref mut mm_regions,
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
                    let new_pt_region =
                        HostPhysicalRegion::allocate_ref(mm_region_granularity, None)?;

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

    /// Load user ELF file into the guest address space.
    /// Setup the entry point and stack region.
    ///
    /// Returns the entry point of the ELF file.
    pub fn setup_user_elf(&mut self, elf_data: &[u8]) -> AxResult {
        use xmas_elf::program::{Flags, SegmentData, Type};
        use xmas_elf::{ElfFile, header};

        let elf = ElfFile::new(elf_data).map_err(|err_str| {
            error!("Failed to parse ELF file: {}", err_str);
            ax_err_type!(InvalidInput)
        })?;

        if elf.header.pt2.type_().as_type() != header::Type::Executable {
            return ax_err!(InvalidInput, "ELF file is not executable");
        }

        fn get_mapping_flags(flags: Flags) -> MappingFlags {
            let mut mapping_flags = MappingFlags::USER;
            if flags.is_read() {
                mapping_flags |= MappingFlags::READ;
            }
            if flags.is_write() {
                mapping_flags |= MappingFlags::WRITE;
            }
            if flags.is_execute() {
                mapping_flags |= MappingFlags::EXECUTE;
            }
            mapping_flags
        }

        info!(
            "Instance [{}] Process {} load user ELF file",
            self.instance_id(),
            self.process_id()
        );

        for ph in elf.program_iter() {
            if ph.get_type() != Ok(Type::Load) {
                continue;
            }
            let mapping_flags = get_mapping_flags(ph.flags());
            info!(
                "Mapping user elf segment: type={:?}, flags={:?}, {:?}\n\toffset={:#x}, vaddr={:#x}, paddr={:#x}, file_size={:#x}, mem_size={:#x}",
                ph.get_type(),
                ph.flags(),
                mapping_flags,
                ph.offset(),
                ph.virtual_addr(),
                ph.physical_addr(),
                ph.file_size(),
                ph.mem_size()
            );

            let vaddr = GuestVirtAddr::from_usize(ph.virtual_addr() as usize);
            // let offset = vaddr.align_offset_4k();
            let area_start = vaddr.align_down_4k();
            let area_end = GuestVirtAddr::from_usize((ph.virtual_addr() + ph.mem_size()) as usize)
                .align_up_4k();
            let ph_data = match ph.get_data(&elf).unwrap() {
                SegmentData::Undefined(data) => data,
                _ => {
                    error!("failed to get ELF segment data");
                    return ax_err!(InvalidInput);
                }
            };

            self.guest_map_alloc(
                area_start,
                area_end.as_usize() - area_start.as_usize(),
                mapping_flags,
                true,
            )?;

            self.copy_into_guest(
                HostVirtAddr::from_ptr_of(ph_data.as_ptr()),
                vaddr,
                ph.mem_size() as usize,
            )?;
        }

        // Setup shim process's stack region.
        self.guest_map_alloc(
            USER_STACK_BASE,
            USER_STACK_SIZE,
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
            true,
        )?;

        self.process_inner_region_mut().user_stack_top =
            USER_STACK_BASE.as_usize() + USER_STACK_SIZE;
        self.process_inner_region_mut().user_entry = elf.header.pt2.entry_point() as usize;

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
    fn process_inner_region(&self) -> &ProcessInnerRegion {
        unsafe {
            self.process_inner_region
                .as_ptr_of::<ProcessInnerRegion>()
                .as_ref()
        }
        .unwrap()
    }

    pub fn process_inner_region_mut(&mut self) -> &mut ProcessInnerRegion {
        unsafe {
            self.process_inner_region
                .as_mut_ptr_of::<ProcessInnerRegion>()
                .as_mut()
        }
        .unwrap()
    }

    fn mm_frame_allocator(&mut self) -> &mut MMFrameAllocator {
        &mut self.process_inner_region_mut().mm_frame_allocator
    }

    fn pt_frame_allocator(&mut self) -> &mut PTFrameAllocator {
        &mut self.process_inner_region_mut().pt_frame_allocator
    }

    pub fn process_id(&self) -> usize {
        self.process_inner_region().process_id as usize
    }

    pub fn instance_id(&self) -> usize {
        unsafe {
            H::phys_to_virt(self.instance_region_base)
                .as_ptr_of::<InstanceRegion>()
                .as_ref()
        }
        .unwrap()
        .instance_id as usize
    }
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    /// Creates a new guest address space,
    /// alone with the shim kernel memory region mapped.
    pub fn new(
        process_id: usize,
        instance_region_base: HostPhysAddr,
        gmt: GuestMappingType,
    ) -> AxResult<Self> {
        info!("Generate GuestAddrSpace with {:?}", gmt);

        let mut ept_addrspace = AddrSpace::new_empty(
            GuestPhysAddr::from_usize(0),
            0xffff << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
        )?;

        // Allocate and map the shim memory region.
        let shim_region = GLOBAL_SHIM_REGION.get().ok_or_else(|| {
            error!("Failed to get shim kernel region");
            ax_err_type!(BadState, "Failed to get shim kernel region")
        })?;

        // Todo: distinguish data, text, rodata, bss sections.
        ept_addrspace.map_linear(
            SHIM_BASE_GPA,
            shim_region.base(),
            shim_region.size(),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        )?;

        debug!(
            "Process inner region size {}",
            core::mem::size_of::<ProcessInnerRegion>()
        );

        // Allocate and map the process inner region.
        let process_inner_region =
            HostPhysicalRegion::allocate(PROCESS_INNER_REGION_SIZE, Some(PAGE_SIZE_4K))?;

        let process_inner_region_size_aligned = process_inner_region.size();

        debug!(
            "Process inner region base: {:?} size {:#x}",
            process_inner_region.base(),
            process_inner_region.size()
        );

        ept_addrspace.map_linear(
            PROCESS_INNER_REGION_BASE_GPA,
            process_inner_region.base(),
            process_inner_region.size(),
            MappingFlags::READ | MappingFlags::WRITE,
            false,
        )?;

        // Map the instance inner region.
        // The instance inner region is shared by all processes in the same instance.
        ept_addrspace.map_linear(
            INSTANCE_INNER_REGION_BASE_GPA,
            instance_region_base,
            INSTANCE_REGION_SIZE,
            MappingFlags::READ | MappingFlags::WRITE,
            false,
        )?;

        let mut region_granularity = 0;
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
                let first_mm_region =
                    HostPhysicalRegion::allocate_ref(mm_region_granularity, None)?;
                ept_addrspace.map_linear(
                    GUEST_MEM_REGION_BASE_GPA,
                    first_mm_region.base(),
                    mm_region_granularity,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                    true,
                )?;
                mm_regions.insert(GUEST_MEM_REGION_BASE_GPA, first_mm_region);

                // Map the first page table region.
                let first_pt_region = HostPhysicalRegion::allocate(PAGE_SIZE_2M, None)?;
                ept_addrspace.map_linear(
                    GUEST_PT_ROOT_GPA,
                    first_pt_region.base(),
                    PAGE_SIZE_2M,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;

                region_granularity = mm_region_granularity;
                GuestMapping::CoarseGrainedSegmentation {
                    mm_regions,
                    pt_regions: vec![first_pt_region],
                }
            }
        };

        let mut guest_addrspace = Self {
            ept_addrspace,
            process_inner_region,
            instance_region_base,
            guest_mapping,
            gva_range: AddrRange::from_start_size(
                GuestVirtAddr::from_usize(0),
                1 << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
            ),
            gva_areas: BTreeMap::new(),
            levels: M::LEVELS,
            phontom: core::marker::PhantomData,
        };

        // Init the process inner region.
        let process_inner_region = guest_addrspace.process_inner_region_mut();
        process_inner_region.is_primary = true;
        process_inner_region.process_id = process_id;
        process_inner_region.cpu_id = process_id;
        process_inner_region.mm_region_granularity = region_granularity;
        process_inner_region.mm_frame_allocator.init_with_page_size(
            PAGE_SIZE_4K,
            region_granularity,
            GUEST_MEM_REGION_BASE_GPA.as_usize(),
            region_granularity,
        );
        process_inner_region.pt_frame_allocator.init_with_page_size(
            PAGE_SIZE_4K,
            PAGE_SIZE_2M,
            GUEST_PT_ROOT_GPA.as_usize(),
            PAGE_SIZE_2M,
        );
        // Init process's context frame.
        process_inner_region.init_kernel_stack_frame(SHIM_USER_ENTRY);

        process_inner_region.dump_kernel_context_frame();

        // Alloc the page table root frame first.
        let guest_pg_root = guest_addrspace.alloc_pt_frame().map_err(|e| {
            error!("Failed to allocate page table root frame: {:?}", e);
            ax_err_type!(NoMemory, "Failed to allocate PT frame")
        })?;
        if GUEST_PT_ROOT_GPA != guest_pg_root {
            error!(
                "Guest page table root GPA: {:?} != {:?}, something wrong",
                GUEST_PT_ROOT_GPA, guest_pg_root
            );
            return ax_err!(BadState, "Guest page table root GPA mismatch");
        }

        match gmt {
            // If one-to-one mapping, map 512GB memory with 1GB huge page,
            // include the shim memory region.
            GuestMappingType::One2OneMapping => {
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
            GuestMappingType::CoarseGrainedSegmentation1G
            | GuestMappingType::CoarseGrainedSegmentation2M => {
                // Map shim kernel sections.
                info!("Map shim kernel sections");
                // Text section.
                guest_addrspace
                    .guest_map_region(
                        GuestVirtAddr::from_usize(SHIM_STEXT),
                        |gva| SHIM_BASE_GPA.add(gva.sub(SHIM_PHYS_VIRT_OFFSET).as_usize()),
                        SHIM_ETEXT - SHIM_STEXT,
                        MappingFlags::READ | MappingFlags::EXECUTE,
                        false,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;
                // Rodata section.
                guest_addrspace
                    .guest_map_region(
                        GuestVirtAddr::from_usize(SHIM_SRODATA),
                        |gva| SHIM_BASE_GPA.add(gva.sub(SHIM_PHYS_VIRT_OFFSET).as_usize()),
                        SHIM_ERODATA - SHIM_SRODATA,
                        MappingFlags::READ,
                        false,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;
                // Data, bss section.
                guest_addrspace
                    .guest_map_region(
                        GuestVirtAddr::from_usize(SHIM_SDATA),
                        |gva| SHIM_BASE_GPA.add(gva.sub(SHIM_PHYS_VIRT_OFFSET).as_usize()),
                        SHIM_EKERNEL - SHIM_SDATA,
                        MappingFlags::READ | MappingFlags::WRITE,
                        // Ouch!!! since nimbos do not support huge page,
                        // do not use huge page here to avoid its complaint.
                        false,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;

                for (base, size) in SHIM_MMIO_REGIONS {
                    info!("Map shim mmio region: {:#x} {:#x}", base, size);
                    guest_addrspace
                        .guest_map_region(
                            GuestVirtAddr::from_usize(base + SHIM_PHYS_VIRT_OFFSET),
                            |gva| GuestPhysAddr::from_usize(gva.as_usize() - SHIM_PHYS_VIRT_OFFSET),
                            *size,
                            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE,
                            false,
                            false,
                        )
                        .map_err(paging_err_to_ax_err)?;
                    guest_addrspace.ept_map_linear(
                        GuestPhysAddr::from_usize(*base),
                        HostPhysAddr::from_usize(*base),
                        *size,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE,
                        false,
                    )?;
                }

                info!("Mapping shim course-grained memory region into high addr space");

                // Map the guest normal memory region to high address space.
                guest_addrspace
                    .guest_map_region(
                        GUEST_MEMORY_REGION_BASE_GVA,
                        |_gva| GUEST_MEM_REGION_BASE_GPA,
                        region_granularity,
                        MappingFlags::READ | MappingFlags::WRITE,
                        true,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;

                info!("Mapping shim page table region");
                // Map guest page table region.
                guest_addrspace
                    .guest_map_region(
                        GUEST_PT_BASE_GVA,
                        |_| GUEST_PT_ROOT_GPA,
                        PAGE_SIZE_2M,
                        MappingFlags::READ | MappingFlags::WRITE,
                        true,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;

                info!("Mapping instance inner region");
                // Map instance inner region and process inner region.
                guest_addrspace
                    .guest_map_region(
                        INSTANCE_INNER_REGION_BASE_GVA,
                        |gva| {
                            INSTANCE_INNER_REGION_BASE_GPA
                                .add(gva.sub_addr(INSTANCE_INNER_REGION_BASE_GVA))
                        },
                        PAGE_SIZE_4K,
                        MappingFlags::READ | MappingFlags::WRITE,
                        false,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;

                info!("Mapping process inner region with user permission");
                guest_addrspace
                    .guest_map_region(
                        PROCESS_INNER_REGION_BASE_GVA,
                        |gva| {
                            PROCESS_INNER_REGION_BASE_GPA
                                .add(gva.sub_addr(PROCESS_INNER_REGION_BASE_GVA))
                        },
                        process_inner_region_size_aligned,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                        false,
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
        start_gpa: M::VirtAddr,
        start_hpa: HostPhysAddr,
        size: usize,
        flags: MappingFlags,
        allow_huge: bool,
    ) -> AxResult {
        self.ept_addrspace
            .map_linear(start_gpa, start_hpa, size, flags, allow_huge)
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
                mm_regions: _,
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
                mm_regions: _,
                pt_regions: _,
            } => {
                let ept_root = self.ept_addrspace.page_table_root();
                let mm_allocator = self.mm_frame_allocator();

                let allocated_frame_base = GuestPhysAddr::from_usize(
                    mm_allocator.alloc_pages(1, PAGE_SIZE_4K).map_err(|e| {
                        error!("Failed to allocate memory frame: {:?}", e);
                        ax_err_type!(NoMemory, "Failed to allocate memory frame")
                    })?,
                );

                debug!(
                    "GAS[@{:?}]Allocating memory frame at {:?}, used/total:[{}/{}]",
                    ept_root,
                    allocated_frame_base,
                    mm_allocator.used_pages(),
                    mm_allocator.total_pages(),
                );

                self.check_memory_region()?;
                Ok(allocated_frame_base)
            }
        }
    }

    fn alloc_pt_frame(&mut self) -> AxResult<GuestPhysAddr> {
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
                mm_regions: _,
                pt_regions: _,
            } => {
                let pt_allocator = self.pt_frame_allocator();

                let allocated_frame_base = GuestPhysAddr::from_usize(
                    pt_allocator.alloc_pages(1, PAGE_SIZE_4K).map_err(|e| {
                        error!("Failed to allocate page table frame: {:?}", e);
                        ax_err_type!(NoMemory, "Failed to allocate page table frame")
                    })?,
                );

                trace!(
                    "Allocating page table frame at {:?}, used/total:[{}/{}]",
                    allocated_frame_base,
                    pt_allocator.used_pages(),
                    pt_allocator.total_pages(),
                );

                self.check_pt_region()?;
                allocated_frame_base
            }
        };

        Ok(allocated_frame_base)
    }

    fn check_memory_region(&mut self) -> AxResult {
        warn!("alloc or recycle memory region here");

        // let mm_region_granularity = self.get_process_inner_region().mm_region_granularity;
        // let mm_region_base =
        //     GuestPhysAddr::from_usize(self.get_process_inner_region().mm_region_base);
        // let mm_page_idx = self.get_process_inner_region().mm_page_idx;
        // let frame_allocator = &self.get_process_inner_region().mm_frame_allocator;

        // frame_allocator.

        // match self.guest_mapping {
        //     GuestMapping::One2OneMapping { page_pos: _ } => {
        //         error!("Do not need to check memory region for one-to-one mapping");
        //     }
        //     GuestMapping::CoarseGrainedSegmentation {
        //         ref mut mm_regions,
        //         pt_regions: _,
        //     } => {
        //         if mm_page_idx < mm_region_granularity / PAGE_SIZE_4K - 1 {
        //             return Ok(());
        //         }

        //         let mm_region_base = mm_region_base.add(mm_region_granularity);

        //         warn!(
        //             "Memory region exhausted, allocating new region at {:?}",
        //             mm_region_base
        //         );

        //         // Allocate new region.
        //         let allocated_region =
        //             HostPhysicalRegion::allocate_ref(mm_region_granularity, None)?;

        //         self.ept_addrspace.map_linear(
        //             mm_region_base,
        //             allocated_region.base(),
        //             mm_region_granularity,
        //             MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
        //             true,
        //         )?;

        //         mm_regions.insert(mm_region_base, allocated_region.clone());
        //         self.get_process_inner_region_mut().mm_page_idx = 0;
        //     }
        // }

        // self.get_process_inner_region_mut().mm_page_idx = 0;
        // self.get_process_inner_region_mut()
        //     .mm_region_base
        //     .add_assign(mm_region_granularity);

        Ok(())
    }

    fn check_pt_region(&mut self) -> AxResult {
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
        trace!(
            "EPTP@[{:?}]GPT@[{:?}] mapping {:?} -> {:?} {:?} {:?}",
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
            "(GPT@{:#x})guest_map_region GVA:[{:#x}, {:#x}) to GPA:[{:#x}, {:#x}] {:?}",
            self.guest_page_table_root_gpa(),
            vaddr_usize,
            vaddr_usize + size,
            get_paddr(vaddr).as_usize(),
            get_paddr(vaddr).as_usize() + size,
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
        if let Ok(gpa) = self.alloc_pt_frame() {
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
                mm_regions: _,
                pt_regions: _,
            } => {
                debug!("CoarseGrainedSegmentation drop");
            }
        }
    }
}
