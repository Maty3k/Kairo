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
//!
//! [`steps`] gives each step on its own, in any whole content unit per second and at any whole
//! output rate: 100 ns for timestamps, or testbed frames at their nominal rate for barcodes
//! (docs/research/16-verify-testbed/README.md §2 items 11–12). [`cadence`] is built on it.

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
    /// Content units per second must be at least 1.
    Units(i64),
    /// The output rate must be at least 1 Hz.
    Rate(u32),
}

impl fmt::Display for CadenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(frames) => {
                write!(f, "cadence needs at least 2 frames, not {frames}")
            }
            Self::Units(units) => {
                write!(
                    f,
                    "content units per second must be at least 1, not {units}"
                )
            }
            Self::Rate(rate) => write!(f, "the output rate must be at least 1 Hz, not {rate}"),
        }
    }
}

impl std::error::Error for CadenceError {}

/// One step from an output frame to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The index of the step's first frame.
    pub index: usize,
    /// How far the content moved, in content units.
    pub delta: i128,
    /// The step in ticks, rounded to nearest, halves up.
    pub ticks: i128,
    /// How far the step is from one tick, in content units times the output rate:
    /// `delta × rate − units per second`. Exact, so a step can be compared with any fraction of
    /// a tick without rounding.
    pub off_tick_scaled: i128,
    /// One tick in the same scaled units: the content units per second.
    pub tick_scaled: i128,
}

impl Step {
    /// A judder step (docs/research/16-verify-testbed/README.md §2 item 12): it rounds to exactly
    /// one tick but is more than a quarter of a tick away from it. A step exactly a quarter
    /// away is not one, so it can't flip with timestamp jitter.
    pub fn is_judder(&self) -> bool {
        self.ticks == 1 && 4 * self.off_tick_scaled.abs() > self.tick_scaled
    }
}

/// The steps of a content sequence, in order; see [`steps`].
#[derive(Debug, Clone)]
pub struct Steps<'a> {
    content: &'a [i64],
    units: i128,
    rate: i128,
    next: usize,
}

impl Iterator for Steps<'_> {
    type Item = Step;

    fn next(&mut self) -> Option<Step> {
        let index = self.next;
        let (&from, &to) = (self.content.get(index)?, self.content.get(index + 1)?);
        self.next += 1;
        let delta = i128::from(to) - i128::from(from);
        let scaled = delta * self.rate;
        Some(Step {
            index,
            delta,
            ticks: (2 * scaled + self.units).div_euclid(2 * self.units),
            off_tick_scaled: scaled - self.units,
            tick_scaled: self.units,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.content.len().saturating_sub(self.next + 1);
        (left, Some(left))
    }
}

/// The steps of `content`, which holds for each output frame in order the content it shows, in
/// whole units of which there are `units_per_second`, at an output rate of `rate_hz`. One tick
/// is `units_per_second / rate_hz` units, which need not be whole.
pub fn steps(
    content: &[i64],
    units_per_second: i64,
    rate_hz: u32,
) -> Result<Steps<'_>, CadenceError> {
    if units_per_second < 1 {
        return Err(CadenceError::Units(units_per_second));
    }
    if rate_hz == 0 {
        return Err(CadenceError::Rate(rate_hz));
    }
    Ok(Steps {
        content,
        units: i128::from(units_per_second),
        rate: i128::from(rate_hz),
        next: 0,
    })
}

