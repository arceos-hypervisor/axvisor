use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use std::os::arceos;
use std::thread;

use arceos::api::config::SMP;
use arceos::api::task::AxCpuMask;
use arceos::modules::axhal::arch::wait_for_irqs;
use arceos::modules::axhal::cpu::{this_cpu_id, this_cpu_is_reserved};
use arceos::modules::{axalloc, axhal, axtask};

use memory_addr::{PAGE_SIZE_4K, align_up_4k};

use axaddrspace::{HostPhysAddr, HostVirtAddr};
use axerrno::{AxResult, ax_err_type};
use axvcpu::{AxArchVCpu, AxVCpuHal};
use axvm::{AxVMHal, AxVMPerCpu};

use crate::libos::instance::get_instances_by_id;
use crate::vmm::VCpuRef;
use crate::vmm::config::descrease_instance_cpus;
use crate::vmm::config::{get_instance_cpus, get_reserved_cpus};

pub(crate) const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

/// Implementation for `AxVMHal` trait.
pub struct AxVMHalImpl;

impl AxVMHal for AxVMHalImpl {
    type PagingHandler = axhal::paging::PagingHandlerImpl;

    fn alloc_memory_region_at(base: HostPhysAddr, size: usize) -> bool {
        axalloc::global_allocator()
            .alloc_pages_at(
                base.as_usize(),
                align_up_4k(size) / PAGE_SIZE_4K,
                PAGE_SIZE_4K,
            )
            .map_err(|err| {
                error!(
                    "Failed to allocate memory region [{:?}~{:?}]: {:?}",
                    base,
                    base + size,
                    err
                );
            })
            .is_ok()
    }

    fn dealloc_memory_region_at(base: HostPhysAddr, size: usize) {
        axalloc::global_allocator().dealloc_pages(base.as_usize(), size / PAGE_SIZE_4K)
    }

    fn virt_to_phys(vaddr: HostVirtAddr) -> HostPhysAddr {
        axhal::mem::virt_to_phys(vaddr)
    }

    fn current_time_nanos() -> u64 {
        axhal::time::monotonic_time_nanos()
    }
}

pub struct EPTTranslatorImpl;

impl axaddrspace::EPTTranslator for EPTTranslatorImpl {
    fn guest_phys_to_host_phys(gpa: axaddrspace::GuestPhysAddr) -> Option<HostPhysAddr> {
        use crate::task_ext::TaskExtType;
        use std::os::arceos::modules::axtask::{self, TaskExtRef};

        match &axtask::current().task_ext().ext {
            TaskExtType::VM(vm) => vm.guest_phys_to_host_phys(gpa),
            TaskExtType::LibOS => get_instances_by_id(0).unwrap().guest_phys_to_host_phys(gpa),
        }
    }
}

pub struct AxVCpuHalImpl;

impl AxVCpuHal for AxVCpuHalImpl {
    type EPTTranslator = EPTTranslatorImpl;
    type PagingHandler = axhal::paging::PagingHandlerImpl;

    fn virt_to_phys(vaddr: HostVirtAddr) -> axaddrspace::HostPhysAddr {
        std::os::arceos::modules::axhal::mem::virt_to_phys(vaddr)
    }

    #[cfg(target_arch = "aarch64")]
    fn irq_fetch() -> usize {
        axhal::irq::fetch_irq()
    }

    #[cfg(target_arch = "aarch64")]
    fn irq_hanlder() {
        let irq_num = axhal::irq::fetch_irq();
        debug!("IRQ handler {irq_num}");
        axhal::irq::handler_irq(irq_num);
    }
}

#[percpu::def_percpu]
static mut AXVM_PER_CPU: AxVMPerCpu<AxVCpuHalImpl> = AxVMPerCpu::<AxVCpuHalImpl>::new_uninit();

/// Init hardware virtualization support in each core.
///
/// It will spawn a task on each core to enable virtualization.
pub(crate) fn enable_virtualization() {
    static CORES: AtomicUsize = AtomicUsize::new(0);

    for cpu_id in 0..SMP {
        // Avoid to use `thread::spawn` and `ax_set_current_affinity` here,
        // in case "irq" is not enabled and the system result in deadlock.
        let task = axtask::TaskInner::new(
            move || {
                assert_eq!(
                    cpu_id,
                    this_cpu_id(),
                    "CPU ID mismatch when enabling virtualization"
                );

                #[cfg(feature = "irq")]
                crate::vmm::init_timer_percpu();

                let percpu = unsafe { AXVM_PER_CPU.current_ref_mut_raw() };
                percpu
                    .init(this_cpu_id())
                    .expect("Failed to initialize percpu state");
                percpu
                    .hardware_enable()
                    .expect("Failed to enable virtualization");

                info!("Hardware virtualization support enabled on core {}", cpu_id);

                let _ = CORES.fetch_add(1, Ordering::Release);
            },
            format!("Cpu[{}]-Enable-Virt", cpu_id),
            KERNEL_STACK_SIZE,
        );

        // Set the CPU affinity for the task,
        // so that it can run on the specified CPU core directly without migration.
        task.set_cpumask(AxCpuMask::from_raw_bits(1 << cpu_id));
        let _ = axtask::spawn_task(task);
    }

    // Wait for all cores to enable virtualization.
    while CORES.load(Ordering::Acquire) != SMP {
        // Use `yield_now` instead of `core::hint::spin_loop` to avoid deadlock.
        thread::yield_now();
    }
}

