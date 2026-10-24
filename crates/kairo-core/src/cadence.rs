// SPDX-License-Identifier: GPL-3.0-or-later
//! Cadence: how evenly the content advances from one output frame to the next (docs/PLAN.md M2,
//! SP2's gate; docs/milestones/M1.md, `kairo-verify`'s drop, duplicate and judder checks).
//!
//! The input holds, for each output frame in order, the time of the content it shows: the
//! timestamps a pacer replay shows, or the testbed frames a clip's barcodes name. A step is the
//! difference between consecutive values, ideally one tick. A step of zero is a repeat, and a
//! step of `n` ticks (rounded to nearest, halves up) skips `n − 1` content frames. Deviations
//! are compared as exact integers, so the percentile is the same on every platform.
//!
//! How the gates read these fields, until SP2's ADR confirms it: SP2's "≤0.5% repeated or
//! skipped frames" is `(repeats + skipped) / steps`, and one stall counts in both; M3 task 4's
//! "no gap >2 ticks" is `longest_hold ≤ 2` and `max_step_ticks ≤ 2` (rounded, so steps under
//! 2.5 ticks pass). Timestamps are whole counts of 100 ns, so a cadence that sits exactly on
//! SP2's "p99 ≤ one refresh period" measures just above it; that comparison needs a tolerance.

use std::fmt;

use crate::timeline::{FrameRate, HNS_PER_SECOND};

/// Cadence statistics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cadence {
    /// Steps measured: one fewer than the frames.
    pub steps: usize,
    /// Steps of zero: the same content shown again.
    pub repeats: usize,
    /// Steps that go back in content time.
    pub backwards: usize,
    /// Content frames skipped: `n − 1` for each step of `n ≥ 2` ticks.
    pub skipped: u64,
    /// The longest run of consecutive frames showing the same content.
    pub longest_hold: usize,
    /// The largest step, in ticks rounded to nearest.
    pub max_step_ticks: i64,
    /// The 99th percentile (nearest rank) of |step − one tick|, in 100 ns.
    pub p99_deviation_hns: f64,
}

/// Why cadence could not be measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadenceError {
    /// Fewer than two frames, so there is no step.
    TooShort(usize),
}

impl fmt::Display for CadenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(frames) => {
                write!(f, "cadence needs at least 2 frames, not {frames}")
            }
        }
    }
}

impl std::error::Error for CadenceError {}

