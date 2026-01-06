#![no_std]

extern crate axklib;

use core::ptr::NonNull;

use rdrive::probe::OnProbeError;
use rdif_block::IQueue;
use spin::Once;
use spin::Mutex;
use alloc::sync::Arc;

#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

#[allow(unused_imports)]
#[macro_use]
extern crate log;

mod blk;
mod soc;
// mod serial;

/// Global block device reference
static BLOCK_DEVICE: Once<Arc<Mutex<dyn IQueue>>> = Once::new();

/// Get the block device
pub fn get_block_device() -> Option<Arc<Mutex<dyn IQueue>>> {
    BLOCK_DEVICE.get().cloned()
}

/// Set the block device (used by block drivers)
pub fn set_block_device(dev: Arc<Mutex<dyn IQueue>>) {
    BLOCK_DEVICE.call_once(|| dev);
}

/// Get block device queue from rdrive Block device
/// This function is called from axruntime to extract the IQueue from a Block device
pub fn get_block_queue() -> Option<Arc<Mutex<dyn IQueue>>> {
    #[cfg(target_arch = "aarch64")]
    {
        use rdrive::get_one;
        if let Some(_block) = get_one::<rdif_block::Block>() {
            // We need to create a queue from the Block device
            // Since Device<Block> doesn't expose the internal Block directly,
            // we can't easily extract the queue here
            // This function will be implemented properly once we figure out how to access the Block internals
            info!("Found Block device from rdrive, but queue extraction not yet implemented");
            None
        } else {
            None
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        None
    }
}

#[allow(unused)]
fn iomap(base: u64, size: usize) -> Result<NonNull<u8>, OnProbeError> {
    axklib::mem::iomap((base as usize).into(), size)
        .map(|ptr| unsafe { NonNull::new_unchecked(ptr.as_mut_ptr()) })
        .map_err(|e| OnProbeError::Other(format!("{e}:?").into()))
}
