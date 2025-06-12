use alloc::vec::Vec;
use fdt_parser::Fdt;
use vm_fdt::{FdtWriter, FdtWriterNode};
use axvm::config::AxVMCrateConfig;


pub fn print_all_fdt_nodes(dtb_addr: usize, dtb_size: usize) {
    info!("TEST SSSSSZZZZZYYYYYY");
    let fdt_bytes = unsafe {
        std::slice::from_raw_parts(dtb_addr as *const u8, dtb_size) // Assuming the DTB is 4KB
    };
    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
        .expect("Failed to parse FDT");
    for node in fdt.all_nodes() {
        // info!("Node: {}", node.name());
        for prop in node.propertys() {
            if node.name() == "memory" {
                info!("node{},  Property: {} = {:?}", node.name(), prop.name, prop.raw_value());
                // let new_value: [u32; 4] = [0x00, 0x80000000, 0x00, 0x10000000]; 
                // info!("new_value: {:?}", new_value);    
            }
            // info!("  Property: {} = {:?}", prop.name, prop.raw_value());
        }
    }
}

pub fn updated_fdt(config: AxVMCrateConfig, dtb_size: usize) -> Vec<u8> {
    let dtb_addr = config.kernel.dtb_load_addr.unwrap();
    let mut new_fdt = FdtWriter::new().unwrap();
    let mut old_node_level = 0;
    let mut child_node: Vec<FdtWriterNode> = Vec::new();

    let fdt_bytes = unsafe { std::slice::from_raw_parts(dtb_addr as *const u8, dtb_size) };
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
                info!("node{},  Property: {} = {:?}", node.name(), prop.name, prop.raw_value());
                let mut new_value: Vec<u32> = Vec::new();
                for mem in &config.kernel.memory_regions {
                    let gpa = mem.gpa as u64;
                    let size = mem.size as u64;
                    new_value.push((gpa >> 32) as u32);
                    new_value.push((gpa & 0xFFFFFFFF) as u32);
                    new_value.push((size >> 32) as u32);
                    new_value.push((size & 0xFFFFFFFF) as u32);
                }
                info!("new_value: {:?}", new_value);    
                new_fdt
                    .property_array_u32(prop.name, new_value.as_ref())
                    .unwrap();
            } else {
                // Copy other properties as they are
                new_fdt.property(prop.name, prop.raw_value()).unwrap();
            }
        }
    }
    while let Some(node) = child_node.pop() {
        new_fdt.end_node(node).unwrap();
    }
    let actual_new_fdt = new_fdt.finish().unwrap();
    actual_new_fdt
}
