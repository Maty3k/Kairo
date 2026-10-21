// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-cli`: talks to Kairo's named pipes, typically from an SSH session (docs/PLAN.md §2.1).
//!
//! - `kairo-cli ping [--pipe NAME]` calls `engine.hello` (default pipe `kairo-hello`).
//! - `kairo-cli keys [--pipe NAME]` calls `hello.keys`.
//!
//! It prints one JSON object (`ok`, `pipe`, `method`, then `result` or `error`) and exits 0 on
//! success, 1 when the call failed and 2 for a bad command line.
//! On other systems it builds to a stub that says it is Windows only.

#![forbid(unsafe_code)]

#[cfg(windows)]
mod client;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    client::main()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("kairo-cli is Windows only");
    ExitCode::FAILURE
}
