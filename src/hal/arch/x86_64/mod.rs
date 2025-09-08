use crate::hal::CacheOp;
use memory_addr::VirtAddr;

pub mod cache;

pub fn hardware_check() {}
pub fn inject_interrupt(_vector: u8) {}
