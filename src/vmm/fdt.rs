use crate::vmm::{VMRef, images::load_vm_image_from_memory};
use alloc::vec::Vec;
use axvm::config::{AxVMCrateConfig, VmMemConfig};
use fdt_parser::Fdt;
use vm_fdt::{FdtWriter, FdtWriterNode};
use axerrno::AxResult;


pub fn print_fdt(fdt_addr: usize, dtb_size: usize, vm: VMRef) {
    let fdt_bytes = unsafe { core::slice::from_raw_parts(fdt_addr as *const u8, dtb_size) };
    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
        .expect("Failed to parse FDT");


    for node in fdt.all_nodes() {
        info!("node.name: {}", node.name());
        for prop in node.propertys() {
            info!("prop.name: {}, node.name: {}", prop.name, node.name());
        }
    }
}


pub fn updated_fdt(config: AxVMCrateConfig, fdt_addr: usize, dtb_size: usize, vm: VMRef) {
    let mut new_fdt = FdtWriter::new().unwrap();
    let mut old_node_level = 0;
    let mut child_node: Vec<FdtWriterNode> = Vec::new();


    let fdt_bytes = unsafe { core::slice::from_raw_parts(fdt_addr as *const u8, dtb_size) };
    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
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
            info!("Adding memory node with regions: {:?}", config.kernel.memory_regions);
            let memory_node = new_fdt.begin_node("memory").unwrap();
            add_memory_node(&config.kernel.memory_regions, &mut new_fdt);
            new_fdt.end_node(memory_node).unwrap();
        }
    }
    assert_eq!(old_node_level , 0);
    let new_fdt = new_fdt.finish().unwrap();
    load_vm_image_from_memory(&new_fdt, config.kernel.dtb_load_addr.unwrap(), vm.clone())
            .expect("Failed to load VM images");

}


fn add_memory_node(new_memory: &Vec<VmMemConfig>, new_fdt: &mut FdtWriter) {
    let mut new_value: Vec<u32> = Vec::new();
    for mem in new_memory {
        let gpa = mem.gpa as u64;
        let size = mem.size as u64;
        new_value.push((gpa >> 32) as u32);
        new_value.push((gpa & 0xFFFFFFFF) as u32);
        new_value.push((size >> 32) as u32);
        new_value.push((size & 0xFFFFFFFF) as u32);
    }
    info!("new_value: {:?}", new_value);
    new_fdt
        .property_array_u32("reg", new_value.as_ref())
        .unwrap();
    new_fdt
        .property_string("device_type", "memory")
        .unwrap();
}