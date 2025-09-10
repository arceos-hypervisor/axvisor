use core::ptr::NonNull;

use crate::vmm::{VMRef, images::load_vm_image_from_memory};
use alloc::vec::Vec;
use axaddrspace::GuestPhysAddr;
use axvm::VMMemoryRegion;
use fdt_parser::Fdt;
use vm_fdt::{FdtWriter, FdtWriterNode};

#[allow(dead_code)]
pub fn print_fdt(fdt_addr: NonNull<u8>) {
    let fdt = Fdt::from_ptr(fdt_addr)
        .map_err(|e| format!("Failed to parse FDT: {e:#?}"))
        .expect("Failed to parse FDT");

    for rsv in fdt.memory_reservation_block() {
        info!(
            "Reserved memory: addr={:#p}, size={:#x}",
            rsv.address, rsv.size
        );
    }

    for node in fdt.all_nodes() {
        info!("node.name: {}", node.name());
        for prop in node.propertys() {
            info!("prop.name: {}, node.name: {}", prop.name, node.name());
        }
    }
}

pub fn updated_fdt(dest_addr: GuestPhysAddr, fdt_src: NonNull<u8>, dtb_size: usize, vm: VMRef) {
    let mut new_fdt = FdtWriter::new().unwrap();
    let mut old_node_level = 0;
    let mut child_node: Vec<FdtWriterNode> = Vec::new();

    let fdt_bytes = unsafe { core::slice::from_raw_parts(fdt_src.as_ptr(), dtb_size) };
    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {e:#?}"))
        .expect("Failed to parse FDT");

    for node in fdt.all_nodes() {
        if node.name() == "/" {
            child_node.push(new_fdt.begin_node("").unwrap());
        } else if node.name().starts_with("memory") {
            // Skip memory nodes, will add them later
            continue;
        } else {
            if node.level <= old_node_level {
                for _ in node.level..=old_node_level {
                    let end_node = child_node.pop().unwrap();
                    new_fdt.end_node(end_node).unwrap();
                }
            }
            child_node.push(new_fdt.begin_node(node.name()).unwrap());
        }

        old_node_level = node.level;

        for prop in node.propertys() {
            new_fdt.property(prop.name, prop.raw_value()).unwrap();
        }
    }
    while let Some(node) = child_node.pop() {
        old_node_level -= 1;
        new_fdt.end_node(node).unwrap();

        // add memory node
        if old_node_level == 1 {
            let memory_regions = vm.memory_regions();
            info!("Adding memory node with regions: {:?}", memory_regions);
            let memory_node = new_fdt.begin_node("memory").unwrap();
            add_memory_node(&memory_regions, &mut new_fdt);
            new_fdt.end_node(memory_node).unwrap();
        }
    }
    assert_eq!(old_node_level, 0);
    let new_fdt = new_fdt.finish().unwrap();
    // print_fdt(NonNull::new(new_fdt.as_ptr() as usize as _).unwrap());
    load_vm_image_from_memory(&new_fdt, dest_addr, vm.clone()).expect("Failed to load VM images");
}

fn add_memory_node(new_memory: &[VMMemoryRegion], new_fdt: &mut FdtWriter) {
    let mut new_value: Vec<u32> = Vec::new();
    for mem in new_memory {
        let gpa = mem.gpa.as_usize() as u64;
        let size = mem.size() as u64;
        new_value.push((gpa >> 32) as u32);
        new_value.push((gpa & 0xFFFFFFFF) as u32);
        new_value.push((size >> 32) as u32);
        new_value.push((size & 0xFFFFFFFF) as u32);
    }
    info!("new_value: {:#x?}", new_value);
    new_fdt
        .property_array_u32("reg", new_value.as_ref())
        .unwrap();
    new_fdt.property_string("device_type", "memory").unwrap();
}
