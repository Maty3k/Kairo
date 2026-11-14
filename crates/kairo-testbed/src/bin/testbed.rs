// SPDX-License-Identifier: GPL-3.0-or-later
//! `testbed.exe` (docs/research/16-verify-testbed/README.md §4.10). A Windows program with no
//! console window: `\\Kairo\\Run` jobs start it, and `kairo-devrun` keeps its standard output and
//! error as the job's logs. Elsewhere it builds to a stub that says it is Windows only.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

use std::process::ExitCode;

use kairo_testbed::cli::{self, Command, exit};

fn main() -> ExitCode {
    let Some(args) = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        eprintln!("testbed: an argument isn't valid Unicode\n{}", cli::USAGE);
        return ExitCode::from(exit::USAGE);
    };
    match cli::parse(&args) {
        Ok(Command::Version) => {
            println!("testbed {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Help) => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Command::Run(config)) => run(&config),
        Err(message) => {
            eprintln!("testbed: {message}");
            ExitCode::from(exit::USAGE)
        }
    }
}

#[cfg(windows)]
fn run(config: &cli::Config) -> ExitCode {
    ExitCode::from(kairo_testbed::run::run(config))
}

#[cfg(not(windows))]
fn run(_config: &cli::Config) -> ExitCode {
    eprintln!("testbed is Windows only");
    ExitCode::from(exit::DEVICE)
}
