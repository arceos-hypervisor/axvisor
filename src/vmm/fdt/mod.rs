//! FDT (Flattened Device Tree) processing module for AxVisor.
//!
//! This module provides functionality for parsing and processing device tree blobs,
//! including CPU configuration, passthrough device detection, and FDT generation.

mod create;
mod device;
mod parser;
mod print;

// Re-export public functions
pub use parser::*;
// pub use print::print_fdt;
pub use create::*;
pub use device::build_node_path;
