// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-window`: the Windows code M1's instruments share (docs/research/17-loadgen-bench/README.md
//! §2 item 2, §3.1). It was the testbed's (research 16 §4.1–4.4, §4.8, §4.10) and moved here
//! unchanged, so `testbed.exe`, `kairo-loadgen` and `kairo-harness` keep one copy:
//!
//! - `gpu` (Windows): the D3D11 device on the adapter with a given LUID (feature level 11.1 or
//!   11.0, BGRA support), the adapter's DXGI index and the output that shows the monitor; the
//!   output's HDR state and a watch on it; a swap chain's back buffer 0 and its render target
//!   view; `Present` with the caller's flags and a lost device told apart; the frame-latency
//!   wait. Each program makes its own swap chain and draws its own way.
//! - `platform` (Windows): QPC time, the EcoQoS opt-out, `KeepDisplayOn`, the session's idle time,
//!   the foreground lock time-out and the `--activate` wait; the window (its class name and title
//!   given by the program) with its panic-safe window procedure; and `Cover`, a black, topmost
//!   window over one display that never takes focus (research 17 §4.3).
//! - [`ready`]: the ready file, written whole and only if it doesn't exist.
//! - [`stdin`]: the `--stop-on-stdin-eof` watcher.
//!
//! The crate has the testbed's shape: `unsafe` only in the Windows-only `gpu` and `platform`
//! modules, every block with a `// SAFETY:` comment (CLAUDE.md). Off Windows it builds to its pure
//! parts, `ready` and `stdin`, so the workspace builds, lints and tests on the Mac and Linux.
//! Every call is on the program's own window, device, process or thread: nothing here synthesises
//! input, hooks anything, attaches to another thread's input, captures the screen or opens
//! another process (CLAUDE.md house rules 1–4).
//!
//! **The exit-rule clock.** This is instrument code, outside the exit-rule clock like the testbed,
//! loadgen and the bench (M1's goal and "The exit-rule clock"). One edge, as with `kairo-display`
//! (research 17 §3.12, research 16 §10): if `wgc-probe` uses this crate, fixes made for it after
//! the clock starts count as capture code.

#![deny(unsafe_code)]

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod gpu;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod platform;
pub mod ready;
pub mod stdin;
