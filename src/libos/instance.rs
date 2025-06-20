use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use lazyinit::LazyInit;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::sync::Mutex;

use axerrno::{AxResult, ax_err, ax_err_type};
use bitmaps::Bitmap;
use memory_addr::{MemoryAddr, PAGE_SIZE_4K};
use page_table_multiarch::{PageSize, PagingHandler};

use axaddrspace::npt::EPTPointer;
use axaddrspace::{GuestPhysAddr, GuestVirtAddr, HostPhysAddr, HostVirtAddr, MappingFlags};
use axvcpu::AxVcpuAccessGuestState;
use equation_defs::context::TaskContext;
use equation_defs::task::EqTask;
use equation_defs::{
    FIRST_PROCESS_ID, GuestMappingType, INSTANCE_PERCPU_REGION_SIZE, InstanceRegion, InstanceType,
    MAX_CPUS_NUM, MAX_INSTANCES_NUM, SHIM_INSTANCE_ID,
};

use crate::libos::config::get_gate_process_data;
use crate::libos::def::{
    EPTP_LIST_REGION_SIZE, EPTPList, GP_ALL_EPTP_LIST_REGIN_GPA, GP_ALL_EPTP_LIST_REGION_GVA,
    GP_ALL_INSTANCE_PERCPU_REGION_GPA, GP_ALL_INSTANCE_PERCPU_REGION_GVA,
    GP_PERCPU_EPT_LIST_REGION_GVA, INSTANCE_REGION_SIZE, PERCPU_EPTP_LIST_REGION_GPA,
    PERCPU_REGION_BASE_GPA, PERCPU_REGION_BASE_GVA, PERCPU_REGION_SIZE,
};
use crate::libos::gaddrspace::{GuestAddrSpace, init_shim_kernel};
use crate::libos::process::Process;
use crate::libos::region::{HostPhysicalRegion, HostPhysicalRegionRef};
use crate::vmm::VCpu;
use crate::vmm::config::{get_instance_cpus, get_instance_cpus_mask};

/// Maximum number of processes in an instance,
/// Intel's EPTP List can only support 512 EPTP entries,
/// since the first entry is reserved for the gate process,
/// theoretically, we can only have 511 processes in an instance.
const MAX_PROCESS_NUM: usize = 512;

static INSTANCE_ID_MAP: Mutex<Bitmap<MAX_INSTANCES_NUM>> = Mutex::new(Bitmap::new());
static INSTANCES: Mutex<BTreeMap<usize, InstanceRef>> = Mutex::new(BTreeMap::new());

static INSTANCES_EPTP_LIST_REGIONS: LazyInit<HostPhysicalRegion<PagingHandlerImpl>> =
    LazyInit::new();
static PERCPU_REGIONS: LazyInit<HostPhysicalRegion<PagingHandlerImpl>> = LazyInit::new();
static INSTANCE_REGION_POOL: LazyInit<
    [HostPhysicalRegionRef<PagingHandlerImpl>; MAX_INSTANCES_NUM],
> = LazyInit::new();

/// The reference type of a task.
pub type InstanceRef = Arc<Instance<PagingHandlerImpl>>;

fn get_instance_id() -> AxResult<usize> {
    let mut instance_id_map = INSTANCE_ID_MAP.lock();
    let id = match instance_id_map.first_false_index() {
        Some(id) => id,
        None => {
            warn!("Instance ID overflow");
            return ax_err!(ResourceBusy, "Instance ID overflow");
        }
    };
    instance_id_map.set(id, true);
    Ok(id)
}

fn free_instance_id(id: usize) -> AxResult {
    let mut instance_id_map = INSTANCE_ID_MAP.lock();
    if !instance_id_map.get(id) {
        warn!("Instance ID {} is already free", id);
        return ax_err!(BadState, "Instance ID is already free");
    }
    instance_id_map.set(id, false);
    Ok(())
}

