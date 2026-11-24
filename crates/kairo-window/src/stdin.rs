// SPDX-License-Identifier: GPL-3.0-or-later
//! `--stop-on-stdin-eof` (docs/research/16-verify-testbed/README.md §4.10, §5; research 17
//! §3.2). A program started with the flag ends its run when its standard input ends: the harness
//! and the bench pipe it and close the pipe to stop the program. It is a flag because
//! `kairo-devrun` starts every job with a NUL standard input, which ends at once (research 16
//! R7), so a job started straight from devrun leaves it off.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Reads standard input to its end on a thread of its own, then sets `eof`.
pub fn watch_eof(eof: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0_u8; 256];
        while let Ok(n) = stdin.read(&mut buffer) {
            if n == 0 {
                break;
            }
        }
        eof.store(true, Ordering::Relaxed);
    });
}
