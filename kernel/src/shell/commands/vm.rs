//! Virtual machine management commands
//!
//! Commands for managing virtual machines (create, start, stop, list, etc.).

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
};

#[cfg(feature = "fs")]
use axstd::fs::read_to_string;
use axstd::println;

use crate::vmm::config::build_vmconfig;
use crate::vmm::vm_list;
use axvm::VMStatus;
use axvm::config::AxVMCrateConfig;

use super::super::parser::{CommandNode, FlagDef, OptionDef, ParsedCommand};

/// Format memory size in a human-readable way.
fn format_memory_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{}KB", bytes / 1024)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{}MB", bytes / (1024 * 1024))
    } else {
        format!("{}GB", bytes / (1024 * 1024 * 1024))
    }
}

// ============================================================================
// Command Handlers
// ============================================================================

fn vm_help(_cmd: &ParsedCommand) {
    println!("VM - virtual machine management");
    println!();
    println!("Most commonly used vm commands:");
    println!("  create    Create a new virtual machine");
    println!("  start     Start a virtual machine");
    println!("  stop      Stop a virtual machine");
    println!("  suspend   Suspend (pause) a running virtual machine");
    println!("  resume    Resume a suspended virtual machine");
    println!("  delete    Delete a virtual machine");
    println!();
    println!("Information commands:");
    println!("  list      Show table of all VMs");
    println!("  show      Show VM details (requires VM_ID)");
    println!("            - Default: basic information");
    println!("            - --full: complete detailed information");
    println!("            - --config: show configuration");
    println!("            - --stats: show statistics");
    println!();
    println!("Use 'vm <command> --help' for more information on a specific command.");
}

#[cfg(feature = "fs")]
fn vm_create(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    println!("Positional args: {:?}", args);

    if args.is_empty() {
        println!("Error: No VM configuration file specified");
        println!("Usage: vm create [CONFIG_FILE]");
        return;
    }

    let initial_vm_count = vm_list::get_vm_list().len();

    for config_path in args.iter() {
        println!("Creating VM from config: {}", config_path);

        // Read file content first
        let raw_cfg = match read_to_string(config_path) {
            Ok(content) => content,
            Err(e) => {
                println!("✗ Failed to read config file {}: {:?}", config_path, e);
                continue;
            }
        };

        // Parse TOML from string content
        let config_info: AxVMCrateConfig = match toml::from_str(&raw_cfg) {
            Ok(cfg) => cfg,
            Err(e) => {
                println!("✗ Failed to parse TOML from {}: {:?}", config_path, e);
                continue;
            }
        };

        match build_vmconfig(config_info) {
            Ok(vm_config) => match axvm::Vm::new(vm_config) {
                Ok(vm) => {
                    let vm = vm_list::push_vm(vm);
                    let vm_id = vm.id();
                    println!(
                        "✓ Successfully created VM[{}] from config: {}",
                        vm_id, config_path
                    );
                    println!("{:?}", vm.status());
                }
                Err(e) => {
                    println!("✗ Failed to create VM from {}: {:?}", config_path, e);
                }
            },
            Err(e) => {
                println!("✗ Failed to build VM config from {}: {:?}", config_path, e);
            }
        }
    }

    // Check the actual number of VMs created
    let final_vm_count = vm_list::get_vm_list().len();
    let created_count = final_vm_count - initial_vm_count;

    if created_count > 0 {
        println!("Successfully created {} VM(s)", created_count);
        println!("Use 'vm start <VM_ID>' to start the created VMs.");
    } else {
        println!("No VMs were created.");
    }
}

#[cfg(feature = "fs")]
fn vm_start(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    if args.is_empty() {
        // start all VMs
        info!("VMM starting, booting all VMs...");
        let mut started_count = 0;

        for vm in vm_list::get_vm_list() {
            let vm: vm_list::VMRef = vm;
            // Check current status before starting
            let status = vm.status();
            if status == VMStatus::Running {
                println!("⚠ VM[{}] is already running, skipping", vm.id());
                continue;
            }

            if status != VMStatus::Inited && status != VMStatus::Stopped {
                println!("⚠ VM[{}] is in {:?} state, cannot start", vm.id(), status);
                continue;
            }

            // Use vm.id() to get usize VM ID
            let vm_id = usize::from(vm.id());

            // Try to start the VM
            match vm.boot() {
                Ok(_) => {
                    println!("✓ VM[{}] started successfully", vm_id);
                    started_count += 1;
                }
                Err(e) => {
                    println!("✗ VM[{}] failed to start: {:?}", vm_id, e);
                }
            }
        }
        println!("Started {} VM(s)", started_count);
    } else {
        // Start specified VMs
        for arg in args {
            // Try to parse as VM ID or lookup VM name
            let arg: &String = arg;
            if let Ok(vm_id) = arg.parse::<usize>() {
                if !start_vm_by_id(vm_id) {
                    // VM not found, show available VMs
                    println!("Available VMs:");
                    vm_list_simple();
                }
            } else {
                println!("Error: VM name lookup not implemented. Use VM ID instead.");
                println!("Available VMs:");
                vm_list_simple();
            }
        }
    }
}

