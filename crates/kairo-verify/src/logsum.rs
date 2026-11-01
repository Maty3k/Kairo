// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify log FILE`: what a testbed or tone log says about the run itself, with
//! research 16 §4.12's acceptance limits (docs/research/16-verify-testbed/README.md §4.9, §4.12,
//! §5). It judges the source, not a clip: missed vblanks, display times that advance at the
//! refresh, beeps that play on their targets, and the exclusive row's focus events.
//!
//! A log names no device or path, so the summary doesn't either.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use kairo_core::testbed_log::{Event, Mode, Pace, TestbedLog, TestbedRecord, ToneLog, ToneRecord};
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

/// The beep rule: every beep played within [`BEEP_ERROR_MAX_MS`] of its target. Not judged only
/// for a log with no beeps and no audio trouble (a testbed run with `--beep off`); `e
/// audio_failed`, or a tone log without a beep, fails.
fn beep_criterion(
    count: usize,
    played: usize,
    errors: &[f64],
    audio_failed: bool,
    must_beep: bool,
) -> Criterion {
    let spread = Spread::of(errors.to_vec());
    let worst = errors.iter().fold(0.0_f64, |m, e| m.max(e.abs()));
    let pass = if audio_failed {
        Some(false)
    } else if count == 0 {
        must_beep.then_some(false)
    } else {
        Some(played == count && worst <= BEEP_ERROR_MAX_MS)
    };
    let mut detail = match spread {
        _ if count == 0 => "no beeps".to_owned(),
        Some(s) => format!(
            "{played} of {count} played; played − target {:+.3} to {:+.3} ms, median {:+.3} (limit ±{BEEP_ERROR_MAX_MS} ms)",
            s.min, s.max, s.median
        ),
        None => format!("none of {count} played"),
    };
    if audio_failed {
        detail.push_str("; e audio_failed");
    }
    Criterion {
        name: "beeps play on target",
        pass,
        detail,
    }
}

/// Each event's line, `name k=v …`.
fn event_line(e: &Event) -> String {
    let mut line = e.name().to_owned();
    for (k, v) in e.fields() {
        let _ = write!(line, " {k}={v}");
    }
    line
}

/// Whether `events` hold the event `name` (with or without fields).
fn has(events: &[String], name: &str) -> bool {
    events.iter().any(|e| {
        e == name
            || e.strip_prefix(name)
                .is_some_and(|rest| rest.starts_with(' '))
    })
}

/// The run-end rule: the log ends with `e end reason=seconds` or `reason=stdin`. A run that was
/// killed or crashed has no `e end`; one that failed, or met an HDR toggle, says so.
fn end_criterion(events: &[String], length_s: Option<f64>) -> Criterion {
    let end = events
        .iter()
        .rev()
        .find_map(|e| e.strip_prefix("end reason="));
    let mut detail = match end {
        Some(reason) => format!("e end reason={reason}"),
        None => "no e end: the run was stopped from outside, or crashed".to_owned(),
    };
    if let Some(seconds) = length_s {
        let _ = write!(detail, "; {seconds:.1} s of frames");
    }
    Criterion {
        name: "the run ran to its end",
        pass: Some(matches!(end, Some("seconds" | "stdin"))),
        detail,
    }
}

