#![no_std]
#![cfg(all(target_arch = "x86_64", target_os = "none"))]
#![allow(missing_abi)]

use core::ptr::addr_of;

extern crate axplat_x86_pc;

pub fn cpu_count() -> usize {
    unsafe extern {
        static SMP: usize;
    }

    addr_of!(SMP) as _
}
