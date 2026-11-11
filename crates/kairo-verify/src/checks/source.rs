// SPDX-License-Identifier: GPL-3.0-or-later
//! What the testbed's log says about the run a clip shows (docs/research/16-verify-testbed/
//! README.md §4.9, §6.3): the rates the barcode line would otherwise estimate, the run tag, the
//! HDR levels, each frame's display time, the beeps' played times and the frames the testbed
//! itself missed, so that a capture isn't blamed for them.
//!
//! A frame counts as the testbed's miss when the log has an `m` line for it (a frame skipped, or
//! shown more than k vblanks after the one before it), when it was never drawn (a gap between
//! drawn frames' numbers), or, with vsync pacing, when its display time is `-` just before a
//! frame the testbed marked late: the statistics passed over an irregular stretch, where the
//! unreported frame may be the late one. Any other `-` means the statistics didn't say (none
//! yet, a stretch they never reported, or none at all after `e no_frame_statistics`: the first
//! frame they report is never marked late, so `-` frames before statistics start are never read
//! as misses), and with timer pacing it is a frame the display passed over by design.
//!
//! What the misses excuse is bounded by what they can do to the screen ([`Excuses`]).

use std::collections::{BTreeMap, BTreeSet};

use kairo_core::testbed_log::{Pace, TestbedLog, TestbedRecord, ToneLog};

/// A testbed log, read for the checks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    /// The frames per second the testbed drew (F_s).
    pub fps: u32,
    /// The rate that times its flash (F).
    pub nominal_fps: u32,
    /// Its display's refresh rate, in Hz.
    pub refresh_hz: f64,
    /// Whether it was timer-paced: its display shows the newest frame at each vblank.
    pub timer: bool,
    /// Its run tag.
    pub run_tag: u8,
    /// The OS's SDR white and the display's peak, in nits, when it drew in HDR.
    pub hdr: Option<(f64, f64)>,
    /// Whether the log says when frames showed: some frame has a display time (statistics that
    /// came after `e no_frame_statistics` count).
    pub timed: bool,
    /// Each drawn frame's display time in seconds, `None` if never shown or unknown.
    shown: BTreeMap<i64, Option<f64>>,
    /// The frames the testbed missed.
    misses: BTreeSet<i64>,
    /// Each beep: its number, its flash frame N_f and when it played (seconds), if it did.
    pub beeps: Vec<(i64, i64, Option<f64>)>,
}

/// What the log says about the frames a clip shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stability {
    /// The testbed frames in the clip's span.
    pub frames: u64,
    /// How many of them the testbed missed.
    pub missed: u64,
    /// The span's frames the log holds no record of: neither drawn nor missed, so the log can't
    /// be the run's.
    pub unlogged: u64,
}

impl Stability {
    /// The missed share.
    pub fn share(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.missed as f64 / self.frames as f64
        }
    }
}

fn seconds(hns: i64) -> f64 {
    hns as f64 / 10_000_000.0
}

impl Source {
    /// Reads `log`.
    pub fn new(log: &TestbedLog) -> Source {
        let h = &log.header;
        let refresh_hz = h.refresh_mhz as f64 / 1_000.0;
        let shown: BTreeMap<i64, Option<f64>> = log
            .frames()
            .filter_map(|f| Some((i64::try_from(f.frame).ok()?, f.display_hns.map(seconds))))
            .collect();
        let mut misses = BTreeSet::new();
        for record in &log.records {
            if let TestbedRecord::Miss(frame, _) = record
                && let Ok(frame) = i64::try_from(*frame)
            {
                misses.insert(frame);
            }
        }
        // Gaps between drawn frames: frames the testbed never drew.
        let drawn: Vec<i64> = shown.keys().copied().collect();
        for pair in drawn.windows(2) {
            misses.extend(pair[0] + 1..pair[1]);
        }
        let timed = shown.values().any(Option::is_some);
        let timer = h.pace == Pace::Timer;
        // With vsync, a `-` just before a frame marked late: from the last frame back, each
        // `-` frame belongs to the next frame that has a display time.
        if timed && !timer {
            let mut before_late = false;
            let mut unknown_before_late = Vec::new();
            for (&n, display) in shown.iter().rev() {
                match display {
                    Some(_) => before_late = misses.contains(&n),
                    None if before_late => unknown_before_late.push(n),
                    None => {}
                }
            }
            misses.extend(unknown_before_late);
        }
        let beeps = log
            .beeps()
            .filter_map(|b| {
                Some((
                    i64::try_from(b.beep).ok()?,
                    i64::try_from(b.flash_frame).ok()?,
                    b.played_hns.map(seconds),
                ))
            })
            .collect();
        Source {
            fps: h.fps,
            nominal_fps: h.nominal_fps,
            refresh_hz,
            timer,
            run_tag: h.run_tag,
            hdr: h.sdr_white_nits.zip(h.max_nits).filter(|_| h.hdr),
            timed,
            shown,
            misses,
            beeps,
        }
    }

