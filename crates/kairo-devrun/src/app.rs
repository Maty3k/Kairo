// SPDX-License-Identifier: GPL-3.0-or-later
//! Command dispatch and the checks every command shares.

use std::io::Write;
use std::process::ExitCode;

use kairo_core::args::{self, DevrunCommand};
use kairo_core::layout::Layout;

use crate::{devlog, platform, run, spawn, watch};

/// Bad command line.
pub const EXIT_USAGE: u8 = 2;
/// `kairo-devrun` itself failed; details in `C:\kairo\logs\devrun.log`.
pub const EXIT_FAILED: u8 = 3;
/// Refused: not running in the owner's interactive session.
pub const EXIT_NOT_INTERACTIVE: u8 = 4;
/// Refused: another job is still running (no result file is written).
pub const EXIT_BUSY: u8 = 5;

/// Runs the command line.
pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let layout = Layout::default();
    let command = match args::parse_devrun(&args) {
        Ok(command) => command,
        Err(message) => {
            say(&message);
            devlog::event(&layout, "usage", &message);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match command {
        DevrunCommand::Run { job } => {
            let job_file = job.unwrap_or_else(|| layout.next_job_file());
            run::main(&layout, &job_file)
        }
        DevrunCommand::Watch => watch::main(&layout),
        DevrunCommand::Ensure => spawn::ensure(&layout),
        DevrunCommand::Engine => spawn::engine(&layout),
        DevrunCommand::Version => {
            say(concat!("kairo-devrun ", env!("CARGO_PKG_VERSION")));
            ExitCode::SUCCESS
        }
        DevrunCommand::Help => {
            say(args::DEVRUN_USAGE);
            ExitCode::SUCCESS
        }
    }
}

/// Prints a line if there is somewhere to print it (a GUI-subsystem program usually has no
/// console). Never panics.
pub fn say(message: &str) {
    let _ = writeln!(std::io::stdout(), "{message}");
}

/// The session id, or the exit code to return outside the owner's interactive session.
pub fn require_interactive(layout: &Layout, command: &str) -> Result<u32, ExitCode> {
    match platform::session_id() {
        Ok(0) => {
            let message = "refused: session 0 (SSH or a service) has no desktop and never sees \
                           the owner's input; start this through its \\Kairo\\ scheduled task";
            say(message);
            devlog::event(layout, command, message);
            Err(ExitCode::from(EXIT_NOT_INTERACTIVE))
        }
        Ok(session) => Ok(session),
        Err(error) => {
            let message = format!("cannot read the session id: {error}");
            say(&message);
            devlog::event(layout, command, &message);
            Err(ExitCode::from(EXIT_FAILED))
        }
    }
}
