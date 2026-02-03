/// Memory API implementation for axvisor.
///
/// This module implements the axvisor_api::memory interface using the underlying
/// platform memory management functions.
#[axvisor_api::api_mod_impl(axvisor_api::memory)]
mod memory_api_impl {
    use axvisor_api::memory::{PhysAddr, VirtAddr};
    use memory_addr::{PhysAddr as HalPhysAddr, VirtAddr as HalVirtAddr};

    extern fn alloc_frame() -> Option<PhysAddr> {
        // Use axalloc for frame allocation
        axalloc::GlobalPage::alloc()
            .ok()
            .map(|page: axalloc::GlobalPage| {
                // Get the physical address of the allocated page
                let paddr = page.start_paddr(|vaddr| axhal::mem::virt_to_phys(HalVirtAddr::from(vaddr.as_usize())));
                PhysAddr::from(paddr.as_usize())
            })
    }

    extern fn alloc_contiguous_frames(
        num_frames: usize,
        frame_align_pow2: usize,
    ) -> Option<PhysAddr> {
        axalloc::GlobalPage::alloc_contiguous(num_frames, frame_align_pow2)
            .ok()
            .map(|page: axalloc::GlobalPage| {
                let paddr = page.start_paddr(|vaddr| axhal::mem::virt_to_phys(HalVirtAddr::from(vaddr.as_usize())));
                PhysAddr::from(paddr.as_usize())
            })
    }

    extern fn dealloc_frame(addr: PhysAddr) {
        // Convert physical address to virtual address
        let vaddr = axhal::mem::phys_to_virt(HalPhysAddr::from(addr.as_usize()));
        // Note: GlobalPage uses RAII and will auto-deallocate when dropped.
        // For manual deallocation, we need to use the allocator directly.
        // However, the current API design doesn't support easy manual deallocation.
        // This is a limitation that should be addressed.
        // For now, we'll leak the memory as a workaround.
        core::mem::forget(vaddr);
    }

    extern fn dealloc_contiguous_frames(first_addr: PhysAddr, num_frames: usize) {
        let vaddr = axhal::mem::phys_to_virt(HalPhysAddr::from(first_addr.as_usize()));
        // Same issue as dealloc_frame - RAII makes manual deallocation difficult
        for i in 0..num_frames {
            let offset = i * 0x1000; // 4K pages
            let page_vaddr = axhal::mem::phys_to_virt(HalPhysAddr::from(vaddr.as_usize() + offset));
            core::mem::forget(page_vaddr);
        }
    }

    extern fn phys_to_virt(addr: PhysAddr) -> VirtAddr {
        let vaddr = axhal::mem::phys_to_virt(HalPhysAddr::from(addr.as_usize()));
        VirtAddr::from(vaddr.as_usize())
    }

    extern fn virt_to_phys(addr: VirtAddr) -> PhysAddr {
        let paddr = axhal::mem::virt_to_phys(HalVirtAddr::from(addr.as_usize()));
        PhysAddr::from(paddr.as_usize())
    }
}
