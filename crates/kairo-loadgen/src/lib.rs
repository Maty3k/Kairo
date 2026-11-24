// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-loadgen`: a deterministic GPU load for M1's bench (docs/research/17-loadgen-bench/README.md
//! §3). It draws a dim, slowly drifting colour field with faint moving grain, borderless on the
//! monitor named by its device path (the 180 Hz monitor in M1), on a flip-model swap chain with
//! tearing, uncapped, and makes each frame cost a fixed amount of GPU work: shader maths (`alu`,
//! `--iters`) or random texture reads (`bw`, `--reads`). `calibrate` finds the count that takes a
//! target time per frame, and `run` takes that count explicitly and never changes it.
//!
//! The command line ([`cli`]), the workloads' maths ([`workload`]: bands, the start-up ramp and
//! its TDR guards, the calibration, the per-frame constants), the picture's CPU reference and its
//! photosensitivity contract ([`visual`]), the log ([`log`]) and the ready, summary and
//! calibration JSON ([`summary`]) are pure and tested everywhere. The shaders are HLSL in
//! `loadgen.hlsl`, compiled at run time with `D3DCompile` ([`workload::HLSL`]). The window, the
//! D3D11 device on the monitor's adapter, QPC, the foreground wait, the stdin watcher and the
//! ready writer are `kairo-window`'s, shared with the testbed and the harness (research 17 §3.1).
//! Loadgen's own D3D11, DXGI and D3DKMT calls go in the Windows-only `gpu` module, the only one
//! allowed `unsafe`, every block with a `// SAFETY:` comment (CLAUDE.md); `run` ties them
//! together on Windows.
//!
//! Every call is on loadgen's own device, swap chain, window, process or thread: it captures
//! nothing, never synthesises input, hooks nothing and opens no other process (CLAUDE.md house
//! rules 1–4, research 17 §3.12). Its logs and files name no device path, monitor, adapter or PC
//! name, window title or wall-clock time; they are raw results of the PC and stay in
//! `C:\kairo\logs` and `.dogfood/`.
//!
//! **The exit-rule clock.** Loadgen is instrument code, outside the exit-rule clock (M1's goal
//! and "The exit-rule clock").

#![deny(unsafe_code)]

pub mod cli;
#[cfg(windows)]
#[allow(unsafe_code)]
mod gpu;
pub mod log;
#[cfg(windows)]
pub mod run;
pub mod summary;
pub mod visual;
pub mod workload;
