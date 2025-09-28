//! FDT (Flattened Device Tree) processing module for AxVisor.
//!
//! This module provides functionality for parsing and processing device tree blobs,
//! including CPU configuration, passthrough device detection, and FDT generation.

mod create;
mod device;
mod parser;
mod print;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use axvm::config::AxVMCrateConfig;
use lazyinit::LazyInit;
use spin::Mutex;

// Re-export public functions
pub use parser::*;
// pub use print::print_fdt;
pub use create::*;
pub use device::build_node_path;

// DTB cache for generated device trees
static GENERATED_DTB_CACHE: LazyInit<Mutex<BTreeMap<usize, Vec<u8>>>> = LazyInit::new();

/// Initialize the DTB cache
pub fn init_dtb_cache() {
    GENERATED_DTB_CACHE.init_once(Mutex::new(BTreeMap::new()));
}

/// Get reference to the DTB cache
pub fn dtb_cache() -> &'static Mutex<BTreeMap<usize, Vec<u8>>> {
    GENERATED_DTB_CACHE.get().unwrap()
}

/// Generate guest FDT cache the result
/// # Return Value
/// Returns the generated DTB data and stores it in the global cache
pub fn crate_guest_fdt_with_cache(dtb_data: Vec<u8>, crate_config: &AxVMCrateConfig) {
    // Store data in global cache
    let mut cache_lock = dtb_cache().lock();
    cache_lock.insert(crate_config.base.id, dtb_data);
}
