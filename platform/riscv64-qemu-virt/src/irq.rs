use core::{
    num::NonZeroU32,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use axplat::{
    irq::{HandlerTable, IpiTarget, IrqHandler, IrqIf},
    percpu::this_cpu_id,
};
use kspin::SpinNoIrq;
use riscv::register::sie;
use riscv_plic::Plic;
use sbi_rt::HartMask;

use crate::config::{devices::PLIC_PADDR, plat::PHYS_VIRT_OFFSET};

/// `Interrupt` bit in `scause`
pub(super) const INTC_IRQ_BASE: usize = 1 << (usize::BITS - 1);

/// Supervisor software interrupt in `scause`
#[allow(unused)]
pub(super) const S_SOFT: usize = INTC_IRQ_BASE + 1;

/// Supervisor timer interrupt in `scause`
pub(super) const S_TIMER: usize = INTC_IRQ_BASE + 5;

/// Supervisor external interrupt in `scause`
pub(super) const S_EXT: usize = INTC_IRQ_BASE + 9;

static TIMER_HANDLER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

static IPI_HANDLER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Last external IRQ that was claimed from PLIC.
/// 0 means no pending external IRQ.
static LAST_EXTERNAL_IRQ: AtomicU32 = AtomicU32::new(0);

/// UART IRQ number (for console input detection)
const UART_IRQ: u32 = 10;

/// Flag indicating UART input may be available.
/// Set when UART IRQ fires, cleared when console is polled.
static UART_INPUT_PENDING: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Check if UART input may be pending (and clear the flag).
///
/// This is used by the VirtIO console polling mechanism to know
/// when to check for input from the host UART.
pub fn take_uart_input_pending() -> bool {
    UART_INPUT_PENDING.swap(false, Ordering::Acquire)
}

/// Check if UART input may be pending (without clearing the flag).
pub fn has_uart_input_pending() -> bool {
    UART_INPUT_PENDING.load(Ordering::Acquire)
}

/// Gets and clears the last external IRQ number.
///
/// Returns `Some(irq)` if there was a pending external IRQ, `None` otherwise.
/// This should be called after `irq_handler` returns for external interrupts.
pub fn take_last_external_irq() -> Option<u32> {
    let irq = LAST_EXTERNAL_IRQ.swap(0, Ordering::Acquire);
    if irq != 0 {
        Some(irq)
    } else {
        None
    }
}

/// Re-enables an external IRQ after guest has finished processing it.
///
/// This should be called when the guest completes the vPLIC IRQ.
/// For level-triggered interrupts, the IRQ was disabled during claim
/// to prevent re-triggering before guest handles it.
pub fn re_enable_external_irq(irq: u32) {
    if let Some(irq) = core::num::NonZeroU32::new(irq) {
        let mut plic = PLIC.lock();
        plic.set_priority(irq, 6);
        plic.enable(irq, this_context());
        trace!("Re-enabled external IRQ {irq}");
    }
}

/// The maximum number of IRQs.
pub const MAX_IRQ_COUNT: usize = 1024;

static IRQ_HANDLER_TABLE: HandlerTable<MAX_IRQ_COUNT> = HandlerTable::new();

static PLIC: SpinNoIrq<Plic> = SpinNoIrq::new(unsafe {
    Plic::new(NonNull::new((PHYS_VIRT_OFFSET + PLIC_PADDR) as *mut _).unwrap())
});

fn this_context() -> usize {
    let hart_id = this_cpu_id();
    hart_id * 2 + 1 // supervisor context
}

pub(super) fn init_percpu() {
    // enable soft interrupts, timer interrupts, and external interrupts
    unsafe {
        sie::set_ssoft();
        sie::set_stimer();
        sie::set_sext();
    }
    let mut plic = PLIC.lock();
    plic.init_by_context(this_context());

    // When VirtIO console is enabled, the hypervisor owns UART.
    // Enable UART IRQ in PLIC so hypervisor receives interrupts.
    #[cfg(feature = "virtio-console")]
    {
        if let Some(uart_irq) = NonZeroU32::new(UART_IRQ) {
            plic.set_priority(uart_irq, 6);
            plic.enable(uart_irq, this_context());
            info!("UART IRQ {} enabled for VirtIO console", UART_IRQ);
        }
        // Also enable UART receive interrupt in UART's IER register
        crate::console::enable_receive_interrupt();
    }
}

macro_rules! with_cause {
    ($cause: expr, @S_TIMER => $timer_op: expr, @S_SOFT => $ipi_op: expr, @S_EXT => $ext_op: expr, @EX_IRQ => $plic_op: expr $(,)?) => {
        match $cause {
            S_TIMER => $timer_op,
            S_SOFT => $ipi_op,
            S_EXT => $ext_op,
            other => {
                if other & INTC_IRQ_BASE == 0 {
                    // Device-side interrupts read from PLIC
                    $plic_op
                } else {
                    // Other CPU-side interrupts
                    panic!("Unknown IRQ cause: {other}");
                }
            }
        }
    };
}

struct IrqIfImpl;

#[impl_plat_interface]
impl IrqIf for IrqIfImpl {
    /// Enables or disables the given IRQ.
    fn set_enable(irq: usize, enabled: bool) {
        with_cause!(
            irq,
            @S_TIMER => {
                unsafe {
                    if enabled {
                        sie::set_stimer();
                    } else {
                        sie::clear_stimer();
                    }
                }
            },
            @S_SOFT => {},
            @S_EXT => {},
            @EX_IRQ => {
                let Some(irq) = NonZeroU32::new(irq as _) else {
                    return;
                };
                trace!("PLIC set enable: {irq} {enabled}");
                let mut plic = PLIC.lock();
                if enabled {
                    plic.set_priority(irq, 6);
                    plic.enable(irq, this_context());
                } else {
                    plic.disable(irq, this_context());
                }
            }
        );
    }

    /// Registers an IRQ handler for the given IRQ.
    ///
    /// It also enables the IRQ if the registration succeeds. It returns `false` if
    /// the registration failed.
    ///
    /// The `irq` parameter has the following semantics
    /// 1. If its highest bit is 1, it means it is an interrupt on the CPU side. Its
    /// value comes from `scause`, where [`S_SOFT`] represents software interrupt
    /// and [`S_TIMER`] represents timer interrupt. If its value is [`S_EXT`], it
    /// means it is an external interrupt, and the real IRQ number needs to
    /// be obtained from PLIC.
    /// 2. If its highest bit is 0, it means it is an interrupt on the device side,
    /// and its value is equal to the IRQ number provided by PLIC.
    fn register(irq: usize, handler: IrqHandler) -> bool {
        with_cause!(
            irq,
            @S_TIMER => TIMER_HANDLER.compare_exchange(core::ptr::null_mut(), handler as *mut _, Ordering::AcqRel, Ordering::Acquire).is_ok(),
            @S_SOFT => IPI_HANDLER.compare_exchange(core::ptr::null_mut(), handler as *mut _, Ordering::AcqRel, Ordering::Acquire).is_ok(),
            @S_EXT => {
                warn!("External IRQ should be got from PLIC, not scause");
                false
            },
            @EX_IRQ => {
                if IRQ_HANDLER_TABLE.register_handler(irq, handler) {
                    Self::set_enable(irq, true);
                    true
                } else {
                    warn!("register handler for External IRQ {irq} failed");
                    false
                }
            }
        )
    }

    /// Unregisters the IRQ handler for the given IRQ.
    ///
    /// It also disables the IRQ if the unregistration succeeds. It returns the
    /// existing handler if it is registered, `None` otherwise.
    fn unregister(irq: usize) -> Option<IrqHandler> {
        with_cause!(
            irq,
            @S_TIMER => {
                let handler = TIMER_HANDLER.swap(core::ptr::null_mut(), Ordering::AcqRel);
                if !handler.is_null() {
                    Some(unsafe { core::mem::transmute::<*mut (), IrqHandler>(handler) })
                } else {
                    None
                }
            },
            @S_SOFT => {
                let handler = IPI_HANDLER.swap(core::ptr::null_mut(), Ordering::AcqRel);
                if !handler.is_null() {
                    Some(unsafe { core::mem::transmute::<*mut (), IrqHandler>(handler) })
                } else {
                    None
                }
            },
            @S_EXT => {
                warn!("External IRQ should be got from PLIC, not scause");
                None
            },
            @EX_IRQ => IRQ_HANDLER_TABLE.unregister_handler(irq).inspect(|_| Self::set_enable(irq, false))
        )
    }

    /// Handles the IRQ.
    ///
    /// It is called by the common interrupt handler. It should look up in the
    /// IRQ handler table and calls the corresponding handler. If necessary, it
    /// also acknowledges the interrupt controller after handling.
    fn handle(irq: usize) -> Option<usize> {
        with_cause!(
            irq,
            @S_TIMER => {
                trace!("IRQ: timer");
                let handler = TIMER_HANDLER.load(Ordering::Acquire);
                if !handler.is_null() {
                    // SAFETY: The handler is guaranteed to be a valid function pointer.
                    unsafe { core::mem::transmute::<*mut (), IrqHandler>(handler)() };
                }
                Some(irq)
            },
            @S_SOFT => {
                trace!("IRQ: IPI");
                let handler = IPI_HANDLER.load(Ordering::Acquire);
                if !handler.is_null() {
                    // SAFETY: The handler is guaranteed to be a valid function pointer.
                    unsafe { core::mem::transmute::<*mut (), IrqHandler>(handler)() };
                }
                Some(irq)
            },
            @S_EXT => {
                // TODO: judge irq's ownership before handling (axvisor or any vm).
                // Maybe later it will be done by registering all irqs IQR_HANDLER_TABLE.

                let mut plic = PLIC.lock();
                let Some(irq) = plic.claim(this_context()) else {
                    // Spurious IRQ - can happen in SMP when another CPU claimed first
                    trace!("Spurious external IRQ on CPU {}", this_cpu_id());
                    return None;
                };

                let irq_num = irq.get();
                trace!("IRQ: external {irq_num}");

                // Handle UART IRQ based on console mode
                #[cfg(feature = "virtio-console")]
                if irq_num == UART_IRQ {
                    // VirtIO console mode: hypervisor owns UART.
                    // Set flag for vCPU loop to poll console input.
                    // No passthrough — guest uses /dev/hvc0, not /dev/ttyS0.
                    plic.complete(this_context(), irq);
                    drop(plic);
                    UART_INPUT_PENDING.store(true, Ordering::Release);
                    return Some(irq_num as usize);
                }

                // For passthrough device IRQs (e.g., UART in non-virtio-console mode):
                // 1. Disable the IRQ to prevent level-triggered re-triggering
                // 2. Store for injection to guest
                // 3. Guest will re-enable via re_enable_external_irq() after processing
                LAST_EXTERNAL_IRQ.store(irq_num, Ordering::Release);
                plic.disable(irq, this_context());
                plic.complete(this_context(), irq);
                drop(plic);

                // Also call registered handlers if any
                IRQ_HANDLER_TABLE.handle(irq_num as usize);

                Some(irq_num as usize)
            },
            @EX_IRQ => {
                unreachable!("Device-side IRQs should be handled by triggering the External Interrupt.");
            }
        )
    }

    /// Sends an inter-processor interrupt (IPI) to the specified target CPU or all CPUs.
    fn send_ipi(_irq_num: usize, target: IpiTarget) {
        match target {
            IpiTarget::Current { cpu_id } => {
                let res = sbi_rt::send_ipi(HartMask::from_mask_base(1 << cpu_id, 0));
                if res.is_err() {
                    warn!("send_ipi failed: {res:?}");
                }
            }
            IpiTarget::Other { cpu_id } => {
                let res = sbi_rt::send_ipi(HartMask::from_mask_base(1 << cpu_id, 0));
                if res.is_err() {
                    warn!("send_ipi failed: {res:?}");
                }
            }
            IpiTarget::AllExceptCurrent { cpu_id, cpu_num } => {
                for i in 0..cpu_num {
                    if i != cpu_id {
                        let res = sbi_rt::send_ipi(HartMask::from_mask_base(1 << i, 0));
                        if res.is_err() {
                            warn!("send_ipi_all_others failed: {res:?}");
                        }
                    }
                }
            }
        }
    }
}