/// Measures the cadence of `content_hns` at output frame rate `rate`.
pub fn cadence(content_hns: &[i64], rate: FrameRate) -> Result<Cadence, CadenceError> {
    if content_hns.len() < 2 {
        return Err(CadenceError::TooShort(content_hns.len()));
    }
    let fps = rate.hz();
    let steps = steps(content_hns, HNS_PER_SECOND, fps as u32)?;
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
    for step in steps {
        deviations.push(step.off_tick_scaled.unsigned_abs());
        stats.steps += 1;
        if step.delta == 0 {
            stats.repeats += 1;
            hold += 1;
            stats.longest_hold = stats.longest_hold.max(hold);
        } else {
            hold = 1;
        }
        if step.delta < 0 {
            stats.backwards += 1;
        }
        if step.ticks >= 2 {
            let skipped = u64::try_from(step.ticks - 1).unwrap_or(u64::MAX);
            stats.skipped = stats.skipped.saturating_add(skipped);
        }
        let ticks =
            i64::try_from(step.ticks).unwrap_or(if step.ticks < 0 { i64::MIN } else { i64::MAX });
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

    fn fields(step: &Step) -> (usize, i128, i128, i128, i128, bool) {
        (
            step.index,
            step.delta,
            step.ticks,
            step.off_tick_scaled,
            step.tick_scaled,
            step.is_judder(),
        )
    }

    #[test]
    fn steps_carry_their_exact_distance_from_a_tick() {
        let content = [0, 166_667, 333_333, 333_333, 666_667];
        let all: Vec<_> = steps(&content, HNS_PER_SECOND, 60)
            .unwrap()
            .map(|s| fields(&s))
            .collect();
        assert_eq!(
            all,
            [
                (0, 166_667, 1, 20, 10_000_000, false),
                (1, 166_666, 1, -40, 10_000_000, false),
                (2, 0, 0, -10_000_000, 10_000_000, false),
                (3, 333_334, 2, 10_000_040, 10_000_000, false),
            ]
        );
        assert_eq!(
            steps(&content, HNS_PER_SECOND, 60).unwrap().size_hint(),
            (4, Some(4))
        );
        assert_eq!(steps(&[7], HNS_PER_SECOND, 60).unwrap().count(), 0);
    }

    #[test]
    fn backward_steps_round_half_up_too() {
        // −0.6 tick rounds to −1 (not 0), and −1.5 ticks to −1 (halves up, not away from 0).
        let back: Vec<_> = steps(&[0, -100_000, -350_000], HNS_PER_SECOND, 60)
            .unwrap()
            .map(|s| fields(&s))
            .collect();
        assert_eq!(
            back,
            [
                (0, -100_000, -1, -16_000_000, 10_000_000, false),
                (1, -250_000, -1, -25_000_000, 10_000_000, false),
            ]
        );
    }

    #[test]
    fn steps_at_export_rates() {
        // Exports run at 23–60 fps (README §2 item 1). At a 180 Hz source a tick is 7.5 frames
        // at 24 fps and 7.2 at 25 fps.
        let at = |rate: u32, frames: &[i64]| -> Vec<_> {
            steps(frames, 180, rate)
                .unwrap()
                .map(|s| fields(&s))
                .collect()
        };
        assert_eq!(
            at(24, &[0, 7, 15, 30]),
            [
                (0, 7, 1, -12, 180, false),
                (1, 8, 1, 12, 180, false),
                (2, 15, 2, 180, 180, false),
            ]
        );
        assert_eq!(
            at(25, &[0, 7, 14, 22, 26]),
            [
                (0, 7, 1, -5, 180, false),
                (1, 7, 1, -5, 180, false),
                (2, 8, 1, 20, 180, false),
                (3, 4, 1, -80, 180, true),
            ]
        );
    }

    #[test]
    fn judder_steps_in_testbed_frames() {
        // Barcode frame numbers at the testbed's nominal rate, under a 60 fps output: a tick is
        // 3 frames at 180 Hz, 2.4 at 144 and 2.92 at 175.
        let judder = |units: i64, frames: &[i64]| -> Vec<(i128, i128, i128, bool)> {
            steps(frames, units, 60)
                .unwrap()
                .map(|s| (s.delta, s.ticks, s.off_tick_scaled, s.is_judder()))
                .collect()
        };
        // 90/s on a 180 Hz panel: every other step is 2 or 4 frames.
        assert_eq!(
            judder(180, &[0, 3, 6, 8, 12, 14, 18]),
            [
                (3, 1, 0, false),
                (3, 1, 0, false),
                (2, 1, -60, true),
                (4, 1, 60, true),
                (2, 1, -60, true),
                (4, 1, 60, true),
            ]
        );
        // Every vblank at 144 Hz: 3 frames is exactly a quarter tick off, which isn't judder.
        assert_eq!(
            judder(144, &[0, 2, 5, 7, 10, 12]),
            [
                (2, 1, -24, false),
                (3, 1, 36, false),
                (2, 1, -24, false),
                (3, 1, 36, false),
                (2, 1, -24, false),
            ]
        );
        // Every vblank at 175 Hz: the odd 2-frame step is.
        assert_eq!(
            judder(175, &[0, 3, 6, 8, 11]),
            [
                (3, 1, 5, false),
                (3, 1, 5, false),
                (2, 1, -55, true),
                (3, 1, 5, false)
            ]
        );
    }

    #[test]
    fn steps_need_whole_positive_units_and_rate() {
        assert_eq!(
            steps(&[0, 1], 0, 60).map(|_| ()),
            Err(CadenceError::Units(0))
        );
        assert_eq!(
            steps(&[0, 1], -5, 60).map(|_| ()),
            Err(CadenceError::Units(-5))
        );
        assert_eq!(
            steps(&[0, 1], 180, 0).map(|_| ()),
            Err(CadenceError::Rate(0))
        );
        // The widest step at the highest rate still fits.
        let wide: Vec<_> = steps(&[i64::MIN, i64::MAX], 1, u32::MAX).unwrap().collect();
        assert_eq!(wide[0].ticks, 79_228_162_495_817_593_515_539_431_425);
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
