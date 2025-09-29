use std::{
    collections::btree_map::BTreeMap,
    fs::read_to_string,
    println,
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    shell::command::{CommandNode, FlagDef, OptionDef, ParsedCommand},
    vmm::{
        config::init_guest_vm, get_running_vm_count, set_running_vm_count, vcpus, vm_list, with_vm,
    },
};

fn vm_help(_cmd: &ParsedCommand) {
    println!("VM - virtual machine management");
    println!("Most commonly used vm commands:");
    println!("  create    Create a new virtual machine");
    println!("  start     Start a virtual machine");
    println!("  stop      Stop a virtual machine");
    println!("  restart   Restart a virtual machine");
    println!("  delete    Delete a virtual machine");
    println!("  list      Show virtual machine lists");
    println!("  show      Show virtual machine details");
    println!("  status    Show virtual machine status");
    println!();
    println!("Use 'vm <command> --help' for more information on a specific command.");
}

fn vm_create(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    println!("Positional args: {:?}", args);

    if args.is_empty() {
        println!("Error: No VM configuration file specified");
        println!("Usage: vm create [CONFIG_FILE]");
        return;
    }

    let initial_vm_count = vm_list::get_vm_list().len();

    let mut processed_count = 0;
    for config_path in args.iter() {
        println!("Creating VM from config: {}", config_path);

        match read_to_string(config_path) {
            Ok(raw_cfg) => match init_guest_vm(&raw_cfg) {
                Ok(_) => {
                    println!("✓ Successfully created VM from config: {}", config_path);
                    processed_count += 1;
                }
                Err(_) => {
                    println!(
                        "✗ Failed to create VM from {}: Configuration error or panic occurred",
                        config_path
                    );
                }
            },
            Err(e) => {
                println!("✗ Failed to read config file {}: {:?}", config_path, e);
            }
        }
    }

    // Check the actual number of VMs created
    let final_vm_count = vm_list::get_vm_list().len();
    let created_count = final_vm_count - initial_vm_count;

    if created_count > 0 {
        println!("Successfully created {} VM(s)", created_count);
    } else if processed_count > 0 {
        println!(
            "Processed {} config file(s) but no VMs were actually created",
            processed_count
        );
    }
}

fn vm_start(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let detach = cmd.flags.get("detach").unwrap_or(&false);

    if args.is_empty() {
        // start all VMs
        info!("VMM starting, booting all VMs...");
        let mut started_count = 0;

        for vm in vm_list::get_vm_list() {
            // Set up primary virtual CPU before starting
            vcpus::setup_vm_primary_vcpu(vm.clone());

            match vm.boot() {
                Ok(_) => {
                    vcpus::notify_primary_vcpu(vm.id());
                    set_running_vm_count(1);
                    println!("✓ VM[{}] started successfully", vm.id());
                    started_count += 1;
                }
                Err(err) => {
                    println!("✗ VM[{}] failed to start: {:?}", vm.id(), err);
                }
            }
        }
        println!("Started {} VM(s)", started_count);
    } else {
        // Start specified VMs
        for vm_name in args {
            // Try to parse as VM ID or lookup VM name
            if let Ok(vm_id) = vm_name.parse::<usize>() {
                start_vm_by_id(vm_id);
            } else {
                println!("Error: VM name lookup not implemented. Use VM ID instead.");
                println!("Available VMs:");
                vm_list_simple();
            }
        }
    }

    if *detach {
        println!("VMs started in background mode");
    }
}

fn start_vm_by_id(vm_id: usize) {
    // Set up primary virtual CPU before starting
    match with_vm(vm_id, |vm| {
        vcpus::setup_vm_primary_vcpu(vm.clone());
        vm.boot()
    }) {
        Some(Ok(_)) => {
            vcpus::notify_primary_vcpu(vm_id);
            set_running_vm_count(1);
            println!("✓ VM[{}] started successfully", vm_id);
        }
        Some(Err(err)) => {
            println!("✗ VM[{}] failed to start: {:?}", vm_id, err);
        }
        None => {
            println!("✗ VM[{}] not found", vm_id);
        }
    }
}