pub struct Instance<H: PagingHandler> {
    /// The region for instance inner data, which is shared by all processes in the instance,
    /// it stores the instance ID.
    instance_region_base: HostPhysAddr,
    /// The bitmap of process IDs in the instance,
    /// - `true` means the PID is used.
    /// - `false` means the PID is free.
    pid_bitmap: Mutex<Bitmap<MAX_PROCESS_NUM>>,
    /// The list of processes in the instance.
    pub processes: Mutex<BTreeMap<HostPhysAddr, Process<H>>>,
    /// The region for EPTP list.
    /// Allocated from `INSTANCES_EPTP_LIST_REGIONS` according to the instance ID.
    eptp_list_region: HostPhysAddr,
    /// Dirty flag for EPTP list, if `true`, the EPTP list has been modified.
    /// This flag is used to indicate that the EPTP list needs to be updated in the vCPUs.
    eptp_list_dirty: AtomicBool,
}

impl<H: PagingHandler> Instance<H> {
    pub fn id(&self) -> usize {
        self.instance_region().instance_id as usize
    }

    pub fn instance_region(&self) -> &InstanceRegion {
        unsafe {
            H::phys_to_virt(self.instance_region_base)
                .as_ptr_of::<InstanceRegion>()
                .as_ref()
                .expect("Failed to get instance region")
        }
    }

    #[allow(unused)]
    fn instance_inner_region_mut(&self) -> &mut InstanceRegion {
        unsafe {
            H::phys_to_virt(self.instance_region_base)
                .as_mut_ptr_of::<InstanceRegion>()
                .as_mut()
                .expect("Failed to get instance inner region")
        }
    }
}

impl<H: PagingHandler> Instance<H> {
    pub fn create_shim() -> AxResult<Arc<Self>> {
        let id = get_instance_id()?;

        if id != SHIM_INSTANCE_ID {
            return ax_err!(BadState, "Shim instance has been created");
        }

        // Init shim kernel, loading shim binary and setting up the `GLOBAL_SHIM_REGION`.
        init_shim_kernel()?;

        // Init instances eptp list regions.
        INSTANCES_EPTP_LIST_REGIONS.init_once(HostPhysicalRegion::allocate(
            EPTP_LIST_REGION_SIZE * MAX_INSTANCES_NUM,
            Some(PAGE_SIZE_4K),
        )?);
        // Init per CPU regions.
        PERCPU_REGIONS.init_once(HostPhysicalRegion::allocate(
            PERCPU_REGION_SIZE * MAX_CPUS_NUM,
            Some(PAGE_SIZE_4K),
        )?);
        // Init instance region pool.
        INSTANCE_REGION_POOL.init_once(core::array::from_fn(|id| {
            let region = HostPhysicalRegion::allocate_ref(INSTANCE_REGION_SIZE, Some(PAGE_SIZE_4K))
                .expect("Failed to allocate instance region");
            let instance_region_mut =
                unsafe { region.as_mut_ptr_of::<InstanceRegion>().as_mut() }.unwrap();
            instance_region_mut.instance_id = id as u64;
            region
        }));

        warn!(
            "PERCPU_REGIONS range [{:?}, {:?}]",
            PERCPU_REGIONS.base(),
            PERCPU_REGIONS.base() + PERCPU_REGIONS.size()
        );

        // Gate processses' pid is equal to its running CPU ID.
        let pid = get_instance_cpus_mask().first_index().ok_or_else(|| {
            warn!("No CPU available for instance");
            ax_err_type!(InvalidInput, "No CPU available for instance")
        })?;

        info!("Init shim instance, first process ID: {}", pid);

        // Get the instance region from the instance region pool.
        let instance_region_ref = INSTANCE_REGION_POOL[id].clone();
        let instance_region_base = instance_region_ref.base();

        let mut shim_addrspace = GuestAddrSpace::new(
            pid,
            instance_region_base,
            GuestMappingType::CoarseGrainedSegmentation2M,
        )?;

        // Load elf data for shim process.
        let gate_process_data = get_gate_process_data();
        let _ctx = shim_addrspace.setup_user_elf(gate_process_data)?;

        let init_ept_root_hpa = shim_addrspace.ept_root_hpa();

        info!(
            "Shim instance {}: init eptp at: {:?}",
            id, init_ept_root_hpa
        );

        let mut processes = BTreeMap::new();
        let init_process = Process::new(pid, shim_addrspace);
        // Other processes, including the gate processes, may be forked from this process.
        processes.insert(init_ept_root_hpa, init_process);

        // Shim instance does not need a specific EPTP list region,
        // because the first (index 0) EPTP entry in each vCPU's EPTP list is always
        // the gate process's EPTP.
        // So we just allocate a dummy region for it.
        let dummy_eptp_list_region = HostPhysAddr::from_usize(0);

        // Set the first process ID to `true` in the bitmap.
        let mut pid_bitmap = Bitmap::mask(FIRST_PROCESS_ID);
        // Set the gate process ID to `true` in the bitmap.
        pid_bitmap.set(pid, true);

        Ok(Arc::new(Self {
            pid_bitmap: Mutex::new(pid_bitmap),
            processes: Mutex::new(processes),
            instance_region_base,
            eptp_list_region: dummy_eptp_list_region,
            // Shim instance's EPTP list is always NOT dirty.
            eptp_list_dirty: AtomicBool::new(false),
        }))
    }