    /// How many vblanks each testbed frame lasts: refresh / fps rounded, at least 1.
    pub fn every(&self) -> u32 {
        ((self.refresh_hz / f64::from(self.fps.max(1))).round() as u32).max(1)
    }

    /// Whether the testbed missed frame `n`.
    #[cfg(test)]
    pub fn is_miss(&self, n: i64) -> bool {
        self.misses.contains(&n)
    }

    /// The runs of consecutive missed frames, first and last, in order.
    fn miss_runs(&self) -> Vec<(i64, i64)> {
        let mut runs: Vec<(i64, i64)> = Vec::new();
        for &n in &self.misses {
            match runs.last_mut() {
                Some(run) if run.1 + 1 == n => run.1 = n,
                _ => runs.push((n, n)),
            }
        }
        runs
    }

    /// When frame `n` first showed, in seconds of QPC time, if the log says.
    pub fn display_s(&self, n: i64) -> Option<f64> {
        self.shown.get(&n).copied().flatten()
    }

    /// Whether frame `n` was on screen within `slack` seconds of QPC time `t`, by the log's
    /// display times: from its display to the next frame's that has one. A frame drawn but not
    /// reported could have shown any time between its neighbours that were; a frame never drawn
    /// was never on screen. `None` where the log can't say.
    pub fn on_screen(&self, n: i64, t: f64, slack: f64) -> Option<bool> {
        let shown = self.shown.get(&n)?;
        let from = match shown {
            Some(d) => *d,
            None => self.shown.range(..n).rev().find_map(|(_, d)| *d)?,
        };
        let until = self.shown.range(n + 1..).find_map(|(_, d)| *d);
        Some(from <= t + slack && until.is_none_or(|u| u > t - slack))
    }

    /// Whether frame `n` was never drawn: the log has an `m` line or a gap for it.
    fn never_drawn(&self, n: i64) -> bool {
        self.misses.contains(&n) && !self.shown.contains_key(&n)
    }

    /// How many frame times later than their numbers say the frames shown around the run of
    /// misses `run` came, by the log's display times: the time from the last frame before the
    /// run that has one to the first after it that has one, less their numbers' distance,
    /// rounded up. A frame shown late holds the one before it on screen that much longer,
    /// however many numbers the testbed then passes over to catch up. Those frames are looked
    /// for only `within` the frames up to the runs on either side; 0 where none has a display
    /// time.
    fn late_frames(&self, run: (i64, i64), within: (i64, i64)) -> f64 {
        let (first, last) = run;
        let before = self
            .shown
            .range(within.0..first)
            .rev()
            .find_map(|(&n, d)| Some((n, (*d)?)));
        let after = self
            .shown
            .range(last + 1..=within.1)
            .find_map(|(&n, d)| Some((n, (*d)?)));
        let (Some((p, dp)), Some((q, dq))) = (before, after) else {
            return 0.0;
        };
        // A tenth of a frame of slack, for panels a little off their reported rate.
        let frames = ((dq - dp) * f64::from(self.fps.max(1)) - 0.1).ceil();
        (frames - (q - p) as f64).max(0.0)
    }

