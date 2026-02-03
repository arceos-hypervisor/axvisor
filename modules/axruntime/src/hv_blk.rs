//! Block device storage for hypervisor use.
//!
//! This module provides a mechanism to save block devices during initialization
//! so that they can be used later by the hypervisor to create virtual block devices
//! for guest VMs.

use spin::Mutex;

pub use axdriver::prelude::AxBlockDevice;
pub use axdriver::AxDeviceContainer;

/// Global storage for block devices.
static BLOCK_DEVICES: Mutex<Option<AxDeviceContainer<AxBlockDevice>>> = Mutex::new(None);

/// Save block devices for later use by the hypervisor.
///
/// This function is called during runtime initialization to store the block devices.
pub(crate) fn save_block_devices(devices: AxDeviceContainer<AxBlockDevice>) {
    let mut guard = BLOCK_DEVICES.lock();
    if guard.is_some() {
        log::warn!("Block devices already saved, overwriting...");
    }
    *guard = Some(devices);
    log::info!("Block devices saved for hypervisor use");
}

/// Take the saved block devices.
///
/// This function returns the saved block devices and removes them from storage.
/// It should be called by the hypervisor when creating virtual block devices.
///
/// # Returns
///
/// - `Some(devices)` if block devices were saved
/// - `None` if no block devices were saved or they were already taken
pub fn take_block_devices() -> Option<AxDeviceContainer<AxBlockDevice>> {
    BLOCK_DEVICES.lock().take()
}
