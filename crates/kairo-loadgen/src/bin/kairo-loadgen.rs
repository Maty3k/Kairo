// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-loadgen.exe` (docs/research/17-loadgen-bench/README.md §3). A Windows program with no
//! console window: `\\Kairo\\Run` jobs start it, and the bench keeps its standard output (the
//! summary or calibration line) and standard error. Elsewhere it builds to a stub that says it is
//! Windows only.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

use std::process::ExitCode;

use kairo_loadgen::cli::{self, Command, exit};

fn main() -> ExitCode {
    let Some(args) = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        eprintln!(
            "kairo-loadgen: an argument isn't valid Unicode\n{}",
            cli::USAGE
        );
        return ExitCode::from(exit::USAGE);
    };
    match cli::parse(&args) {
        Ok(Command::Version) => {
            println!("kairo-loadgen {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Help) => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Ok(command) => run(&command),
        Err(message) => {
            eprintln!("kairo-loadgen: {message}");
            ExitCode::from(exit::USAGE)
        }
    }
}

#[cfg(windows)]
fn run(command: &Command) -> ExitCode {
    ExitCode::from(kairo_loadgen::run::run(command))
}

#[cfg(not(windows))]
fn run(_command: &Command) -> ExitCode {
    eprintln!("kairo-loadgen is Windows only");
    ExitCode::from(exit::DEVICE)
}
