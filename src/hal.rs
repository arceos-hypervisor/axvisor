use std::os::arceos;

use arceos::modules::{axalloc, axhal};

use memory_addr::{PAGE_SIZE_4K, align_up_4k};

use axaddrspace::{HostPhysAddr, HostVirtAddr};
use axerrno::{AxResult, ax_err_type};
use axvcpu::{AxArchVCpu, AxVCpuHal};
use axvm::{AxVMHal, AxVMPerCpu};

use crate::vmm::VCpuRef;

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
            _ => None,
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
pub(crate) fn enable_virtualization() {
    use core::sync::atomic::AtomicUsize;
    use core::sync::atomic::Ordering;

    use std::thread;

    use arceos::api::config;
    use arceos::api::task::{AxCpuMask, ax_set_current_affinity};
    use arceos::modules::axhal::cpu::this_cpu_id;

    static CORES: AtomicUsize = AtomicUsize::new(0);

    for cpu_id in 0..config::SMP {
        thread::spawn(move || {
            // Initialize cpu affinity here.
            assert!(
                ax_set_current_affinity(AxCpuMask::one_shot(cpu_id)).is_ok(),
                "Initialize CPU affinity failed!"
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
        });
    }

    // Wait for all cores to enable virtualization.
    while CORES.load(Ordering::Acquire) != config::SMP {
        // Use `yield_now` instead of `core::hint::spin_loop` to avoid deadlock.
        thread::yield_now();
    }
}

pub(crate) fn disable_virtualization(vcpu: VCpuRef, ret_code: usize) -> AxResult {
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
    let host_ctx = vcpu.get_arch_vcpu().load_host()?;
    vcpu.unbind()?;
    percpu.hardware_disable()?;
    host_ctx.restore();

    host_ctx.return_to_linux(vcpu.get_arch_vcpu().regs());

    Ok(())
}
