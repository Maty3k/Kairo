// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-hello`: the M0 hello probe (docs/PLAN.md M0, exit criterion 2).
//!
//! It opens a window titled "Hello from Kairo" on the primary monitor, logs each key pressed
//! in it with its QPC timestamp to `C:\kairo\logs\hello.log`, and answers JSON-RPC on
//! `\\.\pipe\kairo-hello` (`engine.hello`, `hello.keys`), which only SYSTEM and the current
//! user may open and remote clients may not. It reads keys only from its own window: no hooks.
//! On other systems it builds to a stub that says it is Windows only.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

#[cfg(windows)]
mod app;
#[cfg(windows)]
#[allow(unsafe_code)]
mod platform;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    app::main()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("kairo-hello is Windows only");
    ExitCode::FAILURE
}