fn vm_stop(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let force = cmd.flags.get("force").unwrap_or(&false);

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm stop [OPTIONS] <VM_ID>");
        return;
    }

    for vm_name in args {
        if let Ok(vm_id) = vm_name.parse::<usize>() {
            stop_vm_by_id(vm_id, *force);
        } else {
            println!("Error: Invalid VM ID: {}", vm_name);
        }
    }
}

fn stop_vm_by_id(vm_id: usize, force: bool) {
    match with_vm(vm_id, |vm| {
        if force {
            println!("Force stopping VM[{}]...", vm_id);
            // Force shutdown, directly call shutdown
            vm.shutdown()
        } else {
            println!("Stopping VM[{}]...", vm_id);
            vm.shutdown()
        }
    }) {
        Some(Ok(_)) => {
            println!("✓ VM[{}] stopped successfully", vm_id);
        }
        Some(Err(err)) => {
            println!("✗ Failed to stop VM[{}]: {:?}", vm_id, err);
        }
        None => {
            println!("✗ VM[{}] not found", vm_id);
        }
    }
}

fn vm_restart(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let force = cmd.flags.get("force").unwrap_or(&false);

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm restart [OPTIONS] <VM_ID>");
        return;
    }

    for vm_name in args {
        if let Ok(vm_id) = vm_name.parse::<usize>() {
            restart_vm_by_id(vm_id, *force);
        } else {
            println!("Error: Invalid VM ID: {}", vm_name);
        }
    }
}

fn restart_vm_by_id(vm_id: usize, force: bool) {
    println!("Restarting VM[{}]...", vm_id);

    // First stop the virtual machine
    stop_vm_by_id(vm_id, force);

    // Wait for a period to ensure complete shutdown
    // In actual implementation, more complex state checking may be needed

    // Restart the virtual machine
    start_vm_by_id(vm_id);
}

fn vm_delete(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let force = cmd.flags.get("force").unwrap_or(&false);
    let keep_data = cmd.flags.get("keep-data").unwrap_or(&false);

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm delete [OPTIONS] <VM_ID>");
        return;
    }

    let vm_name = &args[0];

    if let Ok(vm_id) = vm_name.parse::<usize>() {
        if !force {
            println!(
                "Are you sure you want to delete VM[{}]? (This operation cannot be undone)",
                vm_id
            );
            println!("Use --force to skip confirmation");
            return;
        }

        delete_vm_by_id(vm_id, *keep_data);
    } else {
        println!("Error: Invalid VM ID: {}", vm_name);
    }
}

fn delete_vm_by_id(vm_id: usize, keep_data: bool) {
    // First ensure VM is stopped
    with_vm(vm_id, |vm| vm.shutdown()).unwrap_or(Ok(())).ok();

    // Remove VM from global list
    match crate::vmm::vm_list::remove_vm(vm_id) {
        Some(_) => {
            if keep_data {
                println!("✓ VM[{}] deleted (data preserved)", vm_id);
            } else {
                println!("✓ VM[{}] deleted completely", vm_id);
                // Here all VM-related data files should be cleaned up
            }
        }
        None => {
            println!("✗ VM[{}] not found", vm_id);
        }
    }
}

fn vm_list_simple() {
    let vms = vm_list::get_vm_list();
    println!("ID    NAME           STATE      VCPU   MEMORY");
    println!("----  -----------    -------    ----   ------");
    for vm in vms {
        let state = if vm.running() {
            "running"
        } else if vm.shutting_down() {
            "stopping"
        } else {
            "stopped"
        };

        // Calculate total memory size
        let total_memory: usize = vm.memory_regions().iter().map(|region| region.size()).sum();

        println!(
            "{:<4}  {:<11}    {:<7}    {:<4}   {}MB",
            vm.id(),
            vm.with_config(|cfg| cfg.name()),
            state,
            vm.vcpu_num(),
            total_memory / (1024 * 1024) // Convert to MB
        );
    }
}

