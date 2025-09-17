//! Device passthrough and dependency analysis for FDT processing.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use fdt_parser::{Fdt, Node};
use axvm::config::{AxVMConfig};

/// 完善直通设备配置
/// 修改为返回配置文件中所有直通设备和查找到的新添加设备的合集，而不是直接修改VM配置
/// 优化版本：预构建节点缓存以提高性能
pub fn find_all_passthrough_devices(vm_cfg: &mut AxVMConfig, fdt: &Fdt) -> Vec<String> {
    // 先获取初始设备数量，避免借用冲突
    let initial_device_count = vm_cfg.pass_through_devices().len();
    info!("Starting passthrough devices analysis...");
    info!(
        "Original passthrough_devices count: {}",
        initial_device_count
    );

    // 预构建节点缓存，将所有节点按路径存储以提高查找性能
    let node_cache: BTreeMap<String, Vec<Node>> = build_optimized_node_cache(fdt);
    

    // 获取已配置设备的名称列表
    let initial_device_names: Vec<String> = vm_cfg
        .pass_through_devices()
        .iter()
        .map(|dev| dev.name.clone())
        .collect();

    // 第一阶段：发现所有配置文件中直通设备的后代节点
    // 构建已配置设备的集合，使用BTreeSet提高查找效率
    let mut configured_device_names: BTreeSet<String> =
        initial_device_names.iter().cloned().collect();

    // 用于存储新发现的相关设备名称
    let mut additional_device_names = Vec::new();

    // 第一阶段：处理初始设备及其后代节点
    // 注意：这里我们直接使用设备路径而不是设备名称
    for device_name in &initial_device_names {
        // 获取该设备的所有后代节点路径
        let descendant_paths = get_descendant_nodes_by_path(&node_cache, device_name);
        trace!(
            "Found {} descendant paths for {}",
            descendant_paths.len(),
            device_name
        );

        // 直接使用路径而不是通过节点获取路径
        for descendant_path in descendant_paths {
            if !configured_device_names.contains(&descendant_path) {
                trace!(
                    "Found descendant device: {}",
                    descendant_path
                );
                configured_device_names.insert(descendant_path.clone());

                // 收集后代节点路径
                additional_device_names.push(descendant_path.clone());
            } else {
                trace!("Device already exists: {}", descendant_path);
            }
        }
    }

    info!(
        "Phase 1 completed: Found {} new descendant device names",
        additional_device_names.len()
    );

    // 第二阶段：发现所有已有设备（包括后代设备）的依赖节点
    let mut dependency_device_names = Vec::new();
    // 使用设备名称的工作队列，包含初始设备和后代设备的名称
    let mut devices_to_process: Vec<String> = configured_device_names.iter().cloned().collect();
    let mut processed_devices: BTreeSet<String> = BTreeSet::new();

    // 构建phandle映射表
    let phandle_map = build_phandle_map(fdt);

    // 使用工作队列递归查找所有依赖设备
    while let Some(device_node_path) = devices_to_process.pop() {
        // 避免重复处理同一设备
        if processed_devices.contains(&device_node_path) {
            continue;
        }
        processed_devices.insert(device_node_path.clone());

        trace!("Analyzing dependencies for device: {}", device_node_path);

        // 查找当前设备的直接依赖
        let dependencies = find_device_dependencies(&device_node_path, &phandle_map, &node_cache);
        trace!("Found {} dependencies: {:?}", dependencies.len(), dependencies);
        for dep_node_name in dependencies {
            // 检查依赖是否已经在配置中
            if !configured_device_names.contains(&dep_node_name) {
                trace!("Found new dependency device: {}", dep_node_name);
                dependency_device_names.push(dep_node_name.clone());

                // 将依赖设备名称添加到工作队列中，以便进一步查找其依赖
                devices_to_process.push(dep_node_name.clone());
                configured_device_names.insert(dep_node_name.clone());
            }
        }
    }

    info!(
        "Phase 2 completed: Found {} new dependency device names",
        dependency_device_names.len()
    );

    // 合并所有设备名称列表
    let mut all_device_names = initial_device_names.clone();
    all_device_names.extend(additional_device_names);
    all_device_names.extend(dependency_device_names);

    let final_device_count = all_device_names.len();
    info!(
        "Passthrough devices analysis completed. Total devices: {} (added: {})",
        final_device_count,
        final_device_count - initial_device_count
    );
    
    // 打印最终的设备列表
    for (i, device_name) in all_device_names.iter().enumerate() {
        trace!("Final passthrough device[{}]: {}", i, device_name);
    }

    // 返回所有设备名称的合集
    all_device_names
}

