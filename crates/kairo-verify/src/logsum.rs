// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify log FILE`: what a testbed or tone log says about the run itself, with
//! research 16 §4.12's acceptance limits (docs/research/16-verify-testbed/README.md §4.9, §4.12,
//! §5). It judges the source, not a clip: missed vblanks, display times that advance at the
//! refresh, beeps that play on their targets, and the exclusive row's focus events.
//!
//! A log names no device or path, so the summary doesn't either.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use kairo_core::testbed_log::{Mode, Pace, TestbedLog, TestbedRecord, ToneLog, ToneRecord};
use serde::Serialize;
use serde_json::{Value, json};

/// The most frames a testbed may miss, as a share (README §4.12; §6.5's `source.stability`).
pub const MISSED_SHARE_MAX: f64 = 0.001;
/// How far a beep's first sample may play from its target, in ms (README §4.12, an estimate).
pub const BEEP_ERROR_MAX_MS: f64 = 1.0;
/// How far the median display interval may be from the frame period, as a share.
pub const DISPLAY_RATE_TOLERANCE: f64 = 0.01;

/// A few order statistics of some values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Spread {
    /// How many.
    pub n: usize,
    /// The smallest.
    pub min: f64,
    /// The median.
    pub median: f64,
    /// The 99th percentile (nearest rank).
    pub p99: f64,
    /// The largest.
    pub max: f64,
}

impl Spread {
    fn of(mut values: Vec<f64>) -> Option<Spread> {
        if values.is_empty() {
            return None;
        }
        values.sort_by(f64::total_cmp);
        let rank = |q: f64| {
            let r = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
            values[r - 1]
        };
        Some(Spread {
            n: values.len(),
            min: values[0],
            median: rank(0.5),
            p99: rank(0.99),
            max: values[values.len() - 1],
        })
    }
}

/// One acceptance rule: `None` when it doesn't apply to this run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Criterion {
    /// What it checks.
    pub name: &'static str,
    /// Whether it held.
    pub pass: Option<bool>,
    /// What was measured, or why it doesn't apply.
    pub detail: String,
}

/// A log's summary and verdict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// `testbed` or `tone`.
    pub kind: &'static str,
    /// The figures.
    pub figures: Value,
    /// README §4.12's rules.
    pub criteria: Vec<Criterion>,
    /// Whether the log ended inside a line (dropped by the parser).
    pub torn_tail: bool,
}

impl Summary {
    /// Whether every rule that applies held.
    pub fn passed(&self) -> bool {
        self.criteria.iter().all(|c| c.pass != Some(false))
    }

    /// One line per rule, after a headline.
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "{} log: {}\n",
            self.kind,
            if self.passed() { "PASS" } else { "FAIL" }
        );
        for c in &self.criteria {
            let mark = match c.pass {
                Some(true) => "ok  ",
                Some(false) => "FAIL",
                None => "n/a ",
            };
            let _ = writeln!(text, "  {mark} {}: {}", c.name, c.detail);
        }
        if self.torn_tail {
            text.push_str("  (the log's last line was cut off and left out)\n");
        }
        text
    }
}

fn beep_errors(pairs: impl Iterator<Item = (i64, Option<i64>)>) -> (usize, usize, Vec<f64>) {
    let (mut count, mut played, mut errors) = (0, 0, Vec::new());
    for (target, play) in pairs {
        count += 1;
        if let Some(play) = play {
            played += 1;
            errors.push((play - target) as f64 / 10_000.0);
        }
    }
    (count, played, errors)
}

fn beep_criterion(count: usize, played: usize, errors: &[f64]) -> Criterion {
    let spread = Spread::of(errors.to_vec());
    let worst = errors.iter().fold(0.0_f64, |m, e| m.max(e.abs()));
    Criterion {
        name: "beeps play on target",
        pass: (count > 0).then_some(played == count && worst <= BEEP_ERROR_MAX_MS),
        detail: match spread {
            _ if count == 0 => "no beeps".to_owned(),
            Some(s) => format!(
                "{played} of {count} played; played − target {:+.3} to {:+.3} ms, median {:+.3} (limit ±{BEEP_ERROR_MAX_MS} ms)",
                s.min, s.max, s.median
            ),
            None => format!("none of {count} played"),
        },
    }
}

