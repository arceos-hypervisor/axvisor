use crate::vmm::{VMRef, images::LoadRange};
use alloc::vec::Vec;
use axvm::config::{AxVMCrateConfig, VmMemConfig};
use fdt_parser::Fdt;
use vm_fdt::{FdtWriter, FdtWriterNode};
use axerrno::AxResult;


pub fn updated_fdt(config: AxVMCrateConfig, dtb_size: usize, vm: VMRef) -> AxResult<Vec<LoadRange>>  {
    let dtb_addr = config.kernel.dtb_load_addr.unwrap();
    let mut new_fdt = FdtWriter::new().unwrap();
    let mut old_node_level = 0;
    let mut child_node: Vec<FdtWriterNode> = Vec::new();
    let mut found_memory = false;

    let fdt_bytes = unsafe { core::slice::from_raw_parts(dtb_addr as *const u8, dtb_size) };
    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
        .expect("Failed to parse FDT");

    for node in fdt.all_nodes() {
        if node.level <= old_node_level {
            for _ in node.level..=old_node_level {
                let end_node = child_node.pop().unwrap();
                new_fdt.end_node(end_node).unwrap();
            }
        }
        old_node_level = node.level;

        if node.name() == "/" {
            child_node.push(new_fdt.begin_node("").unwrap());
        } else {
            child_node.push(new_fdt.begin_node(node.name()).unwrap());
        }

        for prop in node.propertys() {
            if node.name() == "memory" && prop.name == "reg" {
                found_memory = true;
                info!("node{},  Property: {} = {:?}", node.name(), prop.name, prop.raw_value());
                update_memory_node(&config.kernel.memory_regions, &mut new_fdt);
            } else {
                // Copy other properties as they are
                new_fdt.property(prop.name, prop.raw_value()).unwrap();
            }
        }
    }
    while let Some(node) = child_node.pop() {
        old_node_level -= 1;
        new_fdt.end_node(node).unwrap();

        // If we haven't found the memory node, add it now
        if old_node_level == 1 && !found_memory {
            info!("Adding memory node with regions: {:?}", config.kernel.memory_regions);
            let memory_node = new_fdt.begin_node("memory").unwrap();
            add_memory_node(&config.kernel.memory_regions, &mut new_fdt);
            new_fdt.end_node(memory_node).unwrap();
        }
    }
    assert_eq!(old_node_level , 0);
    let new_fdt = new_fdt.finish().unwrap();
    let load_ranges = copy_new_fdt_to_new_addr(new_fdt, dtb_addr, vm);

    // panic!("FDT parsing complete, starting to update FDT...");
    Ok(load_ranges)
}

fn update_memory_node(new_memory: &Vec<VmMemConfig>, new_fdt: &mut FdtWriter) {
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
}

fn add_memory_node(new_memory: &Vec<VmMemConfig>, new_fdt: &mut FdtWriter) {
    update_memory_node(new_memory, new_fdt);
    new_fdt
        .property_string("device_type", "memory")
        .unwrap();
}

fn copy_new_fdt_to_new_addr(
    new_fdt: Vec<u8>,
    new_dtb_addr: usize,
    vm: VMRef
) -> Vec<LoadRange> {
    unsafe {
        core::ptr::copy_nonoverlapping(new_fdt.as_ptr(), new_dtb_addr as *mut u8, new_fdt.len());
    }
    let new_fdt_regions = vm
        .get_image_load_region(new_dtb_addr.into(), new_fdt.len())
        .unwrap();
    let mut load_ranges = alloc::vec![];
    for buffer in new_fdt_regions {
        load_ranges.push(LoadRange {
            start: (buffer.as_ptr() as usize).into(),
            size: buffer.len(),
        });
    }
    load_ranges
}