fn start_vm_by_id(vm_id: usize) -> bool {
    let vm: vm_list::VMRef = match vm_list::get_vm_by_id(vm_id) {
        Some(vm) => vm,
        None => {
            println!("✗ VM[{}] not found", vm_id);
            return false;
        }
    };

    // Boot the VM
    match vm.boot() {
        Ok(_) => {
            println!("{:?}", vm.status());
            println!("✓ VM[{}] started successfully", vm_id);
            true
        }
        Err(e) => {
            println!("✗ VM[{}] failed to boot: {:?}", vm_id, e);
            true
        }
    }
}

fn vm_status(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    // If no arguments, show status of all VMs
    if args.is_empty() {
        let vm_list = vm_list::get_vm_list();
        if vm_list.is_empty() {
            println!("No VMs found.");
            return;
        }
        println!("VM Status:");
        println!("-----------");
        for vm in vm_list {
            let vm: vm_list::VMRef = vm;
            let vm_id = usize::from(vm.id());
            let name = vm.name();
            let status = vm.status();
            println!("VM[{}] \"{}\": {:?}", vm_id, name, status);
        }
        return;
    }

    // Show status of specified VM(s)
    for arg in args {
        let arg: &String = arg;
        if let Ok(vm_id) = arg.parse::<usize>() {
            if let Some(vm) = vm_list::get_vm_by_id(vm_id) {
                let vm: vm_list::VMRef = vm;
                let name = vm.name();
                let status = vm.status();
                println!("VM[{}] \"{}\": {:?}", vm_id, name, status);
            } else {
                println!("✗ VM[{}] not found", vm_id);
            }
        } else {
            println!("Error: Invalid VM ID: {}", arg);
        }
    }
}

fn vm_stop(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm stop <VM_ID>");
        return;
    }

    for vm_name in args {
        let vm_name: &String = vm_name;
        if let Ok(vm_id) = vm_name.parse::<usize>() {
            stop_vm_by_id(vm_id);
        } else {
            println!("Error: Invalid VM ID: {}", vm_name);
        }
    }
}

fn stop_vm_by_id(vm_id: usize) {
    let vm: vm_list::VMRef = match vm_list::get_vm_by_id(vm_id) {
        Some(vm) => vm,
        None => {
            println!("✗ VM[{}] not found", vm_id);
            return;
        }
    };

    let status = vm.status();

    // Check if VM can be stopped
    match status {
        VMStatus::Running => {
            println!("Stopping VM[{}]...", vm_id);
        }
        VMStatus::Stopping => {
            println!("⚠ VM[{}] is already stopping", vm_id);
            return;
        }
        VMStatus::Stopped => {
            println!("⚠ VM[{}] is already stopped", vm_id);
            return;
        }
        VMStatus::Inited => {
            println!("⚠ VM[{}] is not running yet", vm_id);
            return;
        }
        _ => {
            println!("⚠ VM[{}] is in {:?} state, cannot stop", vm_id, status);
            return;
        }
    }

    // Call shutdown
    match vm.shutdown() {
        Ok(_) => {
            println!("✓ VM[{}] stop signal sent successfully", vm_id);
            println!("  Note: VM status will transition to Stopped");
        }
        Err(e) => {
            println!("✗ Failed to stop VM[{}]: {:?}", vm_id, e);
        }
    }
}

fn vm_delete(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let force = cmd.flags.get("force").unwrap_or(&false);

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm delete [OPTIONS] <VM_ID>");
        return;
    }

    for arg in args {
        let arg: &String = arg;
        if let Ok(vm_id) = arg.parse::<usize>() {
            delete_vm_by_id(vm_id, *force);
        } else {
            println!("Error: Invalid VM ID: {}", arg);
        }
    }
}