/// A testbed log's summary (README §4.12).
pub fn testbed(log: &TestbedLog) -> Summary {
    let h = &log.header;
    let refresh_hz = h.refresh_mhz as f64 / 1_000.0;
    let period_ms = 1_000.0 / refresh_hz;
    let vblanks_per_frame = match h.pace {
        Pace::Vsync => (refresh_hz / f64::from(h.fps)).round().max(1.0),
        Pace::Timer => 1.0,
    };
    let frame_ms = match h.pace {
        Pace::Vsync => vblanks_per_frame * period_ms,
        Pace::Timer => 1_000.0 / f64::from(h.fps),
    };
    let frames: Vec<_> = log.frames().collect();
    let first = frames.first().map(|f| f.frame);
    let last = frames.last().map(|f| f.frame);
    let span = first.zip(last).map_or(0, |(a, b)| b - a + 1);
    let mut missed: BTreeSet<u64> = BTreeSet::new();
    let mut misses = 0;
    let mut events = Vec::new();
    for record in &log.records {
        match record {
            TestbedRecord::Miss(frame, _) => {
                misses += 1;
                missed.insert(*frame);
            }
            TestbedRecord::Event(e) => {
                let mut line = e.name().to_owned();
                for (k, v) in e.fields() {
                    let _ = write!(line, " {k}={v}");
                }
                events.push(line);
            }
            _ => {}
        }
    }
    let unknown: Vec<u64> = frames
        .iter()
        .filter(|f| f.display_hns.is_none())
        .map(|f| f.frame)
        .collect();
    // Without frame statistics every display time is `-`, which means unknown, not missed.
    let untimed = events
        .iter()
        .any(|e| e == "no_frame_statistics" || e == "no_vblank_timing");
    let timed = !untimed && unknown.len() < frames.len();
    if timed {
        missed.extend(&unknown);
    }
    // Display intervals per frame step, between consecutive frames with display times.
    let shown: Vec<(u64, i64)> = frames
        .iter()
        .filter_map(|f| Some((f.frame, f.display_hns?)))
        .collect();
    let intervals: Vec<f64> = shown
        .windows(2)
        .map(|w| (w[1].1 - w[0].1) as f64 / 10_000.0 / (w[1].0 - w[0].0) as f64)
        .collect();
    let spread = Spread::of(intervals);
    let (count, played, errors) = beep_errors(log.beeps().map(|b| (b.target_hns, b.played_hns)));
    let fullscreen = matches!(h.mode, Mode::Borderless | Mode::Exclusive);
    let at_or_below_refresh = f64::from(h.fps) <= refresh_hz * 1.005;
    let share = if span > 0 {
        missed.len() as f64 / span as f64
    } else {
        0.0
    };
    let mut criteria = vec![Criterion {
        name: "missed vblanks",
        pass: (fullscreen && at_or_below_refresh && span > 0).then_some(share <= MISSED_SHARE_MAX),
        detail: if span == 0 {
            "no frames".to_owned()
        } else if !(fullscreen && at_or_below_refresh) {
            format!(
                "{} of {span} frames missed or unknown ({:.3}%); judged only borderless or exclusive at or below the refresh",
                missed.len(),
                share * 100.0
            )
        } else if !timed {
            format!(
                "{} of {span} frames missed ({:.3}%; limit {:.1}%), by m lines only: the log has no display times",
                missed.len(),
                share * 100.0,
                MISSED_SHARE_MAX * 100.0
            )
        } else {
            format!(
                "{} of {span} frames missed or never shown ({:.3}%; limit {:.1}%)",
                missed.len(),
                share * 100.0,
                MISSED_SHARE_MAX * 100.0
            )
        },
    }];
    let advancing =
        spread.map(|s| (s.median - frame_ms).abs() <= DISPLAY_RATE_TOLERANCE * frame_ms);
    let known = if frames.is_empty() {
        0.0
    } else {
        shown.len() as f64 / frames.len() as f64
    };
    criteria.push(Criterion {
        name: "display times advance at the refresh",
        pass: at_or_below_refresh
            .then(|| advancing == Some(true) && known >= 1.0 - MISSED_SHARE_MAX),
        detail: match spread {
            Some(s) => format!(
                "{:.1}% of frames have a display time; interval per frame median {:.4} ms, p99 {:.4}, max {:.4} (the frame period is {:.4} ms)",
                known * 100.0,
                s.median,
                s.p99,
                s.max,
                frame_ms
            ),
            None => "no display times".to_owned(),
        },
    });
    criteria.push(beep_criterion(count, played, &errors));
    if h.mode == Mode::Exclusive {
        for (event, name) in [
            ("activate", "the window took the foreground"),
            ("fse", "exclusive fullscreen held"),
        ] {
            let ok = events.iter().any(|e| e == &format!("{event} result=ok"));
            criteria.push(Criterion {
                name,
                pass: Some(ok),
                detail: format!(
                    "e {event} result={}",
                    if ok { "ok" } else { "failed or missing" }
                ),
            });
        }
    }
    Summary {
        kind: "testbed",
        figures: json!({
            "mode": h.mode.to_string(), "pace": h.pace.to_string(), "fps": h.fps,
            "refresh_mhz": h.refresh_mhz, "size": [h.width, h.height], "hdr": h.hdr,
            "frames": frames.len(), "span": span, "never_drawn": span.saturating_sub(frames.len() as u64),
            "m_lines": misses, "unknown_display": unknown.len(), "missed_share": share,
            "display_interval_ms": spread, "frame_period_ms": frame_ms,
            "beeps": count, "beeps_played": played, "beep_error_ms": Spread::of(errors),
            "events": events,
        }),
        criteria,
        torn_tail: log.torn_tail(),
    }
}

