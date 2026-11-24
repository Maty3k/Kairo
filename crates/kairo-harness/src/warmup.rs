// SPDX-License-Identifier: GPL-3.0-or-later
//! Phase 2 (docs/research/16-verify-testbed/README.md §8.3): after 3 s, whether the testbed
//! presents steadily and its display shows frames at the refresh, judged from its log over the
//! last second of frames. A display that has gone dark gives no display times, and nothing wakes
//! it: the harness stops with "display off" (exit 3).

use kairo_core::testbed_log::{TestbedLog, TestbedRecord};

/// How far the median step per frame may be from the frame period, as a share.
pub const STEP_TOLERANCE: f64 = 0.01;
/// The most frames in the window that may be missed, as a share.
pub const MISSED_MAX: f64 = 0.01;
/// The least share of the window's frames that must have display times.
pub const DISPLAY_SHARE_MIN: f64 = 0.9;

/// What the steady display was judged by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The display times DXGI's statistics gave.
    Display,
    /// The present times, after `e no_frame_statistics` (research 16 risk 7).
    Present,
}

impl Basis {
    /// Its name in the report.
    pub fn name(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Present => "present",
        }
    }
}

/// A steady warm-up's figures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steady {
    /// What it was judged by.
    pub basis: Basis,
    /// The frames in the window.
    pub frames: usize,
    /// The share of them with a display time.
    pub display_share: f64,
    /// The median step per frame of the judged times, in 100 ns.
    pub median_step_hns: f64,
    /// The frame period: k vblanks, in 100 ns.
    pub step_hns: f64,
    /// The window's missed frames.
    pub misses: usize,
}

