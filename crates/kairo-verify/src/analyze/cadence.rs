// SPDX-License-Identifier: GPL-3.0-or-later
//! Drops, duplicates and judder from the barcodes (docs/research/16-verify-testbed/README.md
//! §2 items 11–12, §6.3): steps in testbed frames between consecutive readable output frames,
//! classified by `kairo_core::cadence::steps`, against an ideal every-vblank delivery replayed
//! through `kairo_core`'s pacer at the same rates, so a panel isn't blamed for its refresh rate.

use kairo_core::cadence::{self, Step};
use kairo_core::timeline::{FrameRate, HNS_PER_SECOND};
use kairo_core::trace::{self, Trace, TraceFrame};

/// One step worth listing: the output frame it ends on and the frame numbers on each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    /// The output frame the step ends on.
    pub frame: usize,
    /// The testbed frame before.
    pub from: i64,
    /// The testbed frame after.
    pub to: i64,
}

/// What the steps show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    /// Steps measured.
    pub steps: usize,
    /// Steps of zero frames: the same picture again.
    pub repeats: usize,
    /// Testbed frames skipped: `n − 1` for each step of `n ≥ 2` ticks.
    pub skipped: u64,
    /// Steps back in time.
    pub backwards: usize,
    /// Judder steps (README §2 item 12).
    pub judder: usize,
    /// The longest run of frames showing one picture.
    pub longest_hold: usize,
    /// The largest step, in ticks.
    pub max_step_ticks: i128,
    /// The repeats, skips and backward steps, in order.
    pub events: Vec<Event>,
    /// The judder steps, in order.
    pub judder_events: Vec<Event>,
    /// Steps left out because they touch a frame the testbed itself missed (README §6.3: with a
    /// log, events at testbed misses count as source), in order.
    pub source_events: Vec<Event>,
}

impl Counts {
    /// `(repeats + skipped) / steps` (core/cadence.rs: one stall counts in both).
    pub fn drop_dup_share(&self) -> f64 {
        share(self.repeats as f64 + self.skipped as f64, self.steps)
    }

    /// Judder steps over steps.
    pub fn judder_share(&self) -> f64 {
        share(self.judder as f64, self.steps)
    }

    fn add(&mut self, step: &Step, frame: usize, from: i64, to: i64, hold: &mut usize) {
        self.steps += 1;
        let event = Event { frame, from, to };
        if step.delta == 0 {
            self.repeats += 1;
            *hold += 1;
        } else {
            *hold = 1;
        }
        self.longest_hold = self.longest_hold.max(*hold);
        if step.delta < 0 {
            self.backwards += 1;
        }
        if step.ticks >= 2 {
            self.skipped = self
                .skipped
                .saturating_add(u64::try_from(step.ticks - 1).unwrap_or(u64::MAX));
        }
        if step.delta <= 0 || step.ticks >= 2 {
            self.events.push(event);
        }
        if step.is_judder() {
            self.judder += 1;
            self.judder_events.push(event);
        }
        self.max_step_ticks = self.max_step_ticks.max(step.ticks);
    }
}

fn share(count: f64, steps: usize) -> f64 {
    if steps == 0 {
        0.0
    } else {
        count / steps as f64
    }
}

/// The steps of `numbers` (each output frame's testbed frame, `None` if unreadable) within each
/// run of readable frames, at `source_fps` testbed frames per second and `fps` output frames
/// per second.
pub fn count(numbers: &[Option<i64>], source_fps: u32, fps: u32) -> Option<Counts> {
    count_excluding(numbers, source_fps, fps, |_, _| false)
}

