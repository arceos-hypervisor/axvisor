//! AxVisor Shell
//!
//! A command-line shell for AxVisor with support for:
//! - Command history and navigation
//! - File system operations (when fs feature is enabled)
//! - Virtual machine management
//!
//! # Example
//!
//! ```no_run
//! use axvisor_shell::console_init;
//!
//! // Start the shell in blocking mode
//! console_init();
//! ```

#![no_std]

#[macro_use]
extern crate alloc;
extern crate axstd as std;
#[macro_use]
extern crate log;

mod completion;
mod parser;
mod shell;

mod commands;

// Re-export shell types and functions
pub use shell::{Shell, console_init, console_init_non_blocking};

// Re-export parser types for external use
pub use parser::{
    CommandHistory, CommandNode, CommandParser, FlagDef, OptionDef, ParseError, ParsedCommand,
};

// Re-export commands module
pub use commands::{COMMAND_TREE, execute_command, show_available_commands, show_help};