fn delete_vm_by_id(vm_id: usize, force: bool) {
    // Check if VM exists and get its status
    let vm: vm_list::VMRef = match vm_list::get_vm_by_id(vm_id) {
        Some(vm) => vm,
        None => {
            println!("✗ VM[{}] not found", vm_id);
            return;
        }
    };

    let status = vm.status();

    // Check if VM is running
    match status {
        VMStatus::Running => {
            if !force {
                println!("✗ VM[{}] is currently running", vm_id);
                println!(
                    "  Use 'vm stop {}' first, or use '--force' to force delete",
                    vm_id
                );
                return;
            }
            println!("⚠ Force deleting running VM[{}]...", vm_id);
        }
        VMStatus::Stopping => {
            if !force {
                println!("⚠ VM[{}] is currently stopping", vm_id);
                println!("  Wait for it to stop completely, or use '--force' to force delete");
                return;
            }
            println!("⚠ Force deleting stopping VM[{}]...", vm_id);
        }
        VMStatus::Stopped | VMStatus::Inited => {
            println!("Deleting VM[{}] (status: {:?})...", vm_id, status);
            // Resources will be automatically released when VM is dropped
            println!("  ✓ VM resources will be released on drop");
        }
        _ => {
            println!("⚠ VM[{}] is in {:?} state", vm_id, status);
            if !force {
                println!("  Use --force to force delete");
                return;
            }
            println!("  Force deleting...");
        }
    }

    // If VM is running, try to stop it first
    if matches!(status, VMStatus::Running | VMStatus::Stopping) {
        println!("  Sending shutdown signal...");
        match vm.shutdown() {
            Ok(_) => {
                println!("  ✓ Shutdown signal sent");
            }
            Err(e) => {
                println!("  ⚠ Warning: Failed to send shutdown signal: {:?}", e);
            }
        }
    }

    // Remove VM from global list
    match vm_list::remove_vm(vm_id) {
        Some(_) => {
            println!("✓ VM[{}] deleted successfully", vm_id);
        }
        None => {
            println!("✗ Failed to remove VM[{}] from list", vm_id);
        }
    }
}

#[cfg(feature = "fs")]
fn vm_list_simple() {
    let vms = vm_list::get_vm_list();
    println!("ID    NAME           STATE      VCPU   MEMORY");
    println!("----  -----------    -------    ----   ------");
    for vm in vms {
        let vm: vm_list::VMRef = vm;
        let status = vm.status();
        let vcpu_num = vm.vcpu_num();
        let memory_size = vm.memory_size();

        println!(
            "{:<4}  {:<11}    {:<7}    {:<4}   {}",
            usize::from(vm.id()),
            vm.name(),
            format!("{:?}", status),
            vcpu_num,
            format_memory_size(memory_size)
        );
    }
}

fn vm_list(cmd: &ParsedCommand) {
    let binding = "table".to_string();
    let format = cmd.options.get("format").unwrap_or(&binding);

    let display_vms = vm_list::get_vm_list();

    if display_vms.is_empty() {
        println!("No virtual machines found.");
        return;
    }

    if format == "json" {
        // JSON output
        println!("{{");
        println!("  \"vms\": [");
        for (i, vm) in display_vms.iter().enumerate() {
            let vm: &vm_list::VMRef = vm;
            let status = vm.status();
            let total_memory = vm.memory_size();
            let vcpu_num = vm.vcpu_num();

            println!("    {{");
            println!("      \"id\": {},", usize::from(vm.id()));
            println!("      \"name\": \"{}\",", vm.name());
            println!("      \"state\": {:?},", status);
            println!("      \"vcpu\": {},", vcpu_num);
            println!("      \"memory\": \"{}\"", format_memory_size(total_memory));

            if i < display_vms.len() - 1 {
                println!("    }},");
            } else {
                println!("    }}");
            }
        }
        println!("  ]");
        println!("}}");
    } else {
        // Table output (default)
        println!(
            "{:<6} {:<15} {:<12} {:<10} {:<10}",
            "VM ID", "NAME", "STATUS", "VCPU", "MEMORY"
        );
        println!("{:-<6} {:-<15} {:-<12} {:-<10} {:-<10}", "", "", "", "", "");

        for vm in display_vms {
            let vm: vm_list::VMRef = vm;
            let status = vm.status();
            let total_memory = vm.memory_size();
            let vcpu_num = vm.vcpu_num();

            println!(
                "{:<6} {:<15} {:<12} {:<10} {}",
                usize::from(vm.id()),
                vm.name(),
                format!("{:?}", status),
                vcpu_num,
                format_memory_size(total_memory)
            );
        }
    }
}