/// 根据节点的level关系构建节点的完整路径
/// 通过遍历所有节点并根据level关系构建路径，避免同名节点路径冲突问题
pub fn build_node_path(all_nodes: &Vec<Node>, target_index: usize) -> String {
    // 用于维护路径栈
    let mut path_stack: Vec<String> = Vec::new();
    
    // 遍历从根节点到目标节点的所有节点
    for i in 0..=target_index {
        let node = &all_nodes[i];
        let level = node.level;
        
        if level == 1 {
            // 根节点特殊处理
            path_stack.clear();
            if node.name() != "/" {
                path_stack.push(node.name().to_string());
            }
        } else {
            // 根据level关系处理节点
            while path_stack.len() >= level - 1 {
                path_stack.pop();
            }
            path_stack.push(node.name().to_string());
        }
    }
    
    // 构建当前节点的完整路径
    if path_stack.is_empty() || (path_stack.len() == 1 && path_stack[0] == "/") {
        "/".to_string()
    } else {
        "/".to_string() + &path_stack.join("/")
    }
}

/// 构建简化的节点缓存表，一次性遍历所有节点并按全路径分组
/// 使用level关系直接构建路径，避免同名节点路径冲突问题
pub fn build_optimized_node_cache<'a>(fdt: &'a Fdt) -> BTreeMap<String, Vec<Node<'a>>> {
    let mut node_cache: BTreeMap<String, Vec<Node<'a>>> = BTreeMap::new();
    
    // 收集所有节点
    let all_nodes: Vec<Node> = fdt.all_nodes().collect();
    
    // 遍历所有节点，根据level关系构建路径
    for (index, node) in all_nodes.iter().enumerate() {
        // 使用独立函数构建节点路径
        let node_path = build_node_path(&all_nodes, index);
        
        // 检查是否有相同的node_path，如果有则报错
        if let Some(existing_nodes) = node_cache.get(&node_path) {
            if !existing_nodes.is_empty() {
                error!(
                    "Duplicate node path found: {} for node '{}' at level {}, existing node: '{}'",
                    node_path,
                    node.name(),
                    node.level,
                    existing_nodes[0].name()
                );
            }
        }
        
        trace!("Adding node to cache: {} (level: {}, index: {})", node_path, node.level, index);
        node_cache
            .entry(node_path)
            .or_insert_with(Vec::new)
            .push(node.clone());
    }

    debug!(
        "Built simplified node cache with {} unique device paths",
        node_cache.len()
    );
    node_cache
}

/// 构建phandle到节点信息的映射表，优化版本使用fdt-parser的便利方法
/// 使用完整路径而不是节点名称
/// 使用level关系直接构建路径，避免同名节点路径冲突问题
fn build_phandle_map(fdt: &Fdt) -> BTreeMap<u32, (String, BTreeMap<String, u32>)> {
    let mut phandle_map = BTreeMap::new();
    
    // 收集所有节点
    let all_nodes: Vec<Node> = fdt.all_nodes().collect();
    
    // 遍历所有节点，根据level关系构建路径
    for (index, node) in all_nodes.iter().enumerate() {
        // 使用独立函数构建节点路径
        let node_path = build_node_path(&all_nodes, index);
        
        let mut phandle = None;
        let mut cells_map = BTreeMap::new();

        // 收集节点的属性
        for prop in node.propertys() {
            match prop.name {
                "phandle" | "linux,phandle" => {
                    phandle = Some(prop.u32());
                }
                "#address-cells"
                | "#size-cells"
                | "#clock-cells"
                | "#reset-cells"
                | "#gpio-cells"
                | "#interrupt-cells"
                | "#power-domain-cells"
                | "#thermal-sensor-cells"
                | "#phy-cells"
                | "#dma-cells"
                | "#sound-dai-cells"
                | "#mbox-cells"
                | "#pwm-cells"
                | "#iommu-cells" => {
                    cells_map.insert(prop.name.to_string(), prop.u32());
                }
                _ => {}
            }
        }

        // 如果找到phandle，将其与节点完整路径一起存储
        if let Some(ph) = phandle {
            phandle_map.insert(ph, (node_path, cells_map));
        }
    }
    phandle_map
}