/// A testbed log's summary (README §4.12).
pub fn testbed(log: &TestbedLog) -> Summary {
    let h = &log.header;
    let refresh_hz = h.refresh_mhz as f64 / 1_000.0;
    let period_ms = 1_000.0 / refresh_hz;
    let fps = f64::from(h.fps.max(1));
    let timer = h.pace == Pace::Timer;
    // A timer-paced testbed above the refresh: the display shows the newest frame at each
    // vblank and passes the others over, by design.
    let above_refresh = timer && fps > refresh_hz * 1.005;
    let frame_ms = match h.pace {
        Pace::Vsync => (refresh_hz / fps).round().max(1.0) * period_ms,
        Pace::Timer => 1_000.0 / fps,
    };
    let frames: Vec<_> = log.frames().collect();
    let first = frames.first().map(|f| f.frame);
    let last = frames.last().map(|f| f.frame);
    let span = first.zip(last).map_or(0, |(a, b)| b - a + 1);
    let mut m_lines: BTreeSet<u64> = BTreeSet::new();
    let mut misses = 0;
    let mut events = Vec::new();
    for record in &log.records {
        match record {
            TestbedRecord::Miss(frame, _) => {
                misses += 1;
                m_lines.insert(*frame);
            }
            TestbedRecord::Event(e) => events.push(event_line(e)),
            _ => {}
        }
    }
    let shown: Vec<(u64, i64)> = frames
        .iter()
        .filter_map(|f| Some((f.frame, f.display_hns?)))
        .collect();
    let unknown = frames.iter().filter(|f| f.display_hns.is_none()).count();
    let untimed = has(&events, "no_frame_statistics") || has(&events, "no_vblank_timing");
    let timed = !untimed && !shown.is_empty();
    // Missed: m lines and frames never drawn; with vsync, also a `-` just before a frame marked
    // late (the statistics passed over an irregular stretch). Any other `-` is unknown.
    let mut missed: BTreeSet<u64> = m_lines.clone();
    let drawn: BTreeSet<u64> = frames.iter().map(|f| f.frame).collect();
    if let (Some(a), Some(b)) = (first, last) {
        missed.extend((a..=b).filter(|n| !drawn.contains(n)));
    }
    if timed && !timer {
        let mut before_late = false;
        for f in frames.iter().rev() {
            match f.display_hns {
                Some(_) => before_late = m_lines.contains(&f.frame),
                None if before_late => {
                    missed.insert(f.frame);
                }
                None => {}
            }
        }
    }
    // Between frames shown one after another: the vblanks, and the time per frame.
    let vblank_steps: Vec<i64> = shown
        .windows(2)
        .map(|w| ((w[1].1 - w[0].1) as f64 / 10_000.0 / period_ms).round() as i64)
        .collect();
    let intervals: Vec<f64> = shown
        .windows(2)
        .map(|w| (w[1].1 - w[0].1) as f64 / 10_000.0)
        .collect();
    let (count, played, errors) = beep_errors(log.beeps().map(|b| (b.target_hns, b.played_hns)));
    let fullscreen = matches!(h.mode, Mode::Borderless | Mode::Exclusive);
    let share_of = |part: usize, whole: u64| {
        if whole == 0 {
            0.0
        } else {
            part as f64 / whole as f64
        }
    };
    let missed_share;
    let mut criteria = Vec::new();
    if above_refresh {
        // Judged by what the display showed: a vblank that brought no new frame is a miss.
        let total: i64 = vblank_steps.iter().sum();
        let extra: i64 = vblank_steps.iter().map(|&s| (s - 1).max(0)).sum();
        missed_share = share_of(extra.max(0) as usize, total.max(0) as u64);
        criteria.push(Criterion {
            name: "missed vblanks",
            pass: fullscreen.then_some(total > 0 && missed_share <= MISSED_SHARE_MAX),
            detail: if total == 0 {
                "no display times to judge the vblanks by".to_owned()
            } else {
                format!(
                    "{extra} of {total} vblanks brought no new frame ({:.3}%; limit {:.1}%); {misses} slots missed",
                    missed_share * 100.0,
                    MISSED_SHARE_MAX * 100.0
                )
            },
        });
    } else {
        missed_share = share_of(missed.len(), span);
        criteria.push(Criterion {
            name: "missed vblanks",
            pass: (fullscreen && span > 0).then_some(missed_share <= MISSED_SHARE_MAX),
            detail: if span == 0 {
                "no frames".to_owned()
            } else {
                let unknown_only = frames
                    .iter()
                    .filter(|f| f.display_hns.is_none() && !missed.contains(&f.frame))
                    .count();
                let unknown_note = match unknown_only {
                    0 => String::new(),
                    n => format!("; {n} display times unknown"),
                };
                let scope = if fullscreen {
                    String::new()
                } else {
                    "; judged only borderless or exclusive".to_owned()
                };
                format!(
                    "{} of {span} frames missed ({:.3}%; limit {:.1}%){unknown_note}{scope}",
                    missed.len(),
                    missed_share * 100.0,
                    MISSED_SHARE_MAX * 100.0
                )
            },
        });
    }
    let spread = Spread::of(intervals);
    criteria.push(if !timed {
        Criterion {
            name: "display times advance at the refresh",
            pass: Some(false),
            detail: if untimed {
                "no display times: the swap chain gave no frame statistics".to_owned()
            } else {
                "no display times".to_owned()
            },
        }
    } else if above_refresh {
        // The newest frame at each vblank: about refresh/fps of the frames show, a period apart.
        let share = share_of(shown.len(), frames.len() as u64);
        let expected = refresh_hz / fps;
        let median = spread.map_or(0.0, |s| s.median);
        Criterion {
            name: "display times advance at the refresh",
            pass: Some(
                share >= 0.99 * expected
                    && (median - period_ms).abs() <= DISPLAY_RATE_TOLERANCE * period_ms,
            ),
            detail: format!(
                "{:.1}% of frames shown (the newest at each vblank: {:.1}%); median interval {:.4} ms (the refresh period is {:.4} ms)",
                share * 100.0,
                expected * 100.0,
                median,
                period_ms
            ),
        }
    } else {
        // The rate over the whole run, so frames that hold one or two vblanks (a timer rate
        // that doesn't divide the refresh) still average to the frame period.
        let known = share_of(shown.len(), frames.len() as u64);
        let rate_ms = match (shown.first(), shown.last()) {
            (Some(a), Some(b)) if b.0 > a.0 => {
                Some((b.1 - a.1) as f64 / 10_000.0 / (b.0 - a.0) as f64)
            }
            _ => None,
        };
        let advancing =
            rate_ms.is_some_and(|r| (r - frame_ms).abs() <= DISPLAY_RATE_TOLERANCE * frame_ms);
        Criterion {
            name: "display times advance at the refresh",
            pass: Some(advancing && known >= 1.0 - MISSED_SHARE_MAX),
            detail: format!(
                "{:.1}% of frames have a display time; over the run {} a frame (the frame period is {:.4} ms)",
                known * 100.0,
                rate_ms.map_or("—".to_owned(), |r| format!("{r:.4} ms")),
                frame_ms
            ),
        }
    });
    criteria.push(beep_criterion(
        count,
        played,
        &errors,
        has(&events, "audio_failed"),
        false,
    ));
    if h.mode == Mode::Exclusive {
        let activated = events.iter().any(|e| e == "activate result=ok");
        criteria.push(Criterion {
            name: "the window took the foreground",
            pass: Some(activated),
            detail: format!(
                "e activate result={}",
                if activated { "ok" } else { "failed or missing" }
            ),
        });
        let entered = events.iter().any(|e| e == "fse result=ok");
        let lost = has(&events, "fullscreen_lost");
        criteria.push(Criterion {
            name: "exclusive fullscreen held",
            pass: Some(entered && !lost),
            detail: match (entered, lost) {
                (true, false) => "e fse result=ok".to_owned(),
                (true, true) => "e fse result=ok, then e fullscreen_lost".to_owned(),
                (false, _) => "e fse result=failed or missing".to_owned(),
            },
        });
    }
    let length_s = (span > 0).then(|| span as f64 / fps);
    criteria.push(end_criterion(&events, length_s));
    Summary {
        kind: "testbed",
        figures: json!({
            "mode": h.mode.to_string(), "pace": h.pace.to_string(), "fps": h.fps,
            "refresh_mhz": h.refresh_mhz, "size": [h.width, h.height], "hdr": h.hdr,
            "frames": frames.len(), "span": span, "never_drawn": span.saturating_sub(drawn.len() as u64),
            "m_lines": misses, "missed": missed.len(), "unknown_display": unknown,
            "missed_share": missed_share, "display_interval_ms": spread, "frame_period_ms": frame_ms,
            "length_s": length_s, "beeps": count, "beeps_played": played,
            "beep_error_ms": Spread::of(errors), "events": events,
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
            ToneRecord::Event(e) => Some(event_line(e)),
            ToneRecord::Voice(_) => None,
        })
        .collect();
    let h = &log.header;
    let criteria = vec![
        beep_criterion(count, played, &errors, has(&events, "audio_failed"), true),
        end_criterion(&events, None),
    ];
    Summary {
        kind: "tone",
        figures: json!({
            "hz": h.hz, "offset_ms": h.offset_ms, "dbfs": h.dbfs,
            "beeps": count, "beeps_played": played, "beep_error_ms": Spread::of(errors.clone()),
            "events": events,
        }),
        criteria,
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
        // Five frames never drawn, each with an m line, and one whose display time is unknown
        // (not before a late frame, so not a miss): 5 of 3,600.
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
        let good = "# kairo-tone 1\nh run=5 hz=2000 offset_ms=500 dbfs=-30\nv 0 15000000 15004000\nv 1 25000000 24996000\ne end reason=seconds\n";
        assert!(summarize(good).unwrap().passed());
        assert!(summarize("# something else\n").is_err());
        // A tone run that couldn't open its device beeps nothing: it fails.
        let silent = "# kairo-tone 1\nh run=5 hz=2000 offset_ms=500 dbfs=-30\ne audio_failed\ne end reason=error\n";
        let s = summarize(silent).unwrap();
        assert_eq!(s.criteria[0].pass, Some(false));
        assert!(!s.passed());
    }

    /// A timer-paced testbed log of `frames` frames at `fps` on 180 Hz, each shown at the vblank
    /// `shown(n)` says (`None`: never shown), ending with `end`.
    fn timer_text(fps: u32, frames: u64, shown: impl Fn(u64) -> Option<i64>, end: &str) -> String {
        let period = 10_000_000.0 / 180.0;
        let mut text = format!(
            "# kairo-testbed 1\nh pattern=1 run=5 mode=borderless pace=timer fps={fps} F={fps} size=1920x1080 refresh_mhz=180000 hdr=0 sdr_white_nits=- max_nits=- start_hns=10000000\n"
        );
        for n in 0..frames {
            let slot = 10_000_000 + (n as f64 * 10_000_000.0 / f64::from(fps)).ceil() as i64;
            match shown(n) {
                Some(v) => {
                    let t = 10_000_000 + (v as f64 * period).round() as i64;
                    let _ = writeln!(text, "f {n} {slot} {slot} {t}");
                }
                None => {
                    let _ = writeln!(text, "f {n} {slot} {slot} -");
                }
            }
        }
        text.push_str(end);
        text
    }

    #[test]
    fn timer_rows_above_the_refresh_are_judged_by_what_showed() {
        let end = "e end reason=seconds\n";
        // 360 fps on 180 Hz: every other frame shows, one a vblank.
        let good = timer_text(
            360,
            7_200,
            |n| (n % 2 == 1).then_some((n as i64 + 1) / 2),
            end,
        );
        let s = summarize(&good).unwrap();
        assert!(s.passed(), "{}", s.to_text());
        // Only every sixth frame showed: the display ran at 60 Hz, two vblanks in three empty.
        let slow = timer_text(
            360,
            7_200,
            |n| (n % 6 == 5).then_some((n as i64 + 1) / 2),
            end,
        );
        let s = summarize(&slow).unwrap();
        assert_eq!(s.criteria[0].pass, Some(false), "{}", s.to_text());
        assert_eq!(s.criteria[1].pass, Some(false), "{}", s.to_text());
        // A log with no frames at all, of a run that failed.
        let empty = timer_text(240, 0, |_| None, "e end reason=error\n");
        assert!(!summarize(&empty).unwrap().passed());
    }

    #[test]
    fn a_timer_rate_that_doesnt_divide_the_refresh_passes() {
        // 120 fps on 180 Hz: each frame at the first vblank after its slot, one or two vblanks
        // apart, 8.33 ms a frame over the run.
        let text = timer_text(
            120,
            2_400,
            |n| Some((n as i64 * 3).div_euclid(2) + 1),
            "e end reason=seconds\n",
        );
        let s = summarize(&text).unwrap();
        assert!(s.passed(), "{}", s.to_text());
    }

    #[test]
    fn runs_cut_short_or_gone_wrong_fail() {
        let cases = [
            ("e audio_failed\n", "beeps play on target"),
            ("", "the run ran to its end"),
        ];
        // No e end at all (killed), audio that failed.
        for (events, rule) in cases {
            let mut text = testbed_text(1_800, "borderless", events, |_| None);
            if events.is_empty() {
                text = text.replace("e end reason=seconds\n", "");
            }
            let s = summarize(&text).unwrap();
            let c = s.criteria.iter().find(|c| c.name == rule).unwrap();
            assert_eq!(c.pass, Some(false), "{rule}: {}", s.to_text());
        }
        // A run that ended in an error or at an HDR toggle.
        for reason in ["error", "hdr_changed"] {
            let text = testbed_text(360, "borderless", "", |_| None)
                .replace("e end reason=seconds", &format!("e end reason={reason}"));
            assert!(!summarize(&text).unwrap().passed(), "{reason}");
        }
        // Exclusive fullscreen entered, then lost.
        let lost = "e activate result=ok\ne fse result=ok\ne fullscreen_lost\n";
        let s = summarize(&testbed_text(1_800, "exclusive", lost, |_| None)).unwrap();
        let held = s
            .criteria
            .iter()
            .find(|c| c.name == "exclusive fullscreen held")
            .unwrap();
        assert_eq!(held.pass, Some(false));
    }

    #[test]
    fn an_unknown_display_time_is_a_miss_only_before_a_late_frame() {
        // Frame 0 unreported and frame 600 shown late (its own m line): 1 of 3,600 missed, under
        // the limit; frame 599, unreported just before it, would make it 2.
        let s = summarize(&testbed_text(3_600, "borderless", "", |n| match n {
            0 => Some("f 0 1 2 -\n".to_owned()),
            600 => Some(format!(
                "f 600 1 2 {}\nm 600 vblank\n",
                10_000_000 + 55_556 * 601
            )),
            _ => None,
        }))
        .unwrap();
        assert!(
            s.criteria[0].detail.contains("1 of 3600 frames missed"),
            "{}",
            s.to_text()
        );
        let s = summarize(&testbed_text(3_600, "borderless", "", |n| match n {
            599 => Some("f 599 1 2 -\n".to_owned()),
            600 => Some(format!(
                "f 600 1 2 {}\nm 600 vblank\n",
                10_000_000 + 55_556 * 601
            )),
            _ => None,
        }))
        .unwrap();
        assert!(
            s.criteria[0].detail.contains("2 of 3600 frames missed"),
            "{}",
            s.to_text()
        );
    }
}
