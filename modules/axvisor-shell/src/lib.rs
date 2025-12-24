#![no_std]

#[macro_use]
extern crate alloc;
extern crate axstd as std;
#[macro_use]
extern crate log;

mod command;

use std::io::prelude::*;
use std::println;
use std::string::ToString;

use crate::command::{
    CommandHistory, clear_line_and_redraw, handle_builtin_commands, print_prompt, run_cmd_bytes,
};

const LF: u8 = b'\n';
const CR: u8 = b'\r';
const DL: u8 = b'\x7f';
const BS: u8 = b'\x08';
const ESC: u8 = 0x1b; // ESC key

const MAX_LINE_LEN: usize = 256;

/// Shell state that can be stored and restored
pub struct Shell {
    stdin: std::io::Stdin,
    stdout: std::io::Stdout,
    history: CommandHistory,
    buf: [u8; MAX_LINE_LEN],
    cursor: usize,
    line_len: usize,
    input_state: InputState,
    initialized: bool,
}

#[derive(Clone, Copy)]
enum InputState {
    Normal,
    Escape,
    EscapeSeq,
}

impl Shell {
    /// Create a new shell instance
    pub fn new() -> Self {
        Self {
            stdin: std::io::stdin(),
            stdout: std::io::stdout(),
            history: CommandHistory::new(100),
            buf: [0; MAX_LINE_LEN],
            cursor: 0,
            line_len: 0,
            input_state: InputState::Normal,
            initialized: false,
        }
    }