    /// Create a new instance alone with its first process.
    /// The first process is initialized by the ELF segments in `elf_regions`
    /// with a newly constructed `GuestAddrSpace`.
    pub fn create(
        itype: InstanceType,
        mapping_type: GuestMappingType,
        raw_file: Vec<u8>,
    ) -> AxResult<(Arc<Self>, TaskContext)> {
        let id = get_instance_id()?;

        if id == SHIM_INSTANCE_ID {
            return ax_err!(BadState, "Shim instance should be created first");
        }

        info!("Generating {:?} instance {} {:?}", itype, id, mapping_type);

        // Get the instance region from the instance region pool.
        let instance_region_ref = INSTANCE_REGION_POOL[id].clone();
        let instance_region_base = instance_region_ref.base();
        let instance_region =
            unsafe { instance_region_ref.as_ptr_of::<InstanceRegion>().as_ref() }.unwrap();
        if instance_region.instance_id != id as u64 {
            error!(
                "Instance region ID mismatch: expected {}, got {}, there is some bug in instance region pool",
                id, instance_region.instance_id
            );
            return ax_err!(BadState, "Instance region ID mismatch");
        }
        info!("Instance {}: region {:?}", id, instance_region_ref);

        let mut init_addrspace =
            GuestAddrSpace::new(FIRST_PROCESS_ID, instance_region_base, mapping_type)?;

        let task_context = match itype {
            InstanceType::LibOS => {
                // init_addrspace.setup_kernel_stack_frame()?;

                // Init process's context frame.
                // Load ELF data for libos process and setup libos process's stack region.
                init_addrspace.setup_user_elf(raw_file.as_ref())?
            }
            InstanceType::Kernel => {
                init_addrspace.setup_kernel_stack_frame()?;
                let (rsp_gva, kstack_top_gva) = init_addrspace
                    .process_inner_region_mut()
                    .kernel_context_rsp_stack_top_gva();
                TaskContext {
                    kstack_top: HostVirtAddr::from_usize(kstack_top_gva),
                    rsp: rsp_gva as _,
                    fs_base: 0,
                }
            }
        };

        let init_ept_root_hpa = init_addrspace.ept_root_hpa();

        info!("Instance {}: init eptp at: {:?}", id, init_ept_root_hpa);

        let mut processes = BTreeMap::new();
        let init_process = Process::new(FIRST_PROCESS_ID, init_addrspace);
        // Other processes, including the gate processes, may be forked from this process.
        processes.insert(init_ept_root_hpa, init_process);

        let eptp_list_region = INSTANCES_EPTP_LIST_REGIONS
            .get()
            .expect("EPTP list region uninitialized")
            .base()
            .add(id * EPTP_LIST_REGION_SIZE);

        let eptp_list = unsafe {
            H::phys_to_virt(eptp_list_region)
                .as_mut_ptr_of::<EPTPList>()
                .as_mut()
        }
        .expect("Failed to get EPTP list");
        eptp_list.set(
            FIRST_PROCESS_ID,
            EPTPointer::from_table_phys(init_ept_root_hpa),
        );

        Ok((
            Arc::new(Self {
                pid_bitmap: Mutex::new(Bitmap::mask(FIRST_PROCESS_ID)),
                instance_region_base,
                eptp_list_region,
                eptp_list_dirty: AtomicBool::new(false),
                processes: Mutex::new(processes),
            }),
            task_context,
        ))
    }

