use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::sync::Mutex;

use axerrno::{AxResult, ax_err_type};
use memory_addr::{MemoryAddr, PAGE_SIZE_4K, is_aligned_4k};
use page_table_multiarch::{PageSize, PagingHandler};

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, HostPhysAddr, HostVirtAddr, MappingFlags};
use axvcpu::AxVcpuAccessGuestState;
use axvm::HostContext;

use crate::libos::def::{
    GP_EPTP_LIST_REGION_BASE, INSTANCE_SHARED_REGION_BASE, ProcessMemoryRegion, USER_STACK_BASE,
    USER_STACK_SIZE,
};
use crate::libos::gaddrspace::{GuestAddrSpace, GuestMappingType};
use crate::libos::process::Process;
use crate::vmm::VCpu;
use crate::vmm::config::{get_instance_cpus, get_instance_cpus_mask};

// How to init gate instance
// First, init addrspace on one core.
// can we use remap pfn range???
// the most simplist way is that axcli to extablish it's own mapping first, can we can copy from it's memory region.
// Then we can reuse the `CMemoryRegion` and `ProcessMemoryRegion`.
// Next, fork each gate process on each core.

static INSTANCES: Mutex<BTreeMap<usize, InstanceRef>> = Mutex::new(BTreeMap::new());

/// The reference type of a task.
pub type InstanceRef = Arc<Instance<PagingHandlerImpl>>;

pub struct Instance<H: PagingHandler> {
    /// The ID of the instance.
    id: usize,
    /// The list of processes in the instance.
    pub processes: Mutex<BTreeMap<HostPhysAddr, Process<H>>>,
    /// The initialized context of the instance's first process.
    /// It is used to initialize the vCPU context for the first process.
    /// See `init_gate_processes` for details.
    ctx: HostContext,
}

impl<H: PagingHandler> Instance<H> {
    /// Create a new instance alone with its first process.
    /// The first process is initialized by the ELF segments in `elf_regions`
    /// with a newly constructed `GuestAddrSpace`.
    /// The `ctx` is used to initialize the vCPU context for the first process.
    pub fn new(
        id: usize,
        elf_regions: Vec<ProcessMemoryRegion>,
        mut ctx: HostContext,
        mapping_type: GuestMappingType,
    ) -> AxResult<Arc<Self>> {
        debug!("Generate instance {}", id);
        let mut init_addrspace = GuestAddrSpace::new(mapping_type)?;

        // Parse and copy ELF segments to guest process's address space.
        // Todo: distinguish shared regions.
        for p_region in &elf_regions {
            if !p_region.gva.is_aligned_4k() || !is_aligned_4k(p_region.size) {
                warn!(
                    "Process memory region [{:?} - {:?}] {:?} is not aligned to 4K, skipping",
                    p_region.gva,
                    p_region.gva + p_region.size,
                    p_region.flags
                );
                continue;
            }

            for (p_gva, mapping) in &p_region.mappings {
                trace!(
                    "Processing instance memory region GVA [{:?} - {:?}] {:?}",
                    p_region.gva,
                    p_region.gva + p_region.size,
                    p_region.flags
                );

                if let Some(mapping) = mapping {
                    if mapping.hpa.is_none() {
                        error!(
                            "Process memory region GVA [{:?} - {:?}] {:?} GPA {:?} is not mapped by hypervisor, skipping",
                            p_region.gva,
                            p_region.gva + PAGE_SIZE_4K,
                            mapping.gpa,
                            p_region.flags
                        );
                        continue;
                    }

                    // Map as populated mapping to ensure that the memory mapping is established.
                    init_addrspace.guest_map_alloc(
                        *p_gva,
                        mapping.page_size as usize,
                        p_region.flags,
                        true,
                    )?;

                    let host_region_hpa = mapping.hpa.unwrap();

                    init_addrspace.copy_into_guest(
                        H::phys_to_virt(host_region_hpa),
                        *p_gva,
                        mapping.page_size.into(),
                    )?;
                } else {
                    warn!(
                        "Process memory region [{:?} - {:?}] {:?} is not mapped by Linux, skipping",
                        p_gva,
                        p_region.gva + PAGE_SIZE_4K,
                        p_region.flags
                    );
                }
            }
        }

        // Setup guest process's stack region.
        init_addrspace.guest_map_alloc(
            USER_STACK_BASE,
            USER_STACK_SIZE,
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
            true,
        )?;

        // Manipulate guest process's context.
        // `ctx.rip` has been set in `create_instance` in hypercall execution.
        // Todo: manipulate IDT and syscall entry point.
        ctx.cr3 = init_addrspace.guest_page_table_root_gpa().as_usize() as u64;
        ctx.rsp = USER_STACK_BASE.add(USER_STACK_SIZE).as_usize() as u64;

        let init_ept_root_hpa = init_addrspace.ept_root_hpa();

        info!("Instance {}: init eptp at: {:?}", id, init_ept_root_hpa);

        let mut processes = BTreeMap::new();
        let init_process = Process::new(0, init_addrspace);
        processes.insert(init_ept_root_hpa, init_process);
        Ok(Arc::new(Self {
            id,
            processes: Mutex::new(processes),
            ctx,
        }))
    }

