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
//! yet, a stretch they never reported, or none at all after `e no_frame_statistics`), and with
//! timer pacing it is a frame the display passed over by design.
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
    /// Whether the log says when frames showed: not after `e no_frame_statistics` or
    /// `e no_vblank_timing`, nor when no frame has a display time.
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
        let untimed = log.records.iter().any(|r| {
            matches!(r, TestbedRecord::Event(e)
                if matches!(e.name(), "no_frame_statistics" | "no_vblank_timing"))
        });
        let timed = !untimed && shown.values().any(Option::is_some);
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

/// What the testbed's misses can excuse among a clip's steps (README §6.3), bounded by what they
/// can do to the screen. A run of m missed frames in a row held the frame before it on screen
/// for m more frame times, so it can account for at most ⌈(m + 1)·fps / F_s⌉ + 1 of the steps
/// that touch it (a step from N to N′ touches the misses in N + 1 ..= N′ + 1), each off its
/// nominal size by at most m + 1 testbed frames. The testbed's numbers never go back, so no step
/// back in time is its. The runs that excused a step are kept, so `source.stability` counts them.
pub(crate) struct Excuses {
    /// Each run of missed frames: first, last, and how many more steps it may excuse.
    runs: Vec<(i64, i64, u64)>,
    /// Testbed frames per output frame.
    nominal: f64,
    /// The last missed frame of the runs that excused a step.
    used_until: Option<i64>,
}

impl Excuses {
    /// The excuses `source`'s misses give a clip at `fps` output frames per second.
    pub fn new(source: &Source, fps: u32) -> Excuses {
        let per_frame = f64::from(fps.max(1)) / f64::from(source.fps.max(1));
        let runs = source
            .miss_runs()
            .into_iter()
            .map(|(a, b)| {
                let m = (b - a + 1) as f64;
                (a, b, ((m + 1.0) * per_frame).ceil() as u64 + 1)
            })
            .collect();
        Excuses {
            runs,
            nominal: 1.0 / per_frame,
            used_until: None,
        }
    }

    /// Whether the testbed's misses explain the step from testbed frame `from` to `to`; uses up
    /// one step of the run that does.
    pub fn excuse(&mut self, from: i64, to: i64) -> bool {
        if to < from {
            return false;
        }
        let off = ((to - from) as f64 - self.nominal).abs();
        let first = self.runs.partition_point(|r| r.1 < from + 1);
        for run in &mut self.runs[first..] {
            if run.0 > to + 1 {
                break;
            }
            if run.2 > 0 && off <= (run.1 - run.0 + 2) as f64 {
                run.2 -= 1;
                self.used_until = Some(self.used_until.map_or(run.1, |u| u.max(run.1)));
                return true;
            }
        }
        false
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
    fn a_miss_excuses_only_what_it_can_do_to_the_screen() {
        // At 60 fps a tick is 3 testbed frames. One miss holds a frame two vblanks, too short
        // for a capture to show it twice, so a freeze at it stays the capture's; it can account
        // for the short and long steps around it, two of them, by a frame each.
        let one = missing(|n| n == 1_121);
        let mut x = Excuses::new(&one, 60);
        assert!(!x.excuse(1_120, 1_120), "a repeat");
        assert!(!x.excuse(1_120, 1_138), "the freeze's jump");
        assert!(x.excuse(1_118, 1_120));
        assert!(x.excuse(1_120, 1_124));
        assert!(!x.excuse(1_120, 1_124), "used up");
        assert_eq!(x.used_until(), Some(1_121));
        // A stall of 30 frames held the frame half a second: its repeats at 60 fps and the jump
        // after them are the testbed's, up to ⌈31/3⌉ + 1 = 12 steps.
        let stall = missing(|n| (1_121..=1_150).contains(&n));
        let mut x = Excuses::new(&stall, 60);
        assert!((0..11).all(|_| x.excuse(1_120, 1_120)));
        assert!(x.excuse(1_120, 1_151));
        assert!(!x.excuse(1_120, 1_120));
        // Never a step back in time, nor the step that returns past a splice.
        let one = missing(|n| n == 1_600);
        let mut x = Excuses::new(&one, 60);
        assert!(!x.excuse(1_897, 1_450));
        assert!(!x.excuse(1_450, 1_903));
        assert_eq!(x.used_until(), None);
        // At the testbed's own rate a miss shows as a repeat and the skip after it.
        let mut x = Excuses::new(&missing(|n| n == 1_500), 180);
        assert!(x.excuse(1_499, 1_499) && x.excuse(1_499, 1_501));
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
