// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-devrun`: the M0 dev-loop runner in the owner's Windows session (docs/PLAN.md §3.1).
//!
//! - `run --job C:\kairo\jobs\next.json` (task `\Kairo\Run`): runs one job with a time limit,
//!   stopped at the owner's first input if the job asks; writes `logs\<id>.result.json`.
//! - `watch` (started by `ensure`): rewrites `C:\kairo\state\watch.json` every second.
//! - `ensure` (task `\Kairo\Ensure`, at logon and every minute): starts `watch` if needed.
//! - `engine` (task `\Kairo\Engine`): runs the engine; in M0 that is `kairo-hello.exe`.
//!
//! It is a GUI-subsystem program, so starting it never flashes a console window over a game.
//! On other systems it builds to a stub that says it is Windows only.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod devlog;
#[cfg(windows)]
#[allow(unsafe_code)]
mod platform;
#[cfg(windows)]
mod run;
#[cfg(windows)]
mod spawn;
#[cfg(windows)]
mod watch;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    app::main()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("kairo-devrun is Windows only");
    ExitCode::FAILURE
}