/// 根据#*-cells属性智能解析包含phandle引用的属性
/// 支持多种格式:
/// - 单个phandle: <phandle>
/// - phandle+specifier: <phandle specifier1 specifier2 ...>
/// - 多个phandle引用: <phandle1 spec1 spec2 phandle2 spec1 spec2 ...>
fn parse_phandle_property_with_cells(
    prop_data: &[u8],
    prop_name: &str,
    phandle_map: &BTreeMap<u32, (String, BTreeMap<String, u32>)>,
) -> Vec<(u32, Vec<u32>)> {
    let mut results = Vec::new();

    debug!(
        "Parsing property '{}' with cells info, data length: {} bytes",
        prop_name,
        prop_data.len()
    );

    // 检查数据长度是否有效
    if prop_data.is_empty() || prop_data.len() % 4 != 0 {
        warn!(
            "Property '{}' data length ({} bytes) is invalid",
            prop_name,
            prop_data.len()
        );
        return results;
    }

    let u32_values: Vec<u32> = prop_data
        .chunks(4)
        .map(|chunk| u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    let mut i = 0;
    while i < u32_values.len() {
        let potential_phandle = u32_values[i];

        // 检查是否为有效的phandle
        if let Some((device_name, cells_info)) = phandle_map.get(&potential_phandle) {
            // 根据属性名确定需要的cells数量
            let cells_count = get_cells_count_for_property(prop_name, cells_info);
            trace!(
                "Property '{}' requires {} cells for device '{}'",
                prop_name, cells_count, device_name
            );

            // 检查是否有足够的数据
            if i + cells_count < u32_values.len() {
                let specifiers: Vec<u32> = u32_values[i + 1..=i + cells_count].to_vec();
                debug!(
                    "Parsed phandle reference: phandle={:#x}, specifiers={:?}",
                    potential_phandle, specifiers
                );
                results.push((potential_phandle, specifiers));
                i += cells_count + 1; // 跳过phandle和所有specifier
            } else {
                warn!(
                    "Property:{} not enough data for phandle {:#x}, expected {} cells but only {} values remaining",
                    prop_name,
                    potential_phandle,
                    cells_count,
                    u32_values.len() - i - 1
                );
                break;
            }
        } else {
            // 如果不是有效phandle，跳过这个值
            i += 1;
        }
    }

    results
}

/// 根据属性名和目标节点的cells信息确定需要的cells数量
fn get_cells_count_for_property(prop_name: &str, cells_info: &BTreeMap<String, u32>) -> usize {
    let cells_property = match prop_name {
        "clocks" | "assigned-clocks" => "#clock-cells",
        "resets" => "#reset-cells",
        "power-domains" => "#power-domain-cells",
        "phys" => "#phy-cells",
        "interrupts" | "interrupts-extended" => "#interrupt-cells",
        "gpios" | _ if prop_name.ends_with("-gpios") || prop_name.ends_with("-gpio") => {
            "#gpio-cells"
        }
        "dmas" => "#dma-cells",
        "thermal-sensors" => "#thermal-sensor-cells",
        "sound-dai" => "#sound-dai-cells",
        "mboxes" => "#mbox-cells",
        "pwms" => "#pwm-cells",
        _ => {
            debug!("Unknown property '{}', defaulting to 0 cell", prop_name);
            return 0;
        }
    };

    cells_info.get(cells_property).copied().unwrap_or(0) as usize
}

/// 通用的phandle属性解析函数
/// 根据cells信息按正确的块大小解析phandle引用
/// 支持单个phandle和多个phandle+specifier格式
/// 返回完整路径而不是节点名称
fn parse_phandle_property(
    prop_data: &[u8],
    prop_name: &str,
    phandle_map: &BTreeMap<u32, (String, BTreeMap<String, u32>)>,
) -> Vec<String> {
    let mut dependencies = Vec::new();

    let phandle_refs = parse_phandle_property_with_cells(prop_data, prop_name, phandle_map);

    for (phandle, specifiers) in phandle_refs {
        if let Some((device_path, _cells_info)) = phandle_map.get(&phandle) {
            let spec_info = if !specifiers.is_empty() {
                format!(" (specifiers: {:?})", specifiers)
            } else {
                String::new()
            };
            debug!(
                "Found {} dependency: phandle={:#x}, device={}{}",
                prop_name, phandle, device_path, spec_info
            );
            dependencies.push(device_path.clone());
        }
    }

    dependencies
}

/// 设备属性分类器 - 用于识别需要特殊处理的属性
struct DevicePropertyClassifier;

impl DevicePropertyClassifier {
    /// 需要特殊处理的phandle属性 - 包含所有需要解析依赖关系的属性
    const PHANDLE_PROPERTIES: &'static [&'static str] = &[
        "clocks",
        "power-domains",
        "phys",
        "resets",
        "dmas",
        "thermal-sensors",
        "mboxes",
        "assigned-clocks",
        "interrupt-parent",
        "phy-handle",
        "msi-parent",
        "memory-region",
        "syscon",
        "regmap",
        "iommus",
        "interconnects",
        "nvmem-cells",
        "sound-dai",
        "pinctrl-0",
        "pinctrl-1",
        "pinctrl-2",
        "pinctrl-3",
        "pinctrl-4",
    ];

    /// 判断是否为需要处理的phandle属性
    fn is_phandle_property(prop_name: &str) -> bool {
        Self::PHANDLE_PROPERTIES.contains(&prop_name)
            || prop_name.ends_with("-supply")
            || prop_name == "gpios"
            || prop_name.ends_with("-gpios")
            || prop_name.ends_with("-gpio")
            || (prop_name.contains("cells") && !prop_name.starts_with("#") && prop_name.len() >= 4)
    }
}

