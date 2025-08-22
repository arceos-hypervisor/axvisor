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
use equation_defs::allocator::PageAllocator;
use equation_defs::scf::SyscallQueueBufferMetadata;
use equation_defs::task::context::TaskContext;
use equation_defs::{
    EQUATION_MAGIC_NUMBER, GuestMappingType, INSTANCE_REGION_SIZE, InstanceRegion,
    MMFrameAllocator, PAGE_CACHE_POOL_SIZE, PTFrameAllocator, SCF_QUEUE_REGION_SIZE,
    USER_HEAP_BASE_VA, get_pgcache_region_by_instance_id, instance_normal_region_pa_to_va,
};
use equation_defs::{get_scf_queue_buff_region_by_instance_id, get_user_memory_va_range};

use crate::libos::config::{
    SHIM_EKERNEL, SHIM_ERODATA, SHIM_ETEXT, SHIM_MMIO_REGIONS, SHIM_PHYS_VIRT_OFFSET, SHIM_SDATA,
    SHIM_SKERNEL, SHIM_SRODATA, SHIM_STEXT, get_shim_image,
};
use crate::libos::def::{
    GUEST_PROCESS_MM_REGION_BASE_GPA, GUEST_PT_ROOT_GPA, INSTANCE_REGION_BASE_GPA,
    PAGE_CACHE_POOL_BASE_GVA, PROCESS_INNER_REGION_BASE_GPA, SCF_QUEUE_REGION_BASE_GVA,
    SHIM_BASE_GPA, USER_STACK_BASE, USER_STACK_SIZE,
};
use crate::libos::def::{
    GUEST_PT_BASE_GVA, INSTANCE_REGION_BASE_GVA, PROCESS_INNER_REGION_BASE_GVA,
};
use crate::libos::def::{PROCESS_INNER_REGION_SIZE, ProcessInnerRegion};
use crate::libos::mm::area::GuestMemoryArea;
use crate::libos::mm::gpt::{
    ENTRY_COUNT, GuestEntry, MoreGenericPTE, p1_index, p2_index, p3_index, p4_index, p5_index,
};
use crate::region::{HostPhysicalRegion, HostPhysicalRegionRef};
use crate::vmm::ivc::{self, IVCChannel, ShmFlags};

pub type EqAddrSpace<H> = GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H>;

