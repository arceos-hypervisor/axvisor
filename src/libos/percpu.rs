use alloc::sync::Arc;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos::modules::axhal::cpu::this_cpu_id;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::os::arceos::modules::{axconfig, axtask};
use std::thread;

use axaddrspace::npt::EPTPointer;
use axaddrspace::{GuestPhysAddr, HostPhysAddr};
use lazyinit::LazyInit;

use axconfig::SMP;
use axtask::{AxCpuMask, TaskInner};
use axvcpu::{AxVCpuExitReason, AxVcpuAccessGuestState, VCpuState};
use page_table_multiarch::{MappingFlags, PageSize, PagingHandler};

use crate::libos::def::{EPTPList, InstanceSharedRegion};
use crate::libos::hvc::InstanceCall;
use crate::libos::instance::{InstanceRef, get_instances_by_id};
use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::config::get_instance_cpus_mask;
use crate::vmm::{VCpu, VCpuRef};

const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

#[derive(Debug, Clone, Copy)]
enum LibOSPerCpuStatus {
    Ready = 0,
    Running = 1,
    Idle = 2,
}

impl Into<usize> for LibOSPerCpuStatus {
    fn into(self) -> usize {
        self as usize
    }
}

impl From<usize> for LibOSPerCpuStatus {
    fn from(value: usize) -> Self {
        match value {
            0 => LibOSPerCpuStatus::Ready,
            1 => LibOSPerCpuStatus::Running,
            2 => LibOSPerCpuStatus::Idle,
            _ => panic!("Invalid LibOSPerCpuStatus value: {}", value),
        }
    }
}

pub(super) struct LibOSPerCpu<H: PagingHandler> {
    cpu_id: usize,
    vcpu: VCpuRef,
    shared_region_base: HostPhysAddr,
    cpu_eptp_list_region: HostPhysAddr,
    status: AtomicUsize,
    _phantom: PhantomData<H>,
}

impl<H: PagingHandler> LibOSPerCpu<H> {
    pub fn shared_region(&self) -> &'static InstanceSharedRegion {
        unsafe {
            H::phys_to_virt(self.shared_region_base)
                .as_ptr()
                .cast::<InstanceSharedRegion>()
                .as_ref()
                .unwrap()
        }
    }

    pub fn shared_region_mut(&mut self) -> &'static mut InstanceSharedRegion {
        unsafe {
            H::phys_to_virt(self.shared_region_base)
                .as_mut_ptr()
                .cast::<InstanceSharedRegion>()
                .as_mut()
                .unwrap()
        }
    }

    #[allow(unused)]
    fn dump_current_eptp_list(&self) {
        info!(
            "Current EPTP list region {:?} for CPU {}, vcpu {}",
            self.cpu_eptp_list_region,
            self.cpu_id,
            self.vcpu.id()
        );
        unsafe {
            EPTPList::dump_region(H::phys_to_virt(self.cpu_eptp_list_region));
        }
    }

    /// Update the EPTP list region on this CPU with the given EPTP list.
    fn update_eptp_list_region(&self, eptp_list: &EPTPList) {
        debug!(
            "Updating EPTP list region {:?} for CPU {}, vcpu {}",
            self.cpu_eptp_list_region,
            self.cpu_id,
            self.vcpu.id()
        );

        // self.dump_current_eptp_list();

        unsafe {
            eptp_list.copy_into_region(H::phys_to_virt(self.cpu_eptp_list_region));
        }
    }

    /// Sync the EPTP list region on all vCPUs that the given instance is running on.
    pub fn sync_eptp_list_region_on_all_vcpus(instance_id: usize, eptp_list: &EPTPList) {
        debug!("Syncing EPTP list for Instance {}", instance_id);

        for pcpu_id in get_instance_cpus_mask().into_iter() {
            let remote_percpu = unsafe { LIBOS_PERCPU.remote_ref_mut_raw(pcpu_id) };
            if remote_percpu.is_inited() && remote_percpu.current_instance_id() == instance_id {
                debug!(
                    "Syncing EPTP list for Instance {} on CPU {}, vcpu {}",
                    instance_id,
                    pcpu_id,
                    remote_percpu.vcpu.id()
                );
                remote_percpu.update_eptp_list_region(eptp_list);
            } else {
                warn!("PerCPU data for CPU {} not initialized", pcpu_id);
            }
        }
    }

    pub fn current_process_id(&self) -> usize {
        self.shared_region().process_id as usize
    }

    pub fn current_instance_id(&self) -> usize {
        self.shared_region().instance_id as usize
    }
    pub fn current_instance(&self) -> InstanceRef {
        get_instances_by_id(self.current_instance_id() as usize).expect("Instance not found")
    }

    pub fn current_ept_pointer(&self) -> EPTPointer {
        self.vcpu.get_arch_vcpu().ept_pointer()
    }

    pub fn current_ept_root(&self) -> HostPhysAddr {
        self.current_ept_pointer().into_ept_root()
    }

    pub fn guest_phys_to_host_phys(
        &self,
        gpa: GuestPhysAddr,
    ) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
        self.current_instance()
            .guest_phys_to_host_phys(self.current_ept_root(), gpa)
    }

    pub fn mark_idle(&self) {
        self.status
            .store(LibOSPerCpuStatus::Idle.into(), Ordering::SeqCst);
    }
}

