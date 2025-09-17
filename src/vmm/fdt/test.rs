//! FDT parsing and processing functionality.

use fdt_parser::{Fdt, FdtHeader};

#[allow(dead_code)]
pub fn print_fdt(fdt_addr: usize) {
    const FDT_VALID_MAGIC: u32 = 0xd00d_feed;
    let header = unsafe {
        core::slice::from_raw_parts(fdt_addr as *const u8, core::mem::size_of::<FdtHeader>())
    };
    let fdt_header = FdtHeader::from_bytes(header)
        .map_err(|e| format!("Failed to parse FDT header: {:#?}", e))
        .unwrap();

    if fdt_header.magic.get() != FDT_VALID_MAGIC {
        error!(
            "FDT magic is invalid, expected {:#x}, got {:#x}",
            FDT_VALID_MAGIC,
            fdt_header.magic.get()
        );
        return;
    }

    let fdt_bytes =
        unsafe { core::slice::from_raw_parts(fdt_addr as *const u8, fdt_header.total_size()) };

    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
        .expect("Failed to parse FDT");

    // 统计节点数量和层级分布
    let mut node_count = 0;
    let mut level_counts = alloc::collections::BTreeMap::new();
    let mut max_level = 0;

    info!("=== FDT节点信息统计 ===");

    // 一次性遍历所有节点进行统计（遵循优化策略）
    for node in fdt.all_nodes() {
        node_count += 1;

        // 按层级统计节点数量
        *level_counts.entry(node.level).or_insert(0) += 1;

        // 记录最大层级
        if node.level > max_level {
            max_level = node.level;
        }

        // 统计属性数量
        let node_properties_count = node.propertys().count();

        trace!(
            "节点[{}]: {} (层级: {}, 属性: {})",
            node_count,
            node.name(),
            node.level,
            node_properties_count
        );

        for _prop in node.propertys() {
            // info!("属性: {}, 节点: {}", prop.name, node.name());
        }
    }

    info!("=== FDT统计结果 ===");
    info!("总节点数量: {}", node_count);
    info!("FDT总大小: {} 字节", fdt_header.total_size());
    info!("最大层级深度: {}", max_level);

    info!("各层级节点分布:");
    for (level, count) in level_counts {
        let percentage = (count as f32 / node_count as f32) * 100.0;
        info!("  层级 {}: {} 个节点 ({:.1}%)", level, count, percentage);
    }
}


#[allow(dead_code)]
pub fn print_guest_fdt(fdt_bytes: &[u8]) {

    let fdt = Fdt::from_bytes(fdt_bytes)
        .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
        .expect("Failed to parse FDT");
    // 统计节点数量和层级分布
    let mut node_count = 0;
    let mut level_counts = alloc::collections::BTreeMap::new();
    let mut max_level = 0;

    info!("=== FDT节点信息统计 ===");

    // 一次性遍历所有节点进行统计（遵循优化策略）
    for node in fdt.all_nodes() {
        node_count += 1;

        // 按层级统计节点数量
        *level_counts.entry(node.level).or_insert(0) += 1;

        // 记录最大层级
        if node.level > max_level {
            max_level = node.level;
        }

        // 统计属性数量
        let node_properties_count = node.propertys().count();

        trace!(
            "节点[{}]: {} (层级: {}, 属性: {})",
            node_count,
            node.name(),
            node.level,
            node_properties_count
        );

        for _prop in node.propertys() {
            trace!("属性: {}, 节点: {}", _prop.name, node.name());
        }
    }

    info!("=== FDT统计结果 ===");
    info!("总节点数量: {}", node_count);
    info!("最大层级深度: {}", max_level);

    info!("各层级节点分布:");
    for (level, count) in level_counts {
        let percentage = (count as f32 / node_count as f32) * 100.0;
        info!("  层级 {}: {} 个节点 ({:.1}%)", level, count, percentage);
    }
}