/// [`count`], leaving out (into [`Counts::source_events`]) each repeat, skip, backward or judder
/// step from testbed frame `from` to `to` for which `source(from, to)` holds: an event the
/// testbed's own miss explains. A step on cadence needs no excuse and always counts.
pub fn count_excluding(
    numbers: &[Option<i64>],
    source_fps: u32,
    fps: u32,
    source: impl Fn(i64, i64) -> bool,
) -> Option<Counts> {
    let mut counts = Counts {
        longest_hold: 1,
        ..Counts::default()
    };
    let mut start = 0;
    while start < numbers.len() {
        if numbers[start].is_none() {
            start += 1;
            continue;
        }
        let end = numbers[start..]
            .iter()
            .position(Option::is_none)
            .map_or(numbers.len(), |n| start + n);
        let run: Vec<i64> = numbers[start..end].iter().flatten().copied().collect();
        let mut hold = 1;
        for step in cadence::steps(&run, i64::from(source_fps), fps).ok()? {
            let (from, to) = (run[step.index], run[step.index + 1]);
            let frame = start + step.index + 1;
            let event = step.delta <= 0 || step.ticks >= 2 || step.is_judder();
            if event && source(from, to) {
                counts.source_events.push(Event { frame, from, to });
                hold = 1;
                continue;
            }
            counts.add(&step, frame, from, to, &mut hold);
        }
        start = end;
    }
    Some(counts)
}

/// The output rates `kairo_core`'s pacer runs at.
pub fn pacer_rate(fps: u32) -> Option<FrameRate> {
    match fps {
        30 => Some(FrameRate::Fps30),
        60 => Some(FrameRate::Fps60),
        120 => Some(FrameRate::Fps120),
        _ => None,
    }
}

/// How many phases of the pacer's ticks against the vblanks the ideal tries, worst case kept
/// (README §2 item 12).
pub const IDEAL_PHASES: i64 = 7;

/// How long the ideal delivery runs.
const IDEAL_SECONDS: f64 = 10.0;

