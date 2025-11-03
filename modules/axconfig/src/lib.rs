//! Platform-specific constants and parameters for [ArceOS].
//!
//! Currently supported platform configs can be found in the [configs] directory of
//! the [ArceOS] root.
//!
//! [ArceOS]: https://github.com/arceos-org/arceos
//! [configs]: https://github.com/arceos-org/arceos/tree/main/configs

#![no_std]

#[doc = " Architecture identifier."]
pub const ARCH: &str = "aarch64";
#[doc = " Platform package."]
pub const PACKAGE: &str = "axplat-aarch64-generic";
#[doc = " Platform identifier."]
pub const PLATFORM: &str = "aarch64-generic";
#[doc = " Stack size of each task."]
pub const TASK_STACK_SIZE: usize = 0x40000;
#[doc = " Number of timer ticks per second (Hz). A timer tick may contain several timer"]
#[doc = " interrupts."]
pub const TICKS_PER_SEC: usize = 100;
#[doc = ""]
#[doc = " Device specifications"]
#[doc = ""]
pub mod devices {
    #[doc = " MMIO regions with format (`base_paddr`, `size`)."]
    pub const MMIO_REGIONS: &[(usize, usize)] = &[];
    #[doc = " End PCI bus number."]
    pub const PCI_BUS_END: usize = 0;
    #[doc = " Base physical address of the PCIe ECAM space."]
    pub const PCI_ECAM_BASE: usize = 0;
    #[doc = " PCI device memory ranges."]
    pub const PCI_RANGES: &[(usize, usize)] = &[];
    #[doc = " Timer interrupt num (PPI, physical timer)."]
    pub const TIMER_IRQ: usize = 26;
    #[doc = " VirtIO MMIO regions with format (`base_paddr`, `size`)."]
    pub const VIRTIO_MMIO_REGIONS: &[(usize, usize)] = &[];
}
#[doc = ""]
#[doc = " Platform configs"]
#[doc = ""]
pub mod plat {
    #[doc = " Number of CPUs."]
    pub const CPU_NUM: usize = 16;
    #[doc = " Platform family (deprecated)."]
    pub const FAMILY: &str = "";
    #[doc = " Kernel address space base."]
    pub const KERNEL_ASPACE_BASE: usize = 0x0000_0000_0000;
    #[doc = " Kernel address space size."]
    pub const KERNEL_ASPACE_SIZE: usize = 0xffff_ffff_f000;
    #[doc = " No need."]
    pub const KERNEL_BASE_PADDR: usize = 0x0;
    #[doc = " Base virtual address of the kernel image."]
    pub const KERNEL_BASE_VADDR: usize = 0x8000_0000_0000;
    #[doc = " Offset of bus address and phys address. some boards, the bus address is"]
    #[doc = " different from the physical address."]
    pub const PHYS_BUS_OFFSET: usize = 0;
    #[doc = " No need."]
    pub const PHYS_MEMORY_BASE: usize = 0;
    #[doc = " No need."]
    pub const PHYS_MEMORY_SIZE: usize = 0x0;
    #[doc = " No need."]
    pub const PHYS_VIRT_OFFSET: usize = 0;
}