impl<H: PagingHandler> Drop for LibOSPerCpu<H> {
    fn drop(&mut self) {
        H::dealloc_frame(self.shared_region_base)
    }
}

pub fn gpa_to_hpa(gpa: GuestPhysAddr) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
    current_libos_percpu().guest_phys_to_host_phys(gpa)
}

fn current_libos_percpu() -> &'static LibOSPerCpu<PagingHandlerImpl> {
    unsafe { LIBOS_PERCPU.current_ref_raw() }
}

#[percpu::def_percpu]
static LIBOS_PERCPU: LazyInit<LibOSPerCpu<PagingHandlerImpl>> = LazyInit::new();

pub fn init_instance_percore_task(cpu_id: usize, vcpu: VCpuRef, shared_region_base: HostPhysAddr) {
    assert!(cpu_id < SMP, "Invalid CPU ID: {}", cpu_id);
    if !get_instance_cpus_mask().get(cpu_id) {
        warn!(
            "CPU {} is not in the instance CPU mask, skipping task creation",
            cpu_id
        );
        return;
    }

    // Safety:
    // this function will only be called once on reserved CPUs from host VM
    // during instance runtime initialization.
    // It is safe to get the remote percpu reference here.
    let remote_percpu = unsafe { LIBOS_PERCPU.remote_ref_mut_raw(cpu_id) };

    if !remote_percpu.is_inited() {
        remote_percpu.init_once(LibOSPerCpu {
            cpu_id,
            vcpu: vcpu.clone(),
            shared_region_base,
            cpu_eptp_list_region: vcpu.get_arch_vcpu().eptp_list_region(),
            status: AtomicUsize::new(LibOSPerCpuStatus::Ready.into()),
            _phantom: PhantomData,
        });
    } else if remote_percpu.status.load(Ordering::SeqCst) == LibOSPerCpuStatus::Idle.into() {
        info!("Re-initializing LibOSPerCpu for CPU {}", cpu_id);
        remote_percpu.vcpu = vcpu.clone();
        remote_percpu.shared_region_base = shared_region_base;
        remote_percpu.cpu_eptp_list_region = vcpu.get_arch_vcpu().eptp_list_region();
        remote_percpu
            .status
            .store(LibOSPerCpuStatus::Ready.into(), Ordering::SeqCst);
    } else {
        warn!(
            "PerCPU data for CPU {} bad status {:?}",
            cpu_id,
            Into::<LibOSPerCpuStatus>::into(remote_percpu.status.load(Ordering::SeqCst))
        );
    }

    remote_percpu.shared_region_mut().instance_id = 0;
    remote_percpu.shared_region_mut().process_id = cpu_id as _;

    info!(
        "LibOSPerCpu CPU[{}] initialized, vcpu {}, shared region {:?}, eptp list region {:?}",
        remote_percpu.cpu_id,
        remote_percpu.vcpu.id(),
        remote_percpu.shared_region_base,
        remote_percpu.cpu_eptp_list_region
    );

    let mut vcpu_task: TaskInner = TaskInner::new(
        crate::vmm::vcpu_run,
        format!("ICore [{}]", cpu_id),
        KERNEL_STACK_SIZE,
    );

    vcpu_task.init_task_ext(TaskExt::new(TaskExtType::LibOS, vcpu));

    vcpu_task.set_cpumask(AxCpuMask::one_shot(cpu_id));

    axtask::spawn_task(vcpu_task);
}