/// The ideal: a display at `refresh_hz` showing testbed frame `⌊v / every⌋` on vblank `v`
/// (`every` vblanks per testbed frame), delivered on every vblank and paced at `rate`. Returns
/// the worst drop-and-duplicate share and the worst judder share over [`IDEAL_PHASES`] phases,
/// ticks falling 1/14 to 13/14 of a vblank after one.
pub fn ideal(refresh_hz: f64, every: u32, source_fps: u32, rate: FrameRate) -> Option<(f64, f64)> {
    if !(refresh_hz.is_finite() && refresh_hz >= 1.0) || every == 0 {
        return None;
    }
    let vblank = HNS_PER_SECOND as f64 / refresh_hz;
    let frames: Vec<TraceFrame> = (0..(refresh_hz * IDEAL_SECONDS) as i64)
        .map(|v| {
            let ts = (v as f64 * vblank).round() as i64;
            TraceFrame {
                ts_hns: ts,
                arr_hns: ts,
                ready_hns: Some(ts),
            }
        })
        .collect();
    let trace = Trace::from_frames(frames).ok()?;
    let mut worst = (0.0_f64, 0.0_f64);
    for j in 0..IDEAL_PHASES {
        let phase = ((2 * j + 1) as f64 * vblank / 14.0).round() as i64;
        let replay = trace::replay(&trace, rate, phase);
        let numbers: Vec<Option<i64>> = replay
            .shown_hns
            .iter()
            .map(|&ts| Some((ts as f64 / vblank).round() as i64 / i64::from(every)))
            .collect();
        let counts = count(&numbers, source_fps, rate.hz() as u32)?;
        worst.0 = worst.0.max(counts.drop_dup_share());
        worst.1 = worst.1.max(counts.judder_share());
    }
    Some(worst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(values: &[i64]) -> Vec<Option<i64>> {
        values.iter().map(|&v| Some(v)).collect()
    }

    #[test]
    fn only_events_at_source_misses_are_set_aside() {
        // 180 fps content at 60 fps: a repeat of 9 and the skip after it, around a miss at 10.
        let n = numbers(&[0, 3, 6, 9, 9, 15, 18, 21]);
        let blind = count(&n, 180, 60).unwrap();
        assert_eq!((blind.repeats, blind.skipped), (1, 1));
        let counts = count_excluding(&n, 180, 60, |from, to| {
            let (low, high) = (from.min(to), from.max(to));
            (low + 1..=high + 1).contains(&10)
        })
        .unwrap();
        assert_eq!((counts.repeats, counts.skipped, counts.steps), (0, 0, 5));
        let set_aside: Vec<(i64, i64)> = counts
            .source_events
            .iter()
            .map(|e| (e.from, e.to))
            .collect();
        // 6 → 9 lands just before the miss but is on cadence: it counts.
        assert_eq!(set_aside, [(9, 9), (9, 15)]);
    }

    #[test]
    fn counts_repeats_skips_backward_steps_and_judder() {
        // 180 fps content at 60 fps: one tick is 3 testbed frames.
        let good = count(&numbers(&[0, 3, 6, 9, 12]), 180, 60).unwrap();
        assert_eq!(
            (good.steps, good.repeats, good.skipped, good.judder),
            (4, 0, 0, 0)
        );
        let bad = count(&numbers(&[0, 3, 3, 9, 6, 8, 11]), 180, 60).unwrap();
        assert_eq!(bad.steps, 6);
        assert_eq!(bad.repeats, 1);
        assert_eq!(bad.skipped, 1);
        assert_eq!(bad.backwards, 1);
        // 6 → 8 is 2/3 of a tick: rounds to one but is more than a quarter off.
        assert_eq!(bad.judder, 1);
        assert_eq!(
            bad.judder_events,
            [Event {
                frame: 5,
                from: 6,
                to: 8
            }]
        );
        assert_eq!(
            bad.events,
            [
                Event {
                    frame: 2,
                    from: 3,
                    to: 3
                },
                Event {
                    frame: 3,
                    from: 3,
                    to: 9
                },
                Event {
                    frame: 4,
                    from: 9,
                    to: 6
                }
            ]
        );
        assert_eq!(bad.longest_hold, 2);
        assert_eq!(bad.max_step_ticks, 2);
        assert!((bad.drop_dup_share() - 2.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn unreadable_frames_split_the_runs() {
        // No step bridges an unreadable frame, so the gap isn't a skip.
        let counts = count(&[Some(0), Some(3), None, Some(9), Some(12)], 180, 60).unwrap();
        assert_eq!((counts.steps, counts.skipped), (2, 0));
        assert_eq!(count(&[None, None], 180, 60).unwrap().steps, 0);
        assert!(count(&numbers(&[0, 3]), 0, 60).is_none());
    }

    #[test]
    fn the_ideal_matches_the_design_table() {
        // README §2 item 12 (judder_rule.py): every-vblank delivery paced at 60 fps.
        let rate = FrameRate::Fps60;
        let at = |hz: f64| ideal(hz, 1, hz as u32, rate).unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 0.005;
        assert_eq!(at(180.0), (0.0, 0.0));
        assert_eq!(at(120.0), (0.0, 0.0));
        let (drops, judder) = at(144.0);
        assert!(close(judder, 0.0) && close(drops, 0.0), "{drops} {judder}");
        assert!(close(at(175.0).1, 1.0 / 12.0), "{:?}", at(175.0));
        assert!(close(at(170.0).1, 1.0 / 6.0), "{:?}", at(170.0));
        assert!(close(at(165.0).1, 0.25), "{:?}", at(165.0));
        assert!(close(at(75.0).0, 0.251), "{:?}", at(75.0));
        assert!(close(at(50.0).0, 1.0 / 6.0), "{:?}", at(50.0));
        // A 90 fps testbed on a 180 Hz panel (each frame shows for two vblanks) judders at
        // 60 fps however well it is delivered, so the ideal carries it and the clip isn't blamed.
        assert_eq!(ideal(180.0, 2, 90, rate).map(|i| i.1), Some(1.0));
        assert!(ideal(f64::NAN, 1, 180, rate).is_none());
        assert!(ideal(180.0, 0, 180, rate).is_none());
        assert_eq!(pacer_rate(25), None);
    }
}