fn vm_list(cmd: &ParsedCommand) {
    let show_all = cmd.flags.get("all").unwrap_or(&false);
    let binding = "table".to_string();
    let format = cmd.options.get("format").unwrap_or(&binding);

    let vms = vm_list::get_vm_list();

    if format == "json" {
        println!("{{");
        println!("  \"vms\": [");
        for (i, vm) in vms.iter().enumerate() {
            let state = if vm.running() {
                "running"
            } else if vm.shutting_down() {
                "stopping"
            } else {
                "stopped"
            };

            let total_memory: usize = vm.memory_regions().iter().map(|region| region.size()).sum();

            println!("    {{");
            println!("      \"id\": {},", vm.id());
            println!("      \"name\": \"{}\",", vm.with_config(|cfg| cfg.name()));
            println!("      \"state\": \"{}\",", state);
            println!("      \"vcpu\": {},", vm.vcpu_num());
            println!("      \"memory\": \"{}MB\",", total_memory / (1024 * 1024));
            println!(
                "      \"interrupt_mode\": \"{:?}\"",
                vm.with_config(|cfg| cfg.interrupt_mode())
            );

            if i < vms.len() - 1 {
                println!("    }},");
            } else {
                println!("    }}");
            }
        }
        println!("  ]");
        println!("}}");
    } else {
        println!("Virtual Machines:");
        if vms.is_empty() {
            println!("No virtual machines found.");
            return;
        }

        // Count running VMs before filtering
        let running_count = vms.iter().filter(|vm| vm.running()).count();
        let total_count = vms.len();

        // Filter displayed VMs
        let display_vms: Vec<_> = if *show_all {
            vms
        } else {
            vms.into_iter().filter(|vm| vm.running()).collect()
        };

        if display_vms.is_empty() && !*show_all {
            println!("No running virtual machines found.");
            println!("Use --all to show all VMs including stopped ones.");
            return;
        }

        println!("ID    NAME           STATE      VCPU   MEMORY");
        println!("----  -----------    -------    ----   ------");
        for vm in display_vms {
            let state = if vm.running() {
                "🟢 running"
            } else if vm.shutting_down() {
                "🟡 stopping"
            } else {
                "🔴 stopped"
            };

            let total_memory: usize = vm.memory_regions().iter().map(|region| region.size()).sum();

            println!(
                "{:<4}  {:<11}    {:<9}    {:<4}   {:<8}",
                vm.id(),
                vm.with_config(|cfg| cfg.name()),
                state,
                vm.vcpu_num(),
                format!("{}MB", total_memory / (1024 * 1024))
            );
        }

        if !show_all && running_count < total_count {
            println!(
                "\nShowing {} running VMs. Use --all to show all {} VMs.",
                running_count, total_count
            );
        }
    }
}

fn vm_show(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let show_config = cmd.flags.get("config").unwrap_or(&false);
    let show_stats = cmd.flags.get("stats").unwrap_or(&false);

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm show [OPTIONS] <VM_ID>");
        return;
    }

    let vm_name = &args[0];
    if let Ok(vm_id) = vm_name.parse::<usize>() {
        show_vm_details(vm_id, *show_config, *show_stats);
    } else {
        println!("Error: Invalid VM ID: {}", vm_name);
    }
}

