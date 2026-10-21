// SPDX-License-Identifier: GPL-3.0-or-later
//! Kairo's platform-independent core.
//!
//! In M0 this crate holds the dev-loop contract that `kairo-devrun`, `kairo-hello` and
//! `kairo-cli` share (docs/PLAN.md §3.1 and M0): the PC folder layout, job files and job
//! results, the watch agent's status, the hello probe's JSON-RPC methods and the pipe framing.
//! It has no Windows code, so its tests run on the Mac and on Linux CI.

#![forbid(unsafe_code)]

pub mod args;
pub mod hello;
pub mod job;
pub mod layout;
pub mod rpc;
pub mod time;
pub mod watch;
