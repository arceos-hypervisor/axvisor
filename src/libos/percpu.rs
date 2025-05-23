use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos::modules::axhal::cpu::this_cpu_id;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::os::arceos::modules::{axconfig, axtask};
use std::thread;

use axaddrspace::npt::EPTPointer;
use axaddrspace::{GuestPhysAddr, HostPhysAddr};
use axerrno::{AxResult, ax_err, ax_err_type};
use lazyinit::LazyInit;

use axconfig::SMP;
use axtask::{AxCpuMask, TaskInner};
use axvcpu::{AxVCpuExitReason, AxVcpuAccessGuestState, VCpuState};
use axvm::HostContext;
use page_table_multiarch::{MappingFlags, PageSize, PagingHandler};

use crate::libos::config::SHIM_ENTRY;
use crate::libos::def::{EPTPList, GUEST_PT_ROOT_GPA, PerCPURegion};
use crate::libos::hvc::InstanceCall;
use crate::libos::instance::{self, InstanceRef, get_instances_by_id};
use crate::libos::region::HostPhysicalRegion;
use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::VCpuRef;
use crate::vmm::config::get_instance_cpus_mask;
use equation_defs::task::EqTask;
use equation_defs::{FIRST_PROCESS_ID, SHIM_INSTANCE_ID};

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
    percpu_region: HostPhysicalRegion<H>,
    cpu_eptp_list_region: HostPhysAddr,
    status: AtomicUsize,
    _phantom: PhantomData<H>,
}

impl<H: PagingHandler> LibOSPerCpu<H> {
    pub fn percpu_region(&self) -> &'static PerCPURegion {
        unsafe { self.percpu_region.as_ptr_of::<PerCPURegion>().as_ref() }.unwrap()
    }

    pub fn percpu_region_mut(&mut self) -> &'static mut PerCPURegion {
        unsafe { self.percpu_region.as_mut_ptr_of::<PerCPURegion>().as_mut() }.unwrap()
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
        self.percpu_region().process_id()
    }

    pub fn current_instance_id(&self) -> usize {
        self.percpu_region().instance_id()
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
        warn!(
            "Dropping LibOSPerCpu for CPU {}, vcpu {}",
            self.cpu_id,
            self.vcpu.id()
        );
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

pub fn init_instance_percore_task(
    cpu_id: usize,
    vcpu: VCpuRef,
    percpu_region: HostPhysicalRegion<PagingHandlerImpl>,
) {
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
            percpu_region,
            cpu_eptp_list_region: vcpu.get_arch_vcpu().eptp_list_region(),
            status: AtomicUsize::new(LibOSPerCpuStatus::Ready.into()),
            _phantom: PhantomData,
        });
    } else if remote_percpu.status.load(Ordering::SeqCst) == LibOSPerCpuStatus::Idle.into() {
        info!("Re-initializing LibOSPerCpu for CPU {}", cpu_id);
        remote_percpu.vcpu = vcpu.clone();
        remote_percpu.percpu_region = percpu_region;
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

    remote_percpu.percpu_region_mut().current_task.instance_id = SHIM_INSTANCE_ID as _;
    remote_percpu.percpu_region_mut().current_task.process_id = cpu_id as _;
    remote_percpu.percpu_region_mut().cpu_id = cpu_id as _;

    info!(
        "LibOSPerCpu CPU[{}] initialized, vcpu {}, shared region {:?}, eptp list region {:?}",
        remote_percpu.cpu_id,
        remote_percpu.vcpu.id(),
        remote_percpu.percpu_region,
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

    let shim_context =
        HostContext::construct_guest64(SHIM_ENTRY as u64, GUEST_PT_ROOT_GPA.as_usize() as u64);

    vcpu.setup_from_context(ept_root_hpa, shim_context)
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
                let instance_id = curcpu.percpu_region().instance_id();

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

                        match instance_id as usize {
                            SHIM_INSTANCE_ID => {
                                warn!(
                                    "SHIM Instance[{}] run on Vcpu [{}] system down",
                                    instance_id, vcpu_id
                                );
                                // DO NOT remove process for SHIM instance.
                                break;
                            }
                            _ => {
                                instance_ref
                                    .remove_process(curcpu.current_ept_root())
                                    .unwrap_or_else(|err| {
                                        error!("Failed to remove process: {:?}", err);
                                    });
                            }
                        }

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

/// Insert a new instance (alone with its first process) to the ready_queue of the CPU with the least number of running tasks.
/// Return the target CPU ID where the instance is inserted.
pub fn insert_instance(first_task: EqTask) -> AxResult<usize> {
    let instance_id = first_task.instance_id;

    // Find the cpu with the lowest running task number.
    let mut target_cpu_id = 0;
    let mut min_task_num = usize::MAX;
    for cpu in get_instance_cpus_mask().into_iter() {
        let remote_percpu = unsafe { LIBOS_PERCPU.remote_ref_mut_raw(cpu) };
        if !remote_percpu.is_inited() {
            warn!("PerCPU data for CPU {} not initialized", cpu);
            continue;
        }
        let task_num = remote_percpu.percpu_region().run_queue.get_task_num();
        if min_task_num > task_num {
            min_task_num = task_num;
            target_cpu_id = cpu;
        }
    }

    // Since we'll at least reserve one CPU for the host OS,
    // the target CPU should not be 0.
    if target_cpu_id == 0 {
        error!("No available CPU for instance {}", instance_id);
        return ax_err!(NotFound, "No available CPU for instance");
    }

    debug!("Insert instance {} to CPU {}", instance_id, target_cpu_id);
    let target_cpu = unsafe { LIBOS_PERCPU.remote_ref_mut_raw(target_cpu_id) };

    target_cpu
        .percpu_region_mut()
        .ready_queue
        .insert(first_task)
        .map_err(|task| {
            error!(
                "Failed to insert instance {} to CPU {}: {:?}",
                instance_id, target_cpu_id, task
            );
            ax_err_type!(ResourceBusy, "Failed to insert instance")
        })?;

    debug!(
        "After insert, CPU {} ready queue\n{:?}",
        target_cpu_id,
        target_cpu.percpu_region().ready_queue,
    );

    Ok(target_cpu_id)
}