fn vm_show(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    if args.is_empty() {
        println!("Error: No VM specified");
        println!("Usage: vm show <VM_ID>");
        println!();
        println!("Use 'vm list' to see all VMs");
        return;
    }

    // Show specific VM details
    let vm_name: &String = &args[0];
    if let Ok(vm_id) = vm_name.parse::<usize>() {
        show_vm_details(vm_id);
    } else {
        println!("Error: Invalid VM ID: {}", vm_name);
    }
}

/// Show VM information
fn show_vm_details(vm_id: usize) {
    let vm: vm_list::VMRef = match vm_list::get_vm_by_id(vm_id) {
        Some(vm) => vm,
        None => {
            println!("✗ VM[{}] not found", vm_id);
            return;
        }
    };

    let status = vm.status();

    println!("=== VM Details: {} ===", vm_id);
    println!();

    // Basic Information
    println!("  VM ID:     {}", usize::from(vm.id()));
    println!("  Name:      {}", vm.name());
    println!("  Status:    {:?}", status);
    println!("  VCPUs:     {}", vm.vcpu_num());
    println!("  Memory:    {}", format_memory_size(vm.memory_size()));

    // Add state-specific information
    match status {
        VMStatus::Inited => {
            println!();
            println!("  ℹ VM is ready. Use 'vm start {}' to boot.", vm_id);
        }
        VMStatus::Running => {
            println!();
            println!("  ℹ VM is running.");
        }
        VMStatus::Stopped => {
            println!();
            println!("  ℹ VM is stopped.");
        }
        _ => {}
    }
}

// ============================================================================
// Command Registration
// ============================================================================

/// Build the VM command tree and register it.
pub fn register_vm_commands(tree: &mut BTreeMap<String, CommandNode>) {
    #[cfg(feature = "fs")]
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

    #[cfg(feature = "fs")]
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

    let status_cmd = CommandNode::new("Stop a virtual machine")
        .with_handler(vm_status)
        .with_usage("vm stop [OPTIONS] <VM_ID>...");

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

    let delete_cmd = CommandNode::new("Delete a virtual machine")
        .with_handler(vm_delete)
        .with_usage("vm delete [OPTIONS] <VM_ID>")
        .with_flag(
            FlagDef::new("force", "Force delete without stopping VM first")
                .with_short('f')
                .with_long("force"),
        );

    let list_cmd = CommandNode::new("Show virtual machine lists")
        .with_handler(vm_list)
        .with_usage("vm list [OPTIONS]")
        .with_flag(
            FlagDef::new("all", "Show all VMs including stopped ones")
                .with_short('a')
                .with_long("all"),
        )
        .with_option(OptionDef::new("format", "Output format (table, json)").with_long("format"));

    let show_cmd = CommandNode::new("Show detailed VM information")
        .with_handler(vm_show)
        .with_usage("vm show [OPTIONS] <VM_ID>")
        .with_flag(
            FlagDef::new("full", "Show full detailed information")
                .with_short('f')
                .with_long("full"),
        )
        .with_flag(
            FlagDef::new("config", "Show configuration details")
                .with_short('c')
                .with_long("config"),
        )
        .with_flag(
            FlagDef::new("stats", "Show device statistics")
                .with_short('s')
                .with_long("stats"),
        );

    // main VM command
    let mut vm_node = CommandNode::new("Virtual machine management")
        .with_handler(vm_help)
        .with_usage("vm <command> [options] [args...]")
        .add_subcommand(
            "help",
            CommandNode::new("Show VM help").with_handler(vm_help),
        );

    #[cfg(feature = "fs")]
    {
        vm_node = vm_node
            .add_subcommand("create", create_cmd)
            .add_subcommand("start", start_cmd);
    }

    vm_node = vm_node
        .add_subcommand("status", status_cmd)
        .add_subcommand("stop", stop_cmd)
        .add_subcommand("delete", delete_cmd)
        .add_subcommand("list", list_cmd)
        .add_subcommand("show", show_cmd);

    tree.insert("vm".to_string(), vm_node);
}
