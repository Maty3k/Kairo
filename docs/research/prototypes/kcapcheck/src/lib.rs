#![cfg(windows)]
#![allow(dead_code)]
//! Compile-check harness for the Kairo capture research sketches (windows 0.62.2).

pub mod affinity;
pub mod clock;
pub mod dda;
pub mod device;
pub mod fullscreen;
pub mod monitors;
pub mod probe;
pub mod target;
pub mod wgc;
pub mod winrt_init;