// Copy from `axmm`.
pub(crate) fn paging_err_to_ax_err(err: PagingError) -> AxError {
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

pub(crate) fn init_shim_kernel() -> AxResult {
    info!("Initializing shim kernel region");

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

    region_granularity: usize,
    /// Process inner region, each process has its own inner region, storing process ID and
    /// memory allocation information, see `get_process_inner_region()`.
    process_inner_region: HostPhysicalRegion<H>,
    /// Instance inner region, shared by all processes in the same instance.
    /// This region is used to store instance information, perCPU run queue, etc.
    instance_region_base: HostPhysAddr,

    /// SCF queue region, shared by all processes in the same instance.
    scf_region_base: Option<HostPhysAddr>,
    /// Page cache region, shared by all processes in the same instance.
    page_cache_region_base: Option<HostPhysAddr>,
    /// IVC shared memory regions, used for inter-VM communication.
    ivc_shm_keys: BTreeMap<u32, GuestPhysAddr>,

    // Below are used for guest addrspace.
    gva_range: AddrRange<GuestVirtAddr>,

    /// Guest mapping type.
    guest_mapping: GuestMapping<H>,
    /// Guest virtual address areas in GVA.
    /// This fields only stores GVA areas that mapped through `guest_map_alloc()`,
    /// specially, this area is used to stored the mapping info of `gate process` and `eqloader`,
    /// which is loaded into guest address space by AxVisor through `guest_map_alloc()`.
    /// It may be cleared by `HClearGuestAreas` hypercall.
    gva_areas: BTreeMap<GuestVirtAddr, GuestMemoryArea>,

    /// Guest Page Table levels.
    levels: usize,

    phantom: core::marker::PhantomData<GPTE>,
}

impl<
    M: PagingMetaData<VirtAddr = GuestPhysAddr>,
    EPTE: GenericPTE,
    GPTE: MoreGenericPTE<PhysAddr = GuestPhysAddr>,
    H: PagingHandler,
> GuestAddrSpace<M, EPTE, GPTE, H>
{
    pub fn fork(
        &mut self,
        pid: usize,
        shared_mm_regions: &BTreeMap<GuestPhysAddr, HostPhysicalRegion<H>>,
    ) -> AxResult<Self> {
        info!("Forking GuestAddrSpace for process {}", pid);

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
        info!(
            "Mapping shim region @{:?} to base: {:?} size {:#x}",
            SHIM_BASE_GPA,
            shim_region.base(),
            shim_region.size()
        );
        forked_addrspace.map_linear(
            SHIM_BASE_GPA,
            shim_region.base(),
            shim_region.size(),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        )?;

        // Allocate and map the process inner region.
        // The process inner region is used to store the process ID and memory allocation information, which is process-specific.
        info!("Allocating process inner region for process {}", pid);
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

        info!(
            "Mapping [{}] process inner region @ {:?} to {:?}",
            pid,
            PROCESS_INNER_REGION_BASE_GPA,
            forked_process_inner_region.base()
        );
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
        info!(
            "Mapping instance region @{:?} to {:?} size {:#x}",
            INSTANCE_REGION_BASE_GPA, self.instance_region_base, INSTANCE_REGION_SIZE
        );
        forked_addrspace.map_linear(
            INSTANCE_REGION_BASE_GPA,
            self.instance_region_base,
            INSTANCE_REGION_SIZE,
            MappingFlags::READ | MappingFlags::WRITE,
            false,
        )?;

        // handle SCF queue region and page cache region
        if self.scf_region_base.is_some() {
            let scf_queue_region_base = GuestPhysAddr::from_usize(
                get_scf_queue_buff_region_by_instance_id(self.instance_id()),
            );
            let page_cache_region_base =
                GuestPhysAddr::from_usize(get_pgcache_region_by_instance_id(self.instance_id()));

            info!(
                "Mapping SCF queue region @{:?} to base: {:?} size {:#x}",
                scf_queue_region_base,
                self.scf_region_base.unwrap(),
                SCF_QUEUE_REGION_SIZE
            );

            // Map the SCF queue region.
            // Do not allocate a new SCF queue region, since it is shared by all processes in the same instance.
            forked_addrspace.map_linear(
                scf_queue_region_base,
                self.scf_region_base.unwrap(),
                SCF_QUEUE_REGION_SIZE,
                MappingFlags::READ | MappingFlags::WRITE,
                true,
            )?;

            info!(
                "Mapping page cache region @{:?} to {:?} size {:#x}",
                page_cache_region_base,
                self.page_cache_region_base
                    .expect("Page cache region must be set"),
                PAGE_CACHE_POOL_SIZE
            );

            // Map the page cache region.
            // Do not allocate a new page cache region, since it is shared by all processes in the same instance.
            forked_addrspace.map_linear(
                page_cache_region_base,
                self.page_cache_region_base
                    .expect("Page cache region must be set"),
                PAGE_CACHE_POOL_SIZE,
                MappingFlags::READ | MappingFlags::WRITE,
                true,
            )?;
        }

        // Handle ivc shared memory regions.
        for (key, shm_base_gpa) in &self.ivc_shm_keys {
            let (base_hpa, size, _) = ivc::get_channel_info(key)?;

            info!(
                "Mapping IVC region key [{:#x}] @{:?} to {:?}, size {:#x}",
                key, shm_base_gpa, base_hpa, size
            );

            forked_addrspace.map_linear(
                *shm_base_gpa,
                base_hpa,
                size,
                MappingFlags::READ | MappingFlags::WRITE,
                true,
            )?;
        }

        // Handle shared memory regions.
        for (base_gpa, region) in shared_mm_regions {
            info!(
                "GuestAddrSpace map shared region @{:?} to base {:?} size {:#x}",
                base_gpa,
                region.base(),
                region.size()
            );

            forked_addrspace.map_linear(
                *base_gpa,
                region.base(),
                region.size(),
                MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                true,
            )?;
        }

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
                    info!(
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
            region_granularity: self.region_granularity,
            ept_addrspace: forked_addrspace,
            process_inner_region: forked_process_inner_region,
            scf_region_base: self.scf_region_base,
            page_cache_region_base: self.page_cache_region_base,
            instance_region_base: self.instance_region_base,
            gva_range: self.gva_range.clone(),
            guest_mapping: forked_guest_mapping,
            gva_areas: self.gva_areas.clone(),
            levels: self.levels,
            ivc_shm_keys: self.ivc_shm_keys.clone(),
            phantom: core::marker::PhantomData,
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

                    // Unmap the original region first.
                    // self.ept_addrspace
                    //     .unmap(fault_mm_region_base, mm_region_granularity)?;

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
    pub fn setup_user_elf(
        &mut self,
        elf_data: &[u8],
        args_data: Option<&[u8]>,
    ) -> AxResult<TaskContext> {
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

        // Map and copy the ELF segments into the guest address space.
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

            self.guest_map_alloc(
                area_start,
                area_end.as_usize() - area_start.as_usize(),
                mapping_flags,
                true,
            )?;

            if ph.file_size() > 0 {
                let ph_data = match ph.get_data(&elf).unwrap() {
                    SegmentData::Undefined(data) => data,
                    _ => {
                        error!("failed to get ELF segment data");
                        return ax_err!(InvalidInput);
                    }
                };
                self.copy_into_guest(
                    HostVirtAddr::from_ptr_of(ph_data.as_ptr()),
                    vaddr,
                    ph.file_size() as usize,
                )?;
            }

            let zeroed_size = ph.mem_size() as usize - ph.file_size() as usize;

            if zeroed_size > 0 {
                // Zero the memory region after the file size,
                // mainly for BSS segments.
                self.zero_range(vaddr.add(ph.file_size() as usize), zeroed_size)?;
            }
        }

        // Setup user process's stack region.
        self.guest_map_alloc(
            USER_STACK_BASE,
            USER_STACK_SIZE,
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
            true,
        )?;

        let user_entry = elf.header.pt2.entry_point() as usize;
        let user_sp_top = GuestVirtAddr::from_usize(USER_STACK_BASE.as_usize() + USER_STACK_SIZE);

        // Note: the `user_stack_top` and `user_entry` stored in `ProcessInnerRegion`
        // are only used for shim kernel to enter the gate process.
        self.process_inner_region_mut().user_stack_top = user_sp_top.into();
        self.process_inner_region_mut().user_entry = user_entry;

        // Below we setup the user task context.
        let mut cur_sp_top = user_sp_top;

        info!("user stack top: {:?}", user_sp_top);

        // Setup the arguments and environment variables in user stack.
        let args_content_len = if let Some(args_data) = args_data {
            // Copy the arguments data to the user stack.
            let args_size = args_data.len();
            if args_size > USER_STACK_SIZE {
                error!("Arguments data size exceeds user stack size");
                return ax_err!(InvalidInput, "Arguments data size exceeds user stack size");
            }

            let args_ptr_gva = user_sp_top.sub(args_size).align_down(size_of::<u64>());

            // Copy the arguments data to the top of the user stack.
            self.copy_into_guest(
                HostVirtAddr::from_ptr_of(args_data.as_ptr()),
                args_ptr_gva,
                args_size,
            )?;
            cur_sp_top = args_ptr_gva;
            args_size
        } else {
            0
        };

        warn!("Args at user stack: {:?}", cur_sp_top);

        // Setup the user task context.
        // Note: this is only used for user processed other than the gate process.
        // Because only gate process will be executed from the shim kernel,
        // while other user processes will be switched from the gate process
        // through "instance switch" happened in ring 3.

        use equation_defs::task::context::ContextSwitchFrame;
        // x86_64 calling convention: the stack must be 16-byte aligned before
        // calling a function. That means when entering a new task (`ret` in `context_switch`
        // is executed), (stack pointer + 8) should be 16-byte aligned.
        cur_sp_top = cur_sp_top
            .align_down(size_of::<u64>())
            .sub(size_of::<u64>());
        // Allocate a context switch frame on the user stack.
        let frame_ptr_gva = cur_sp_top.sub(size_of::<ContextSwitchFrame>());
        // Construct the context switch frame.
        let ctx_frame = ContextSwitchFrame {
            rip: user_entry as _,
            r15: EQUATION_MAGIC_NUMBER as _, // Magic number for Equation.
            r14: args_content_len as _,      // Length of the arguments data, zero if no arguments.
            ..Default::default()
        };
        // Copy the context switch frame to the user stack.
        self.copy_into_guest(
            HostVirtAddr::from_ptr_of(&ctx_frame),
            frame_ptr_gva,
            size_of::<ContextSwitchFrame>(),
        )?;
        cur_sp_top = frame_ptr_gva;

        info!("Context switch frame at user stack: {:?}", cur_sp_top);
        let mut ctx = TaskContext::new();
        ctx.rsp = cur_sp_top.as_usize() as _;
        // `kstack_top` is unused?
        ctx.kstack_top = HostVirtAddr::from_usize(user_sp_top.as_usize());

        Ok(ctx)
    }

    #[allow(dead_code)]
    pub fn setup_init_task(
        &mut self,
        user_entry: usize,
        stack_top: usize,
    ) -> AxResult<TaskContext> {
        debug!("Setup init task for process {}", self.process_id());
        use equation_defs::task::context::ContextSwitchFrame;
        let user_sp_top = GuestVirtAddr::from_usize(stack_top);

        let mut cur_sp_top = user_sp_top;

        debug!("APP AuxLayout at stack top: {:?}", user_sp_top);

        // x86_64 calling convention: the stack must be 16-byte aligned before
        // calling a function. That means when entering a new task (`ret` in `context_switch`
        // is executed), (stack pointer + 8) should be 16-byte aligned.
        // We DO NOT need to decrement the stack pointer by 8 bytes here!!!

        // Allocate a context switch frame on the user stack.
        let frame_ptr_gva = cur_sp_top.sub(size_of::<ContextSwitchFrame>());
        // Construct the context switch frame.
        let ctx_frame = ContextSwitchFrame {
            rip: user_entry as _,
            ..Default::default()
        };
        // Copy the context switch frame to the user stack.
        self.copy_into_guest(
            HostVirtAddr::from_ptr_of(&ctx_frame),
            frame_ptr_gva,
            size_of::<ContextSwitchFrame>(),
        )?;
        cur_sp_top = frame_ptr_gva;
        debug!("Context switch frame at user stack: {:?}", cur_sp_top);
        let mut ctx = TaskContext::new();
        ctx.rsp = cur_sp_top.as_usize() as _;
        // `kstack_top` is unused?
        ctx.kstack_top = HostVirtAddr::from_usize(user_sp_top.as_usize());

        Ok(ctx)
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

    pub fn instance_region_mut(&mut self) -> &mut InstanceRegion {
        unsafe {
            H::phys_to_virt(self.instance_region_base)
                .as_mut_ptr_of::<InstanceRegion>()
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
        scf_region_base: Option<HostPhysAddr>,
        page_cache_region_base: Option<HostPhysAddr>,
    ) -> AxResult<Self> {
        let instance_id = unsafe {
            H::phys_to_virt(instance_region_base)
                .as_ptr_of::<InstanceRegion>()
                .as_ref()
        }
        .ok_or_else(|| {
            error!(
                "Failed to get instance region from base address: {:?}",
                instance_region_base
            );
            ax_err_type!(
                InvalidInput,
                "Failed to get instance region from base address"
            )
        })?
        .instance_id as usize;

        info!(
            "Generate GuestAddrSpace for Instance {} process {} with {:?}",
            instance_id, process_id, gmt
        );

        // Q: Set the right virtual address range according to process ID?
        let mut ept_addrspace = AddrSpace::new_empty(
            GuestPhysAddr::from_usize(0),
            0xffff << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
        )?;

        // Allocate and map the process inner region.
        let process_inner_region =
            HostPhysicalRegion::allocate(PROCESS_INNER_REGION_SIZE, Some(PAGE_SIZE_4K))?;

        debug!(
            "Mapping Process inner region base: {:?} size {:#x}",
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

        // Map the instance region.
        // The instance region is shared by all processes in the same instance.
        debug!(
            "Mapping Instance region base: {:?} size {:#x}",
            instance_region_base, INSTANCE_REGION_SIZE
        );
        ept_addrspace.map_linear(
            INSTANCE_REGION_BASE_GPA,
            instance_region_base,
            INSTANCE_REGION_SIZE,
            MappingFlags::READ | MappingFlags::WRITE,
            false,
        )?;

        if let Some(scf_region_base) = scf_region_base {
            // Map the SCF queue region.
            // The SCF queue region is shared by all processes in the same instance.
            let scf_region_base_gpa =
                GuestPhysAddr::from_usize(get_scf_queue_buff_region_by_instance_id(instance_id));
            debug!(
                "Mapping SCF queue region [{:?}~{:?}] size {:#x}",
                scf_region_base_gpa,
                scf_region_base_gpa + SCF_QUEUE_REGION_SIZE,
                SCF_QUEUE_REGION_SIZE
            );
            ept_addrspace.map_linear(
                scf_region_base_gpa,
                scf_region_base,
                SCF_QUEUE_REGION_SIZE,
                MappingFlags::READ | MappingFlags::WRITE,
                true,
            )?;
        }
        if let Some(page_cache_region_base) = page_cache_region_base {
            // Map the page cache region.
            // The page cache region is shared by all processes in the same instance.
            let page_cache_region_base_gpa =
                GuestPhysAddr::from_usize(get_pgcache_region_by_instance_id(instance_id));
            debug!(
                "Mapping Page Cache region [{:?}~{:?}] size {:#x}",
                page_cache_region_base_gpa,
                page_cache_region_base_gpa + PAGE_CACHE_POOL_SIZE,
                PAGE_CACHE_POOL_SIZE
            );
            ept_addrspace.map_linear(
                page_cache_region_base_gpa,
                page_cache_region_base,
                PAGE_CACHE_POOL_SIZE,
                MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE, // Allow execute for page cache region.
                true,
            )?;
        }

        let mut region_granularity = 0;
        let guest_mapping = match gmt {
            GuestMappingType::One2OneMapping => GuestMapping::One2OneMapping { page_pos: 0 },
            GuestMappingType::CoarseGrainedSegmentation2M
            | GuestMappingType::CoarseGrainedSegmentation1G => {
                region_granularity = match gmt {
                    GuestMappingType::CoarseGrainedSegmentation2M => PAGE_SIZE_2M,
                    GuestMappingType::CoarseGrainedSegmentation1G => PAGE_SIZE_1G,
                    _ => unreachable!(),
                };
                GuestMapping::CoarseGrainedSegmentation {
                    mm_regions: BTreeMap::new(),
                    pt_regions: Vec::new(),
                }
            }
        };

        let mut guest_addrspace = Self {
            ept_addrspace,
            region_granularity,
            process_inner_region,
            instance_region_base,
            scf_region_base,
            page_cache_region_base,
            guest_mapping,
            ivc_shm_keys: BTreeMap::new(),
            gva_range: AddrRange::from_start_size(
                GuestVirtAddr::from_usize(0),
                1 << <EPTMetadata as PagingMetaData>::VA_MAX_BITS,
            ),
            gva_areas: BTreeMap::new(),
            levels: M::LEVELS,
            phantom: core::marker::PhantomData,
        };

        // Call `init_process_inner_region` first to ensure that the basic metadata
        // like `mm_frame_allocator` and `pt_frame_allocator` are initialized.
        guest_addrspace.init_process_inner_region(process_id)?;

        guest_addrspace.setup_mm_regions()?;
        guest_addrspace.setup_pt_regions()?;

        // These setup methods should be called after `setup_pt_regions`.
        guest_addrspace.setup_shim_region()?;
        guest_addrspace.setup_process_inner_region()?;
        guest_addrspace.setup_instance_region()?;
        guest_addrspace.setup_shm_region()?;

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
            | GuestMappingType::CoarseGrainedSegmentation2M => {}
        }

        Ok(guest_addrspace)
    }

    /// Setup the shim kernel region for the guest address space.
    /// This region is shared by all processes from all instances.
    fn setup_shim_region(&mut self) -> AxResult {
        // Allocate and map the shim memory region.
        let shim_region = GLOBAL_SHIM_REGION.get().ok_or_else(|| {
            error!("Failed to get shim kernel region");
            ax_err_type!(BadState, "Failed to get shim kernel region")
        })?;

        debug!(
            "Mapping shim kernel region base: {:?} size {:#x}",
            shim_region.base(),
            shim_region.size()
        );
        // Todo: distinguish data, text, rodata, bss sections.
        self.ept_addrspace.map_linear(
            SHIM_BASE_GPA,
            shim_region.base(),
            shim_region.size(),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        )?;

        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                // Do nothing.
                // For one-to-one mapping, the shim kernel region is already mapped.
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_regions: _,
                pt_regions: _,
            } => {
                // Map shim kernel sections.
                info!("Maping shim kernel sections");
                // Text section.
                self.guest_map_region(
                    GuestVirtAddr::from_usize(SHIM_STEXT),
                    |gva| SHIM_BASE_GPA.add(gva.sub(SHIM_PHYS_VIRT_OFFSET).as_usize()),
                    SHIM_ETEXT - SHIM_STEXT,
                    // TODO: only equation segments should be mapped with USER permission.
                    MappingFlags::READ | MappingFlags::EXECUTE | MappingFlags::USER,
                    false,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;
                // Rodata section.
                self.guest_map_region(
                    GuestVirtAddr::from_usize(SHIM_SRODATA),
                    |gva| SHIM_BASE_GPA.add(gva.sub(SHIM_PHYS_VIRT_OFFSET).as_usize()),
                    SHIM_ERODATA - SHIM_SRODATA,
                    MappingFlags::READ,
                    false,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;
                // Data, bss section.
                self.guest_map_region(
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
                    self.guest_map_region(
                        GuestVirtAddr::from_usize(base + SHIM_PHYS_VIRT_OFFSET),
                        |gva| GuestPhysAddr::from_usize(gva.as_usize() - SHIM_PHYS_VIRT_OFFSET),
                        *size,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE,
                        false,
                        false,
                    )
                    .map_err(paging_err_to_ax_err)?;
                    self.ept_map_linear(
                        GuestPhysAddr::from_usize(*base),
                        HostPhysAddr::from_usize(*base),
                        *size,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE,
                        false,
                    )?;
                }

                // info!("Mapping shim course-grained memory region into high addr space");

                // // Map the guest normal memory region to high address space.
                // self.guest_map_region(
                //     GUEST_MEMORY_REGION_BASE_GVA,
                //     |_gva| GUEST_MEM_REGION_BASE_GPA,
                //     self.region_granularity,
                //     MappingFlags::READ | MappingFlags::WRITE,
                //     true,
                //     false,
                // )
                // .map_err(paging_err_to_ax_err)?;
            }
        }

        Ok(())
    }

    /// Init basic process inner region metadata,
    /// this function should be called before setting up everything else in the guest address space.
    /// because it is used to init the memory frame allocator and page table frame allocator.
    fn init_process_inner_region(&mut self, process_id: usize) -> AxResult {
        info!("Init process inner region for process {}", process_id);
        let region_granularity = self.region_granularity;

        // Init the process inner region.
        let process_inner_region = self.process_inner_region_mut();
        process_inner_region.is_primary = true;
        process_inner_region.process_id = process_id;
        process_inner_region.mm_region_granularity = region_granularity;
        process_inner_region.mm_frame_allocator.init_with_page_size(
            PAGE_SIZE_4K,
            region_granularity,
            GUEST_PROCESS_MM_REGION_BASE_GPA.as_usize(),
            0, // Just init mm_frame_allocator with 0 size!!!
        );
        process_inner_region.pt_frame_allocator.init_with_page_size(
            PAGE_SIZE_4K,
            PAGE_SIZE_2M,
            GUEST_PT_ROOT_GPA.as_usize(),
            0,
        );

        // Set the heap base and top in the process inner region.
        // The mapping will not be extablished until user process requests it through `brk` or `sbrk`.
        process_inner_region.heap_base = USER_HEAP_BASE_VA;
        process_inner_region.heap_top = USER_HEAP_BASE_VA;
        let (va_start, va_end) = get_user_memory_va_range(process_id);

        info!(
            "Process {} user memory VA range: {:?} - {:?}",
            process_id, va_start, va_end
        );

        // Init the gas for the process inner region.
        // The gas is used to manage the user memory allocation.
        process_inner_region
            .gas
            .init(va_start, va_end, GUEST_PT_ROOT_GPA.as_usize());

        info!("Process {} user memory gas initialized", process_id);

        process_inner_region.dump_allocator_status();
        process_inner_region.dump_mm_regions();

        Ok(())
    }

    /// Setup the process inner region for the guest address space.
    /// The process inner region is used to store the process ID, memory allocation information,
    /// and other process-specific information.
    fn setup_process_inner_region(&mut self) -> AxResult {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                // Do nothing, the process inner region is already mapped.
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_regions: _,
                pt_regions: _,
            } => {
                // Map the process inner region in the guest address space.
                self.guest_map_region(
                    PROCESS_INNER_REGION_BASE_GVA,
                    |gva| {
                        PROCESS_INNER_REGION_BASE_GPA
                            .add(gva.sub_addr(PROCESS_INNER_REGION_BASE_GVA))
                    },
                    self.process_inner_region.size(),
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                    false,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;
            }
        }

        Ok(())
    }

    fn setup_instance_region(&mut self) -> AxResult {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                // Do nothing, already mapped.
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_regions: _,
                pt_regions: _,
            } => {
                info!("Mapping instance inner region with user permission");
                // Map instance region.
                self.guest_map_region(
                    INSTANCE_REGION_BASE_GVA,
                    |gva| INSTANCE_REGION_BASE_GPA.add(gva.sub_addr(INSTANCE_REGION_BASE_GVA)),
                    INSTANCE_REGION_SIZE,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                    false,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;
            }
        }
        Ok(())
    }

    /// Setup the shared memory region for the syscall-forwarding mechanism and page cache pool.
    fn setup_shm_region(&mut self) -> AxResult {
        let instance_id = self.instance_id();

        if self.scf_region_base.is_none() && self.page_cache_region_base.is_none() {
            info!("No shared memory region base provided, skip shared memory region setup");
            return Ok(());
        }

        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                // Do nothing, already mapped.
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_regions: _,
                pt_regions: _,
            } => {
                info!("Mapping SCF queue region with user permission");
                let scf_queue_base_gpa = GuestPhysAddr::from_usize(
                    get_scf_queue_buff_region_by_instance_id(instance_id),
                );
                self.guest_map_region(
                    SCF_QUEUE_REGION_BASE_GVA,
                    |gva| scf_queue_base_gpa.add(gva.sub_addr(SCF_QUEUE_REGION_BASE_GVA)),
                    SCF_QUEUE_REGION_SIZE,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                    true,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;

                info!("Mapping page cache region without user permission");
                let page_cache_base_gpa =
                    GuestPhysAddr::from_usize(get_pgcache_region_by_instance_id(instance_id));
                self.guest_map_region(
                    PAGE_CACHE_POOL_BASE_GVA,
                    |gva| page_cache_base_gpa.add(gva.sub_addr(PAGE_CACHE_POOL_BASE_GVA)),
                    PAGE_CACHE_POOL_SIZE,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;
            }
        }

        let scf_meta = unsafe {
            H::phys_to_virt(self.scf_region_base.expect("SCF region base is None"))
                .as_mut_ptr_of::<SyscallQueueBufferMetadata>()
                .as_mut()
                .ok_or_else(|| {
                    ax_err_type!(
                        InvalidInput,
                        "Failed to get SyscallQueueBufferMetadata from base address"
                    )
                })?
        };
        // AxVisor do not touch scf region for now.
        // BUT we can consider to manipulate its status to enhance the correctness.
        scf_meta.initialize(0);

        Ok(())
    }

    fn setup_mm_regions(&mut self) -> AxResult {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                // Do nothing.
            }
            GuestMapping::CoarseGrainedSegmentation {
                mm_regions: _,
                pt_regions: _,
            } => {
                // `mm_regions` is allocated dynamically when the user process requests memory.
                // So we do not need to setup it here.
                // See `alloc_memory_frame` for details,
                // where the `alloc_mm_region` is called to allocate a new memory region.
            }
        }
        Ok(())
    }

    /// Setup `pt_regions` for the guest address space.
    /// The guest page table root frame is allocated and mapped in this function,
    /// which will trigger the first page table region allocation.
    fn setup_pt_regions(&mut self) -> AxResult {
        // Alloc the page table root frame first.
        let guest_pg_root = self.alloc_pt_frame().map_err(|e| {
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

        match self.guest_mapping {
            GuestMapping::One2OneMapping { ref mut page_pos } => {
                // If one to one mapping, map guest page table root to hpa.
                self.ept_addrspace.map_alloc(
                    GUEST_PT_ROOT_GPA,
                    PAGE_SIZE_4K,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;
                // Set the page position to 1, which means the first page table region is allocated.
                *page_pos = 1;
            }
            GuestMapping::CoarseGrainedSegmentation {
                pt_regions: _,
                mm_regions: _,
            } => {
                info!("Mapping instance page table region");
                // Map guest page table region.
                self.guest_map_region(
                    GUEST_PT_BASE_GVA,
                    |_| GUEST_PT_ROOT_GPA,
                    PAGE_SIZE_2M,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;
            }
        }

        Ok(())
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
                            self.guest_map(addr, gpa_frame, PageSize::Size4K, flags)
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
                .insert(start, GuestMemoryArea::new(mapped_gva_range, flags))
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

    pub fn copy_into_guest_of<T>(&mut self, src: &T, dst: GuestVirtAddr) -> AxResult
    where
        T: Sized + Copy,
    {
        let size = core::mem::size_of::<T>();
        debug!(
            "copy_into_guest_of src: {:p} to dst: {:?} size: {:#x}",
            src, dst, size
        );

        self.copy_into_guest(HostVirtAddr::from_ptr_of(src as *const T), dst, size)
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

        if (!self.gva_range.contains(start_addr) || !self.gva_range.contains(end_addr))
            && !(PROCESS_INNER_REGION_BASE_GVA
                ..=PROCESS_INNER_REGION_BASE_GVA + PROCESS_INNER_REGION_SIZE)
                .contains(&start_addr)
        {
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

    pub fn zero_range(&mut self, dst: GuestVirtAddr, size: usize) -> AxResult {
        debug!("zero_range dst: {:?} size: {:#x}", dst, size);

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

            let zeroed_size = if gva == start_addr_aligned {
                (PAGE_SIZE_4K - dst.align_offset_4k()).min(remained_size)
            } else if remained_size >= PAGE_SIZE_4K {
                PAGE_SIZE_4K
            } else {
                remained_size
            };

            unsafe {
                core::ptr::write_bytes(dst_hva.as_mut_ptr(), 0, zeroed_size);
            }

            remained_size -= zeroed_size;
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
    pub fn ivc_shm_sync(
        &mut self,
        shmkey: u32,
        flags: MappingFlags,
        size: usize,
        alignment: PageSize,
    ) -> AxResult<GuestPhysAddr> {
        let instance_id = self.instance_id();
        debug!(
            "Instance [{instance_id}] ivc_shm_sync key: {:#x}, size: {:#x}, flags: {:#x}, alignment: {:?}",
            shmkey, size, flags, alignment
        );

        // To avoid potential GPA overlap issues, we do not use the same base GPA as
        // the host Linux, instead, we allocate a new base GPA from the instance's ShmManager.
        // Actualy, the shm base GPA wil be returned to the guest Instance through a quite long path,
        // 1. The shmgpa will be return to axcli daemon process as the return value of this hypercall,
        // 2. The axcli daemon will check the returned shmgpa, and fill it into the `ShmArgs` struct,
        // which is passed by the shim as one of the  scf arguments.
        let shm_base = self
            .instance_region_mut()
            .shm_manager_mut()
            .alloc_pages(size / PAGE_SIZE_4K, alignment as usize)?;

        let shm_base_gpa = GuestPhysAddr::from_usize(shm_base);

        if !shm_base_gpa.is_aligned(alignment as usize) {
            error!(
                "Shared memory base GPA {:#x} is not aligned to {:#x}",
                shm_base_gpa, alignment as usize
            );
            return ax_err!(
                InvalidData,
                "Shared memory base GPA is not aligned to the specified alignment"
            );
        }

        // Subscribe to an existing IVC channel.
        let (base_hpa, actual_size) =
            ivc::subscribe_to_channel(shmkey, instance_id, shm_base_gpa, size)?;

        debug!(
            "Instance [{}] subscribing to IVC channel key {:#x}, base HPA: {:?}, size: {:#x}",
            self.instance_id(),
            shmkey,
            base_hpa,
            actual_size
        );

        self.ept_map_linear(
            shm_base_gpa,
            base_hpa,
            actual_size,
            flags,
            true, // Allow huge pages
        )?;

        self.ivc_shm_keys.insert(shmkey, shm_base_gpa);

        Ok(shm_base_gpa)
    }

    pub fn ivc_get(
        &mut self,
        key: u32,
        size: usize,
        flags: usize,
        shm_base_gva_ptr: usize,
    ) -> AxResult<usize> {
        debug!(
            "ivc_get key: {:#x}, size: {:#x}, flags: {:#x}, shm_base_gva_ptr: {:#x}",
            key, size, flags, shm_base_gva_ptr
        );

        debug!("Allocate {} pages for IVC channel", size / PAGE_SIZE_4K);

        let shm_base = self
            .instance_region_mut()
            .shm_manager_mut()
            .alloc_pages(size / PAGE_SIZE_4K, PAGE_SIZE_4K)?;

        let shm_base_gpa = GuestPhysAddr::from_usize(shm_base);

        let size = align_up_4k(size);
        let instance_id = self.instance_id();
        let flags = ShmFlags::from_bits_retain(flags);

        // Try to create a new IVC channel.
        if flags.contains(ShmFlags::IPC_CREAT) && !ivc::contains_channel(key) {
            // Create a new IVC channel.
            let mut channel = IVCChannel::allocate(key, size)?;

            self.ept_map_linear(
                shm_base_gpa,
                channel.base_hpa(),
                channel.size(),
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )?;

            channel.add_subscriber(instance_id, shm_base_gpa, size);

            ivc::insert_channel(key, channel, false)?;
        } else {
            if flags.contains(ShmFlags::IPC_EXCL) && ivc::contains_channel(key) {
                warn!("IVC channel with key {:#x} already exists", key);
                return ax_err!(AlreadyExists, "IVC channel already exists");
            }
            // Subcribe to an existing IVC channel.
            let (base_hpa, actual_size) =
                ivc::subscribe_to_channel(key, instance_id, shm_base_gpa, size)?;

            debug!(
                "Instance [{}] subscribing to IVC channel key {:#x}, base HPA: {:?}, size: {:#x}",
                self.instance_id(),
                key,
                base_hpa,
                actual_size
            );

            self.ept_map_linear(
                shm_base_gpa,
                base_hpa,
                actual_size,
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )?;
        }

        self.ivc_shm_keys.insert(key, shm_base_gpa);

        // Write the base GPA to the guest.
        self.copy_into_guest_of(&shm_base, GuestVirtAddr::from_usize(shm_base_gva_ptr))?;

        Ok(key as usize)
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
                if self.mm_frame_allocator().available_pages() == 0 {
                    self.alloc_mm_region()?;
                }

                let ept_root = self.ept_addrspace.page_table_root();
                let mm_allocator = self.mm_frame_allocator();
                let allocated_frame_base = GuestPhysAddr::from_usize(
                    mm_allocator.alloc_pages(1, PAGE_SIZE_4K).map_err(|e| {
                        error!("Failed to allocate memory frame: {:?}", e);
                        ax_err_type!(NoMemory, "Failed to allocate memory frame")
                    })?,
                );
                debug!(
                    "GAS[@{:?}] Allocating memory frame at {:?}, used/total:[{}/{}]",
                    ept_root,
                    allocated_frame_base,
                    mm_allocator.used_pages(),
                    mm_allocator.total_pages(),
                );

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
                if self.pt_frame_allocator().available_pages() == 0 {
                    self.alloc_pt_region()?;
                }

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

    pub fn alloc_pt_region(&mut self) -> AxResult<HostPhysAddr> {
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                warn!("Do not need to check memory region for one-to-one mapping");
                ax_err!(
                    BadState,
                    "Do not need to check memory region for one-to-one mapping"
                )
            }
            GuestMapping::CoarseGrainedSegmentation {
                ref mut pt_regions,
                mm_regions: _,
            } => {
                let allocated_region = HostPhysicalRegion::allocate(PAGE_SIZE_2M, None)?;
                let allocated_region_hpa = allocated_region.base();
                let current_pt_region_count = pt_regions.len();

                let allocated_region_gpa_base =
                    GUEST_PT_ROOT_GPA.add(current_pt_region_count * PAGE_SIZE_2M);

                self.ept_addrspace.map_linear(
                    allocated_region_gpa_base,
                    allocated_region_hpa,
                    PAGE_SIZE_2M,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                )?;

                pt_regions.push(allocated_region);

                self.process_inner_region_mut()
                    .pt_frame_allocator
                    .increase_segment_at(allocated_region_gpa_base.as_usize(), PAGE_SIZE_2M);

                info!(
                    "Allocating pt region at [{:?} ~ {:?}], total segments: {}, pages used/total:[{}/{}]",
                    allocated_region_gpa_base,
                    allocated_region_gpa_base.add(PAGE_SIZE_2M),
                    self.process_inner_region()
                        .pt_frame_allocator
                        .total_segments(),
                    self.process_inner_region().pt_frame_allocator.used_pages(),
                    self.process_inner_region().pt_frame_allocator.total_pages(),
                );

                Ok(allocated_region_hpa)
            }
        }
    }

    pub fn alloc_mm_region_with_pages(&mut self, requested_pages: usize) -> AxResult {
        let mm_region_granularity = self.process_inner_region().mm_region_granularity;
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
                // First, judge how many mm regions are needed according to the requested pages.
                let pages_per_region = mm_region_granularity / PAGE_SIZE_4K; // 1 region = 512 pages for 2M region.
                let requested_regions = if requested_pages % pages_per_region == 0 {
                    requested_pages / pages_per_region
                } else {
                    requested_pages / pages_per_region + 1
                };

                // Then, allocate the mm regions.
                for _region in 0..requested_regions {
                    self.alloc_mm_region()?;
                }
                Ok(())
            }
        }
    }

    pub fn alloc_mm_region(&mut self) -> AxResult {
        let mm_region_granularity = self.process_inner_region().mm_region_granularity;
        match self.guest_mapping {
            GuestMapping::One2OneMapping { page_pos: _ } => {
                warn!("Do not need to check memory region for one-to-one mapping");
                ax_err!(
                    BadState,
                    "Do not need to check memory region for one-to-one mapping"
                )
            }
            GuestMapping::CoarseGrainedSegmentation {
                ref mut mm_regions,
                pt_regions: _,
            } => {
                let allocated_region =
                    HostPhysicalRegion::allocate_ref(mm_region_granularity, None)?;
                let current_mm_region_count = mm_regions.len();
                // let allocated_region_hpa = allocated_region.base();

                let allocated_region_gpa_base = GUEST_PROCESS_MM_REGION_BASE_GPA
                    .add(current_mm_region_count * mm_region_granularity);

                self.ept_addrspace.map_linear(
                    allocated_region_gpa_base,
                    allocated_region.base(),
                    mm_region_granularity,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
                    true,
                )?;
                mm_regions.insert(allocated_region_gpa_base, allocated_region);

                self.process_inner_region_mut()
                    .mm_frame_allocator
                    .increase_segment_at(
                        allocated_region_gpa_base.as_usize(),
                        mm_region_granularity,
                    );

                debug!(
                    "Extending process region at [{:?} ~ {:?}], total segments: {}, used/total: [{}/{}]",
                    allocated_region_gpa_base,
                    allocated_region_gpa_base.add(mm_region_granularity),
                    self.process_inner_region()
                        .mm_frame_allocator
                        .total_segments(),
                    self.process_inner_region().mm_frame_allocator.used_pages(),
                    self.process_inner_region().mm_frame_allocator.total_pages(),
                );

                trace!("Mapping shim course-grained memory region into high addr space");

                // Map the guest normal memory region to high address space.
                self.guest_map_region(
                    GuestVirtAddr::from_usize(instance_normal_region_pa_to_va(
                        allocated_region_gpa_base.as_usize(),
                    )),
                    |_gva| allocated_region_gpa_base,
                    self.region_granularity,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true,
                    false,
                )
                .map_err(paging_err_to_ax_err)?;

                Ok(())
            }
        }
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
        if let Some((_, before)) = self.gva_areas.range(..range.start).last() {
            if before.va_range().overlaps(range) {
                return true;
            }
        }
        if let Some((_, after)) = self.gva_areas.range(range.start..).next() {
            if after.va_range().overlaps(range) {
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
    fn guest_map(
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

    pub fn guest_clear_mapped_area(&mut self) -> AxResult {
        debug!("guest_clear_mapped_area");

        let unmapped_areas = self.gva_areas.clone();
        // Clear all GVA areas.
        self.gva_areas.clear();

        // Unmap all guest virtual addresses.
        for (gva, area) in unmapped_areas {
            debug!("Unmapping GVA area: {:?}", area);
            for vaddr in PageIter4K::new(gva, gva.add(area.size())).unwrap() {
                let (_paddr, _page_size) = self.unmap(vaddr).map_err(paging_err_to_ax_err)?;
            }
        }

        Ok(())
    }

    #[allow(unused)]
    pub fn guest_unmap_area(&mut self, start: GuestVirtAddr, size: usize) -> AxResult {
        let range = AddrRange::try_from_start_size(start, size).ok_or(AxError::InvalidInput)?;
        if range.is_empty() {
            return Ok(());
        }

        let end = range.end;

        // Unmap entire areas that are contained by the range.
        let mut unmap_areas = Vec::new();
        self.gva_areas.retain(|_, area| {
            if area.va_range().contained_in(range) {
                debug!("Unmapping GVA range: {:?}", area);
                unmap_areas.push(area.clone());
                false // Remove this area
            } else {
                true // Keep this area
            }
        });

        // Shrink right if the area intersects with the left boundary.
        if let Some((&before_start, before)) = self.gva_areas.range_mut(..range.start).last() {
            let before_end = before.end();
            if before_end > start {
                if before_end <= end {
                    // the unmapped area is at the end of `before`
                    warn!(
                        "range {:?} is at the end of before area {:?}",
                        range, before
                    );

                    let unmap_area = before.shrink_right(start.sub_addr(before_start));
                    warn!("Shrinking right range {:?}", unmap_area);
                    unmap_areas.push(unmap_area);
                } else {
                    // the unmapped area is in the middle `before`, need to split.
                    debug!(
                        "range {:?} is in the middle of before area {:?}",
                        range, before
                    );
                    let right_part = before.split(end).unwrap();

                    let unmap_area = before.shrink_right(start.sub_addr(before_start));

                    warn!("Shrinking middle range {:?}", unmap_area);
                    unmap_areas.push(unmap_area);

                    assert_eq!(right_part.start().as_usize(), end.as_usize());
                    self.gva_areas.insert(end, right_part);
                }
            }
        }
        // Shrink left if the area intersects with the right boundary.
        if let Some((&after_start, after)) = self.gva_areas.range_mut(start..).next() {
            let after_end = after.end();
            if after_start < end {
                warn!(
                    "range {:?} is at the start of after area {:?}",
                    range, after
                );
                let mut new_area = self.gva_areas.remove(&after_start).unwrap();
                let unmap_area = new_area.shrink_left(after_end.sub_addr(end));
                warn!("Shrinking left range {:?}", unmap_area);
                unmap_areas.push(unmap_area);
                assert_eq!(new_area.start(), end);
                self.gva_areas.insert(end, new_area);
            }
        }

        for vaddr_range in unmap_areas {
            debug!("Unmapping GVA range: {:?}", vaddr_range);
            for vaddr in PageIter4K::new(vaddr_range.start(), vaddr_range.end()).unwrap() {
                let (_paddr, _page_size) = self.unmap(vaddr).map_err(paging_err_to_ax_err)?;
            }
        }

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
            let _tlb = self
                .guest_map(vaddr, paddr, page_size, flags)
                .inspect_err(|e| {
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
