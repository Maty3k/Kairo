// SPDX-License-Identifier: GPL-3.0-or-later
//! Kairo's platform-independent core.
//!
//! It holds the dev-loop contract that `kairo-devrun`, `kairo-hello` and `kairo-cli` share
//! (docs/PLAN.md §3.1 and M0): the PC folder layout, job files and job results, the watch
//! agent's status, the hello probe's JSON-RPC methods and the pipe framing.
//!
//! From M1 it also holds the engine's media maths (docs/PLAN.md §3.2): the clock that ties
//! output ticks and audio samples to QPC time, the frame pacer, the `wgc-probe` trace format
//! with its offline replay, cadence statistics, the replay ring with the cut that turns it into
//! a clip, the aligner that places each audio source on the 48 kHz timeline, and the mixer
//! with its limiter and meters. These modules take times as arguments and never read a clock.
//! The crate has no Windows code, so its tests run on the Mac and on Linux CI.

#![forbid(unsafe_code)]

pub mod aligner;
pub mod args;
pub mod cadence;
pub mod cut;
pub mod hello;
pub mod job;
pub mod layout;
pub mod limiter;
pub mod meter;
pub mod mix;
pub mod pacer;
pub mod ring;
pub mod rpc;
pub mod time;
pub mod timeline;
pub mod trace;
pub mod watch;