/// Disable virtualization on remaining cores.
/// This function should be called when the hypervisor is shutting down,
/// and the current core is not reserved for the hypervisor.
///
/// It will spawn a task on each core to disable virtualization.
pub(crate) fn disable_virtualization_on_remaining_cores() -> AxResult {
    let reserved_cpus = get_reserved_cpus();

    debug!("Reserved CPUs: {}", reserved_cpus);

    // Disable virtualization on remaining cores.
    for cpu_id in reserved_cpus..SMP {
        // Avoid to use `thread::spawn` and `ax_set_current_affinity` here,
        // in case "irq" is not enabled and the system result in deadlock.
        let task = axtask::TaskInner::new(
            move || {
                assert_eq!(
                    cpu_id,
                    this_cpu_id(),
                    "CPU ID mismatch when disabling virtualization"
                );

                assert!(
                    !this_cpu_is_reserved(),
                    "Reserved CPU {} is trying to disable virtualization",
                    cpu_id
                );

                info!("Trying to disable instance CPU {}", cpu_id);

                // TODO: we need to handle tasks on this Core.

                let percpu = unsafe { AXVM_PER_CPU.current_ref_mut_raw() };
                percpu
                    .hardware_disable()
                    .expect("Failed to disable virtualization");

                descrease_instance_cpus();
                info!("Hardware virtualization disabled on core {}", cpu_id);

                // Enter WFI state actively waiting for this core to be shutdown.
                // See `axhal::shutdown_secondary_cpus()` for more details.
                loop {
                    wait_for_irqs();
                }
            },
            format!("Cpu[{}]-Disable-Virt", cpu_id),
            KERNEL_STACK_SIZE,
        );

        // Set the CPU affinity for the task,
        // so that it can run on the specified CPU core directly without migration.
        task.set_cpumask(AxCpuMask::from_raw_bits(1 << cpu_id));

        debug!(
            "Spawning thread to disable virtualization on core {}",
            cpu_id
        );

        let _ = axtask::spawn_task(task);
    }

    use axhal::time::wall_time;
    use core::time::Duration;

    let deadline = wall_time() + Duration::from_secs(2);

    // Wait for all instance cores to disable virtualization.
    while get_instance_cpus() > 0 {
        // DO NOT need to use `yield_now` here,
        // because current task is not running on instance cores.
        core::hint::spin_loop();
        if axhal::time::wall_time() > deadline {
            warn!("Timeout waiting for instance cores to disable virtualization");
            break;
        }
    }

    // Shutdown all secondary CPUs.
    axhal::shutdown_secondary_cpus();

    info!("All secondary CPUs are shutdown");

    Ok(())
}

/// Disable virtualization on the current core.
/// This function should be called when the hypervisor is shutting down,
/// and the current core is reserved for the hypervisor.
/// It will unbind the vCPU from the core and restore the host context.
#[allow(unreachable_code)]
pub(crate) fn disable_virtualization(vcpu: VCpuRef, ret_code: usize) -> AxResult {
    assert!(this_cpu_is_reserved(), "This CPU is not reserved");

    let percpu = unsafe { AXVM_PER_CPU.current_ref_mut_raw() };

    let cpu_id = percpu
        .cpu_id()
        .ok_or_else(|| ax_err_type!(BadState, "Virtualization is not enabled on this core"))?;

    info!(
        "vCPU {} try to disable virtualization on core {}",
        vcpu.id(),
        cpu_id
    );

    vcpu.set_return_value(ret_code);

    let mut host_ctx = axhal::get_linux_context_list()[this_cpu_id() as usize].clone();
    vcpu.get_arch_vcpu().load_context(&mut host_ctx)?;
    vcpu.unbind()?;
    percpu.hardware_disable()?;
    host_ctx.restore();

    host_ctx.return_to_linux(vcpu.get_arch_vcpu().regs());

    unreachable!("CPU {} vCPU {} not return to Linux", cpu_id, vcpu.id());
}