    pub fn alloc_pid(&self) -> Option<usize> {
        let mut pid_bitmap = self.pid_bitmap.lock();

        if let Some(pid) = pid_bitmap.first_false_index() {
            pid_bitmap.set(pid, true);
            Some(pid)
        } else {
            None
        }
    }

    pub fn guest_phys_to_host_phys(
        &self,
        eptp: HostPhysAddr,
        gpa: GuestPhysAddr,
    ) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
        self.processes
            .lock()
            .get(&eptp)
            .and_then(|process| process.addrspace().translate(gpa))
    }

    pub fn read_from_guest(
        &self,
        eptp: HostPhysAddr,
        gva: GuestVirtAddr,
        size: usize,
    ) -> AxResult<Vec<u8>> {
        let processes = self.processes.lock();

        let process = processes.get(&eptp).ok_or_else(|| {
            warn!("EPTP {:?} not found in processes", eptp);
            ax_err_type!(InvalidInput, "Invalid EPTP")
        })?;

        let mut contents = vec![0; size];

        process.addrspace().copy_from_guest(
            gva,
            HostVirtAddr::from_mut_ptr_of(contents.as_mut_ptr()),
            size,
        )?;

        Ok(contents)
    }

    pub fn handle_ept_page_fault(
        &self,
        eptp: HostPhysAddr,
        addr: GuestPhysAddr,
        access_flags: MappingFlags,
    ) -> AxResult<bool> {
        // Handle the page fault in the process's address space.
        self.processes
            .lock()
            .get_mut(&eptp)
            .ok_or_else(|| {
                warn!("EPTP {:?} not found in processes", eptp);
                ax_err_type!(InvalidInput, "Invalid EPTP")
            })?
            .addrspace_mut()
            .handle_ept_page_fault(addr, access_flags)
    }

    /// Handle the clone hypercall to create a new process.
    /// This function will fork the process with the given EPTP and return the new EPTP's index.
    pub fn handle_clone(&self, eptp: HostPhysAddr) -> AxResult<usize> {
        let new_pid = self.alloc_pid().ok_or_else(|| {
            warn!("Process ID overflow");
            ax_err_type!(ResourceBusy, "Process ID overflow")
        })?;

        let mut processes = self.processes.lock();

        let cur_process = processes.get_mut(&eptp).ok_or_else(|| {
            warn!("EPTP {:?} not found in processes", eptp);
            ax_err_type!(InvalidInput, "Invalid EPTP")
        })?;

        let new_process = cur_process.fork(new_pid)?;
        let new_eptp = EPTPointer::from_table_phys(new_process.ept_root());
        let new_pid = new_process.pid();
        processes.insert(new_eptp.into_ept_root(), new_process);

        // Update the EPTP list of the instance.
        if !self.manipulate_eptp_list(|eptp_list| eptp_list.set(new_pid, new_eptp)) {
            warn!(
                "Instance[{}] failed to set EPTP {} for new process",
                self.id(),
                new_pid
            );
            return Err(ax_err_type!(BadState, "Failed to set EPTP list"));
        }

        Ok(new_pid)
    }

    pub fn remove_process(&self, eptp: HostPhysAddr) -> AxResult {
        let removed_process = self.processes.lock().remove(&eptp).ok_or_else(|| {
            warn!("EPTP {:?} not found in processes", eptp);
            ax_err_type!(InvalidInput, "Invalid EPTP")
        })?;

        match self.get_eptp_list_mut().remove(removed_process.pid()) {
            Some(removed_eptp) => {
                if removed_eptp.into_ept_root() != eptp {
                    warn!(
                        "Process [{}] EPTP {:?} is not the same as the removed EPTP {:?}",
                        removed_process.pid(),
                        eptp,
                        removed_eptp
                    );
                    return Err(ax_err_type!(BadState, "Invalid EPTP list"));
                }
                // Successfully removed the EPTP from the list.
            }
            None => {
                warn!(
                    "Failed to remove Process[{}]'s EPTP {:?} from instance {}",
                    removed_process.pid(),
                    eptp,
                    self.id()
                );
                return Err(ax_err_type!(BadState, "Failed to remove EPTP list"));
            }
        }

        if self.processes.lock().is_empty() {
            // If there are no processes left in the instance,
            // if the instance has no running tasks,
            // it means that all processes have exited and
            // the instance is no longer needed, so
            // we can remove the instance.

            if self.instance_region().running_tasks_count() == 0 {
                info!("No more running tasks in instance [{}]", self.id());
                remove_instance(self.id())?;
            }
        }
        Ok(())
    }

    /// Get the EPTP list of the instance.
    pub fn get_eptp_list(&self) -> &EPTPList {
        unsafe {
            H::phys_to_virt(self.eptp_list_region)
                .as_ptr_of::<EPTPList>()
                .as_ref()
        }
        .expect("Failed to get EPTP list")
    }

    /// Check if the EPTP list is dirty,
    /// if it is dirty, it will be reset to false.
    ///
    /// This function is used to check if the EPTP list has been modified
    /// and needs to be updated in the vCPUs.
    pub fn eptp_list_dirty(&self) -> bool {
        self.eptp_list_dirty
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }
}