/// 查找设备的依赖关系
/// 现在接受设备节点路径而不是节点引用
/// 优化版本：使用node_cache而不是遍历所有节点
fn find_device_dependencies(
    device_node_path: &str,
    phandle_map: &BTreeMap<u32, (String, BTreeMap<String, u32>)>,
    node_cache: &BTreeMap<String, Vec<Node>>,  // 添加node_cache参数
) -> Vec<String> {
    let mut dependencies = Vec::new();

    // 直接从node_cache中查找节点，避免遍历所有节点
    if let Some(nodes) = node_cache.get(device_node_path) {
        // 遍历节点的所有属性查找依赖关系
        for node in nodes {
            for prop in node.propertys() {
                // 判断是否为需要处理的phandle属性
                if DevicePropertyClassifier::is_phandle_property(prop.name) {
                    let mut prop_deps = parse_phandle_property(prop.raw_value(), prop.name, phandle_map);
                    dependencies.append(&mut prop_deps);
                }
            }
        }
    }

    dependencies
}

/// 根据父节点路径获取所有后代节点（包括子节点、孙节点等）
/// 通过在node_cache中查找以父节点路径为前缀的节点来获取所有后代节点
/// 这种方法比get_all_descendant_nodes更高效，因为它直接使用缓存
fn get_descendant_nodes_by_path<'a>(node_cache: &'a BTreeMap<String, Vec<Node<'a>>>, parent_path: &str) -> Vec<String> {
    let mut descendant_paths = Vec::new();
    
    // 如果父路径是根路径，特殊处理
    let search_prefix = if parent_path == "/" {
        "/".to_string()
    } else {
        parent_path.to_string() + "/"
    };
    
    // 遍历node_cache，查找所有以父路径为前缀的节点
    for (path, _nodes) in node_cache {
        // 检查路径是否以父路径为前缀（并且不是父路径本身）
        if path.starts_with(&search_prefix) && path.len() > search_prefix.len() {
            // 这是后代节点的路径，添加到结果中
            descendant_paths.push(path.clone());
        }
    }
    
    descendant_paths
}

