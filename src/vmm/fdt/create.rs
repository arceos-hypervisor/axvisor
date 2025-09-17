use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use fdt_parser::{Fdt, Node};
use vm_fdt::{FdtWriter, FdtWriterNode};
use axvm::config::{AxVMConfig, AxVMCrateConfig};

use crate::vmm::fdt::print_fdt;
use crate::vmm::fdt::test::print_guest_fdt;


pub fn crate_guest_fdt(fdt: &Fdt, passthrough_device_names: &Vec<String>, vm_cfg: &AxVMConfig, crate_config: &AxVMCrateConfig) {
    let mut fdt_writer = FdtWriter::new().unwrap();
    // 跟踪上一个处理节点的层级，用于层级变化处理
    let mut previous_node_level = 0;
    // 维护FDT节点栈，用于正确开始和结束节点
    let mut node_stack: Vec<FdtWriterNode> = Vec::new();
    let phys_cpu_ids = crate_config.base.phys_cpu_ids.clone().expect("ERROR: phys_cpu_ids is None");

    let all_nodes: Vec<Node> = fdt.all_nodes().collect();

    for (index, node) in all_nodes.iter().enumerate() {
        // 使用独立函数构建节点路径
        let node_path = super::build_node_path(&all_nodes, index);
        // 处理不同类型的节点
        let node_action = determine_node_action(
            node,
            &node_path,
            passthrough_device_names,
        );

        match node_action {
            NodeAction::RootNode => {
                node_stack.push(fdt_writer.begin_node("").unwrap());
            },
            NodeAction::CpuNode => {
                let need = need_cpu_node(&phys_cpu_ids, node, &node_path);
                if need {
                    handle_node_level_change(&mut fdt_writer, &mut node_stack, node.level, previous_node_level);
                    node_stack.push(fdt_writer.begin_node(node.name()).unwrap());
                } else {
                    continue;
                }
            },
            NodeAction::Skip => {
                // 不需要包含在客户机FDT中的节点
                continue;
            },
            _ => {
                // 完全匹配的直通设备节点
                trace!("Found exact passthrough device node: {}, path: {}", node.name(), node_path);
                handle_node_level_change(&mut fdt_writer, &mut node_stack, node.level, previous_node_level);
                node_stack.push(fdt_writer.begin_node(node.name()).unwrap());
            },
        }

        previous_node_level = node.level;

        // 复制节点的所有属性
        for prop in node.propertys() {
            fdt_writer.property(prop.name, prop.raw_value()).unwrap();
        }
    }

    // 结束所有未关闭的节点
    while let Some(node) = node_stack.pop() {
        previous_node_level -= 1;
        fdt_writer.end_node(node).unwrap();

        // add memory node
        if previous_node_level == 1 {
            info!("Adding memory node:{:x?}", crate_config.kernel.memory_regions);
            let memory_node = fdt_writer.begin_node("memory").unwrap();
            add_memory_node(&mut fdt_writer, crate_config);
            fdt_writer.end_node(memory_node).unwrap();
        }
    }
    assert_eq!(previous_node_level , 0);

    print_guest_fdt(fdt_writer.finish().unwrap().as_slice());
    // use std::io::Write;
    // use std::fs::File;
    // let guest_fdt_bytes = fdt_writer.finish().unwrap();
    // let mut file = File::create("guest_fdt.dtb").expect("Failed to create file");
    // file.write_all(&guest_fdt_bytes).unwrap(); 
}

/// 节点处理动作枚举
enum NodeAction {
    /// 跳过节点，不包含在客户机FDT中
    Skip,
    /// 根节点
    RootNode,
    /// cpu节点
    CpuNode,
    /// 包含节点作为直通设备节点
    IncludeAsPassthroughDevice,
    /// 包含节点作为直通设备的子节点
    IncludeAsChildNode,
    /// 包含节点作为直通设备的祖先节点
    IncludeAsAncestorNode,
}

/// 确定节点的处理动作
fn determine_node_action(
    node: &Node,
    node_path: &str,
    passthrough_device_names: &Vec<String>,
) -> NodeAction {
    if node.name() == "/" {
        // 根节点特殊处理
        return NodeAction::RootNode;
    } else if node.name().starts_with("memory") {
        // 跳过memory节点，稍后会单独添加
        return NodeAction::Skip;
    } else if node_path.starts_with("/cpus"){
        return NodeAction::CpuNode;
    } else if passthrough_device_names.contains(&node_path.to_string()) {
        // 完全匹配的直通设备节点
        return NodeAction::IncludeAsPassthroughDevice;
    } 
    // 检查是否为直通设备的后代节点（通过路径包含关系和层级验证）
    else if is_descendant_of_passthrough_device(node_path, node.level, passthrough_device_names) {
        return NodeAction::IncludeAsChildNode;
    } 
    // 检查是否为直通设备的祖先节点（通过路径包含关系和层级验证）
    else if is_ancestor_of_passthrough_device(node_path, passthrough_device_names) {
        return NodeAction::IncludeAsAncestorNode;
    } else {
        return NodeAction::Skip;
    }
}

