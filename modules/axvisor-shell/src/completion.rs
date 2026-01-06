//! Tab completion module for file and directory names
//!
//! Provides intelligent filename and directory name completion
//! when the user presses the TAB key.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Completion result containing possible matches
pub struct CompletionResult {
    /// The common prefix of all matches
    pub prefix: String,
    /// List of all possible completions
    pub matches: Vec<String>,
    /// The text that should be inserted (suffix to add)
    pub insert_text: String,
}

impl CompletionResult {
    /// Create a new completion result
    pub fn new(prefix: String, matches: Vec<String>) -> Self {
        let insert_text = if matches.len() == 1 {
            // For single match, insert the full match
            matches[0].clone()
        } else {
            // For multiple matches, insert the common prefix
            prefix.clone()
        };
        Self { prefix, matches, insert_text }
    }

    /// Check if there's exactly one match
    pub fn is_unique(&self) -> bool {
        self.matches.len() == 1
    }

    /// Check if there are no matches
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }
}

/// Perform tab completion for the given input line and cursor position
///
/// # Arguments
/// * `line` - The current input line
/// * `cursor_pos` - The current cursor position
///
/// # Returns
/// * `None` if no completion is possible
/// * `Some(CompletionResult)` with completion suggestions
pub fn complete(line: &str, cursor_pos: usize) -> Option<CompletionResult> {
    // Find the word being completed (word under cursor)
    let word_start = find_word_start(line, cursor_pos);
    let word_to_complete = &line[word_start..cursor_pos];

    // Check if we should complete commands or filenames
    // If we're at the beginning of the line or after a space, complete commands
    // Otherwise, complete filenames
    if is_at_command_position(line, word_start) {
        complete_command(word_to_complete)
    } else {
        // For filename completion, we need to extract the path prefix and filename part
        // Returns a CompletionResult where insert_text is the full path (path_prefix + filename)
        complete_filename(word_to_complete)
    }
}

/// Find the start position of the word under the cursor
pub fn find_word_start(line: &str, cursor_pos: usize) -> usize {
    let bytes = line.as_bytes();
    let mut pos = cursor_pos.saturating_sub(1);

    while pos > 0 && !is_whitespace(bytes[pos]) {
        pos -= 1;
    }

    if is_whitespace(bytes[pos]) {
        pos + 1
    } else {
        pos
    }
}

/// Check if a byte is a whitespace character
fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Check if we're at a position where commands should be completed
/// (either at the start of line or after a pipe)
fn is_at_command_position(line: &str, word_start: usize) -> bool {
    let before_word = &line[..word_start];
    let trimmed = before_word.trim();

    // At the start of the line
    if trimmed.is_empty() {
        return true;
    }

    // After a pipe (simple check)
    if trimmed.ends_with('|') {
        return true;
    }

    false
}

/// Complete command names
#[cfg(feature = "fs")]
fn complete_command(partial: &str) -> Option<CompletionResult> {
    use crate::commands::COMMAND_TREE;

    let partial_lower = partial.to_lowercase();
    let mut matches: Vec<String> = COMMAND_TREE
        .keys()
        .filter(|cmd| cmd.starts_with(&partial_lower))
        .cloned()
        .collect();

    if matches.is_empty() {
        return None;
    }

    matches.sort();

    let common_prefix = find_common_prefix(&matches);
    Some(CompletionResult::new(common_prefix, matches))
}

/// Complete command names (no filesystem feature)
#[cfg(not(feature = "fs"))]
fn complete_command(partial: &str) -> Option<CompletionResult> {
    use crate::commands::COMMAND_TREE;

    let partial_lower = partial.to_lowercase();
    let mut matches: Vec<String> = COMMAND_TREE
        .keys()
        .filter(|cmd| cmd.starts_with(&partial_lower))
        .cloned()
        .collect();

    if matches.is_empty() {
        return None;
    }

    matches.sort();

    let common_prefix = find_common_prefix(&matches);
    Some(CompletionResult::new(common_prefix, matches))
}

/// Complete file and directory names
#[cfg(feature = "fs")]
fn complete_filename(partial: &str) -> Option<CompletionResult> {
    use alloc::string::ToString;
    use axstd::fs;

    // Split into directory path and file prefix
    let (dir_path, file_prefix, path_prefix) = if let Some(last_slash) = partial.rfind('/') {
        // If partial ends with '/', complete everything in that directory
        if last_slash == partial.len() - 1 {
            // dir_path is everything up to and including the last slash
            (&partial[..], "", partial.to_string())
        } else {
            // dir_path is everything up to and including the last slash
            // file_prefix is everything after the last slash
            (&partial[..=last_slash], &partial[last_slash + 1..], partial[..=last_slash].to_string())
        }
    } else {
        (".", partial, String::new())
    };

    // Try to read the directory
    let entries = match fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(_) => return None,
    };

    let mut matches: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();

        // Skip hidden files (starting with '.') unless explicitly requested
        if !file_prefix.starts_with('.') && name.starts_with('.') {
            continue;
        }

        if name.starts_with(file_prefix) {
            // Check if it's a directory by file type
            let file_type = entry.file_type();

            // Add '/' suffix for directories
            let completed_name = if matches!(file_type, axstd::fs::FileType::Dir) {
                format!("{name}/")
            } else {
                name.clone()
            };

            // For matches, we need to include the path prefix
            // e.g., if partial is "/gu" and we find "guest", match should be "/guest/"
            let full_match = if !path_prefix.is_empty() && path_prefix != "/" {
                format!("{}{}", path_prefix, completed_name)
            } else if path_prefix == "/" {
                format!("/{}", completed_name)
            } else {
                completed_name.clone()
            };

            matches.push(full_match);
        }
    }

    if matches.is_empty() {
        return None;
    }

    matches.sort();

    let common_prefix = find_common_prefix(&matches);
    Some(CompletionResult::new(common_prefix, matches))
}

/// Complete file and directory names (no filesystem support)
#[cfg(not(feature = "fs"))]
fn complete_filename(_partial: &str) -> Option<CompletionResult> {
    // No filesystem support, no filename completion
    None
}

/// Find the common prefix among all strings
fn find_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }

    if strings.len() == 1 {
        return strings[0].clone();
    }

    let first = &strings[0];
    let mut end = first.len();

    for s in &strings[1..] {
        let mut new_end = 0;
        for (i, (a, b)) in first.bytes().zip(s.bytes()).enumerate() {
            if a == b {
                new_end = i + 1;
            } else {
                break;
            }
        }
        end = end.min(new_end);
        if end == 0 {
            break;
        }
    }

    first[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_common_prefix() {
        assert_eq!(find_common_prefix(&[]), "");
        assert_eq!(find_common_prefix(&["test".into()]), "test");
        assert_eq!(find_common_prefix(&["test".into(), "testing".into()]), "test");
        assert_eq!(find_common_prefix(&["foo".into(), "bar".into()]), "");
        assert_eq!(find_common_prefix(&["file1.txt".into(), "file2.txt".into()]), "file");
    }

    #[test]
    fn test_find_word_start() {
        assert_eq!(find_word_start("ls test", 6), 3);
        assert_eq!(find_word_start("ls test file", 6), 3);
        assert_eq!(find_word_start("ls", 2), 0);
        assert_eq!(find_word_start("ls   test", 7), 5);
    }

    #[test]
    fn test_is_at_command_position() {
        assert!(is_at_command_position("ls", 0));
        assert!(is_at_command_position("| ls", 2));
        assert!(!is_at_command_position("ls file", 3));
    }
}