/// This function is the main routine for the vCPU task.
pub fn libos_vcpu_run(vcpu: VCpuRef) {
    let vcpu_id = vcpu.id();
    let cpu_id = this_cpu_id();

    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };
    let instance = curcpu.current_instance();

    let ept_root_hpa = instance
        .processes
        .lock()
        .iter()
        .find(|(_, p)| p.pid() == curcpu.current_process_id())
        .map(|(_, p)| p.ept_root())
        .unwrap();

    vcpu.setup_from_context(ept_root_hpa, instance.ctx.clone())
        .expect("Failed to setup vcpu");

    info!(
        "Instance[{}] task on Core[{}] running, VCPU id {}, Init process id {}",
        curcpu.current_instance_id(),
        cpu_id,
        vcpu_id,
        curcpu.current_process_id()
    );

    vcpu.bind()
        .inspect_err(|err| {
            error!("Vcpu[{}] bind error: {:?}", vcpu_id, err);
            // Set `VCpuState::Free` for peaceful unbinding.
            vcpu.transition_state(VCpuState::Invalid, VCpuState::Free)
                .expect("Failed to set state");
            vcpu.unbind().expect("Failed to unbind");
        })
        .unwrap();
    loop {
        if curcpu.status.load(Ordering::SeqCst) == LibOSPerCpuStatus::Idle.into() {
            thread::exit(0);
        }

        match vcpu.run() {
            Ok(exit_reason) => {
                let instance_id = curcpu.shared_region().instance_id;

                let instance_ref = if let Some(instance) = get_instances_by_id(instance_id as usize)
                {
                    instance
                } else {
                    error!("Instance not found: {}", instance_id);
                    curcpu.mark_idle();
                    continue;
                };

                match exit_reason {
                    AxVCpuExitReason::Hypercall { nr, args } => {
                        debug!("Instance call [{:#x}] args: {:x?}", nr, args);

                        match InstanceCall::new(
                            vcpu.clone(),
                            &curcpu,
                            instance_ref.clone(),
                            nr,
                            args,
                        ) {
                            Ok(instance_call) => {
                                let ret_val = match instance_call.execute() {
                                    Ok(ret_val) => ret_val as isize,
                                    Err(err) => {
                                        warn!("Hypercall [{:#x}] failed: {:?}", nr, err);
                                        -1
                                    }
                                };

                                vcpu.set_return_value(ret_val as usize);
                            }
                            Err(err) => {
                                error!("Instance call error: {:?}", err);
                                break;
                            }
                        }
                    }
                    AxVCpuExitReason::FailEntry {
                        hardware_entry_failure_reason,
                    } => {
                        warn!(
                            "Instance[{}] run on Vcpu [{}] failed with exit code {}",
                            instance_id, vcpu_id, hardware_entry_failure_reason
                        );

                        instance_ref
                            .remove_process(curcpu.current_ept_root())
                            .unwrap_or_else(|err| {
                                error!("Failed to remove process: {:?}", err);
                            });
                        curcpu.mark_idle();
                    }
                    AxVCpuExitReason::NestedPageFault { addr, access_flags } => {
                        match instance_ref.handle_ept_page_fault(
                            curcpu.current_ept_root(),
                            addr,
                            access_flags,
                        ) {
                            Ok(_) => {}
                            Err(err) => {
                                error!(
                                    "Failed to handle nested page fault addr {:?} flags {:?}: {:?}",
                                    addr, access_flags, err
                                );
                                break;
                            }
                        }
                    }
                    AxVCpuExitReason::SystemDown => {
                        info!(
                            "Instance[{}] run on Vcpu [{}] system down",
                            instance_id, vcpu_id
                        );
                        instance_ref
                            .remove_process(curcpu.current_ept_root())
                            .unwrap_or_else(|err| {
                                error!("Failed to remove process: {:?}", err);
                            });
                        curcpu.mark_idle();
                    }
                    AxVCpuExitReason::Nothing => {
                        // Nothing to do, just continue.
                        continue;
                    }
                    _ => {
                        warn!("Instance run unexpected exit reason: {:?}", exit_reason);
                        break;
                    }
                };
            }
            Err(err) => {
                error!("Vcpu[{}] run error: {:?}", vcpu_id, err);
                break;
            }
        }
    }
    vcpu.unbind()
        .inspect_err(|err| {
            error!("Vcpu[{}] unbind error: {:?}", vcpu_id, err);
        })
        .unwrap();
}
