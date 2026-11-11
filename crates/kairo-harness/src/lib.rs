// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-harness`: the Windows orchestrator behind `tools/mac/pc harness`
//! (docs/research/16-verify-testbed/README.md §8). It runs as a hidden `\Kairo\Run` job, so it
//! is a console program that `kairo-devrun` starts without a window.
//!
//! `displays` writes what `kairo-display` finds for `pc harness displays`. `run` is gate 2's
//! capture: it starts the testbed on a monitor, waits until it presents steadily, records it with
//! FFmpeg into AMF, stops it, has `kairo-verify` judge the clip, and writes `harness.json`, all in
//! a new folder in `C:\kairo\logs`. The command line, the FFmpeg arguments, the checks of
//! FFmpeg's log and the testbed's ready file and warm-up, the expect file, the read-back table,
//! the report and pruning are pure and tested everywhere; the Windows calls are in the `platform`
//! module, the only one allowed `unsafe` (CLAUDE.md), and `run` ties them together. It never
//! opens another process, sends input or touches a game (CLAUDE.md, M1:15).

#![deny(unsafe_code)]

pub mod capture;
pub mod cli;
pub mod expect;
pub mod ffmpeg_log;
pub mod outcome;
pub mod phases;
#[cfg(windows)]
#[allow(unsafe_code)]
mod platform;
pub mod prune;
pub mod readback;
pub mod ready;
pub mod report;
#[cfg(windows)]
pub mod run;
pub mod warmup;
