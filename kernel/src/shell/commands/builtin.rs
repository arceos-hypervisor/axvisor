//! Built-in commands
//!
//! Commands that are part of the shell itself (help, exit, clear, log, uname).

use std::println;

use super::super::parser::ParsedCommand;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

/// Handle the `uname` command - display system information
pub fn do_uname(cmd: &ParsedCommand) {
    let show_all = cmd.flags.get("all").unwrap_or(&false);
    let show_kernel = cmd.flags.get("kernel-name").unwrap_or(&false);
    let show_arch = cmd.flags.get("machine").unwrap_or(&false);

    let arch = option_env!("AX_ARCH").unwrap_or("");
    let platform = option_env!("AX_PLATFORM").unwrap_or("");
    let smp = match option_env!("AX_SMP") {
        None | Some("1") => "",
        _ => " SMP",
    };
    let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");

    if *show_all {
        println!(
            "ArceOS {ver}{smp} {arch} {plat}",
            ver = version,
            smp = smp,
            arch = arch,
            plat = platform,
        );
    } else if *show_kernel {
        println!("ArceOS");
    } else if *show_arch {
        println!("{}", arch);
    } else {
        println!(
            "ArceOS {ver}{smp} {arch} {plat}",
            ver = version,
            smp = smp,
            arch = arch,
            plat = platform,
        );
    }
}

/// Handle the `exit` command - exit the shell
pub fn do_exit(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;
    let exit_code = if args.is_empty() {
        0
    } else {
        args[0].parse::<i32>().unwrap_or(0)
    };

    println!("Bye~");
    std::process::exit(exit_code);
}

/// Handle the `log` command - change log level
pub fn do_log(cmd: &ParsedCommand) {
    let args = &cmd.positional_args;

    if args.is_empty() {
        println!("Current log level: {:?}", log::max_level());
        return;
    }

    match args[0].as_str() {
        "on" | "enable" => log::set_max_level(log::LevelFilter::Info),
        "off" | "disable" => log::set_max_level(log::LevelFilter::Off),
        "error" => log::set_max_level(log::LevelFilter::Error),
        "warn" => log::set_max_level(log::LevelFilter::Warn),
        "info" => log::set_max_level(log::LevelFilter::Info),
        "debug" => log::set_max_level(log::LevelFilter::Debug),
        "trace" => log::set_max_level(log::LevelFilter::Trace),
        level => {
            println!("Unknown log level: {}", level);
            println!("Available levels: off, error, warn, info, debug, trace");
            return;
        }
    }
    println!("Log level set to: {:?}", log::max_level());
}

/// Register built-in commands to the command tree
pub fn register_builtin_commands(tree: &mut BTreeMap<String, super::super::parser::CommandNode>) {
    use super::super::parser::{CommandNode, FlagDef};

    // uname Command
    tree.insert(
        "uname".to_string(),
        CommandNode::new("System information")
            .with_handler(do_uname)
            .with_usage("uname [OPTIONS]")
            .with_flag(
                FlagDef::new("all", "Show all information")
                    .with_short('a')
                    .with_long("all"),
            )
            .with_flag(
                FlagDef::new("kernel-name", "Show kernel name")
                    .with_short('s')
                    .with_long("kernel-name"),
            )
            .with_flag(
                FlagDef::new("machine", "Show machine architecture")
                    .with_short('m')
                    .with_long("machine"),
            ),
    );

    // exit Command
    tree.insert(
        "exit".to_string(),
        CommandNode::new("Exit the shell")
            .with_handler(do_exit)
            .with_usage("exit [EXIT_CODE]"),
    );

    // log Command
    tree.insert(
        "log".to_string(),
        CommandNode::new("Change log level")
            .with_handler(do_log)
            .with_usage("log [LEVEL]"),
    );
}
