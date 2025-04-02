use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use std::os::arceos::modules::axhal;

use axerrno::{AxResult, ax_err_type};
use memory_addr::{MemoryAddr, PAGE_SIZE_4K, PageIter4K, PhysAddr, is_aligned_4k};
use page_table_entry::x86_64::X64PTE;
use page_table_multiarch::{PagingHandler, PagingMetaData};

use crate::libos::def::{ProcessMemoryRegion, ShadowPageTableMetadata};
use crate::libos::process::{INIT_PROCESS_ID, Process, ProcessRef};
use crate::vmm::VCpuRef;
use axaddrspace::{AddrSpace, GuestPhysAddr, GuestVirtAddr};
use axhal::paging::PagingHandlerImpl;
use axstd::sync::Mutex;

static INSTANCES: Mutex<BTreeMap<usize, InstanceRef>> = Mutex::new(BTreeMap::new());

/// The reference type of a task.
pub type InstanceRef = Arc<Instance<PagingHandlerImpl>>;

#[percpu::def_percpu]
static CURRENT_INSTANCE: MaybeUninit<InstanceRef> = MaybeUninit::uninit();

pub struct Instance<H: PagingHandler> {
    id: usize,
    processes: Mutex<Vec<ProcessRef<H>>>,
    process_regions: Vec<ProcessMemoryRegion>,
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

            if p_region.mapping.is_none() {
                warn!(
                    "Process memory region [{:?} - {:?}] {:?} is not mapped by Linux, skipping",
                    p_region.gva,
                    p_region.gva + p_region.size,
                    p_region.flags
                );
                continue;
            } else if p_region.mapping.unwrap().hpa.is_none() {
                error!(
                    "Process memory region GVA [{:?} - {:?}] {:?} GPA {:?} is not mapped by hypervisor, skipping",
                    p_region.gva,
                    p_region.gva + p_region.size,
                    p_region.mapping.unwrap().gpa,
                    p_region.flags
                );
                continue;
            }

            if !p_region.gva.is_aligned_4k() || !is_aligned_4k(p_region.size) {
                warn!(
                    "Process memory region [{:?} - {:?}] {:?} is not aligned to 4K, skipping",
                    p_region.gva,
                    p_region.gva + p_region.size,
                    p_region.flags
                );
            }

            // Map as populated mapping to ensure that the memory mapping is established.
            init_addrspace.map_alloc(p_region.gva, p_region.size, p_region.flags, true)?;

            for gva in PageIter4K::new(p_region.gva, p_region.gva + p_region.size)
                .expect("Failed to create PageIter4K")
            {
                let host_region_hva = H::phys_to_virt(p_region.mapping.unwrap().hpa.unwrap());

                let instance_region_hva = H::phys_to_virt(
                    init_addrspace
                        .translate(gva)
                        .expect(alloc::format!("GVA {:#x} not mapped", gva).as_str()),
                );

                unsafe {
                    core::ptr::copy_nonoverlapping(
                        host_region_hva.as_ptr(),
                        instance_region_hva.as_mut_ptr(),
                        PAGE_SIZE_4K,
                    )
                };
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
            processes: Mutex::new(processes),
            process_regions,
            linear_addrspace,
            vcpu: Mutex::new(vcpu),
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn create_init_process(&self, pid: usize) -> AxResult {
        info!("Instance {} create init process: pid = {}", self.id, pid);

        Ok(())
    }
}

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
