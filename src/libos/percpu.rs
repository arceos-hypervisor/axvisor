use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos::modules::axhal::cpu::this_cpu_id;
use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;
use std::os::arceos::modules::{axconfig, axhal, axtask};
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
use crate::libos::instance::{InstanceRef, get_instances_by_id, shutdown_instance};
use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::VCpuRef;
use crate::vmm::config::get_instance_cpus_mask;
use equation_defs::{PROCESS_INNER_REGION_BASE_VA, PROCESS_INNER_REGION_SIZE, SHIM_INSTANCE_ID};

const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

#[derive(Debug, Clone, Copy)]
enum EqPerCpuStatus {
    Ready = 0,
    Running = 1,
    Idle = 2,
}

impl Into<usize> for EqPerCpuStatus {
    fn into(self) -> usize {
        self as usize
    }
}

impl From<usize> for EqPerCpuStatus {
    fn from(value: usize) -> Self {
        match value {
            0 => EqPerCpuStatus::Ready,
            1 => EqPerCpuStatus::Running,
            2 => EqPerCpuStatus::Idle,
            _ => panic!("Invalid EqPerCpuStatus value: {}", value),
        }
    }
}

pub(super) struct EqOSPerCpu<H: PagingHandler> {
    cpu_id: usize,
    vcpu: VCpuRef,
    percpu_region: HostPhysAddr,
    cpu_eptp_list_region: HostPhysAddr,
    status: AtomicUsize,
    _phantom: PhantomData<H>,
}

impl<H: PagingHandler> EqOSPerCpu<H> {
    pub fn percpu_region(&self) -> &'static PerCPURegion {
        unsafe {
            H::phys_to_virt(self.percpu_region)
                .as_ptr_of::<PerCPURegion>()
                .as_ref()
        }
        .unwrap()
    }

    pub fn percpu_region_mut(&mut self) -> &'static mut PerCPURegion {
        unsafe {
            H::phys_to_virt(self.percpu_region)
                .as_mut_ptr_of::<PerCPURegion>()
                .as_mut()
        }
        .unwrap()
    }

    pub fn set_next_instance_id(&mut self, instance_id: usize) {
        self.percpu_region_mut().next_instance_id = instance_id as _;
    }

    pub fn get_gate_eptp_list_entry(&self) -> AxResult<EPTPointer> {
        let eptp_list = EPTPList::construct(H::phys_to_virt(self.cpu_eptp_list_region))
            .ok_or_else(|| {
                ax_err_type!(InvalidInput, "Failed to construct EPTPList from region")
            })?;
        eptp_list.get(SHIM_INSTANCE_ID).ok_or_else(|| {
            ax_err_type!(
                InvalidData,
                "Failed to get gate EPTP for SHIM instance from EPTP list"
            )
        })
    }

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
        // self.percpu_region().process_id()
        0
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
            .store(EqPerCpuStatus::Idle.into(), Ordering::SeqCst);
    }

    pub fn set_running(&self) {
        self.status
            .store(EqPerCpuStatus::Running.into(), Ordering::SeqCst);
    }
}

pub fn cpu_is_running(cpu_id: usize) -> bool {
    assert!(cpu_id < SMP, "Invalid CPU ID: {}", cpu_id);
    let remote_percpu = unsafe { LIBOS_PERCPU.remote_ref_raw(cpu_id) };

    // If the remote percpu is not initialized, it means the CPU is not running.
    // This can happen if the CPU is reserved for host Linux and not initialized yet.
    if !remote_percpu.is_inited() {
        return false;
    }

    remote_percpu.status.load(Ordering::SeqCst) == EqPerCpuStatus::Running.into()
}

impl<H: PagingHandler> Drop for EqOSPerCpu<H> {
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

fn current_libos_percpu() -> &'static EqOSPerCpu<PagingHandlerImpl> {
    unsafe { LIBOS_PERCPU.current_ref_raw() }
}

/// Update the next instance ID of the specified per CPU region,
/// which is used to determine the next instance to run on this CPU.
pub fn set_next_instance_id_of_cpu(cpu_id: usize, instance_id: usize) -> AxResult {
    assert!(cpu_id < SMP, "Invalid CPU ID: {}", cpu_id);
    if !get_instance_cpus_mask().get(cpu_id) {
        warn!(
            "CPU {} is not in the instance CPU mask, skipping setting next instance ID",
            cpu_id
        );
        return ax_err!(
            InvalidInput,
            format!("CPU {} is not in the instance CPU mask", cpu_id)
        );
    }

    let remote_percpu = unsafe { LIBOS_PERCPU.remote_ref_mut_raw(cpu_id) };

    remote_percpu.set_next_instance_id(instance_id);

    Ok(())
}

#[percpu::def_percpu]
static LIBOS_PERCPU: LazyInit<EqOSPerCpu<PagingHandlerImpl>> = LazyInit::new();