/// A tone log's summary (README §5).
pub fn tone(log: &ToneLog) -> Summary {
    let (count, played, errors) = beep_errors(log.voices().map(|v| (v.target_hns, v.played_hns)));
    let events: Vec<String> = log
        .records
        .iter()
        .filter_map(|r| match r {
            ToneRecord::Event(e) => {
                let mut line = e.name().to_owned();
                for (k, v) in e.fields() {
                    let _ = write!(line, " {k}={v}");
                }
                Some(line)
            }
            ToneRecord::Voice(_) => None,
        })
        .collect();
    let h = &log.header;
    Summary {
        kind: "tone",
        figures: json!({
            "hz": h.hz, "offset_ms": h.offset_ms, "dbfs": h.dbfs,
            "beeps": count, "beeps_played": played, "beep_error_ms": Spread::of(errors.clone()),
            "events": events,
        }),
        criteria: vec![beep_criterion(count, played, &errors)],
        torn_tail: log.torn_tail(),
    }
}

/// The summary of a log's text: a testbed log or a tone log, told apart by their first line.
pub fn summarize(text: &str) -> Result<Summary, String> {
    if text.starts_with(kairo_core::testbed_log::TONE_LOG_HEADER) {
        ToneLog::parse(text)
            .map(|log| tone(&log))
            .map_err(|e| e.to_string())
    } else {
        TestbedLog::parse(text)
            .map(|log| testbed(&log))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A testbed log of `frames` frames at 180 fps on 180 Hz, borderless vsync, shown on
    /// cadence, with `change` applied to its records' text.
    fn testbed_text(
        frames: u64,
        mode: &str,
        events: &str,
        change: impl Fn(u64) -> Option<String>,
    ) -> String {
        let period = 55_556_i64;
        let mut text = format!(
            "# kairo-testbed 1\nh pattern=1 run=5 mode={mode} pace=vsync fps=180 F=180 size=1920x1080 refresh_mhz=180000 hdr=0 sdr_white_nits=- max_nits=- start_hns=10000000\n{events}"
        );
        for n in 0..frames {
            let t = 10_000_000 + period * n as i64;
            match change(n) {
                Some(line) => text.push_str(&line),
                None => {
                    let _ = writeln!(text, "f {n} {t} {} {t}", t - 30_000);
                }
            }
        }
        for m in 1..frames / 180 {
            let t = 10_000_000 + period * (180 * m) as i64;
            let _ = writeln!(text, "b {m} {} {t} {}", 180 * m, t + 3_000);
        }
        text.push_str("e end reason=seconds\n");
        text
    }

    #[test]
    fn a_clean_run_passes() {
        let s = summarize(&testbed_text(3_600, "borderless", "e ready\n", |_| None)).unwrap();
        assert!(s.passed(), "{}", s.to_text());
        assert_eq!(s.figures["frames"], 3_600);
        assert_eq!(s.figures["beeps"], 19);
        let text = s.to_text();
        assert!(text.starts_with("testbed log: PASS"));
        assert!(text.contains("0 of 3600 frames missed"));
    }

    #[test]
    fn misses_above_a_thousandth_fail() {
        // Five frames never drawn (each with an m line) and one never shown: 6 of 3,600.
        let s = summarize(&testbed_text(3_600, "borderless", "", |n| match n {
            100..=104 => Some(format!("m {n} vblank\n")),
            2_000 => Some("f 2000 1 2 -\n".to_owned()),
            _ => None,
        }))
        .unwrap();
        assert!(!s.passed());
        assert_eq!(s.figures["never_drawn"], 5);
        assert_eq!(s.figures["unknown_display"], 1);
        // Three are fine.
        let s = summarize(&testbed_text(3_600, "borderless", "", |n| {
            (100..103).contains(&n).then(|| format!("m {n} vblank\n"))
        }))
        .unwrap();
        assert!(s.passed(), "{}", s.to_text());
    }

    #[test]
    fn without_frame_statistics_only_m_lines_count_and_display_times_fail() {
        let s = summarize(&testbed_text(
            3_600,
            "borderless",
            "e no_frame_statistics\n",
            |n| {
                let t = 10_000_000 + 55_556 * n as i64;
                Some(if n == 7 {
                    "m 7 vblank\n".to_owned()
                } else {
                    format!("f {n} {t} {} -\n", t - 30_000)
                })
            },
        ))
        .unwrap();
        assert_eq!(s.criteria[0].pass, Some(true), "{}", s.to_text());
        assert!(s.criteria[0].detail.contains("1 of 3600 frames missed"));
        assert_eq!(s.criteria[1].pass, Some(false));
        assert!(!s.passed());
    }

    #[test]
    fn windowed_runs_report_misses_without_judging_them() {
        let s = summarize(&testbed_text(1_800, "windowed", "", |n| {
            (n < 50).then(|| format!("m {n} vblank\n"))
        }))
        .unwrap();
        assert_eq!(s.criteria[0].pass, None);
    }

    #[test]
    fn exclusive_rows_need_both_focus_events() {
        let ok = "e activate result=ok\ne fse result=ok\n";
        assert!(
            summarize(&testbed_text(1_800, "exclusive", ok, |_| None))
                .unwrap()
                .passed()
        );
        let failed = "e activate result=ok\ne fse result=failed\n";
        assert!(
            !summarize(&testbed_text(1_800, "exclusive", failed, |_| None))
                .unwrap()
                .passed()
        );
    }

    #[test]
    fn late_or_unplayed_beeps_fail_and_tone_logs_read() {
        let text = "# kairo-tone 1\nh run=5 hz=2000 offset_ms=500 dbfs=-30\nv 0 15000000 15004000\nv 1 25000000 25012000\nv 2 35000000 -\ne end reason=seconds\n";
        let s = summarize(text).unwrap();
        assert_eq!(s.kind, "tone");
        assert!(!s.passed());
        assert!(s.criteria[0].detail.contains("2 of 3 played"));
        let good = "# kairo-tone 1\nh run=5 hz=2000 offset_ms=500 dbfs=-30\nv 0 15000000 15004000\nv 1 25000000 24996000\n";
        assert!(summarize(good).unwrap().passed());
        assert!(summarize("# something else\n").is_err());
    }
}