    /// The clip time, in seconds, at QPC time 0 by the video: the 5th percentile of
    /// `pts_i − display(N_i)` over the readable frames after the held frame 0 whose display the
    /// log knows, so the freshest frames set it (README §6.3 `av.offset`, with logs). `pts` are
    /// in `timescale` ticks.
    pub fn clip_clock(&self, numbers: &[Option<i64>], pts: &[i64], timescale: u64) -> Option<f64> {
        let mut gaps: Vec<f64> = numbers
            .iter()
            .zip(pts)
            .skip(1)
            .filter_map(|(n, &t)| Some(t as f64 / timescale as f64 - self.display_s((*n)?)?))
            .collect();
        if gaps.is_empty() {
            return None;
        }
        gaps.sort_by(f64::total_cmp);
        Some(gaps[((gaps.len() - 1) as f64 * 0.05).floor() as usize])
    }

    /// The log's word on frames `first..=last`: the span a clip shows, which the caller carries
    /// on over any misses that excused its steps (a stall still under way at the clip's end).
    pub fn stability(&self, first: i64, last: i64) -> Stability {
        if last < first {
            return Stability {
                frames: 0,
                missed: 0,
                unlogged: 0,
            };
        }
        let frames = u64::try_from(last - first + 1).unwrap_or(u64::MAX);
        let missed = self.misses.range(first..=last).count() as u64;
        let recorded = self.shown.range(first..=last).count() as u64;
        let missed_undrawn = self
            .misses
            .range(first..=last)
            .filter(|n| !self.shown.contains_key(n))
            .count() as u64;
        Stability {
            frames,
            missed,
            unlogged: frames.saturating_sub(recorded + missed_undrawn),
        }
    }
}

/// Where a clip's output frames fall on the testbed's clock: each one's QPC time in seconds, by
/// the clip clock ([`Source::clip_clock`]), and how far a capture's picture may be from it.
pub(crate) struct Screen {
    /// Each output frame's QPC time, in seconds.
    pub times: Vec<f64>,
    /// One vblank, in seconds: a capture takes what the screen showed within about that.
    pub slack: f64,
}

/// What the testbed's misses can excuse among a clip's steps (README §6.3). An event step (a
/// repeat, a skip or a judder step) is the testbed's only if it touches a run of missed frames
/// (a step from N to N′ touches the misses in N + 1 ..= N′ + 1), and never if it goes back in
/// time: the testbed's numbers only go forward.
///
/// Where the log's display times say what the screen showed, that decides: the step is the
/// testbed's when N was on screen at the step's first output frame and N′ at its second, each
/// within a vblank, so the capture showed what the screen did. A capture that froze, dropped a
/// frame or took a stale one beside a testbed miss shows a frame the screen had left or not yet
/// shown, and stays the capture's.
///
/// Where they can't (no display times near N or N′, or no clip clock), the misses are bounded
/// by what they can do to the screen. A run of m missed frames held the screen m frame times
/// longer, and e more where the display times there show the frames around it came e frame
/// times later than their numbers say ([`Source::late_frames`]). So it can account for at most
/// ⌈(m + e + 1)·fps / F_s⌉ + 1 of the steps that touch it, each off its nominal size by at most
/// m + e + 1 testbed frames; a step that touches several runs (lateness carried through frames
/// that kept their cadence, then caught up), by the sum of theirs, using up a step of the first
/// that has one left. A step longer than a tick that touches a run only through N′ + 1, the step
/// into the frame it held, is never the run's: a hold can only shorten it. The runs that excused
/// a step are kept, so `source.stability` counts them.
pub(crate) struct Excuses<'a> {
    source: &'a Source,
    /// The output frames on the testbed's clock, when the log can place them.
    screen: Option<Screen>,
    /// Each run of missed frames.
    runs: Vec<Run>,
    /// Testbed frames per output frame.
    nominal: f64,
    /// The last missed frame of the runs that excused a step.
    used_until: Option<i64>,
}

/// A run of missed frames, and what it may still excuse.
#[derive(Debug, Clone, Copy)]
struct Run {
    first: i64,
    last: i64,
    /// How far off its nominal size a step it explains may be, in testbed frames.
    bound: f64,
    /// How many more steps it may excuse.
    left: u64,
}