pub fn init_instance_percore_task(cpu_id: usize, vcpu: VCpuRef, percpu_region: HostPhysAddr) {
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
        remote_percpu.init_once(EqOSPerCpu {
            cpu_id,
            vcpu: vcpu.clone(),
            percpu_region,
            cpu_eptp_list_region: vcpu.get_arch_vcpu().eptp_list_region(),
            status: AtomicUsize::new(EqPerCpuStatus::Ready.into()),
            _phantom: PhantomData,
        });
    } else if remote_percpu.status.load(Ordering::SeqCst) == EqPerCpuStatus::Idle.into() {
        info!("Re-initializing LibOSPerCpu for CPU {}", cpu_id);
        remote_percpu.vcpu = vcpu.clone();
        remote_percpu.percpu_region = percpu_region;
        remote_percpu.cpu_eptp_list_region = vcpu.get_arch_vcpu().eptp_list_region();
        remote_percpu
            .status
            .store(EqPerCpuStatus::Ready.into(), Ordering::SeqCst);
    } else {
        warn!(
            "PerCPU data for CPU {} bad status {:?}",
            cpu_id,
            Into::<EqPerCpuStatus>::into(remote_percpu.status.load(Ordering::SeqCst))
        );
    }

    remote_percpu.percpu_region_mut().cpu_id = cpu_id as _;
    remote_percpu.percpu_region_mut().current_instance_id = SHIM_INSTANCE_ID as _;
    remote_percpu.percpu_region_mut().next_instance_id = SHIM_INSTANCE_ID as _;

    info!(
        "LibOSPerCpu CPU[{}] initialized, vcpu {}, percpu region[{}] @{:?}, eptp list region {:?}",
        remote_percpu.cpu_id,
        remote_percpu.vcpu.id(),
        remote_percpu.percpu_region().cpu_id,
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
    assert_eq!(
        vcpu_id, cpu_id,
        "VCPU ID {} does not match CPU ID {}",
        vcpu_id, cpu_id
    );

    unsafe {
        // SDM 19.17.2 IA32_TSC_AUX Register and RDTSCP Support
        // IA32_TSC_AUX provides a 32-bit field that is initialized by privileged
        // software with a signature value (for example, a logical processor ID).
        // RDTSCP returns the 64-bit time stamp in EDX:EAX and the 32-bit TSC_AUX signature value in ECX.
        x86::msr::wrmsr(x86::msr::IA32_TSC_AUX, cpu_id as u64);
    }

    let curcpu = unsafe { LIBOS_PERCPU.current_ref_raw() };
    let instance = curcpu.current_instance();

    let ept_root_hpa = instance
        .processes
        .lock()
        .iter()
        .find(|(_, p)| p.pid() == cpu_id)
        .map(|(_, p)| p.ept_root())
        .unwrap();

    use x86_64::registers::control::Cr4Flags;
    let linux_ctx = &axhal::get_linux_context_list()[0];
    let cr4 = Cr4Flags::PHYSICAL_ADDRESS_EXTENSION
        // | Cr4Flags::PCID
        | Cr4Flags::FSGSBASE
        | Cr4Flags::PAGE_GLOBAL
        | Cr4Flags::OSFXSR
        | Cr4Flags::OSXMMEXCPT_ENABLE
        | Cr4Flags::OSXSAVE;
    let mut shim_context = HostContext::construct_guest64(
        SHIM_ENTRY as u64,
        GUEST_PT_ROOT_GPA.as_usize() as u64,
        cr4,
        &linux_ctx,
    );
    // Set stack pointer to the end of the process inner region.
    shim_context.rsp = (PROCESS_INNER_REGION_BASE_VA + PROCESS_INNER_REGION_SIZE - 8) as u64;

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
        if curcpu.status.load(Ordering::SeqCst) == EqPerCpuStatus::Idle.into() {
            thread::exit(0);
        }

        curcpu.set_running();

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

                        if nr == axhvc::HyperCallCode::HBenchVMCall as u64 {
                            // Just return directly for benchmark hypercall.
                            vcpu.set_return_value(0);
                            continue;
                        }

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
                    AxVCpuExitReason::CpuDown { _state } => {
                        info!(
                            "Instance[{}] run on Vcpu [{}] is shutting down",
                            instance_id, vcpu_id
                        );
                        curcpu.mark_idle();
                        continue;
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
                                curcpu.mark_idle();
                                continue;
                            }
                            _ => {
                                info!(
                                    "Instance[{}] calls system down on CPU [{}], removing instance",
                                    instance_id, vcpu_id
                                );
                                shutdown_instance(curcpu, &vcpu, &instance_ref)
                                    .expect("Failed to shutdown instance");
                            }
                        }
                    }
                    AxVCpuExitReason::EPTPSwitch { index } => {
                        error!(
                            "Instance[{}] run on Vcpu [{}] EPTP switch to index {} failed",
                            instance_id, vcpu_id, index
                        );

                        current_libos_percpu().dump_current_eptp_list();
                        curcpu.mark_idle();
                    }
                    AxVCpuExitReason::Nothing => {
                        // Nothing to do, just continue.
                        continue;
                    }
                    _ => {
                        error!("Instance run unexpected exit reason: {:?}", exit_reason);
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
