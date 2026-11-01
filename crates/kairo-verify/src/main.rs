// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify`: checks a clip and reports (docs/research/16-verify-testbed/README.md §6). The
//! command line lives in [`kairo_verify::cli`].

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::IsTerminal;
use std::process::ExitCode;

fn main() -> ExitCode {
    // The linked FFmpeg says nothing: its messages could name a path (CLAUDE.md), and the
    // reports carry its error codes instead (README §3.1). The library leaves the log alone.
    #[cfg(windows)]
    ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Quiet);
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let stdout = std::io::stdout();
    let terminal = stdout.is_terminal();
    let code =
        kairo_verify::cli::run_with(&args, &mut stdout.lock(), &mut std::io::stderr(), terminal);
    ExitCode::from(code)
}