impl<'a> Excuses<'a> {
    /// The excuses `source`'s misses give a clip at `fps` output frames per second, whose output
    /// frames `screen` places on the testbed's clock when the log can.
    pub fn new(source: &'a Source, fps: u32, screen: Option<Screen>) -> Excuses<'a> {
        let per_frame = f64::from(fps.max(1)) / f64::from(source.fps.max(1));
        let found = source.miss_runs();
        let runs = found
            .iter()
            .enumerate()
            .map(|(i, &(first, last))| {
                // Up to the runs on either side: another miss would change what the display
                // times say.
                let within = (
                    i.checked_sub(1).map_or(i64::MIN, |j| found[j].1 + 1),
                    found.get(i + 1).map_or(i64::MAX, |r| r.0 - 1),
                );
                let held = (last - first + 1) as f64 + source.late_frames((first, last), within);
                Run {
                    first,
                    last,
                    bound: held + 1.0,
                    left: ((held + 1.0) * per_frame).ceil() as u64 + 1,
                }
            })
            .collect();
        Excuses {
            source,
            screen,
            runs,
            nominal: 1.0 / per_frame,
            used_until: None,
        }
    }

    /// Whether the testbed's misses explain the step from testbed frame `from`, at output frame
    /// `at - 1`, to `to`, at output frame `at`.
    pub fn excuse(&mut self, at: usize, from: i64, to: i64) -> bool {
        if to < from {
            return false;
        }
        let first = self.runs.partition_point(|r| r.last < from + 1);
        let touched = self.runs[first..]
            .iter()
            .take_while(|r| r.first <= to + 1)
            .count();
        if touched == 0 {
            return false;
        }
        let last = self.runs[first..first + touched]
            .iter()
            .map(|r| r.last)
            .max();
        match self.on_screen(at, from, to) {
            Some(true) => {
                self.used_until = self.used_until.max(last);
                return true;
            }
            Some(false) => return false,
            None => {}
        }
        let runs = &mut self.runs[first..first + touched];
        let step = (to - from) as f64;
        let off = (step - self.nominal).abs();
        // The step into the frame a run held: a hold can only shorten it.
        if runs.len() == 1 && runs[0].first == to + 1 && step > self.nominal.ceil() {
            return false;
        }
        let bound: f64 = runs.iter().map(|r| r.bound).sum();
        if off > bound {
            return false;
        }
        let Some(run) = runs.iter_mut().find(|r| r.left > 0) else {
            return false;
        };
        run.left -= 1;
        self.used_until = self.used_until.max(last);
        true
    }

    /// Whether the log puts `from` on screen at output frame `at - 1` and `to` at `at`; `None`
    /// where it can't say.
    fn on_screen(&self, at: usize, from: i64, to: i64) -> Option<bool> {
        let screen = self.screen.as_ref()?;
        let (t0, t1) = (
            *screen.times.get(at.checked_sub(1)?)?,
            *screen.times.get(at)?,
        );
        // A frame never drawn was never shown: the picture isn't the testbed's.
        if self.source.never_drawn(from) || self.source.never_drawn(to) {
            return Some(false);
        }
        let shown = self.source.on_screen(from, t0, screen.slack)?
            && self.source.on_screen(to, t1, screen.slack)?;
        Some(shown)
    }

    /// The last missed frame that excused a step, if any did.
    pub fn used_until(&self) -> Option<i64> {
        self.used_until
    }
}

/// `kairo-tone`'s beeps: each beep's number and when it played, in seconds of QPC time.
pub(crate) fn voices(log: &ToneLog) -> Vec<(i64, f64)> {
    log.voices()
        .filter_map(|v| Some((i64::try_from(v.beep).ok()?, seconds(v.played_hns?))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(lines: &str, fps: u32, pace: &str) -> TestbedLog {
        let text = format!(
            "# kairo-testbed 1\nh pattern=1 run=9 mode=borderless pace={pace} fps={fps} F={fps} size=1920x1080 refresh_mhz=180000 hdr=0 sdr_white_nits=- max_nits=- start_hns=0\n{lines}"
        );
        TestbedLog::parse(&text).unwrap()
    }

    #[test]
    fn misses_are_m_lines_gaps_and_unknown_frames_before_a_late_one() {
        let s = Source::new(&log(
            "f 0 0 1 0\nf 1 1 2 -\nf 2 2 3 4\nm 2 vblank\nf 3 3 4 -\nf 4 4 5 6\n\
             f 7 7 8 9\nm 8 vblank\nf 9 9 10 11\nb 1 180 10 12\n",
            180,
            "vsync",
        ));
        assert_eq!(s.every(), 1);
        assert!(!s.is_miss(0));
        assert!(s.is_miss(1), "unknown, before the late frame 2");
        assert!(s.is_miss(2), "an m line: shown late");
        assert!(!s.is_miss(3), "unknown, before frame 4 on time");
        assert!(s.is_miss(5) && s.is_miss(6), "never drawn");
        assert!(s.is_miss(8), "an m line for a frame never drawn");
        assert_eq!(s.display_s(2), Some(4e-7));
        assert_eq!(s.display_s(1), None);
        let st = s.stability(0, 9);
        assert_eq!((st.frames, st.missed, st.unlogged), (10, 5, 0));
        assert!((st.share() - 0.5).abs() < 1e-12);
        // Frames past the log's end are unlogged.
        assert_eq!(s.stability(5, 12).unlogged, 3);
        assert_eq!(s.beeps, [(1, 180, Some(1.2e-6))]);
    }

    /// A 180 fps vsync log of frames 1,000–2,999, all shown, with these frames missed.
    fn missing(missed: impl Fn(i64) -> bool) -> Source {
        let lines: String = (1_000..3_000)
            .map(|n| {
                if missed(n) {
                    format!("m {n} vblank\n")
                } else {
                    format!("f {n} {n} {n} {n}\n")
                }
            })
            .collect();
        Source::new(&log(&lines, 180, "vsync"))
    }

    #[test]
    fn without_display_times_a_miss_excuses_only_what_it_can_do_to_the_screen() {
        // No display time places these frames (each logged at 100 ns), so the misses are
        // bounded by their size. At 60 fps a tick is 3 testbed frames. One miss holds a frame two vblanks, too short
        // for a capture to show it twice, so a freeze at it stays the capture's; it can account
        // for the short and long steps around it, two of them, by a frame each.
        let one = missing(|n| n == 1_121);
        let mut x = Excuses::new(&one, 60, None);
        assert!(!x.excuse(1, 1_120, 1_120), "a repeat");
        assert!(!x.excuse(1, 1_120, 1_138), "the freeze's jump");
        assert!(x.excuse(1, 1_118, 1_120));
        assert!(x.excuse(1, 1_120, 1_124));
        assert!(!x.excuse(1, 1_120, 1_124), "used up");
        assert_eq!(x.used_until(), Some(1_121));
        // A stall of 30 frames held the frame half a second: its repeats at 60 fps and the jump
        // after them are the testbed's, up to ⌈31/3⌉ + 1 = 12 steps.
        let stall = missing(|n| (1_121..=1_150).contains(&n));
        let mut x = Excuses::new(&stall, 60, None);
        assert!((0..11).all(|_| x.excuse(1, 1_120, 1_120)));
        assert!(x.excuse(1, 1_120, 1_151));
        assert!(!x.excuse(1, 1_120, 1_120));
        // Never a step back in time, nor the step that returns past a splice.
        let one = missing(|n| n == 1_600);
        let mut x = Excuses::new(&one, 60, None);
        assert!(!x.excuse(1, 1_897, 1_450));
        assert!(!x.excuse(1, 1_450, 1_903));
        assert_eq!(x.used_until(), None);
        // At the testbed's own rate a miss shows as a repeat and the skip after it.
        let own = missing(|n| n == 1_500);
        let mut x = Excuses::new(&own, 180, None);
        assert!(x.excuse(1, 1_499, 1_499) && x.excuse(1, 1_499, 1_501));
    }

    /// A vsync log at `fps` on 180 Hz of frames `frames`, each `Some(vblank)` it showed at or
    /// `None` never drawn, with an m line for each never drawn and each in `late`.
    fn shown_at(fps: u32, frames: &[(i64, Option<i64>)], late: &[i64]) -> Source {
        let lines: String = frames
            .iter()
            .map(|&(n, v)| match v {
                Some(v) => {
                    let t = (v as f64 * 10_000_000.0 / 180.0).round() as i64;
                    let m = if late.contains(&n) {
                        format!("m {n} vblank\n")
                    } else {
                        String::new()
                    };
                    format!("f {n} {t} {t} {t}\n{m}")
                }
                None => format!("m {n} vblank\n"),
            })
            .collect();
        Source::new(&log(&lines, fps, "vsync"))
    }

    /// A capture's output frames at 60 fps, sampled 0.1 vblank after the vblanks `vblanks`, on
    /// the testbed's clock.
    fn screen(vblanks: &[i64]) -> Screen {
        Screen {
            times: vblanks.iter().map(|&v| (v as f64 + 0.1) / 180.0).collect(),
            slack: 1.0 / 180.0,
        }
    }

    #[test]
    fn display_times_excuse_what_the_screen_showed_and_nothing_else() {
        // Exclusive at 180: frame 3600 showed 6 vblanks late, 3601 after it, and the testbed
        // passed over 3602–3607 to catch up. 3599 stayed on screen 7 vblanks, so a 60 fps
        // capture sampling every third vblank shows it twice, then 3600, then 3609.
        let frames: Vec<(i64, Option<i64>)> = (3_580..3_620)
            .map(|n| match n {
                3_600 => (n, Some(3_606)),
                3_601 => (n, Some(3_607)),
                3_602..=3_607 => (n, None),
                _ => (n, Some(n)),
            })
            .collect();
        let s = shown_at(180, &frames, &[3_600]);
        let vblanks = [3_591, 3_594, 3_597, 3_600, 3_603, 3_606, 3_609, 3_612];
        let mut x = Excuses::new(&s, 60, Some(screen(&vblanks)));
        for (at, from, to) in [
            (3, 3_597, 3_599),
            (4, 3_599, 3_599),
            (5, 3_599, 3_600),
            (6, 3_600, 3_609),
        ] {
            assert!(x.excuse(at, from, to), "{from} → {to}");
        }
        assert_eq!(x.used_until(), Some(3_607));
        // A capture that lost two output frames before the hold: its picture after 3591 is
        // 3599, which the screen showed only two ticks later.
        assert!(!x.excuse(1, 3_591, 3_599));
        // One that froze on 3600 after it had gone, and one that showed 3601 before the screen
        // did by more than a vblank.
        assert!(!x.excuse(6, 3_600, 3_600));
        assert!(!x.excuse(4, 3_599, 3_601));
        // A picture of a frame never drawn isn't the testbed's.
        assert!(!x.excuse(6, 3_600, 3_603));
        // Steps that touch no miss are never set aside, however well they match the screen.
        assert!(!x.excuse(1, 3_591, 3_594));
    }

    #[test]
    fn lateness_carried_through_a_gpu_bound_stretch_is_the_testbeds() {
        // Borderless 180: the GPU is 3 vblanks slow for frames 3600–3611. 3600 shows 3 late
        // (its m line); 3604 and 3608, drawn for the group after their late predecessor, keep
        // cadence with it and get no m line; the numbers between are passed over; 3612 shows on
        // time. The screen is behind for the whole stretch, which no single run's size says.
        let frames: Vec<(i64, Option<i64>)> = (3_580..3_630)
            .map(|n| match n {
                3_600 => (n, Some(3_603)),
                3_604 => (n, Some(3_607)),
                3_608 => (n, Some(3_611)),
                3_601..=3_611 => (n, None),
                _ => (n, Some(n)),
            })
            .collect();
        let s = shown_at(180, &frames, &[3_600]);
        // A 60 fps capture reads 3597, 3599, 3600, 3600, 3604, 3612: every step is the screen's.
        let vblanks = [3_597, 3_600, 3_603, 3_606, 3_609, 3_612];
        let mut x = Excuses::new(&s, 60, Some(screen(&vblanks)));
        for (at, from, to) in [
            (1, 3_597, 3_599),
            (2, 3_599, 3_600),
            (3, 3_600, 3_600),
            (4, 3_600, 3_604),
            (5, 3_604, 3_612),
        ] {
            assert!(x.excuse(at, from, to), "{from} → {to}");
        }
        // 3608 wasn't on screen yet at the fifth output frame.
        assert!(!x.excuse(4, 3_604, 3_608));
        // Without a clip clock the runs bound it, a step touching several by the sum of theirs:
        // 3604 → 3612 is 5 frames off a tick, more than either run of 3 allows alone.
        let mut x = Excuses::new(&s, 60, None);
        assert!(x.excuse(1, 3_604, 3_612));
    }

    #[test]
    fn without_display_times_the_step_into_a_held_frame_is_never_a_runs() {
        // A run touched only through N′ + 1: the step into the frame it held. A hold can only
        // shorten that step, so a long one is the capture's; a short one may be the hold's.
        let s = missing(|n| (1_100..=1_103).contains(&n));
        let mut x = Excuses::new(&s, 60, None);
        assert!(!x.excuse(1, 1_090, 1_099), "a skip into the held frame");
        assert!(x.excuse(1, 1_097, 1_099), "a short step into it");
        assert!(x.excuse(1, 1_099, 1_099), "a repeat of it");
    }

    #[test]
    fn the_clip_clock_follows_the_freshest_frames() {
        // Frames shown 100 ms apart at QPC 10 s + n/10; the clip shows frame n at n/10 s, a
        // few of them 30 ms late (stale), and frame 0 held.
        let lines: String = (0..40)
            .map(|n| format!("f {n} 0 0 {}\n", 100_000_000 + n * 1_000_000))
            .collect();
        let s = Source::new(&log(&lines, 10, "timer"));
        let numbers: Vec<Option<i64>> = (0..40).map(Some).collect();
        let pts: Vec<i64> = (0..40)
            .map(|n: i64| n * 1_536 + if n % 7 == 3 { 461 } else { 0 })
            .collect();
        let clock = s.clip_clock(&numbers, &pts, 15_360).unwrap();
        assert!((clock + 10.0).abs() < 1e-9, "{clock}");
        assert_eq!(s.clip_clock(&[Some(0)], &[0], 15_360), None);
    }

    #[test]
    fn without_frame_statistics_no_miss_is_read_from_unknown_display_times() {
        let lines =
            "e no_frame_statistics\nf 0 0 1 -\nf 1 1 2 -\nf 3 3 4 -\nm 4 vblank\nf 5 5 6 -\n";
        let s = Source::new(&log(lines, 180, "vsync"));
        assert!(!s.timed);
        assert!(!s.is_miss(0) && !s.is_miss(1) && !s.is_miss(5));
        // Gaps and m lines still count.
        assert!(s.is_miss(2) && s.is_miss(4));
        // A log with no display time at all says the same, event or not.
        let s = Source::new(&log("f 0 0 1 -\nf 1 1 2 -\n", 180, "vsync"));
        assert!(!s.timed && !s.is_miss(0) && !s.is_miss(1));
        let s = Source::new(&log(
            "f 0 0 1 0\nf 1 1 2 -\nf 2 2 3 4\nm 2 vblank\n",
            180,
            "vsync",
        ));
        assert!(s.timed && s.is_miss(1));
        // Statistics that came only after the event read the same: the `-` before a late frame
        // is a miss once frames have display times.
        let s = Source::new(&log(
            "e no_frame_statistics\nf 0 0 1 0\nf 1 1 2 -\nf 2 2 3 4\nm 2 vblank\n",
            180,
            "vsync",
        ));
        assert!(s.timed && s.is_miss(1));
    }

    #[test]
    fn timer_frames_passed_over_are_no_misses() {
        let s = Source::new(&log(
            "f 0 0 1 0\nf 1 1 2 -\nf 2 2 3 4\nm 2 slot\n",
            360,
            "timer",
        ));
        assert!(s.timer);
        assert!(!s.is_miss(1));
        assert!(s.is_miss(2), "its slot missed");
        assert_eq!(s.every(), 1);
    }
}