impl<H: PagingHandler> Instance<H> {
    fn init_gate_processes(&self) -> AxResult {
        info!("Instance {}: init gate processes", self.id());

        let instance_cpu_mask = get_instance_cpus_mask();
        let cpu_ids: Vec<usize> = instance_cpu_mask.into_iter().collect();

        if cpu_ids.len() != get_instance_cpus() {
            return Err(ax_err_type!(InvalidData, "Incorrect CPU mask"));
        }

        let mut processes = self.processes.lock();

        assert_eq!(
            processes.len(),
            1,
            "Instance {}: init_gate_processes: processes num should be 1",
            self.id()
        );

        let mut init_process_entry = processes.first_entry().unwrap();
        let init_process = init_process_entry.get_mut();

        if cpu_ids[0] != init_process.pid() {
            return Err(ax_err_type!(BadState, "Incorrect CPU ID for init process"));
        }

        let mut secondary_gate_processes = Vec::new();

        // Fork gate process on each core from init process.
        for i in 1..get_instance_cpus() {
            let cpu_id = cpu_ids[i];
            secondary_gate_processes.push(init_process.fork(cpu_id)?);
        }

        // Insert forked gate processes into the processes map, indexed by their EPT root HPA.
        for sgp in secondary_gate_processes {
            let sgp_hpa = sgp.ept_root();
            processes.insert(sgp_hpa, sgp);
        }

        // Set up gate process on each core.
        for (_as_root, p) in processes.iter_mut() {
            let cpu_id = p.pid();

            let gp_as = p.addrspace_mut();

            // Init vCPU for each core.
            let vcpu = Arc::new(VCpu::new(cpu_id, 0, Some(1 << cpu_id), cpu_id)?);

            // The PerCPU region is allocated once globaly and mapped to each guest address space,
            // each vCPU will have its own region based on the CPU ID.
            let percpu_region = PERCPU_REGIONS
                .get()
                .expect("PERCPU_REGIONS uninitialized")
                .base()
                .add(cpu_id * PERCPU_REGION_SIZE);

            // Map the PerCPU region for gate process.
            // GVA -> GPA
            gp_as
                .guest_map_region(
                    PERCPU_REGION_BASE_GVA,
                    |gva| PERCPU_REGION_BASE_GPA.add(gva.sub_addr(PERCPU_REGION_BASE_GVA)),
                    PERCPU_REGION_SIZE,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                    false,
                    false,
                )
                .map_err(super::gaddrspace::paging_err_to_ax_err)?;
            // GPA -> HPA
            gp_as.ept_map_linear(
                PERCPU_REGION_BASE_GPA,
                percpu_region,
                PERCPU_REGION_SIZE,
                MappingFlags::READ | MappingFlags::WRITE,
                false,
            )?;

            // Map the percpu EPTP list region for gate process.
            // GVA -> GPA
            gp_as
                .guest_map_region(
                    GP_PERCPU_EPT_LIST_REGION_GVA,
                    |_gva| PERCPU_EPTP_LIST_REGION_GPA,
                    EPTP_LIST_REGION_SIZE,
                    MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                    false,
                    false,
                )
                .map_err(super::gaddrspace::paging_err_to_ax_err)?;
            // GPA -> HPA
            let gp_eptp_list_base_hpa = vcpu.get_arch_vcpu().eptp_list_region();
            gp_as.ept_map_linear(
                PERCPU_EPTP_LIST_REGION_GPA,
                gp_eptp_list_base_hpa,
                EPTP_LIST_REGION_SIZE,
                MappingFlags::READ | MappingFlags::WRITE,
                false,
            )?;

            // Map all instances' EPTP list region for gate process.
            // GVA -> GPA
            gp_as
                .guest_map_region(
                    GP_ALL_EPTP_LIST_REGION_GVA,
                    |gva| GP_ALL_EPTP_LIST_REGIN_GPA.add(gva.sub_addr(GP_ALL_EPTP_LIST_REGION_GVA)),
                    EPTP_LIST_REGION_SIZE * MAX_INSTANCES_NUM,
                    // Map as UNCACHED to make sure the update of EPTP list is visible to VMX immediately.
                    MappingFlags::READ
                        | MappingFlags::WRITE
                        | MappingFlags::UNCACHED
                        | MappingFlags::USER,
                    true,
                    false,
                )
                .map_err(super::gaddrspace::paging_err_to_ax_err)?;
            // GPA -> HPA
            gp_as.ept_map_linear(
                GP_ALL_EPTP_LIST_REGIN_GPA,
                INSTANCES_EPTP_LIST_REGIONS
                    .get()
                    .expect("INSTANCES_EPTP_LIST_REGIONS uninitialized")
                    .base(),
                EPTP_LIST_REGION_SIZE * MAX_INSTANCES_NUM,
                // Map as UNCACHED to make sure the update of EPTP list is visible to VMX immediately.
                MappingFlags::READ | MappingFlags::WRITE | MappingFlags::UNCACHED,
                true,
            )?;

            // Map all instances' perCPU region for gate process.
            for i in 0..MAX_INSTANCES_NUM {
                let instance_percpu_region_base_gva =
                    GP_ALL_INSTANCE_PERCPU_REGION_GVA.add(i * INSTANCE_PERCPU_REGION_SIZE);
                let instance_percpu_region_base_gpa =
                    GP_ALL_INSTANCE_PERCPU_REGION_GPA.add(i * INSTANCE_PERCPU_REGION_SIZE);
                // _____________________________
                // | InstanceRegion				|
                // |____________________________|
                // | [							|
                // |     InstancePerCPURegion0,	|
                // |     InstancePerCPURegion1,	|
                // |	 InstancePerCPURegion2,	|
                // |     ...
                // |     InstancePerCPURegionN	|
                // | ]							|
                // |____________________________|
                let instance_percpu_region_base_hpa = INSTANCE_REGION_POOL[i]
                    .base()
                    .add(cpu_id * INSTANCE_PERCPU_REGION_SIZE);

                // GVA -> GPA
                gp_as
                    .guest_map_region(
                        instance_percpu_region_base_gva,
                        |gva| {
                            instance_percpu_region_base_gpa
                                .add(gva.sub_addr(instance_percpu_region_base_gva))
                        },
                        INSTANCE_PERCPU_REGION_SIZE,
                        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                        false,
                        false,
                    )
                    .map_err(super::gaddrspace::paging_err_to_ax_err)?;
                // GPA -> HPA
                gp_as.ept_map_linear(
                    instance_percpu_region_base_gpa,
                    instance_percpu_region_base_hpa,
                    INSTANCE_PERCPU_REGION_SIZE,
                    MappingFlags::READ | MappingFlags::WRITE,
                    false,
                )?;
            }

            // Init each vCPU's EPTP list region, set the first entry to the gate process's EPTP.
            let vcpu_eptp_list_region_hva = H::phys_to_virt(gp_eptp_list_base_hpa);
            let vcpu_eptp_list_region = EPTPList::construct(vcpu_eptp_list_region_hva)
                .expect("Failed to construct vcpu EPTP list");
            vcpu_eptp_list_region.set_gate_entry(EPTPointer::from_table_phys(gp_as.ept_root_hpa()));

            crate::libos::percpu::init_instance_percore_task(cpu_id, vcpu, percpu_region);
        }

        Ok(())
    }

