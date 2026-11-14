// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-testbed`: `testbed.exe`, pattern v1 on a chosen monitor with a QPC log as ground truth
//! for M1's capture tests, and `kairo-tone.exe`, the "voice chat" beep as its own process
//! (docs/research/16-verify-testbed/README.md §4–5).
//!
//! The window goes on the monitor named by its device path, never "the main screen", and the
//! D3D11 device on the adapter that drives it, by LUID (`kairo-display`). Frames are numbered by
//! the vblank group or timer slot they are drawn for ([`pace`]), drawn from
//! `kairo_core::testsig` with clears and copies only ([`draw`]), and logged with QPC times in
//! `kairo_core::testbed_log`'s format. The command line, pacing maths, draw plan and ready file
//! are pure and tested everywhere; the Win32, DXGI and D3D11 calls are in the Windows-only
//! `platform` and `gpu` modules, the only ones allowed `unsafe` (CLAUDE.md), and `run` ties them
//! together. The beeps ([`beeps`]) play through `wasapi`'s safe API on an audio thread (`audio`);
//! `voice` is `kairo-tone`'s run. The testbed never sends input, hooks anything, opens a capture device or the
//! microphone, or touches another process (CLAUDE.md, M1:15).

#![deny(unsafe_code)]

#[cfg(windows)]
mod audio;
pub mod beeps;
pub mod cli;
pub mod draw;
#[cfg(windows)]
#[allow(unsafe_code)]
mod gpu;
pub mod pace;
#[cfg(windows)]
#[allow(unsafe_code)]
mod platform;
pub mod ready;
#[cfg(windows)]
pub mod run;
#[cfg(windows)]
pub mod voice;
