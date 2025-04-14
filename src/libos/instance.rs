use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::os::arceos::modules::axhal;

use memory_addr::{MemoryAddr, PAGE_SIZE_4K, PhysAddr, is_aligned_4k};
use page_table_entry::x86_64::X64PTE;
use page_table_multiarch::{PagingHandler, PagingMetaData};

use axaddrspace::{AddrSpace, GuestVirtAddr};
use axerrno::{AxResult, ax_err_type};
use axhal::paging::PagingHandlerImpl;
use axstd::sync::Mutex;
use axvm::HostContext;

use crate::libos::def::{ProcessMemoryRegion, ShadowPageTableMetadata};
use crate::libos::process::{INIT_PROCESS_ID, Process, ProcessRef};
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
    id: usize,
    processes: Mutex<Vec<ProcessRef<H>>>,

    ctx: HostContext,

    /// For Stage-1 address translation, which translates guest virtual address to guest physical address,
    /// here we just use a direct one-to-one mapping, so this page table can be used by all processes.
    /// We need to map the page table root (in HPA) to LibOS's CR3 (in GPA) in EPT.
    linear_addrspace: AddrSpace<ShadowPageTableMetadata, X64PTE, H>,
}

impl<H: PagingHandler> Instance<H> {
    pub fn new(
        id: usize,
        elf_regions: Vec<ProcessMemoryRegion>,
        ctx: HostContext,
    ) -> AxResult<Arc<Self>> {
        debug!("Generate instance {}", id);

        let mut linear_addrspace = AddrSpace::new_empty(
            GuestVirtAddr::from_usize(0),
            1 << ShadowPageTableMetadata::VA_MAX_BITS,
        )?;

        let mut init_addrspace = AddrSpace::new_empty(
            GuestVirtAddr::from_usize(0),
            1 << ShadowPageTableMetadata::VA_MAX_BITS,
        )?;

        for p_region in &elf_regions {
            // Map the whole address space as one-to-one mapping by 1G pages.
            // TODO: distinguish user application and LibOS address space.
            linear_addrspace.map_linear(
                p_region.gva,
                PhysAddr::from_usize(p_region.gva.as_usize()),
                p_region.size,
                p_region.flags,
            )?;

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
                    init_addrspace.map_alloc(
                        *p_gva,
                        mapping.page_size as usize,
                        p_region.flags,
                        true,
                    )?;

                    let host_region_hpa = mapping.hpa.unwrap();
                    let (instance_region_hpa, _flags, instance_page_size) = init_addrspace
                        .translate(*p_gva)
                        .expect(alloc::format!("GVA {:#x} not mapped", p_gva).as_str());
                    assert_eq!(
                        instance_page_size, mapping.page_size,
                        "Page size mismatch: {:?} != {:?}",
                        instance_page_size, mapping.page_size
                    );
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            H::phys_to_virt(host_region_hpa).as_ptr(),
                            H::phys_to_virt(instance_region_hpa).as_mut_ptr(),
                            mapping.page_size as usize,
                        )
                    };
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

        let mut processes = Vec::new();
        let init_process = Process::new(INIT_PROCESS_ID, init_addrspace);
        processes.push(init_process);
        Ok(Arc::new(Self {
            id,
            processes: Mutex::new(processes),
            ctx,
            linear_addrspace,
        }))
    }

    pub fn init_gate_processes(&self) -> AxResult {
        info!("Instance {}: init gate processes", self.id());

        let instance_cpu_mask = get_instance_cpus_mask();

        let mut processes = self.processes.lock();
        let cpu_ids: Vec<usize> = instance_cpu_mask.into_iter().collect();

        let first_process = processes[0].clone();

        for i in 1..get_instance_cpus() {
            let process = Process::new(i, first_process.addrspace().clone()?);
            processes.push(process);
        }

        for i in 0..get_instance_cpus() {
            let cpu_id = cpu_ids[i];

            let vcpu = VCpu::new(cpu_id, 0, Some(1 << cpu_id), ())?;
            vcpu.setup_from_context(processes[i].addrspace().page_table_root(), self.ctx.clone())?;

            crate::libos::percpu::init_instance_percore_task(cpu_id, Arc::new(vcpu));
        }

        Ok(())
    }

    pub fn id(&self) -> usize {
        self.id
    }
}

/// Create a new instance.
/// This function will create a new instance and allocate a task for the vCPU.
pub fn create_instance(
    id: usize,
    process_regions: Vec<ProcessMemoryRegion>,
    ctx: HostContext,
) -> AxResult<InstanceRef> {
    if INSTANCES.lock().contains_key(&id) {
        return Err(ax_err_type!(InvalidInput, "Instance ID already exists"));
    }

    let instance_ref = Instance::<PagingHandlerImpl>::new(id, process_regions, ctx)?;

    INSTANCES.lock().insert(id, instance_ref.clone());

    if id == 0 {
        instance_ref.init_gate_processes()?;
    }

    Ok(instance_ref)
}