/// Show detailed information about a specific VM.
fn show_vm_details(vm_id: usize, show_config: bool, show_stats: bool) {
    match with_vm(vm_id, |vm| {
        let state = if vm.running() {
            "🟢 running"
        } else if vm.shutting_down() {
            "🟡 stopping"
        } else {
            "🔴 stopped"
        };

        println!("VM Details: {}", vm_id);
        println!("  ID: {}", vm.id());
        println!("  Name: {}", vm.with_config(|cfg| cfg.name()));
        println!("  State: {}", state);
        println!("  VCPUs: {}", vm.vcpu_num());

        // show VCPU information
        println!("  VCPU List:");
        for (i, vcpu) in vm.vcpu_list().iter().enumerate() {
            if let Some(phys_cpu_set) = vcpu.phys_cpu_set() {
                println!("    VCPU[{}]: CPU affinity mask = {:#x}", i, phys_cpu_set);
            } else {
                println!("    VCPU[{}]: No CPU affinity set", i);
            }
        }

        if show_config {
            println!();
            println!("Configuration:");
            vm.with_config(|cfg| {
                println!("  BSP Entry: {:#x}", cfg.bsp_entry().as_usize());
                println!("  AP Entry: {:#x}", cfg.ap_entry().as_usize());
                println!("  Interrupt Mode: {:?}", cfg.interrupt_mode());

                // show passthrough devices
                if !cfg.pass_through_devices().is_empty() {
                    println!("  Passthrough Devices:");
                    for device in cfg.pass_through_devices() {
                        println!(
                            "    {}: GPA[{:#x}~{:#x}] -> HPA[{:#x}~{:#x}]",
                            device.name,
                            device.base_gpa,
                            device.base_gpa + device.length,
                            device.base_hpa,
                            device.base_hpa + device.length
                        );
                    }
                }

                // show emulated devices
                if !cfg.emu_devices().is_empty() {
                    println!("  Emulated Devices:");
                    for device in cfg.emu_devices() {
                        println!("    {:?}", device);
                    }
                }
            });
        }

        if show_stats {
            println!();
            println!("Statistics:");
            println!("  EPT Root: {:#x}", vm.ept_root().as_usize());
            println!(
                "  Device Count: {}",
                vm.get_devices().iter_mmio_dev().count()
            );

            let mut vcpu_states = BTreeMap::new();
            for vcpu in vm.vcpu_list() {
                let state_key = match vcpu.state() {
                    axvcpu::VCpuState::Free => "Free",
                    axvcpu::VCpuState::Running => "Running",
                    axvcpu::VCpuState::Blocked => "Blocked",
                    axvcpu::VCpuState::Invalid => "Invalid",
                    axvcpu::VCpuState::Created => "Created",
                    axvcpu::VCpuState::Ready => "Ready",
                };
                *vcpu_states.entry(state_key).or_insert(0) += 1;
            }

            println!("  VCPU States:");
            for (state, count) in vcpu_states {
                println!("    {}: {}", state, count);
            }
        }
    }) {
        Some(_) => {}
        None => {
            println!("✗ VM[{}] not found", vm_id);
        }
    }
}

fn vm_status(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let watch = cmd.flags.get("watch").unwrap_or(&false);

    if args.is_empty() {
        // show all VM status
        show_all_vm_status(*watch);
        return;
    }

    let vm_name = &args[0];
    if let Ok(vm_id) = vm_name.parse::<usize>() {
        show_vm_status(vm_id, *watch);
    } else {
        println!("Error: Invalid VM ID: {}", vm_name);
    }
}

/// Show status of a specific VM.
fn show_vm_status(vm_id: usize, watch: bool) {
    if watch {
        println!("Watching VM[{}] status (press Ctrl+C to stop):", vm_id);
        // TODO: add real-time status information
    }

    match with_vm(vm_id, |vm| {
        let state = if vm.running() {
            "🟢 running"
        } else if vm.shutting_down() {
            "🟡 stopping"
        } else {
            "🔴 stopped"
        };

        println!("Virtual machine status for VM[{}]:", vm_id);
        println!("  ID: {}", vm.id());
        println!("  Name: {}", vm.with_config(|cfg| cfg.name()));
        println!("  State: {}", state);
        println!("  VCPUs: {}", vm.vcpu_num());

        // Calculate total memory
        let total_memory: usize = vm.memory_regions().iter().map(|region| region.size()).sum();

        println!("  Total Memory: {}MB", total_memory / (1024 * 1024));

        // Show memory region details
        println!("  Memory Regions:");
        for (i, region) in vm.memory_regions().iter().enumerate() {
            println!(
                "    Region[{}]: GPA[{:#x}~{:#x}] Size={}KB",
                i,
                region.gpa,
                region.gpa + region.size(),
                region.size() / 1024
            );
        }

        println!("  VCPU Details:");
        for vcpu in vm.vcpu_list() {
            let vcpu_state = match vcpu.state() {
                axvcpu::VCpuState::Free => "Free",
                axvcpu::VCpuState::Running => "Running",
                axvcpu::VCpuState::Blocked => "Blocked",
                axvcpu::VCpuState::Invalid => "Invalid",
                axvcpu::VCpuState::Created => "Created",
                axvcpu::VCpuState::Ready => "Ready",
            };

            if let Some(phys_cpu_set) = vcpu.phys_cpu_set() {
                println!(
                    "    VCPU[{}]: {} (CPU affinity: {:#x})",
                    vcpu.id(),
                    vcpu_state,
                    phys_cpu_set
                );
            } else {
                println!("    VCPU[{}]: {} (No CPU affinity)", vcpu.id(), vcpu_state);
            }
        }

        // show device information
        let mmio_dev_count = vm.get_devices().iter_mmio_dev().count();
        println!("  Devices: {} MMIO devices", mmio_dev_count);

        // TODO: add more real-time status information
        // println!("  Network: connected/disconnected");
        // println!("  Uptime: {} seconds", uptime);
    }) {
        Some(_) => {}
        None => {
            println!("✗ VM[{}] not found", vm_id);
        }
    }
}

