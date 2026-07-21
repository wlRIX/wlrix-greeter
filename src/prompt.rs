// SPDX-License-Identifier: GPL-3.0-or-later
//! Asking on a terminal.
//!
//! This is the greeter's first face, not its final one: `greetd` greeters run directly
//! on a VT, so a text prompt is a working login before any of the Indigo Magic UI
//! exists. The graphical greeter replaces this and speaks to the same
//! [`crate::greetd`] underneath.
//!
//! The one thing worth care here is echo. A password typed onto a VT with echo left on
//! is a password on the screen, so it is turned off for the duration and turned back on
//! afterwards, including when the read fails.

use std::io::{self, BufRead, Write};

/// Ask a question and read a line back. `None` if input ended (Ctrl+D).
pub fn ask(question: &str) -> io::Result<Option<String>> {
    print!("{question}");
    io::stdout().flush()?;
    read_line()
}

/// Ask without showing what is typed.
pub fn ask_secret(question: &str) -> io::Result<Option<String>> {
    print!("{question}");
    io::stdout().flush()?;

    let restore = EchoOff::new();
    let line = read_line();
    drop(restore);
    // The newline the person typed was not echoed, so the cursor is still on the
    // prompt line.
    println!();
    line
}

fn read_line() -> io::Result<Option<String>> {
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\n', '\r']).to_string()))
}

/// Terminal echo, turned off for as long as this lives.
///
/// A guard rather than a pair of calls so echo comes back even if the read fails or
/// panics -- leaving a terminal with echo off is the sort of thing that outlives the
/// process and confuses whoever is next at the keyboard.
struct EchoOff {
    previous: Option<libc::termios>,
}

impl EchoOff {
    fn new() -> Self {
        // SAFETY: `tcgetattr`/`tcsetattr` against stdin with a termios we own.
        unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut termios) != 0 {
                // Not a terminal (a pipe, say). Nothing is being shown, so nothing to
                // hide.
                return Self { previous: None };
            }
            let previous = termios;
            termios.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &termios) != 0 {
                return Self { previous: None };
            }
            Self {
                previous: Some(previous),
            }
        }
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            // SAFETY: restoring the attributes read in `new`.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &previous) };
        }
    }
}