/// The judgement.
#[derive(Debug, Clone, PartialEq)]
pub enum Judgement {
    /// Steady: go on to the capture.
    Steady(Steady),
    /// The display is off, or the testbed isn't steady (exit 3): why.
    Off(&'static str),
    /// The testbed ended its log, with this reason.
    Ended(String),
}

/// The median of `values`, which aren't empty.
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Judges `log`, a testbed drawing one frame every `k` vblanks.
pub fn judge(log: &TestbedLog, k: u32) -> Judgement {
    let mut no_statistics = false;
    for record in &log.records {
        if let TestbedRecord::Event(e) = record {
            match e.name() {
                "end" => {
                    let reason = e
                        .fields()
                        .iter()
                        .find(|(key, _)| key == "reason")
                        .map_or("", |(_, v)| v.as_str());
                    return Judgement::Ended(reason.to_owned());
                }
                "no_vblank_timing" => return Judgement::Off("display off: no vblank timing"),
                "no_frame_statistics" => no_statistics = true,
                _ => {}
            }
        }
    }
    let fps = log.header.fps.max(1) as usize;
    let frames: Vec<_> = log.frames().filter(|f| f.frame >= 1).collect();
    if frames.len() < fps {
        return Judgement::Off("display off: too few frames");
    }
    let window = &frames[frames.len() - fps..];
    let (first, last) = (window[0].frame, window[window.len() - 1].frame);
    let misses = log
        .records
        .iter()
        .filter(|r| matches!(r, TestbedRecord::Miss(n, _) if (first..=last).contains(n)))
        .count();
    if misses as f64 > MISSED_MAX * window.len() as f64 {
        return Judgement::Off("the testbed misses frames");
    }
    let step_hns = f64::from(k) * 1e10 / log.header.refresh_mhz.max(1) as f64;
    let off = |median: f64| (median - step_hns).abs() > STEP_TOLERANCE * step_hns;
    let steps = |time: &dyn Fn(usize) -> Option<i64>| -> Vec<f64> {
        window
            .windows(2)
            .enumerate()
            .filter_map(|(i, pair)| {
                let (a, b) = (time(i)?, time(i + 1)?);
                Some((b - a) as f64 / (pair[1].frame - pair[0].frame) as f64)
            })
            .collect()
    };
    let presents = steps(&|i| Some(window[i].present_hns));
    let present_median = median(presents);
    if off(present_median) {
        return Judgement::Off("the testbed doesn't present at the frame rate");
    }
    let shown = window.iter().filter(|f| f.display_hns.is_some()).count();
    let display_share = shown as f64 / window.len() as f64;
    let mut steady = Steady {
        basis: Basis::Present,
        frames: window.len(),
        display_share,
        median_step_hns: present_median,
        step_hns,
        misses,
    };
    if no_statistics {
        return Judgement::Steady(steady);
    }
    if display_share < DISPLAY_SHARE_MIN {
        return Judgement::Off("display off: too few display times");
    }
    let displays = steps(&|i| window[i].display_hns);
    if displays.is_empty() {
        return Judgement::Off("display off: too few display times");
    }
    let display_median = median(displays);
    if off(display_median) {
        return Judgement::Off("display off: display times don't advance at the refresh");
    }
    steady.basis = Basis::Display;
    steady.median_step_hns = display_median;
    Judgement::Steady(steady)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A borderless vsync log at `fps` on a 179.96 Hz panel, frames 0..`frames`, each shown a
    /// vblank after its present, changed by `change` (a record line or `None` to drop it).
    pub(crate) fn log(
        fps: u32,
        frames: u64,
        events: &str,
        change: impl Fn(u64, String) -> Option<String>,
    ) -> TestbedLog {
        let k = (179.96 / f64::from(fps)).round();
        let period = 1e10 / 179_960.0;
        let mut text = format!(
            "# kairo-testbed 1\nh pattern=1 run=5 mode=borderless pace=vsync fps={fps} F={fps} size=1920x1080 refresh_mhz=179960 hdr=0 sdr_white_nits=- max_nits=- start_hns=10000000\n{events}"
        );
        for n in 0..frames {
            let t = 10_000_000 + (n as f64 * k * period).round() as i64;
            let line = format!("f {n} {t} {} {}\n", t - 20_000, t + period.round() as i64);
            if let Some(line) = change(n, line) {
                text.push_str(&line);
            }
        }
        TestbedLog::parse(&text).unwrap()
    }

    fn steady(j: &Judgement) -> Steady {
        match j {
            Judgement::Steady(s) => *s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_steady_run_is_judged_by_its_display_times() {
        for (fps, k) in [(180, 1), (90, 2), (60, 3)] {
            let s = steady(&judge(&log(fps, u64::from(fps) * 3, "", |_, l| Some(l)), k));
            assert_eq!(s.basis, Basis::Display, "{fps}");
            assert_eq!((s.frames, s.misses), (fps as usize, 0));
            assert!((s.median_step_hns - s.step_hns).abs() < 2.0, "{fps}: {s:?}");
        }
    }

    #[test]
    fn display_times_off_by_two_percent_are_off() {
        let skewed = log(180, 540, "", |n, line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let display = 10_000_000 + (n as f64 * 1.02 * 1e10 / 179_960.0).round() as i64;
            Some(format!("f {n} {} {} {display}\n", words[2], words[3]))
        });
        assert_eq!(
            judge(&skewed, 1),
            Judgement::Off("display off: display times don't advance at the refresh")
        );
    }

    #[test]
    fn without_statistics_the_presents_decide() {
        let dark = log(180, 540, "e no_frame_statistics\n", |_, line| {
            let mut words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
            words[4] = "-".to_owned();
            Some(words.join(" ") + "\n")
        });
        assert_eq!(steady(&judge(&dark, 1)).basis, Basis::Present);
        // The same log without the event: its display is off.
        let text = dark.to_text().replace("e no_frame_statistics\n", "");
        assert_eq!(
            judge(&TestbedLog::parse(&text).unwrap(), 1),
            Judgement::Off("display off: too few display times")
        );
        let clockless = log(
            180,
            540,
            "e no_frame_statistics\ne no_vblank_timing\n",
            |_, l| Some(l),
        );
        assert_eq!(
            judge(&clockless, 1),
            Judgement::Off("display off: no vblank timing")
        );
    }

    #[test]
    fn misses_few_frames_and_an_end() {
        // 3 of the last 180 frames missed (2%).
        let missing = log(180, 540, "", |n, line| match n {
            500..=502 => Some(format!("m {n} vblank\n")),
            _ => Some(line),
        });
        assert_eq!(
            judge(&missing, 1),
            Judgement::Off("the testbed misses frames")
        );
        let short = log(180, 100, "", |_, l| Some(l));
        assert_eq!(
            judge(&short, 1),
            Judgement::Off("display off: too few frames")
        );
        let ended = log(180, 540, "e end reason=hdr_changed\n", |_, l| Some(l));
        assert_eq!(judge(&ended, 1), Judgement::Ended("hdr_changed".to_owned()));
        // A torn last line is left out, not misread.
        let text = log(180, 540, "", |_, l| Some(l)).to_text() + "f 540 123";
        let torn = TestbedLog::parse(&text).unwrap();
        assert!(torn.torn_tail());
        assert_eq!(steady(&judge(&torn, 1)).frames, 180);
    }
}
