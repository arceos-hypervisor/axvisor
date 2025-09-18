//! FDT parsing and processing functionality.
use alloc::vec::Vec;
use alloc::string::ToString;
use fdt_parser::{Fdt, FdtHeader, PciSpace};
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

    let _ = super::create::crate_guest_fdt_with_cache(&fdt, &passthrough_device_names, crate_config);
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

pub fn parse_passthrough_devices_address(vm_cfg: &mut AxVMConfig, dtb: &[u8]) { 
    let fdt = Fdt::from_bytes(dtb)
        .expect("Failed to parse DTB image, perhaps the DTB is invalid or corrupted");

    info!("before clear, all: {:?}", vm_cfg.pass_through_devices());
    // 清空现有的直通设备配置
    vm_cfg.clear_pass_through_devices();

    info!("after clear, all: {:?}", vm_cfg.pass_through_devices());

    // 遍历所有设备树节点
    for node in fdt.all_nodes() {
        // 跳过根节点
        if node.name() == "/" {
            continue;
        }
        
        let node_name = node.name().to_string();
        
        // 检查是否为PCIe设备节点
        if node_name.starts_with("pcie@") || node_name.contains("pci") {
            // 处理PCIe设备的ranges属性
            if let Some(pci) = node.into_pci() {
                if let Ok(ranges) = pci.ranges() {
                    for (index, range) in ranges.enumerate() {
                        let base_address = range.cpu_address as usize;
                        let size = range.size as usize;
                        
                        // 只处理有地址信息的设备
                        if size > 0 {
                            // 为每个地址段创建一个设备配置
                            let device_name = if index == 0 {
                                format!("{}-{}", node_name, match range.space {
                                    PciSpace::Configuration => "config",
                                    PciSpace::IO => "io",
                                    PciSpace::Memory32 => "mem32",
                                    PciSpace::Memory64 => "mem64",
                                })
                            } else {
                                format!("{}-{}-region{}", node_name, match range.space {
                                    PciSpace::Configuration => "config",
                                    PciSpace::IO => "io",
                                    PciSpace::Memory32 => "mem32",
                                    PciSpace::Memory64 => "mem64",
                                }, index)
                            };
                            
                            // 添加新的设备配置
                            let pt_dev = axvm::config::PassThroughDeviceConfig {
                                name: device_name,
                                base_gpa: base_address,
                                base_hpa: base_address,
                                length: size,
                                irq_id: 0,
                            };
                            vm_cfg.add_pass_through_device(pt_dev);
                            
                            trace!("Added PCIe passthrough device {}: base=0x{:x}, size=0x{:x}, space={:?}", 
                                node_name, base_address, size, range.space);
                        }
                    }
                }
            }
        } else {
            // 获取设备的reg属性（处理普通设备）
            if let Some(mut reg_iter) = node.reg() {
                // 处理设备的所有地址段
                let mut index = 0;
                while let Some(reg) = reg_iter.next() {
                    // 获取设备的地址和大小信息
                    let base_address = reg.address as usize;
                    let size = reg.size.unwrap_or(0) as usize;
                    
                    // 只处理有地址信息的设备
                    if size > 0 {
                        // 为每个地址段创建一个设备配置
                        // 如果设备有多个地址段，使用索引来区分
                        let device_name = if index == 0 {
                            node_name.clone()
                        } else {
                            format!("{}-region{}", node_name, index)
                        };
                        
                        // 添加新的设备配置
                        let pt_dev = axvm::config::PassThroughDeviceConfig {
                            name: device_name,
                            base_gpa: base_address,
                            base_hpa: base_address,
                            length: size,
                            irq_id: 0,
                        };
                        vm_cfg.add_pass_through_device(pt_dev);
                        
                        trace!("Added passthrough device {}: base=0x{:x}, size=0x{:x}", node_name, base_address, size);
                    }
                    
                    index += 1;
                }
            }
        }
    }
    info!("All passthrough devices: {:#x?}", vm_cfg.pass_through_devices());
    info!("Finished parsing passthrough devices, total: {}", vm_cfg.pass_through_devices().len());
}
