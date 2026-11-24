// SPDX-License-Identifier: GPL-3.0-or-later
//! One `run` or `calibrate` on Windows (docs/research/17-loadgen-bench/README.md §3.3–3.10): find
//! the monitor by its device path and refuse a mirrored or HDR one, open the window at its
//! rectangle and the swap chain on its adapter, take the foreground with `--activate`, draw the
//! start-up ramp ([`crate::workload::Ramp`]) or the calibration ([`crate::workload::Calibrator`])
//! through the TDR guards, write the ready file and the log, and print the summary or calibration
//! line ([`crate::summary`]). It has no `unsafe`: the calls that need it are in `gpu` and
//! `kairo-window`.
//!
//! Not built yet: research 17's PR 4 adds the GPU parts and this loop in its second part. Until
//! then a parsed `run` or `calibrate` stops here with exit 4, before it touches any display.

use crate::cli::{Command, exit};

/// Runs `command`; returns the exit code.
pub fn run(command: &Command) -> u8 {
    let _ = command;
    eprintln!("kairo-loadgen: this build has no GPU code yet");
    exit::DEVICE
}
