use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicBool;
use std::os::arceos::modules::axhal;

use memory_addr::{
    MemoryAddr, PAGE_SIZE_2M, PAGE_SIZE_4K, PageIter2M, PageIter4K, PhysAddr, is_aligned_4k,
};
use page_table_entry::x86_64::X64PTE;
use page_table_multiarch::{PagingHandler, PagingMetaData};

use axaddrspace::{AddrSpace, GuestPhysAddr, GuestVirtAddr};
use axerrno::{AxResult, ax_err_type};
use axhal::paging::PagingHandlerImpl;
use axstd::sync::Mutex;
use axvcpu::{AxArchVCpu, AxVCpuExitReason, AxVcpuAccessGuestState};

use crate::libos::def::{ProcessMemoryRegion, ShadowPageTableMetadata};
use crate::libos::process::{INIT_PROCESS_ID, Process, ProcessRef};
use crate::vmm::VCpuRef;

static INSTANCES: Mutex<BTreeMap<usize, InstanceRef>> = Mutex::new(BTreeMap::new());

/// The reference type of a task.
pub type InstanceRef = Arc<Instance<PagingHandlerImpl>>;

#[percpu::def_percpu]
static CURRENT_INSTANCE: MaybeUninit<InstanceRef> = MaybeUninit::uninit();

pub struct Instance<H: PagingHandler> {
    id: usize,
    processes: Mutex<Vec<ProcessRef<H>>>,
    process_regions: Vec<ProcessMemoryRegion>,

    running: AtomicBool,

    vcpu: Mutex<VCpuRef>,
    /// For Stage-1 address translation, which translates guest virtual address to guest physical address,
    /// here we just use a direct one-to-one mapping, so this page table can be used by all processes.
    /// We need to map the page table root (in HPA) to LibOS's CR3 (in GPA) in EPT.
    linear_addrspace: AddrSpace<ShadowPageTableMetadata, X64PTE, H>,
}

unsafe impl<H: PagingHandler> Send for Instance<H> {}
unsafe impl<H: PagingHandler> Sync for Instance<H> {}
// unsafe impl<U: AxVCpuHal> Sync for AxVMInnerConst<U> {}

impl<H: PagingHandler> Instance<H> {
    pub fn new(
        id: usize,
        process_regions: Vec<ProcessMemoryRegion>,
        vcpu: VCpuRef,
    ) -> AxResult<Self> {
        let mut linear_addrspace = AddrSpace::new_empty(
            GuestVirtAddr::from_usize(0),
            1 << ShadowPageTableMetadata::VA_MAX_BITS,
        )?;

        let mut init_addrspace = AddrSpace::new_empty(
            GuestVirtAddr::from_usize(0),
            1 << ShadowPageTableMetadata::VA_MAX_BITS,
        )?;

        for p_region in &process_regions {
            if p_region.gva + p_region.size > (1 << ShadowPageTableMetadata::VA_MAX_BITS).into() {
                // TODO: handle [vsyscall] region @ 0xffffffffff600000.
                continue;
            }

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

        vcpu.setup(
            // This is just a place holder,
            // For vCpu with host context, the entry will be set as the host context's RIP.
            GuestPhysAddr::from_usize(0),
            init_addrspace.page_table_root(),
            axvm::AxVCpuCreateConfig::default(),
        )?;

        let mut processes = Vec::new();
        let init_process = Process::new(INIT_PROCESS_ID, init_addrspace);
        processes.push(init_process);
        Ok(Self {
            id,
            running: AtomicBool::new(false),
            processes: Mutex::new(processes),
            process_regions,
            linear_addrspace,
            vcpu: Mutex::new(vcpu),
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn running(&self) -> bool {
        self.running.load(core::sync::atomic::Ordering::Acquire)
    }

    pub fn create_init_process(&self, pid: usize) -> AxResult {
        info!("Instance {} create init process: pid = {}", self.id, pid);

        // Use this API to notify the instance to finish fork.
        self.running
            .store(true, core::sync::atomic::Ordering::Release);

        Ok(())
    }

    pub fn run_vcpu(&self) -> AxResult<AxVCpuExitReason> {
        let vcpu = self.vcpu.lock().clone();

        info!("Instance[{}] Vcpu[{}] run_vcpu()", self.id, vcpu.id());

        vcpu.set_return_value(axhal::cpu::this_cpu_id());

        vcpu.bind()?;

        let exit_reason: axvcpu::AxVCpuExitReason = loop {
            let exit_reason = vcpu.run()?;

            debug!("Vcpu[{}] exit reason: {:?}", vcpu.id(), exit_reason);

            break exit_reason;
        };

        vcpu.unbind()?;

        Ok(exit_reason)
    }
}

/// Create a new instance.
/// This function will create a new instance and allocate a task for the vCPU.
pub fn create_instance(
    id: usize,
    process_regions: Vec<ProcessMemoryRegion>,
    vcpu: VCpuRef,
) -> AxResult<InstanceRef> {
    let instance = Instance::<PagingHandlerImpl>::new(id, process_regions, vcpu.clone())?;
    let instance_ref = Arc::new(instance);

    crate::libos::alloc_vcpu_task(instance_ref.clone(), vcpu);

    INSTANCES.lock().insert(id, instance_ref.clone());
    Ok(instance_ref)
}

pub fn manipulate_instance(iid: usize, f: impl FnOnce(&InstanceRef) -> AxResult) -> AxResult {
    let _lock = INSTANCES.lock();
    let instance = _lock.get(&iid).ok_or_else(|| ax_err_type!(InvalidInput))?;
    f(instance)
}
