use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos::modules::axhal::cpu::this_cpu_id;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::os::arceos::modules::{axconfig, axtask};
use std::thread;

use axaddrspace::{GuestPhysAddr, HostPhysAddr};
use lazyinit::LazyInit;

use axconfig::SMP;
use axtask::{AxCpuMask, TaskInner};
use axvcpu::{AxVCpuExitReason, AxVcpuAccessGuestState, VCpuState};
use page_table_multiarch::{MappingFlags, PageSize, PagingHandler};

use crate::libos::def::InstanceSharedRegion;
use crate::libos::hvc::InstanceCall;
use crate::libos::instance::{InstanceRef, get_instances_by_id};
use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::VCpuRef;
use crate::vmm::config::get_instance_cpus_mask;

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

struct LibOSPerCpu<H: PagingHandler> {
    vcpu: VCpuRef,
    shared_region_base: HostPhysAddr,
    status: AtomicUsize,
    _phantom: PhantomData<H>,
    // running: AtomicBool,
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
}

impl<H: PagingHandler> Drop for LibOSPerCpu<H> {
    fn drop(&mut self) {
        H::dealloc_frame(self.shared_region_base)
    }
}

pub fn current_instance_id() -> usize {
    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };
    curcpu.shared_region().instance_id as usize
}

pub fn current_instance() -> InstanceRef {
    get_instances_by_id(current_instance_id()).expect("Instance not found")
}

pub fn current_process_id() -> usize {
    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };
    curcpu.shared_region().process_id as usize
}

pub fn current_eptp() -> HostPhysAddr {
    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };
    let eptp = curcpu.vcpu.get_arch_vcpu().current_ept_root();

    const PHYS_ADDR_MASK: usize = 0x000f_ffff_ffff_f000; // bits 12..52

    HostPhysAddr::from_usize(eptp.as_usize() & PHYS_ADDR_MASK)
}

pub fn gpa_to_hpa(gpa: GuestPhysAddr) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
    current_instance().guest_phys_to_host_phys(current_eptp(), gpa)
}

pub fn mark_idle() {
    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };
    curcpu
        .status
        .store(LibOSPerCpuStatus::Idle.into(), Ordering::SeqCst);
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
            vcpu: vcpu.clone(),
            shared_region_base,
            status: AtomicUsize::new(LibOSPerCpuStatus::Ready.into()),
            _phantom: PhantomData,
        });
    } else if remote_percpu.status.load(Ordering::SeqCst) == LibOSPerCpuStatus::Idle.into() {
        info!("Re-initializing LibOSPerCpu for CPU {}", cpu_id);
        remote_percpu.vcpu = vcpu.clone();
        remote_percpu.shared_region_base = shared_region_base;
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

    let mut vcpu_task: TaskInner = TaskInner::new(
        crate::vmm::vcpu_run,
        format!("ICore [{}]", cpu_id),
        KERNEL_STACK_SIZE,
    );

    vcpu_task.init_task_ext(TaskExt::new(TaskExtType::LibOS, vcpu));

    vcpu_task.set_cpumask(AxCpuMask::one_shot(cpu_id));

    axtask::spawn_task(vcpu_task);
}

/// TODO: maybe we tend to pin each vCpu on every physical CPU.
/// This function is the main routine for the vCPU task.
pub fn libos_vcpu_run(vcpu: VCpuRef) {
    let vcpu_id = vcpu.id();
    let cpu_id = this_cpu_id();

    info!(
        "Instance task on Core[{}] running, VCPU id {}",
        cpu_id, vcpu_id
    );

    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };

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
                let process_id = curcpu.shared_region().process_id;
                match exit_reason {
                    AxVCpuExitReason::Hypercall { nr, args } => {
                        debug!("Instance call [{:#x}] args: {:#x?}", nr, args);

                        let instance_ref =
                            if let Some(instance) = get_instances_by_id(instance_id as usize) {
                                instance
                            } else {
                                error!("Instance not found: {}", instance_id);
                                break;
                            };

                        match InstanceCall::new(
                            vcpu.clone(),
                            instance_ref,
                            process_id as usize,
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
                        match crate::libos::instance::remove_instance(instance_id as usize) {
                            Ok(_) => {
                                info!("Instance[{}] removed successfully", instance_id);
                            }
                            Err(err) => {
                                error!("Failed to remove instance[{}]: {:?}", instance_id, err);
                            }
                        };

                        thread::exit(hardware_entry_failure_reason as i32);
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
