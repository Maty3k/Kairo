// SPDX-License-Identifier: GPL-3.0-or-later
//! What the testbed's log says about the run a clip shows (docs/research/16-verify-testbed/
//! README.md §4.9, §6.3): the rates the barcode line would otherwise estimate, the run tag, the
//! HDR levels, each frame's display time, the beeps' played times and the frames the testbed
//! itself missed, so that a capture isn't blamed for them.
//!
//! A frame counts as the testbed's miss when the log has an `m` line for it, when it was never
//! drawn (a gap between drawn frames' numbers), or when it was drawn but never shown while the
//! testbed ran at or below its display's refresh rate. Above the refresh rate (timer pacing),
//! frames the display passes over are the design, not misses. A run whose log has no display
//! times (`e no_frame_statistics`: DXGI gave none) has `-` for every frame, which means unknown
//! there, so no miss is read from it.

use std::collections::{BTreeMap, BTreeSet};

use kairo_core::testbed_log::{TestbedLog, TestbedRecord, ToneLog};

/// A testbed log, read for the checks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    /// The frames per second the testbed drew (F_s).
    pub fps: u32,
    /// The rate that times its flash (F).
    pub nominal_fps: u32,
    /// Its display's refresh rate, in Hz.
    pub refresh_hz: f64,
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
        // Drawn but never shown, at or below the refresh rate, where the log has display times.
        if timed && f64::from(h.fps) <= refresh_hz * 1.005 {
            misses.extend(shown.iter().filter(|(_, d)| d.is_none()).map(|(n, _)| *n));
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

    /// Whether a step of the clip from testbed frame `from` to `to` touches a testbed miss: one
    /// of the frames it passed over, or the frame after the one it landed on (a frame shown late
    /// keeps its predecessor on screen, so a capture repeats or lands short).
    pub fn affects(&self, from: i64, to: i64) -> bool {
        let (low, high) = if from <= to { (from, to) } else { (to, from) };
        self.misses.range(low + 1..=high + 1).next().is_some()
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

    /// The log's word on frames `first..=last`, the span a clip shows.
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
    fn misses_are_m_lines_gaps_and_frames_never_shown() {
        let s = Source::new(&log(
            "f 0 0 1 0\nf 1 1 2 -\nf 2 2 3 4\nf 5 5 6 7\nm 6 vblank\nf 7 7 8 9\nb 1 180 10 12\n",
            180,
            "vsync",
        ));
        assert_eq!(s.every(), 1);
        assert!(!s.is_miss(0));
        assert!(s.is_miss(1), "never shown");
        assert!(s.is_miss(3) && s.is_miss(4), "never drawn");
        assert!(s.is_miss(6), "an m line");
        assert_eq!(s.display_s(2), Some(4e-7));
        assert_eq!(s.display_s(1), None);
        // A step from 2 to 5 passed over 3 and 4; a repeat of 0 sits before the miss at 1.
        assert!(s.affects(2, 5));
        assert!(s.affects(0, 0));
        assert!(!s.affects(7, 7));
        let st = s.stability(0, 7);
        assert_eq!((st.frames, st.missed, st.unlogged), (8, 4, 0));
        assert!((st.share() - 0.5).abs() < 1e-12);
        // Frames past the log's end are unlogged.
        assert_eq!(s.stability(5, 12).unlogged, 5);
        assert_eq!(s.beeps, [(1, 180, Some(1.2e-6))]);
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
        let s = Source::new(&log("f 0 0 1 0\nf 1 1 2 -\n", 180, "vsync"));
        assert!(s.timed && s.is_miss(1));
    }

    #[test]
    fn above_the_refresh_frames_passed_over_are_no_misses() {
        let s = Source::new(&log("f 0 0 1 0\nf 1 1 2 -\nf 2 2 3 4\n", 360, "timer"));
        assert!(!s.is_miss(1));
        assert_eq!(s.every(), 1);
    }
}
