// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX greeter — scaffold entry point.
//!
//! Will become a greetd-compatible greeter presenting an IRIX-style login. For
//! now, it only proves the crate builds and runs.

fn main() {
    println!(
        "wlrix-greeter {} — scaffold; greetd IPC + login UI pending.",
        env!("CARGO_PKG_VERSION")
    );
}
