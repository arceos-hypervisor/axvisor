//! Instance memory region backends.

use alloc::sync::Arc;

use axaddrspace::HostPhysAddr;
use axerrno::{AxResult, ax_err, ax_err_type};
use memory_addr::{MemoryAddr, PAGE_SIZE_4K, align_up_4k};
use page_table_multiarch::PagingHandler;

pub(crate) struct HostPhysicalRegion<H: PagingHandler> {
    base: HostPhysAddr,
    size: usize,
    phontom: core::marker::PhantomData<H>,
}

impl<H: PagingHandler> core::fmt::Debug for HostPhysicalRegion<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "HostPhysicalRegion [{:?}-{:?}]",
            self.base,
            self.base.add(self.size)
        )
    }
}

pub(crate) type HostPhysicalRegionRef<H> = Arc<HostPhysicalRegion<H>>;

impl<H: PagingHandler> HostPhysicalRegion<H> {
    pub fn allocate(size: usize, align_pow2: Option<usize>) -> AxResult<Self> {
        let size_aligned = align_up_4k(size);

        let hpa = H::alloc_frames(
            size_aligned / PAGE_SIZE_4K,
            if let Some(align_pow2) = align_pow2 {
                align_pow2
            } else {
                PAGE_SIZE_4K
            },
        )
        .ok_or_else(|| {
            ax_err_type!(NoMemory, "Failed to allocate memory for HostPhysicalRegion")
        })?;

        // Clear the memory region.
        unsafe {
            core::ptr::write_bytes(H::phys_to_virt(hpa).as_mut_ptr(), 0, size_aligned);
        }

        Ok(Self {
            base: hpa,
            size: size_aligned,
            phontom: core::marker::PhantomData,
        })
    }

    pub fn allocate_ref(
        size: usize,
        align_pow2: Option<usize>,
    ) -> AxResult<HostPhysicalRegionRef<H>> {
        Ok(Arc::new(Self::allocate(size, align_pow2)?))
    }

    pub fn base(&self) -> HostPhysAddr {
        self.base
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn as_ptr_of<T>(&self) -> *const T {
        assert!(self.size >= core::mem::size_of::<T>());
        H::phys_to_virt(self.base).as_ptr_of::<T>()
    }

    pub fn as_mut_ptr_of<T>(&self) -> *mut T {
        assert!(self.size >= core::mem::size_of::<T>());
        H::phys_to_virt(self.base).as_mut_ptr_of::<T>()
    }

    pub fn zero(&self) {
        unsafe {
            core::ptr::write_bytes(H::phys_to_virt(self.base).as_mut_ptr(), 0, self.size);
        }
    }

    pub fn copy_from(&self, src: &Self) {
        if self.size != src.size {
            warn!(
                "{:?} copying memory regions from {:?} with different sizes: {} vs {}",
                self.base,
                src.base(),
                self.size,
                src.size
            );
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                H::phys_to_virt(src.base()).as_ptr(),
                H::phys_to_virt(self.base).as_mut_ptr(),
                self.size,
            );
        }
    }

    pub fn copy_from_slice(&self, src: &[u8], offset: usize, size: usize) -> AxResult {
        if size > self.size - offset {
            return ax_err!(InvalidInput, "Copy size exceeds region size");
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.as_ptr(),
                H::phys_to_virt(self.base.add(offset)).as_mut_ptr(),
                size,
            );
        }
        Ok(())
    }

    #[allow(unused)]
    pub fn copy_to_slice(&self, dst: &mut [u8], offset: usize, size: usize) -> AxResult {
        if size > self.size - offset {
            return ax_err!(InvalidInput, "Copy size exceeds region size");
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                H::phys_to_virt(self.base.add(offset)).as_ptr(),
                dst.as_mut_ptr(),
                size,
            );
        }
        Ok(())
    }
}

impl<H: PagingHandler> Drop for HostPhysicalRegion<H> {
    fn drop(&mut self) {
        trace!(
            "Dropping HostPhysicalRegion [{:?}-{:?}]",
            self.base,
            self.base.add(self.size)
        );
        H::dealloc_frames(self.base, self.size / PAGE_SIZE_4K);
    }
}
