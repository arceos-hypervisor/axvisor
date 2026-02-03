#![no_std]

#[macro_use]
extern crate log;
#[macro_use]
extern crate axplat;

mod boot;
mod console;
mod init;
#[cfg(feature = "irq")]
mod irq;
mod mem;
mod power;
mod time;

pub mod config {
    //! Platform configuration module.
    //!
    //! If the `AX_CONFIG_PATH` environment variable is set, it will load the configuration from the specified path.
    //! Otherwise, it will fall back to the `axconfig.toml` file in the current directory and generate the default configuration.
    //!
    //! If the `PACKAGE` field in the configuration does not match the package name, it will panic with an error message.
    axconfig_macros::include_configs!(path_env = "AX_CONFIG_PATH", fallback = "axconfig.toml");
    assert_str_eq!(
        PACKAGE,
        env!("CARGO_PKG_NAME"),
        "`PACKAGE` field in the configuration does not match the Package name. Please check your configuration file."
    );
}

pub const fn cpu_count() -> usize {
    config::plat::CPU_NUM
}

pub const fn plic_base() -> usize {
    config::devices::PLIC_PADDR
}

/// Gets and clears the last external IRQ number that was claimed from PLIC.
///
/// Returns `Some(irq)` if there was a pending external IRQ, `None` otherwise.
/// This should be called after handling an external interrupt to inject it to guest.
#[cfg(feature = "irq")]
pub fn take_last_external_irq() -> Option<u32> {
    irq::take_last_external_irq()
}

/// Re-enables an external IRQ after guest has finished processing it.
///
/// This should be called when the guest completes the vPLIC IRQ.
/// For level-triggered interrupts, the IRQ was disabled during claim
/// to prevent re-triggering before guest handles it.
#[cfg(feature = "irq")]
pub fn re_enable_external_irq(irq: u32) {
    irq::re_enable_external_irq(irq)
}

/// Check if UART input may be pending (and clear the flag).
///
/// This is used by the VirtIO console mechanism to know when to
/// poll for input from the host UART after receiving UART IRQ.
#[cfg(all(feature = "irq", feature = "virtio-console"))]
pub fn take_uart_input_pending() -> bool {
    irq::take_uart_input_pending()
}
