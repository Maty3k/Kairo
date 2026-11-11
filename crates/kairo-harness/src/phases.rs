// SPDX-License-Identifier: GPL-3.0-or-later
//! `run`'s phases and their time limits (docs/research/16-verify-testbed/README.md §8.3), and
//! its exit codes.

/// Exit codes of `run` (README §8.3).
pub mod exit {
    /// A report exists, whatever its verdict: gate 2's check is the report.
    pub const OK: u8 = 0;
    /// A step failed or timed out.
    pub const FAILED: u8 = 1;
    /// A bad command line.
    pub const USAGE: u8 = 2;
    /// The PC refused: monitor, OLED, HDR, rate, disk, another FFmpeg, display off, a missing
    /// program.
    pub const REFUSED: u8 = 3;
    /// The encoder's read-back failed: nothing was verified.
    pub const READBACK: u8 = 4;
    /// Internal: the report couldn't be written.
    pub const INTERNAL: u8 = 5;
}

/// A phase of `run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Folder, pruning, disk, FFmpeg, the monitor and adapter, the expect file.
    Setup,
    /// The testbed starts and writes its ready file.
    Testbed,
    /// Steady presents and display times.
    Warmup,
    /// FFmpeg records.
    Capture,
    /// The testbed's standard input closes and it ends its log.
    Stop,
    /// kairo-verify judges the clip.
    Verify,
}

impl Phase {
    /// Every phase, in order.
    pub const ALL: [Phase; 6] = [
        Phase::Setup,
        Phase::Testbed,
        Phase::Warmup,
        Phase::Capture,
        Phase::Stop,
        Phase::Verify,
    ];

    /// Its name in the report.
    pub fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Testbed => "testbed",
            Self::Warmup => "warmup",
            Self::Capture => "capture",
            Self::Stop => "stop",
            Self::Verify => "verify",
        }
    }

    /// Its limit in seconds for a capture of `seconds`. Setup's is recorded, not enforced:
    /// nothing runs beside it.
    pub fn limit_s(self, seconds: u32) -> u32 {
        match self {
            Self::Setup => 5,
            Self::Testbed => 15,
            Self::Warmup => 3,
            Self::Capture => capture_limit_s(seconds),
            Self::Stop => 5,
            Self::Verify => 120,
        }
    }
}

/// How long FFmpeg may take for `seconds` of capture: AMF's open and close and the faststart
/// rewrite fit in 30 s more (README §8.8: about 32 s for 30).
pub fn capture_limit_s(seconds: u32) -> u32 {
    seconds + 30
}

/// How long the testbed runs: past every phase up to the stop, so it never ends first.
pub fn testbed_seconds(seconds: u32) -> u32 {
    seconds + 120
}

/// The job's `max_seconds` that `pc harness capture` gives `kairo-devrun`: every limit, the
/// kills' waits and some slack.
pub fn max_seconds(seconds: u32) -> u32 {
    seconds + 210
}

/// The sum of every phase's limit.
pub fn total_limit_s(seconds: u32) -> u32 {
    Phase::ALL.iter().map(|p| p.limit_s(seconds)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_fit_the_job() {
        assert_eq!(total_limit_s(30), 208);
        for seconds in 5..=60 {
            // Each kill waits up to 2 s more for the process to go, and three can happen.
            assert!(
                total_limit_s(seconds) + 3 * 2 < max_seconds(seconds),
                "{seconds}"
            );
            // The testbed outlives the testbed, warm-up, capture and stop phases.
            let through_stop: u32 = [Phase::Testbed, Phase::Warmup, Phase::Capture, Phase::Stop]
                .iter()
                .map(|p| p.limit_s(seconds))
                .sum();
            assert!(testbed_seconds(seconds) > through_stop, "{seconds}");
        }
        assert_eq!(max_seconds(30), 240);
    }
}