/// Show status of all VMs in a summary format.
fn show_all_vm_status(watch: bool) {
    if watch {
        println!("Watching all VMs status (press Ctrl+C to stop):");
    }

    let vms = vm_list::get_vm_list();
    if vms.is_empty() {
        println!("No virtual machines found.");
        return;
    }

    println!("System Status:");
    println!("  Total VMs: {}", vms.len());
    println!("  Running VMs: {}", get_running_vm_count());

    let mut running_count = 0;
    let mut stopping_count = 0;
    let mut stopped_count = 0;
    let mut total_vcpus = 0;
    let mut total_memory = 0;

    for vm in &vms {
        if vm.running() {
            running_count += 1;
        } else if vm.shutting_down() {
            stopping_count += 1;
        } else {
            stopped_count += 1;
        }

        total_vcpus += vm.vcpu_num();
        total_memory += vm
            .memory_regions()
            .iter()
            .map(|region| region.size())
            .sum::<usize>();
    }

    println!("  Total VCPUs: {}", total_vcpus);
    println!("  Total Memory: {}MB", total_memory / (1024 * 1024));
    println!();

    println!("VM Status Overview:");
    println!("  🟢 Running:  {}", running_count);
    println!("  🟡 Stopping: {}", stopping_count);
    println!("  🔴 Stopped:  {}", stopped_count);
    println!();

    println!("Individual VM Status:");
    for vm in vms {
        let state_icon = if vm.running() {
            "🟢"
        } else if vm.shutting_down() {
            "🟡"
        } else {
            "🔴"
        };

        let vm_memory: usize = vm.memory_regions().iter().map(|region| region.size()).sum();

        println!(
            "  {} VM[{}] {} ({} VCPUs, {}MB)",
            state_icon,
            vm.id(),
            vm.with_config(|cfg| cfg.name()),
            vm.vcpu_num(),
            vm_memory / (1024 * 1024),
        );

        if vm.running() {
            let mut vcpu_summary = BTreeMap::new();
            for vcpu in vm.vcpu_list() {
                let state = match vcpu.state() {
                    axvcpu::VCpuState::Free => "Free",
                    axvcpu::VCpuState::Running => "Running",
                    axvcpu::VCpuState::Blocked => "Blocked",
                    axvcpu::VCpuState::Invalid => "Invalid",
                    axvcpu::VCpuState::Created => "Created",
                    axvcpu::VCpuState::Ready => "Ready",
                };
                *vcpu_summary.entry(state).or_insert(0) += 1;
            }

            let summary_str: Vec<String> = vcpu_summary
                .into_iter()
                .map(|(state, count)| format!("{state}:{count}"))
                .collect();

            if !summary_str.is_empty() {
                println!("      VCPUs: {}", summary_str.join(", "));
            }
        }
    }
}