    pub fn id(&self) -> usize {
        self.id
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
            "Instance {}: init_gate_processes: processes should be 1",
            self.id()
        );

        let mut init_process_entry = processes.first_entry().unwrap();
        let init_process = init_process_entry.get_mut();

        init_process.set_pid(cpu_ids[0]);

        let mut secondary_gate_processes = Vec::new();
        // Fork gate process on each core from init process.
        for i in 1..get_instance_cpus() {
            let cpu_id = cpu_ids[i];
            secondary_gate_processes.push(init_process.fork(cpu_id)?);
        }

        for sgp in secondary_gate_processes {
            let sgp_hpa = sgp.addrspace_root();
            processes.insert(sgp_hpa, sgp);
        }

        // Set up gate process on each core.
        for (_as_root, p) in processes.iter_mut() {
            let cpu_id = p.pid();

            let gp_as = p.addrspace_mut();

            // Init vCPU for each core.
            let vcpu = VCpu::new(cpu_id, 0, Some(1 << cpu_id), cpu_id)?;
            vcpu.setup_from_context(gp_as.ept_root_hpa(), self.ctx.clone())?;

            // Alloc and map percpu instance shared region.
            let shared_region_base_hpa = H::alloc_frame().ok_or_else(|| ax_err_type!(NoMemory))?;
            gp_as.ept_map_linear(
                INSTANCE_SHARED_REGION_BASE,
                shared_region_base_hpa,
                PAGE_SIZE_4K,
                MappingFlags::READ | MappingFlags::WRITE,
                false,
            )?;

            // Map the EPTP list region for gate process.
            let gp_eptp_list_base_hpa = vcpu.get_arch_vcpu().eptp_list_region();
            gp_as.ept_map_linear(
                GP_EPTP_LIST_REGION_BASE,
                gp_eptp_list_base_hpa,
                PAGE_SIZE_4K,
                MappingFlags::READ | MappingFlags::WRITE,
                false,
            )?;

            crate::libos::percpu::init_instance_percore_task(
                cpu_id,
                Arc::new(vcpu),
                shared_region_base_hpa,
            );
        }

        Ok(())
    }
}

impl<H: PagingHandler> Drop for Instance<H> {
    fn drop(&mut self) {
        info!("Destroy instance {}", self.id);
        if self.id == 0 {
            warn!("You are dropping gate instance, you'd better know what you are doing!");
        }
    }
}

/// Create a new instance.
/// This function will create a new instance and allocate a task for the vCPU.
pub fn create_instance(
    id: usize,
    process_regions: Vec<ProcessMemoryRegion>,
    ctx: HostContext,
    mapping_type: GuestMappingType,
) -> AxResult {
    if INSTANCES.lock().contains_key(&id) {
        return Err(ax_err_type!(InvalidInput, "Instance ID already exists"));
    }

    let instance_ref = Instance::<PagingHandlerImpl>::new(id, process_regions, ctx, mapping_type)?;

    INSTANCES.lock().insert(id, instance_ref.clone());

    if id == 0 {
        instance_ref.init_gate_processes()?;
    }

    Ok(())
}

pub fn remove_instance(id: usize) -> AxResult {
    info!("Removing instance {}", id);

    let mut instances = INSTANCES.lock();
    if let Some(instance) = instances.remove(&id) {
        drop(instance);
        Ok(())
    } else {
        Err(ax_err_type!(InvalidInput, "Instance ID not found"))
    }
}

pub fn get_instances_by_id(id: usize) -> Option<InstanceRef> {
    INSTANCES.lock().get(&id).cloned()
}