/// 判断节点是否为直通设备的后代节点
/// 当节点路径包含passthrough_device_names中某个节点路径，且比其长时，即为其后代节点
/// 同时使用node_level作为验证条件
fn is_descendant_of_passthrough_device(node_path: &str, node_level: usize, passthrough_device_names: &Vec<String>) -> bool {
    for passthrough_path in passthrough_device_names {
        // 检查当前节点是否为直通设备的后代节点
        if node_path.starts_with(passthrough_path) && node_path.len() > passthrough_path.len() {
            // 确保是真正的后代路径（以/分隔）
            if passthrough_path == "/" || node_path.chars().nth(passthrough_path.len()) == Some('/') {
                // 使用层级关系进行验证：后代节点的层级应该比父节点高
                // 注意：根节点的层级为1，其直接子节点层级为2，以此类推
                let expected_parent_level = passthrough_path.matches('/').count();
                let current_node_level = node_level;
                
                // 如果passthrough_path是根节点"/"，则其子节点层级应为2
                // 否则，子节点层级应比父节点层级高
                if passthrough_path == "/" && current_node_level >= 2 {
                    return true;
                } else if passthrough_path != "/" && current_node_level > expected_parent_level {
                    return true;
                }
            }
        }
    }
    false
}

/// 处理节点层级变化，确保FDT结构正确
fn handle_node_level_change(
    fdt_writer: &mut FdtWriter,
    node_stack: &mut Vec<FdtWriterNode>,
    current_level: usize,
    previous_level: usize,
) {
    if current_level <= previous_level {
        for _ in current_level..=previous_level {
            if let Some(end_node) = node_stack.pop() {
                fdt_writer.end_node(end_node).unwrap();
            }
        }
    }
}

/// 判断节点是否为直通设备的祖先节点
fn is_ancestor_of_passthrough_device(node_path: &str, passthrough_device_names: &Vec<String>) -> bool {
    for passthrough_path in passthrough_device_names {
        // 检查当前节点是否为直通设备的祖先节点
        if passthrough_path.starts_with(&node_path) && passthrough_path.len() > node_path.len() {
            // 确保是真正的祖先路径（以/分隔）
            let next_char = passthrough_path.chars().nth(node_path.len()).unwrap_or(' ');
            if next_char == '/' || node_path == "/" {
                return true;
            }
        }
    }
    false
}

fn add_memory_node(fdt_writer: &mut FdtWriter, crate_config: &AxVMCrateConfig) {
    let new_memory = &crate_config.kernel.memory_regions;
    let mut new_value: Vec<u32> = Vec::new();
    for mem in new_memory {
        let gpa = mem.gpa as u64;
        let size = mem.size as u64;
        new_value.push((gpa >> 32) as u32);
        new_value.push((gpa & 0xFFFFFFFF) as u32);
        new_value.push((size >> 32) as u32);
        new_value.push((size & 0xFFFFFFFF) as u32);
    }
    debug!("new_value: {:#?}", new_value);
    fdt_writer
        .property_array_u32("reg", new_value.as_ref())
        .unwrap();
    fdt_writer
        .property_string("device_type", "memory")
        .unwrap();
}

fn need_cpu_node(phys_cpu_ids: &Vec<usize>, node: &Node, node_path: &str) -> bool{ 
    let mut should_include_node = false;
    
    if !node_path.starts_with("/cpus/cpu@") {
        should_include_node = true;
    } else {
        if let Some(mut cpu_reg) = node.reg() {
            if let Some(reg_entry) = cpu_reg.next() {
                let cpu_address = reg_entry.address as usize;
                debug!("Checking CPU node {} with address 0x{:x}", node.name(), cpu_address);
                // 检查这个CPU地址是否在配置的phys_cpu_ids中
                if phys_cpu_ids.contains(&&cpu_address) {
                    should_include_node = true;
                    info!("CPU node {} with address 0x{:x} is in phys_cpu_ids, including in guest FDT", node.name(), cpu_address);
                } else {
                    info!("CPU node {} with address 0x{:x} is NOT in phys_cpu_ids, skipping", node.name(), cpu_address);
                }
            }
        }
    }
    should_include_node
}
