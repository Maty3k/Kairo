// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify`: checks a clip and reports (docs/research/16-verify-testbed/README.md §6). The
//! command line lives in [`kairo_verify::cli`].

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let code = kairo_verify::cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(code)
}
