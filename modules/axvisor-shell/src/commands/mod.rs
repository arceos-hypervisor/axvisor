//! Command handlers module
//!
//! Provides command implementations for different categories.

mod builtin;
mod fs;
mod vm;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::std::io::Write;

use axstd::print;
use axstd::println;

use crate::parser::{CommandNode, ParseError};

pub use builtin::register_builtin_commands;
pub use fs::register_fs_commands;
pub use vm::register_vm_commands;

lazy_static::lazy_static! {
    /// Global command tree containing all registered commands
    pub static ref COMMAND_TREE: BTreeMap<String, CommandNode> = build_command_tree();
}

/// Build the complete command tree by registering all command categories
fn build_command_tree() -> BTreeMap<String, CommandNode> {
    let mut tree = BTreeMap::new();

    register_builtin_commands(&mut tree);
    register_fs_commands(&mut tree);
    register_vm_commands(&mut tree);

    tree
}

/// Execute a parsed command
pub fn execute_command(input: &str) -> Result<(), ParseError> {
    let parsed = crate::parser::CommandParser::parse(input, &COMMAND_TREE)?;

    // Find the corresponding command node
    let mut current_node = COMMAND_TREE.get(&parsed.command_path[0]).unwrap();
    for cmd in &parsed.command_path[1..] {
        current_node = current_node.subcommands.get(cmd).unwrap();
    }

    // Execute the command
    if let Some(handler) = current_node.handler() {
        handler(&parsed);
        Ok(())
    } else {
        Err(ParseError::NoHandler(parsed.command_path.join(" ")))
    }
}

/// Display help for a specific command
pub fn show_help(command_path: &[String]) -> Result<(), ParseError> {
    let mut current_node = COMMAND_TREE
        .get(&command_path[0])
        .ok_or_else(|| ParseError::UnknownCommand(command_path[0].clone()))?;

    for cmd in &command_path[1..] {
        current_node = current_node
            .subcommands
            .get(cmd)
            .ok_or_else(|| ParseError::UnknownCommand(cmd.clone()))?;
    }

    println!("Command: {}", command_path.join(" "));
    println!("Description: {}", current_node.description);

    if let Some(usage) = current_node.usage {
        println!("Usage: {}", usage);
    }

    if !current_node.options.is_empty() {
        println!("\nOptions:");
        for option in &current_node.options {
            let mut opt_str = String::new();
            if let Some(short) = option.short {
                opt_str.push_str(&format!("-{short}"));
            }
            if let Some(long) = option.long {
                if !opt_str.is_empty() {
                    opt_str.push_str(", ");
                }
                opt_str.push_str(&format!("--{long}"));
            }
            if opt_str.is_empty() {
                opt_str = option.name.to_string();
            }

            let required_str = if option.required { " (required)" } else { "" };
            println!("  {:<20} {}{}", opt_str, option.description, required_str);
        }
    }

    if !current_node.flags.is_empty() {
        println!("\nFlags:");
        for flag in &current_node.flags {
            let mut flag_str = String::new();
            if let Some(short) = flag.short {
                flag_str.push_str(&format!("-{short}"));
            }
            if let Some(long) = flag.long {
                if !flag_str.is_empty() {
                    flag_str.push_str(", ");
                }
                flag_str.push_str(&format!("--{long}"));
            }
            if flag_str.is_empty() {
                flag_str = flag.name.to_string();
            }

            println!("  {:<20} {}", flag_str, flag.description);
        }
    }

    if !current_node.subcommands.is_empty() {
        println!("\nSubcommands:");
        for (name, node) in &current_node.subcommands {
            println!("  {:<20} {}", name, node.description);
        }
    }

    Ok(())
}

/// Show all available commands
pub fn show_available_commands() {
    println!("ArceOS Shell - Available Commands:");
    println!();

    // Display all top-level commands
    for (name, node) in COMMAND_TREE.iter() {
        println!("  {:<15} {}", name, node.description);

        // Display subcommands
        if !node.subcommands.is_empty() {
            for (sub_name, sub_node) in &node.subcommands {
                println!("    {:<13} {}", sub_name, sub_node.description);
            }
        }
    }

    println!();
    println!("Built-in Commands:");
    println!("  help            Show help information");
    println!("  help <command>  Show help for a specific command");
    println!("  clear           Clear the screen");
    println!("  exit/quit       Exit the shell");
    println!();
    println!("Tip: Use 'help <command>' to see detailed usage of a command");
}

/// Handle built-in shell commands (help, exit, clear)
pub fn handle_builtin_commands(input: &str) -> bool {
    match input.trim() {
        "help" => {
            show_available_commands();
            true
        }
        "exit" | "quit" => {
            println!("Goodbye!");
            axstd::process::exit(0);
        }
        "clear" => {
            print!("\x1b[2J\x1b[H"); // ANSI clear screen sequence
            axstd::io::stdout().flush().unwrap();
            true
        }
        _ if input.starts_with("help ") => {
            let cmd_parts: Vec<String> = input[5..]
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            if let Err(e) = show_help(&cmd_parts) {
                println!("Error: {:?}", e);
            }
            true
        }
        _ => false,
    }
}

/// Print the shell prompt
pub fn print_prompt() {
    #[cfg(feature = "fs")]
    print!("axvisor:{}$ ", axstd::env::current_dir().unwrap());
    #[cfg(not(feature = "fs"))]
    print!("axvisor:$ ");
    axstd::io::stdout().flush().unwrap();
}

/// Execute a command from byte input
pub fn run_cmd_bytes(cmd_bytes: &[u8]) {
    match str::from_utf8(cmd_bytes) {
        Ok(cmd_str) => {
            let trimmed = cmd_str.trim();
            if trimmed.is_empty() {
                return;
            }

            match execute_command(trimmed) {
                Ok(_) => {
                    // Command executed successfully
                }
                Err(ParseError::UnknownCommand(cmd)) => {
                    println!("Error: Unknown command '{}'", cmd);
                    println!("Type 'help' to see available commands");
                }
                Err(ParseError::UnknownOption(opt)) => {
                    println!("Error: Unknown option '{}'", opt);
                }
                Err(ParseError::MissingValue(opt)) => {
                    println!("Error: Option '{}' is missing a value", opt);
                }
                Err(ParseError::MissingRequiredOption(opt)) => {
                    println!("Error: Missing required option '{}'", opt);
                }
                Err(ParseError::NoHandler(cmd)) => {
                    println!("Error: Command '{}' has no handler function", cmd);
                }
            }
        }
        Err(_) => {
            println!("Error: Input contains invalid UTF-8 characters");
        }
    }
}
