// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-tone.exe` (docs/research/16-verify-testbed/README.md §5): the "voice chat" beep as its
//! own process. A Windows program with no console window, like `testbed.exe`; elsewhere it
//! builds to a stub that says it is Windows only.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

use std::process::ExitCode;

use kairo_testbed::cli::{self, ToneCommand, exit};

fn main() -> ExitCode {
    let Some(args) = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        eprintln!(
            "kairo-tone: an argument isn't valid Unicode\n{}",
            cli::TONE_USAGE
        );
        return ExitCode::from(exit::USAGE);
    };
    match cli::parse_tone(&args) {
        Ok(ToneCommand::Version) => {
            println!("kairo-tone {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(ToneCommand::Help) => {
            println!("{}", cli::TONE_USAGE);
            ExitCode::SUCCESS
        }
        Ok(ToneCommand::Beeps(config)) => beeps(&config),
        Err(message) => {
            eprintln!("kairo-tone: {message}");
            ExitCode::from(exit::USAGE)
        }
    }
}

#[cfg(windows)]
fn beeps(config: &cli::ToneConfig) -> ExitCode {
    ExitCode::from(kairo_testbed::voice::run(config))
}

#[cfg(not(windows))]
fn beeps(_config: &cli::ToneConfig) -> ExitCode {
    eprintln!("kairo-tone is Windows only");
    ExitCode::from(exit::DEVICE)
}
