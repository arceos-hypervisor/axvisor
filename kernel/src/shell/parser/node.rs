//! Command node definitions for the command tree
//!
//! Defines the structures used to build the command tree hierarchy.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A node in the command tree
#[derive(Debug, Clone)]
pub struct CommandNode {
    handler: Option<fn(&ParsedCommand)>,
    pub subcommands: BTreeMap<String, CommandNode>,
    pub description: &'static str,
    pub usage: Option<&'static str>,
    #[allow(dead_code)]
    pub log_level: log::LevelFilter,
    pub options: Vec<OptionDef>,
    pub flags: Vec<FlagDef>,
}

impl CommandNode {
    /// Create a new command node with a description
    pub fn new(description: &'static str) -> Self {
        Self {
            handler: None,
            subcommands: BTreeMap::new(),
            description,
            usage: None,
            log_level: log::LevelFilter::Off,
            options: Vec::new(),
            flags: Vec::new(),
        }
    }

    /// Set the handler function for this command
    pub fn with_handler(mut self, handler: fn(&ParsedCommand)) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Set the usage string for this command
    pub fn with_usage(mut self, usage: &'static str) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Set the log level for this command
    #[allow(dead_code)]
    pub fn with_log_level(mut self, level: log::LevelFilter) -> Self {
        self.log_level = level;
        self
    }

    /// Add an option to this command
    pub fn with_option(mut self, option: OptionDef) -> Self {
        self.options.push(option);
        self
    }

    /// Add a flag to this command
    pub fn with_flag(mut self, flag: FlagDef) -> Self {
        self.flags.push(flag);
        self
    }

    /// Add a subcommand to this command
    pub fn add_subcommand<S: Into<String>>(mut self, name: S, node: CommandNode) -> Self {
        self.subcommands.insert(name.into(), node);
        self
    }

    /// Get the handler for this command
    pub fn handler(&self) -> Option<fn(&ParsedCommand)> {
        self.handler
    }
}

/// Definition of a command option (takes a value)
#[derive(Debug, Clone)]
pub struct OptionDef {
    pub name: &'static str,
    pub short: Option<char>,
    pub long: Option<&'static str>,
    pub description: &'static str,
    pub required: bool,
}

impl OptionDef {
    /// Create a new option definition
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            short: None,
            long: None,
            description,
            required: false,
        }
    }

    /// Set the short flag (e.g., -v)
    #[allow(dead_code)]
    pub fn with_short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    /// Set the long flag (e.g., --verbose)
    pub fn with_long(mut self, long: &'static str) -> Self {
        self.long = Some(long);
        self
    }

    /// Mark this option as required
    #[allow(dead_code)]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// Definition of a command flag (boolean)
#[derive(Debug, Clone)]
pub struct FlagDef {
    pub name: &'static str,
    pub short: Option<char>,
    pub long: Option<&'static str>,
    pub description: &'static str,
}

impl FlagDef {
    /// Create a new flag definition
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            short: None,
            long: None,
            description,
        }
    }

    /// Set the short flag (e.g., -v)
    pub fn with_short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    /// Set the long flag (e.g., --verbose)
    pub fn with_long(mut self, long: &'static str) -> Self {
        self.long = Some(long);
        self
    }
}

/// A parsed command with all arguments and options
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub command_path: Vec<String>,
    pub options: BTreeMap<String, String>,
    pub flags: BTreeMap<String, bool>,
    pub positional_args: Vec<String>,
}

/// Errors that can occur during command parsing
#[derive(Debug)]
pub enum ParseError {
    UnknownCommand(String),
    UnknownOption(String),
    MissingValue(String),
    MissingRequiredOption(String),
    NoHandler(String),
}