/// Measures the cadence of `content_hns` at output frame rate `rate`.
pub fn cadence(content_hns: &[i64], rate: FrameRate) -> Result<Cadence, CadenceError> {
    if content_hns.len() < 2 {
        return Err(CadenceError::TooShort(content_hns.len()));
    }
    let fps = i128::from(rate.hz());
    // One tick, scaled by fps so it is a whole number: 10⁷.
    let tick = i128::from(HNS_PER_SECOND);
    let mut stats = Cadence {
        steps: 0,
        repeats: 0,
        backwards: 0,
        skipped: 0,
        longest_hold: 1,
        max_step_ticks: i64::MIN,
        p99_deviation_hns: 0.0,
    };
    // |step × fps − 10⁷| for each step: the deviation in 100 ns, scaled by fps.
    let mut deviations = Vec::with_capacity(content_hns.len() - 1);
    let mut hold = 1;
    for (&from, &to) in content_hns.iter().zip(content_hns.iter().skip(1)) {
        let step = i128::from(to) - i128::from(from);
        let scaled = step * fps;
        deviations.push((scaled - tick).unsigned_abs());
        let ticks = (2 * scaled + tick).div_euclid(2 * tick);
        stats.steps += 1;
        if step == 0 {
            stats.repeats += 1;
            hold += 1;
            stats.longest_hold = stats.longest_hold.max(hold);
        } else {
            hold = 1;
        }
        if step < 0 {
            stats.backwards += 1;
        }
        if ticks >= 2 {
            let skipped = u64::try_from(ticks - 1).unwrap_or(u64::MAX);
            stats.skipped = stats.skipped.saturating_add(skipped);
        }
        let ticks = i64::try_from(ticks).unwrap_or(if ticks < 0 { i64::MIN } else { i64::MAX });
        stats.max_step_ticks = stats.max_step_ticks.max(ticks);
    }
    deviations.sort_unstable();
    // Nearest rank: the ⌈0.99 × n⌉-th smallest.
    let rank = (99 * deviations.len()).div_ceil(100);
    let p99 = deviations
        .get(rank.saturating_sub(1))
        .copied()
        .unwrap_or_default();
    stats.p99_deviation_hns = p99 as f64 / fps as f64;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values come from an independent Python model with exact fractions.

    #[test]
    fn counts_repeats_skips_and_holds() {
        let cadence = cadence(&[0, 166_667, 333_333, 333_333, 666_667], FrameRate::Fps60).unwrap();
        assert_eq!(
            (
                cadence.steps,
                cadence.repeats,
                cadence.backwards,
                cadence.skipped,
                cadence.longest_hold,
                cadence.max_step_ticks
            ),
            (4, 1, 0, 1, 2, 2)
        );
        // The worst step, 333,334, is 166,667.33 from one tick; with 4 steps, p99 is the 4th.
        assert!((cadence.p99_deviation_hns - 166_667.333).abs() < 0.001);
    }

    #[test]
    fn backwards_steps() {
        let cadence = cadence(&[0, 166_667, 100_000, 266_667], FrameRate::Fps60).unwrap();
        assert_eq!(
            (cadence.backwards, cadence.repeats, cadence.skipped),
            (1, 0, 0)
        );
        assert_eq!(cadence.max_step_ticks, 1);
        assert!((cadence.p99_deviation_hns - 233_333.667).abs() < 0.001);
    }

    #[test]
    fn skips_round_half_up() {
        // 1.5 ticks at 60 fps is exactly 250,000: it rounds to 2 and skips one frame.
        let half = cadence(&[0, 250_000], FrameRate::Fps60).unwrap();
        assert_eq!((half.skipped, half.max_step_ticks), (1, 2));
        let under = cadence(&[0, 249_999], FrameRate::Fps60).unwrap();
        assert_eq!((under.skipped, under.max_step_ticks), (0, 1));
        let hold = cadence(&[5, 5, 5, 5], FrameRate::Fps30).unwrap();
        assert_eq!(
            (hold.repeats, hold.longest_hold, hold.max_step_ticks),
            (3, 4, 0)
        );
    }

    #[test]
    fn p99_is_the_nearest_rank() {
        // 100 steps of one tick, then one or two steps of 100,000 more: one outlier in 100 is
        // above the 99th rank, two are not.
        let ideal: Vec<i64> = (0..101).map(|k| k * 166_667).collect();
        let one: Vec<i64> = ideal
            .iter()
            .enumerate()
            .map(|(k, &t)| if k >= 50 { t + 100_000 } else { t })
            .collect();
        let one = cadence(&one, FrameRate::Fps60).unwrap();
        assert_eq!((one.steps, one.skipped), (100, 1));
        assert!((one.p99_deviation_hns - 0.333).abs() < 0.001);
        let two: Vec<i64> = ideal
            .iter()
            .enumerate()
            .map(|(k, &t)| match k {
                0..30 => t,
                30..60 => t + 100_000,
                _ => t + 200_000,
            })
            .collect();
        let two = cadence(&two, FrameRate::Fps60).unwrap();
        assert_eq!(two.skipped, 2);
        assert!((two.p99_deviation_hns - 100_000.333).abs() < 0.001);
    }

    #[test]
    fn extremes_do_not_overflow() {
        // The widest step, 2⁶⁴ − 1 hns, is 221,360,928,884,515 ticks at 120 fps.
        let wild = cadence(&[i64::MIN, i64::MAX, i64::MIN], FrameRate::Fps120).unwrap();
        assert_eq!(wild.backwards, 1);
        assert_eq!(wild.skipped, 221_360_928_884_514);
        assert_eq!(wild.max_step_ticks, 221_360_928_884_515);
    }

    #[test]
    fn needs_two_frames() {
        assert_eq!(
            cadence(&[], FrameRate::Fps60),
            Err(CadenceError::TooShort(0))
        );
        assert_eq!(
            cadence(&[7], FrameRate::Fps60),
            Err(CadenceError::TooShort(1))
        );
    }
}
