//! Command parser implementation
//!
//! Parses command strings into structured command objects.

use super::node::{CommandNode, ParseError, ParsedCommand};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Command parser
pub struct CommandParser;

impl CommandParser {
    /// Parse a command string into a structured command
    pub fn parse(
        input: &str,
        command_tree: &BTreeMap<String, CommandNode>,
    ) -> Result<ParsedCommand, ParseError> {
        let tokens = Self::tokenize(input);
        if tokens.is_empty() {
            return Err(ParseError::UnknownCommand("empty command".to_string()));
        }

        // Find the command path
        let (command_path, command_node, remaining_tokens) =
            Self::find_command(&tokens, command_tree)?;

        // Parse the arguments
        let (options, flags, positional_args) = Self::parse_args(remaining_tokens, command_node)?;

        // Validate required options
        Self::validate_required_options(command_node, &options)?;

        Ok(ParsedCommand {
            command_path,
            options,
            flags,
            positional_args,
        })
    }

    /// Split input string into tokens, handling quotes and escapes
    fn tokenize(input: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current_token = String::new();
        let mut in_quotes = false;
        let mut escape_next = false;

        for ch in input.chars() {
            if escape_next {
                current_token.push(ch);
                escape_next = false;
            } else if ch == '\\' {
                escape_next = true;
            } else if ch == '"' {
                in_quotes = !in_quotes;
            } else if ch.is_whitespace() && !in_quotes {
                if !current_token.is_empty() {
                    tokens.push(current_token.clone());
                    current_token.clear();
                }
            } else {
                current_token.push(ch);
            }
        }

        if !current_token.is_empty() {
            tokens.push(current_token);
        }

        tokens
    }

    /// Find the command node for the given tokens
    fn find_command<'a>(
        tokens: &'a [String],
        command_tree: &'a BTreeMap<String, CommandNode>,
    ) -> Result<(Vec<String>, &'a CommandNode, &'a [String]), ParseError> {
        let mut current_node = command_tree
            .get(&tokens[0])
            .ok_or_else(|| ParseError::UnknownCommand(tokens[0].clone()))?;

        let mut command_path = vec![tokens[0].clone()];
        let mut token_index = 1;

        // Traverse to find the deepest command node
        while token_index < tokens.len() {
            if let Some(subcommand) = current_node.subcommands.get(&tokens[token_index]) {
                current_node = subcommand;
                command_path.push(tokens[token_index].clone());
                token_index += 1;
            } else {
                break;
            }
        }

        Ok((command_path, current_node, &tokens[token_index..]))
    }

    /// Parse arguments into options, flags, and positional args
    #[allow(clippy::type_complexity)]
    fn parse_args(
        tokens: &[String],
        command_node: &CommandNode,
    ) -> Result<
        (
            BTreeMap<String, String>,
            BTreeMap<String, bool>,
            Vec<String>,
        ),
        ParseError,
    > {
        let mut options = BTreeMap::new();
        let mut flags = BTreeMap::new();
        let mut positional_args = Vec::new();
        let mut i = 0;

        while i < tokens.len() {
            let token = &tokens[i];

            if let Some(name) = token.strip_prefix("--") {
                // Long options/flags
                if let Some(eq_pos) = name.find('=') {
                    // --option=value format
                    let (opt_name, value) = name.split_at(eq_pos);
                    let value = &value[1..]; // Skip '='
                    if Self::is_option(opt_name, command_node) {
                        options.insert(opt_name.to_string(), value.to_string());
                    } else {
                        return Err(ParseError::UnknownOption(format!("--{opt_name}")));
                    }
                } else if Self::is_flag(name, command_node) {
                    flags.insert(name.to_string(), true);
                } else if Self::is_option(name, command_node) {
                    // --option value format
                    if i + 1 >= tokens.len() {
                        return Err(ParseError::MissingValue(format!("--{name}")));
                    }
                    options.insert(name.to_string(), tokens[i + 1].clone());
                    i += 1; // Skip value
                } else {
                    return Err(ParseError::UnknownOption(format!("--{name}")));
                }
            } else if token.starts_with('-') && token.len() > 1 {
                // Short options/flags
                let chars: Vec<char> = token[1..].chars().collect();
                for (j, &ch) in chars.iter().enumerate() {
                    if Self::is_short_flag(ch, command_node) {
                        flags.insert(
                            Self::get_flag_name_by_short(ch, command_node)
                                .unwrap()
                                .to_string(),
                            true,
                        );
                    } else if Self::is_short_option(ch, command_node) {
                        let opt_name = Self::get_option_name_by_short(ch, command_node).unwrap();
                        if j == chars.len() - 1 && i + 1 < tokens.len() {
                            // Last character and there is a next token as value
                            options.insert(opt_name.to_string(), tokens[i + 1].clone());
                            i += 1; // Skip value
                        } else {
                            return Err(ParseError::MissingValue(format!("-{ch}")));
                        }
                    } else {
                        return Err(ParseError::UnknownOption(format!("-{ch}")));
                    }
                }
            } else {
                // Positional arguments
                positional_args.push(token.clone());
            }
            i += 1;
        }

        Ok((options, flags, positional_args))
    }

    fn is_option(name: &str, node: &CommandNode) -> bool {
        node.options
            .iter()
            .any(|opt| (opt.long == Some(name)) || opt.name == name)
    }

    fn is_flag(name: &str, node: &CommandNode) -> bool {
        node.flags
            .iter()
            .any(|flag| (flag.long == Some(name)) || flag.name == name)
    }

    fn is_short_option(ch: char, node: &CommandNode) -> bool {
        node.options.iter().any(|opt| opt.short == Some(ch))
    }

    fn is_short_flag(ch: char, node: &CommandNode) -> bool {
        node.flags.iter().any(|flag| flag.short == Some(ch))
    }

    fn get_option_name_by_short(ch: char, node: &CommandNode) -> Option<&str> {
        node.options
            .iter()
            .find(|opt| opt.short == Some(ch))
            .map(|opt| opt.name)
    }

    fn get_flag_name_by_short(ch: char, node: &CommandNode) -> Option<&str> {
        node.flags
            .iter()
            .find(|flag| flag.short == Some(ch))
            .map(|flag| flag.name)
    }

    fn validate_required_options(
        node: &CommandNode,
        options: &BTreeMap<String, String>,
    ) -> Result<(), ParseError> {
        for option in &node.options {
            if option.required && !options.contains_key(option.name) {
                return Err(ParseError::MissingRequiredOption(option.name.to_string()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::node::{CommandNode, FlagDef};

    #[test]
    fn test_tokenize() {
        let tokens = CommandParser::tokenize("hello world");
        assert_eq!(tokens, vec!["hello", "world"]);

        let tokens = CommandParser::tokenize("hello \"world test\"");
        assert_eq!(tokens, vec!["hello", "world test"]);

        let tokens = CommandParser::tokenize("hello\\ world");
        assert_eq!(tokens, vec!["hello world"]);
    }
}
