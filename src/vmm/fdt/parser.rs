//! FDT parsing and processing functionality.
use alloc::vec::Vec;
use alloc::string::ToString;
use fdt_parser::{Fdt, FdtHeader};
use axvm::config::{AxVMConfig, AxVMCrateConfig};


pub fn parse_fdt(fdt_addr: usize, vm_cfg: &mut AxVMConfig, crate_config: &AxVMCrateConfig) {
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

    set_phys_cpu_sets(vm_cfg, &fdt, crate_config);
    // 调用修改后的函数并获取返回的设备名称列表
    let passthrough_device_names = super::device::find_all_passthrough_devices(vm_cfg, &fdt);

    super::create::crate_guest_fdt(&fdt, &passthrough_device_names, crate_config);
    // 注意：这里我们不再需要将设备添加到VM配置中，因为函数已经返回了设备名称列表
}

pub fn set_phys_cpu_sets(vm_cfg: &mut AxVMConfig, fdt: &Fdt, crate_config: &AxVMCrateConfig) {
    // Find and parse CPU information from host DTB
    let host_cpus: Vec<_> = fdt.find_nodes("/cpus/cpu").collect();
    info!("Found {} host CPU nodes", &host_cpus.len());

    let phys_cpu_ids = crate_config.base.phys_cpu_ids.as_ref().expect("ERROR: phys_cpu_ids not found in config.toml");
    debug!("phys_cpu_ids: {:?}", phys_cpu_ids);

    // 收集所有CPU节点信息到Vec中，避免多次使用迭代器
    let cpu_nodes_info: Vec<_> = host_cpus
        .iter()
        .filter_map(|cpu_node| {
            if let Some(mut cpu_reg) = cpu_node.reg() {
                if let Some(r) = cpu_reg.next() {
                    Some((cpu_node.name().to_string(), r.address as usize))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    debug!("cpu_nodes_info: {:?}", cpu_nodes_info);
    // 创建从phys_cpu_id到物理CPU索引的映射
    // 收集所有唯一的CPU地址，保持设备树中的出现顺序
    let mut unique_cpu_addresses = Vec::new();
    for (_, cpu_address) in &cpu_nodes_info {
        if !unique_cpu_addresses.contains(cpu_address) {
            unique_cpu_addresses.push(*cpu_address);
        } else {
            panic!("Duplicate CPU address found");
        }
    }

    // 为设备树中的每个CPU地址分配索引，并打印详细信息
    for (index, &cpu_address) in unique_cpu_addresses.iter().enumerate() {
        // 找到所有使用这个地址的CPU节点
        for (cpu_name, node_address) in &cpu_nodes_info {
            if *node_address == cpu_address {
                debug!(
                    "  CPU node: {}, address: 0x{:x}, assigned index: {}",
                    cpu_name, cpu_address, index
                );
                break; // 每个地址只打印一次
            }
        }
    }

    // 根据vcpu_mappings中的phys_cpu_ids计算phys_cpu_sets
    let mut new_phys_cpu_sets = Vec::new();
    for phys_cpu_id in phys_cpu_ids {
        // 在unique_cpu_addresses中查找phys_cpu_id对应的索引
        if let Some(cpu_index) = unique_cpu_addresses
            .iter()
            .position(|&addr| addr == *phys_cpu_id)
        {
            let cpu_mask = 1usize << cpu_index; // 将索引转换为掩码位
            new_phys_cpu_sets.push(cpu_mask);
            debug!(
                "vCPU {} with phys_cpu_id 0x{:x} mapped to CPU index {} (mask: 0x{:x})",
                vm_cfg.id(), phys_cpu_id, cpu_index, cpu_mask
            );
        } else {
            error!(
                "vCPU {} with phys_cpu_id 0x{:x} not found in device tree!",
                vm_cfg.id(), phys_cpu_id
            );
        }
    }

    // 更新VM配置中的phys_cpu_sets（如果VM配置支持设置的话）
    info!("Calculated phys_cpu_sets: {:?}", new_phys_cpu_sets);

    vm_cfg.phys_cpu_ls_mut().set_guest_cpu_sets(new_phys_cpu_sets);

    let vcpu_mappings = vm_cfg.phys_cpu_ls_mut().get_vcpu_affinities_pcpu_ids();
    info!("vcpu_mappings: {:?}", vcpu_mappings);
}