    fn get_eptp_list_mut(&self) -> &mut EPTPList {
        unsafe {
            H::phys_to_virt(self.eptp_list_region)
                .as_mut_ptr_of::<EPTPList>()
                .as_mut()
        }
        .expect("Failed to get EPTP list")
    }

    fn manipulate_eptp_list<R>(&self, f: impl FnOnce(&mut EPTPList) -> R) -> R {
        let result = f(self.get_eptp_list_mut());

        // Mark the EPTP list as dirty to indicate that it has been modified.
        // we need to update the EPTP list of all the vCPUs that this instance are
        // running on.
        self.eptp_list_dirty.store(true, Ordering::Release);
        result
    }
}

impl<H: PagingHandler> Drop for Instance<H> {
    fn drop(&mut self) {
        info!("Destroy instance {}", self.id());
        if self.id() == 0 {
            warn!("You are dropping gate instance, you'd better know what you are doing!");
        }
    }
}

/// Pick a CPU for the instance by traversing all InstanceRegions from INSTANCE_REGION_POOL.
/// The CPU that has the minimum number of running tasks will be selected.
fn pick_cpu_for_instance() -> AxResult<usize> {
    // Get all instance IDs from the INSTANCE_ID_MAP.
    let instance_ids = INSTANCE_ID_MAP.lock().into_iter().collect::<Vec<usize>>();
    // Filter out the InstanceRegions that are not in use (i.e., instance IDs that are not set in the bitmap).
    let instance_regions: Vec<&InstanceRegion> = instance_ids
        .iter()
        .map(|&id| {
            let region = &INSTANCE_REGION_POOL[id];
            unsafe { region.as_ptr_of::<InstanceRegion>().as_ref() }
                .expect("Failed to get instance region")
        })
        .collect();

    let cpu_mask = get_instance_cpus_mask();
    let cpu_ids = cpu_mask.into_iter().collect::<Vec<usize>>();
    let mut cpu_task_counts = vec![0; cpu_ids.len()];
    for instance_region in instance_regions {
        for (i, &cpu_id) in cpu_ids.iter().enumerate() {
            cpu_task_counts[i] += instance_region.percpu_regions[cpu_id]
                .run_queue
                .get_task_num();
        }
    }

    // Find the CPU with the minimum number of running tasks.
    let min_cpu_index = cpu_task_counts
        .iter()
        .enumerate()
        .min_by_key(|&(_, &count)| count)
        .map(|(index, _)| index)
        .ok_or_else(|| {
            warn!("No CPU available for instance");
            ax_err_type!(InvalidInput, "No CPU available for instance")
        })?;

    Ok(cpu_ids[min_cpu_index])
}

