// SPDX-License-Identifier: GPL-3.0-or-later
//! Where the graphical greeter's logs go.
//!
//! The greeter draws on a Wayland surface and runs under greetd, so its stderr goes to a
//! console nothing can see -- a bad login, or a font that would not load, leaves no evidence.
//! So at startup stderr's file descriptor is pointed at a file. Everything already written with
//! `eprintln!`, and any panic, then lands there with no other change; the terminal (`--text`)
//! face keeps its stderr and does not call this.
//!
//! Truncated at startup: this is for working out what this run did, not a history across boots.

use std::{fs::File, os::unix::io::AsRawFd, path::PathBuf};

/// Named for the process, so the greeter's log sits beside the compositor's.
const LOG_NAME: &str = "wlrix-greeter.log";

/// Point this process's stderr at a fresh log file, returning its path. On any failure the
/// greeter keeps its original stderr and starts anyway -- logging is not worth failing to
/// start over.
pub fn to_file() -> Option<PathBuf> {
    // The temp directory, not `$XDG_RUNTIME_DIR`: the greeter runs as its own user, whose
    // `/run/user/<uid>` is mode 700, so a log left there could not be read from the session
    // afterwards. `/tmp/wlrix-greeter.log` is world-readable and easy to find.
    let path = std::env::temp_dir().join(LOG_NAME);
    let file = match create(&path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("wlrix-greeter: could not open {}: {err}", path.display());
            return None;
        }
    };
    // Replace fd 2 with the file: `dup2` closes the old stderr and makes fd 2 refer to the
    // file, so every existing `eprintln!` and any panic message now goes there. The `File`
    // can drop -- fd 2 keeps the open file alive.
    if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        eprintln!(
            "wlrix-greeter: could not redirect stderr: {}",
            std::io::Error::last_os_error()
        );
        return None;
    }
    Some(path)
}

/// Open for writing, truncating what is there.
///
/// `O_NOFOLLOW` because the path is predictable and the temp directory is world-writable:
/// without it, anyone could leave a symlink there and have the greeter truncate a file of
/// their choosing.
fn create(path: &std::path::Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}