    /// Initialize the shell (print welcome message)
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }

        println!("Welcome to AxVisor Shell!");
        println!("Type 'help' to see available commands");
        println!("Use UP/DOWN arrows to navigate command history");
        #[cfg(not(feature = "fs"))]
        println!("Note: Running with limited features (filesystem support disabled).");
        println!();

        print_prompt();
        self.initialized = true;
    }

    /// Process one character of input.
    /// Returns true if a command was executed (for potential scheduling decisions).
    pub fn process_char(&mut self) -> bool {
        if !self.initialized {
            self.init();
        }

        let mut temp_buf = [0u8; 1];

        let ch = match self.stdin.read(&mut temp_buf) {
            Ok(1) => temp_buf[0],
            _ => return false,
        };

        self.process_input(ch)
    }

    fn process_input(&mut self, ch: u8) -> bool {
        let mut command_executed = false;

        match self.input_state {
            InputState::Normal => match ch {
                CR | LF => {
                    println!();
                    if self.line_len > 0 {
                        let cmd_str = std::str::from_utf8(&self.buf[..self.line_len]).unwrap_or("");

                        // Add to history
                        self.history.add_command(cmd_str.to_string());

                        // Execute command
                        if !handle_builtin_commands(cmd_str) {
                            run_cmd_bytes(&self.buf[..self.line_len]);
                        }

                        command_executed = true;

                        // reset buffer
                        self.buf[..self.line_len].fill(0);
                        self.cursor = 0;
                        self.line_len = 0;
                    }
                    print_prompt();
                }
                BS | DL => {
                    // backspace: delete character before cursor / DEL key: delete character at cursor
                    if self.cursor > 0 {
                        // move characters after cursor forward
                        for i in self.cursor..self.line_len {
                            self.buf[i - 1] = self.buf[i];
                        }
                        self.cursor -= 1;
                        self.line_len -= 1;
                        if self.line_len < self.buf.len() {
                            self.buf[self.line_len] = 0;
                        }

                        let current_content =
                            std::str::from_utf8(&self.buf[..self.line_len]).unwrap_or("");
                        #[cfg(feature = "fs")]
                        let prompt = format!("axvisor:{}$ ", &std::env::current_dir().unwrap());
                        #[cfg(not(feature = "fs"))]
                        let prompt = "axvisor:$ ".to_string();
                        clear_line_and_redraw(&mut self.stdout, &prompt, current_content, self.cursor);
                    }
                }
                ESC => {
                    self.input_state = InputState::Escape;
                }
                0..=31 => {
                    // ignore other control characters
                }
                c => {
                    // insert character
                    if self.line_len < MAX_LINE_LEN - 1 {
                        // move characters after cursor backward to make space for new character
                        for i in (self.cursor..self.line_len).rev() {
                            self.buf[i + 1] = self.buf[i];
                        }
                        self.buf[self.cursor] = c;
                        self.cursor += 1;
                        self.line_len += 1;

                        let current_content =
                            std::str::from_utf8(&self.buf[..self.line_len]).unwrap_or("");
                        #[cfg(feature = "fs")]
                        let prompt = format!("axvisor:{}$ ", &std::env::current_dir().unwrap());
                        #[cfg(not(feature = "fs"))]
                        let prompt = "axvisor:$ ".to_string();
                        clear_line_and_redraw(&mut self.stdout, &prompt, current_content, self.cursor);
                    }
                }
            },
            InputState::Escape => match ch {
                b'[' => {
                    self.input_state = InputState::EscapeSeq;
                }
                _ => {
                    self.input_state = InputState::Normal;
                }
            },
            InputState::EscapeSeq => match ch {
                b'A' => {
                    // UP arrow - previous command
                    if let Some(prev_cmd) = self.history.previous() {
                        // clear current buffer
                        self.buf[..self.line_len].fill(0);

                        let cmd_bytes = prev_cmd.as_bytes();
                        let copy_len = cmd_bytes.len().min(MAX_LINE_LEN - 1);
                        self.buf[..copy_len].copy_from_slice(&cmd_bytes[..copy_len]);
                        self.cursor = copy_len;
                        self.line_len = copy_len;
                        #[cfg(feature = "fs")]
                        let prompt = format!("axvisor:{}$ ", &std::env::current_dir().unwrap());
                        #[cfg(not(feature = "fs"))]
                        let prompt = "axvisor:$ ".to_string();
                        clear_line_and_redraw(&mut self.stdout, &prompt, prev_cmd, self.cursor);
                    }
                    self.input_state = InputState::Normal;
                }
                b'B' => {
                    // DOWN arrow - next command
                    match self.history.next() {
                        Some(next_cmd) => {
                            // clear current buffer
                            self.buf[..self.line_len].fill(0);

                            let cmd_bytes = next_cmd.as_bytes();
                            let copy_len = cmd_bytes.len().min(MAX_LINE_LEN - 1);
                            self.buf[..copy_len].copy_from_slice(&cmd_bytes[..copy_len]);
                            self.cursor = copy_len;
                            self.line_len = copy_len;

                            #[cfg(feature = "fs")]
                            let prompt = format!("axvisor:{}$ ", &std::env::current_dir().unwrap());
                            #[cfg(not(feature = "fs"))]
                            let prompt = "axvisor:$ ".to_string();
                            clear_line_and_redraw(&mut self.stdout, &prompt, next_cmd, self.cursor);
                        }
                        None => {
                            // clear current line
                            self.buf[..self.line_len].fill(0);
                            self.cursor = 0;
                            self.line_len = 0;
                            #[cfg(feature = "fs")]
                            let prompt = format!("axvisor:{}$ ", &std::env::current_dir().unwrap());
                            #[cfg(not(feature = "fs"))]
                            let prompt = "axvisor:$ ".to_string();
                            clear_line_and_redraw(&mut self.stdout, &prompt, "", self.cursor);
                        }
                    }
                    self.input_state = InputState::Normal;
                }
                b'C' => {
                    // RIGHT arrow - move cursor right
                    if self.cursor < self.line_len {
                        self.cursor += 1;
                        self.stdout.write_all(b"\x1b[C").ok();
                        self.stdout.flush().ok();
                    }
                    self.input_state = InputState::Normal;
                }
                b'D' => {
                    // LEFT arrow - move cursor left
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.stdout.write_all(b"\x1b[D").ok();
                        self.stdout.flush().ok();
                    }
                    self.input_state = InputState::Normal;
                }
                b'3' => {
                    // check if this is Delete key sequence (ESC[3~)
                    // need to read next character to confirm
                    self.input_state = InputState::Normal;
                }
                _ => {
                    // ignore other escape sequences
                    self.input_state = InputState::Normal;
                }
            },
        }

        command_executed
    }

    /// Run the shell as a blocking loop (original behavior)
    pub fn run(&mut self) {
        self.init();
        loop {
            self.process_char();
        }
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

// Initialize the console shell with blocking behavior (backward compatible).
pub fn console_init() {
    let mut shell = Shell::new();
    shell.run();
}

// Alternative: Non-blocking initialization that returns a Shell instance.
// This allows the caller to control when to process input.
pub fn console_init_non_blocking() -> Shell {
    Shell::new()
}