/// Create a new instance.
/// This function will create a new instance and setup its address space according
/// to the instance type and binary/executable file.
///
/// This function will return the instance ID if the instance is created successfully.
pub fn create_instance(
    itype: InstanceType,
    mapping_type: GuestMappingType,
    raw_file: Vec<u8>,
) -> AxResult<usize> {
    let (instance_ref, task_context) =
        Instance::<PagingHandlerImpl>::create(itype, mapping_type, raw_file)?;
    let iid = instance_ref.id();
    INSTANCES.lock().insert(iid, instance_ref);

    let init_task = EqTask {
        instance_id: iid,
        process_id: FIRST_PROCESS_ID,
        task_id: FIRST_PROCESS_ID,
        context: task_context,
    };

    let target_core = pick_cpu_for_instance()?;

    info!(
        "Creating instance {} on core {} with task {:?}",
        iid, target_core, init_task
    );
    // Add the init task to the run queue of the target core.
    unsafe {
        INSTANCE_REGION_POOL[iid]
            .as_mut_ptr_of::<InstanceRegion>()
            .as_mut()
    }
    .expect("Failed to get instance region")
    .percpu_regions[target_core]
        .run_queue
        .insert(init_task)
        .map_err(|e| {
            warn!("Failed to insert init task into run queue: {:?}", e);
            ax_err_type!(BadState, "Failed to insert init task into run queue")
        })?;

    crate::libos::percpu::set_next_instance_id_of_cpu(target_core, iid)?;

    // TODO: remove this when we have a better way to send IPI.
    use std::os::arceos::modules::axhal::irq::{IPI_IRQ_NUM, send_ipi_one};
    send_ipi_one(target_core, IPI_IRQ_NUM);

    Ok(iid)
}

