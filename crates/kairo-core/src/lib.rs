// SPDX-License-Identifier: GPL-3.0-or-later
//! Kairo's platform-independent core.
//!
//! It holds the dev-loop contract that `kairo-devrun`, `kairo-hello` and `kairo-cli` share
//! (docs/PLAN.md §3.1 and M0): the PC folder layout, job files and job results, the watch
//! agent's status, the hello probe's JSON-RPC methods and the pipe framing.
//!
//! From M1 it also holds the engine's media maths (docs/PLAN.md §3.2): the clock that ties
//! output ticks and audio samples to QPC time, the frame pacer, the `wgc-probe` trace format
//! with its offline replay, and cadence statistics. These modules take times as arguments and
//! never read a clock. The crate has no Windows code, so its tests run on the Mac and on Linux
//! CI.

#![forbid(unsafe_code)]

pub mod args;
pub mod cadence;
pub mod hello;
pub mod job;
pub mod layout;
pub mod pacer;
pub mod rpc;
pub mod time;
pub mod timeline;
pub mod trace;
pub mod watch;