/// Build the VM command tree and register it.
pub fn build_vm_cmd(tree: &mut BTreeMap<String, CommandNode>) {
    let create_cmd = CommandNode::new("Create a new virtual machine")
        .with_handler(vm_create)
        .with_usage("vm create [OPTIONS] <CONFIG_FILE>...")
        .with_option(
            OptionDef::new("name", "Virtual machine name")
                .with_short('n')
                .with_long("name"),
        )
        .with_option(
            OptionDef::new("cpu", "Number of CPU cores")
                .with_short('c')
                .with_long("cpu"),
        )
        .with_option(
            OptionDef::new("memory", "Amount of memory")
                .with_short('m')
                .with_long("memory"),
        )
        .with_flag(
            FlagDef::new("force", "Force creation without confirmation")
                .with_short('f')
                .with_long("force"),
        );

    let start_cmd = CommandNode::new("Start a virtual machine")
        .with_handler(vm_start)
        .with_usage("vm start [OPTIONS] [VM_ID...]")
        .with_flag(
            FlagDef::new("detach", "Start in background")
                .with_short('d')
                .with_long("detach"),
        )
        .with_flag(
            FlagDef::new("console", "Attach to console")
                .with_short('c')
                .with_long("console"),
        );

    let stop_cmd = CommandNode::new("Stop a virtual machine")
        .with_handler(vm_stop)
        .with_usage("vm stop [OPTIONS] <VM_ID>...")
        .with_flag(
            FlagDef::new("force", "Force stop")
                .with_short('f')
                .with_long("force"),
        )
        .with_flag(
            FlagDef::new("graceful", "Graceful shutdown")
                .with_short('g')
                .with_long("graceful"),
        );

    let restart_cmd = CommandNode::new("Restart a virtual machine")
        .with_handler(vm_restart)
        .with_usage("vm restart [OPTIONS] <VM_ID>...")
        .with_flag(
            FlagDef::new("force", "Force restart")
                .with_short('f')
                .with_long("force"),
        );

    let delete_cmd = CommandNode::new("Delete a virtual machine")
        .with_handler(vm_delete)
        .with_usage("vm delete [OPTIONS] <VM_ID>")
        .with_flag(
            FlagDef::new("force", "Skip confirmation")
                .with_short('f')
                .with_long("force"),
        )
        .with_flag(FlagDef::new("keep-data", "Keep VM data").with_long("keep-data"));

    let list_cmd = CommandNode::new("Show virtual machine lists")
        .with_handler(vm_list)
        .with_usage("vm list [OPTIONS]")
        .with_flag(
            FlagDef::new("all", "Show all VMs including stopped ones")
                .with_short('a')
                .with_long("all"),
        )
        .with_option(OptionDef::new("format", "Output format (table, json)").with_long("format"));

    let show_cmd = CommandNode::new("Show virtual machine details")
        .with_handler(vm_show)
        .with_usage("vm show [OPTIONS] <VM_ID>")
        .with_flag(
            FlagDef::new("config", "Show configuration")
                .with_short('c')
                .with_long("config"),
        )
        .with_flag(
            FlagDef::new("stats", "Show statistics")
                .with_short('s')
                .with_long("stats"),
        );

    let status_cmd = CommandNode::new("Show virtual machine status")
        .with_handler(vm_status)
        .with_usage("vm status [OPTIONS] [VM_ID]")
        .with_flag(
            FlagDef::new("watch", "Watch status changes")
                .with_short('w')
                .with_long("watch"),
        );

    // main VM command
    let vm_node = CommandNode::new("Virtual machine management")
        .with_handler(vm_help)
        .with_usage("vm <command> [options] [args...]")
        .add_subcommand(
            "help",
            CommandNode::new("Show VM help").with_handler(vm_help),
        )
        .add_subcommand("create", create_cmd)
        .add_subcommand("start", start_cmd)
        .add_subcommand("stop", stop_cmd)
        .add_subcommand("restart", restart_cmd)
        .add_subcommand("delete", delete_cmd)
        .add_subcommand("list", list_cmd)
        .add_subcommand("show", show_cmd)
        .add_subcommand("status", status_cmd);

    tree.insert("vm".to_string(), vm_node);
}