pub fn init_shim() -> AxResult {
    let shim_instance = Instance::<PagingHandlerImpl>::create_shim()?;
    INSTANCES
        .lock()
        .insert(shim_instance.id(), shim_instance.clone());
    shim_instance.init_gate_processes()?;
    Ok(())
}

use crate::vmm::VCpuRef;

pub fn shutdown_instance(
    vcpu: &VCpuRef,
    instance_ref: &InstanceRef,
    gate_eptp: EPTPointer,
) -> AxResult {
    let cpu_id = vcpu.id();
    let instance_id = instance_ref.id();
    info!(
        "CPU {} Shutting down instance {}",
        cpu_id,
        instance_ref.id()
    );

    // Get the context of gate task so that this vCPU can return to the gate task
    // directly after next vCPU.run().
    let gate_task = instance_ref.instance_region().percpu_regions[cpu_id].gate_task();

    // // Update the vCPU's EPT pointer to the gate EPTP list entry.
    // let gate_eptp = curcpu
    //     .get_gate_eptp_list_entry()
    //     .expect("Failed to get gate EPTP list entry");
    vcpu.get_arch_vcpu()
        .set_ept_pointer(gate_eptp)
        .expect("Failed to set EPT pointer for vCPU");
    // Set the vCPU's stack pointer to the gate task's stack pointer.
    vcpu.get_arch_vcpu()
        .set_stack_pointer(gate_task.context.rsp as usize);
    // Set the vCPU's instruction pointer to the gate entry point.
    use crate::libos::config::SHIM_GATE_ENTRY;
    vcpu.get_arch_vcpu().set_instr_pointer(SHIM_GATE_ENTRY);
    vcpu.get_arch_vcpu().set_return_value(cpu_id);
    // SHIM_GATE_ENTRY:
    // // Stack pointer `rsp` is prepared by AxVisor.
    // // Restore callee-saved registers
    // "pop     r15",
    // "pop     r14",
    // "pop     r13",
    // "pop     r12",
    // "pop     rbx",
    // "pop     rbp",
    // // cpu_id is in `rax` (return value),
    // "ret",

    // Notify other CPUs to stop running this instance.
    if instance_ref.instance_region().running_tasks_count() > 0 {
        let instance_running_cpu_mask = instance_ref.instance_region().running_cpu_bitmask();

        for cpu_id in get_instance_cpus_mask().into_iter() {
            if (instance_running_cpu_mask & (1 << cpu_id)) != 0 {
                warn!(
                    "TODO: Notifying CPU {} to stop running instance {}",
                    cpu_id, instance_id
                );
                // TODO: Add logic to notify the CPU to stop running this instance.
            }
        }
    }

    // If there are no more running tasks in this instance,
    // we can safely remove the instance.
    if instance_ref.instance_region().all_tasks_count() == 0 {
        info!("No more running tasks in instance [{}]", instance_id);
        let _ = remove_instance(instance_id).inspect_err(|e| {
            error!("Failed to remove instance [{}]: {:?}", instance_id, e);
        });
    }

    Ok(())
}

pub fn remove_instance(id: usize) -> AxResult {
    info!("Removing instance {}", id);

    let mut instances = INSTANCES.lock();
    if let Some(_instance) = instances.remove(&id) {
        // Drop the instance reference.
        free_instance_id(id)?;
        Ok(())
    } else {
        Err(ax_err_type!(InvalidInput, "Instance ID not found"))
    }
}

pub fn get_instances_by_id(id: usize) -> Option<InstanceRef> {
    INSTANCES.lock().get(&id).cloned()
}
