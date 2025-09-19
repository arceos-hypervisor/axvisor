//! FDT (Flattened Device Tree) processing module for AxVisor.
//!
//! This module provides functionality for parsing and processing device tree blobs,
//! including CPU configuration, passthrough device detection, and FDT generation.

mod device;
mod parser;
mod create;
mod test;

// Re-export public functions
pub use parser::{parse_fdt, parse_passthrough_devices_address, parse_vm_interrupt};
// pub use test::print_fdt;
pub use device::{build_node_path};
pub use create::update_fdt;