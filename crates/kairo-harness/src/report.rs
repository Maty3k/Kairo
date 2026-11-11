// SPDX-License-Identifier: GPL-3.0-or-later
//! `harness.json` and the summary on standard output (docs/research/16-verify-testbed/README.md
//! §8.3 phase 6, §8.6). `harness.json` names the adapter, its LUID and the FFmpeg build, so it
//! stays on the PC and in `.dogfood/`. Standard output, which `kairo-devrun` keeps as the job's
//! `out.log` and `pc` prints, carries only the run id, phase names and seconds, exit codes with
//! fixed reasons, counts, the verdict and file names (CLAUDE.md).

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::phases::{Phase, exit};
use crate::prune::Pruned;
use crate::readback::{Counts, Row};

/// The report's name in the run folder.
pub const REPORT: &str = "harness.json";

/// How a phase went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseOutcome {
    /// Done.
    Ok,
    /// The PC refused it.
    Refused,
    /// It failed.
    Failed,
    /// It ran past its limit.
    Timeout,
    /// An earlier phase stopped the run.
    Skipped,
}

/// One phase's record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhaseRecord {
    /// Its name.
    pub name: &'static str,
    /// Its limit.
    pub limit_s: u32,
    /// How long it took.
    pub seconds: f64,
    /// How it went.
    pub outcome: PhaseOutcome,
}

/// What was asked, without the monitor's path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Asked {
    /// `capture` or `smoke`.
    pub profile: &'static str,
    /// The capture's length.
    pub seconds: u32,
    /// `1920x1080@60` or `2560x1440@60`.
    pub output: String,
    /// The testbed's rate.
    pub testbed_fps: u32,
    /// `borderless`.
    pub mode: &'static str,
    /// `gfxcapture` or `ddagrab`.
    pub capture: &'static str,
    /// The capture's `max_framerate`.
    pub max_framerate: u32,
    /// FFmpeg's log level.
    pub loglevel: &'static str,
    /// Whether `h264_metadata` was applied.
    pub finishing: bool,
    /// Whether `kairo-tone` ran.
    pub voice: bool,
}

/// The display, as the run found it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayFacts {
    /// Its refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// Rounded to whole Hz, for the expect file.
    pub source_hz: u64,
    /// The testbed's vblanks per frame.
    pub vsync_k: u32,
    /// Windows' colour mode, if it says.
    pub color_mode: Option<String>,
}

/// The adapter: a PC detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdapterFacts {
    /// Its LUID.
    pub luid: String,
    /// Its DXGI index in the harness's process.
    pub dxgi_index: u32,
    /// Its vendor:device.
    pub device: String,
    /// The vendor:device FFmpeg's log named.
    pub log_device: Option<String>,
    /// Whether they're the same.
    #[serde(rename = "match")]
    pub matches: Option<bool>,
}

/// FFmpeg's build and run.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct FfmpegFacts {
    /// The pinned build's asset name, from `KAIRO-PIN.json`.
    pub asset: Option<String>,
    /// Its SHA-256.
    pub sha256: Option<String>,
    /// When the bootstrap installed it.
    pub installed_utc: Option<String>,
    /// `LGPL` when the asset name says so.
    pub licence: String,
    /// Its exit code.
    pub exit_code: Option<i32>,
    /// Whether it was killed at its limit.
    pub killed: bool,
    /// How long it ran.
    pub seconds: Option<f64>,
    /// The log's problems, by id.
    pub problems: Vec<&'static str>,
    /// AMF's lines about B-frames.
    pub bframe_warnings: usize,
    /// Whether `gfxcapture` couldn't turn off the yellow border.
    pub border_warning: bool,
    /// The clip's size.
    pub bytes: Option<u64>,
}

/// A child program's end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ChildFacts {
    /// Its exit code.
    pub exit_code: Option<i32>,
    /// Its log's `e end` reason.
    pub end: Option<String>,
    /// Whether the harness killed it.
    pub killed: bool,
    /// Whether it ran through the whole capture.
    pub alive_through_capture: bool,
    /// The last line of its standard error, printable ASCII, at most 200 characters.
    pub message: Option<String>,
}

/// The warm-up's figures.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WarmupFacts {
    /// `display` or `present`.
    pub basis: &'static str,
    /// Frames judged.
    pub frames: usize,
    /// The share with display times.
    pub display_share: f64,
    /// The median step per frame, in 100 ns.
    pub median_step_hns: f64,
    /// The frame period, in 100 ns.
    pub step_hns: f64,
    /// Missed frames among them.
    pub misses: usize,
}

/// kairo-verify's run.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct VerifyFacts {
    /// Its exit code.
    pub exit_code: Option<i32>,
    /// Whether it wrote `verify.json`.
    pub report: bool,
    /// `pass`, `fail` or `error`.
    pub verdict: Option<String>,
    /// How long it ran.
    pub seconds: Option<f64>,
    /// The share of frames whose barcode read.
    pub barcode_share: Option<f64>,
    /// The checks that failed or couldn't run, by id.
    pub failing: Vec<String>,
}

/// `harness.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The format's version: 1.
    pub kairo_harness: u32,
    /// This program's version.
    pub tool: &'static str,
    /// The run's id.
    pub run_id: String,
    /// When it started, UTC.
    pub started_utc: String,
    /// When it ended, UTC.
    pub ended_utc: String,
    /// The exit code.
    pub exit_code: u8,
    /// Why, with no PC detail.
    pub reason: String,
    /// What was asked.
    pub request: Asked,
    /// The testbed's run tag.
    pub run_tag: Option<u8>,
    /// The display.
    pub display: Option<DisplayFacts>,
    /// The adapter.
    pub adapter: Option<AdapterFacts>,
    /// FFmpeg.
    pub ffmpeg: FfmpegFacts,
    /// Each phase.
    pub phases: Vec<PhaseRecord>,
    /// The testbed.
    pub testbed: Option<ChildFacts>,
    /// `kairo-tone`, with `--voice`.
    pub tone: Option<ChildFacts>,
    /// The warm-up.
    pub warmup: Option<WarmupFacts>,
    /// kairo-verify.
    pub verify: Option<VerifyFacts>,
    /// The read-back table.
    pub readback: Vec<Row>,
    /// Its counts.
    pub readback_counts: Counts,
    /// What pruning did.
    pub prune: Pruned,
    /// The files in the run folder.
    pub files: Vec<String>,
}

impl Report {
    /// A report of a run that has just started.
    pub fn new(run_id: &str, started_utc: String, request: Asked) -> Report {
        Report {
            kairo_harness: 1,
            tool: env!("CARGO_PKG_VERSION"),
            run_id: run_id.to_owned(),
            started_utc,
            ended_utc: String::new(),
            exit_code: exit::INTERNAL,
            reason: String::new(),
            request,
            run_tag: None,
            display: None,
            adapter: None,
            ffmpeg: FfmpegFacts::default(),
            phases: Vec::new(),
            testbed: None,
            tone: None,
            warmup: None,
            verify: None,
            readback: Vec::new(),
            readback_counts: Counts::default(),
            prune: Pruned::default(),
            files: Vec::new(),
        }
    }

    /// Records `phase`: how long it took and how it went.
    pub fn phase(&mut self, phase: Phase, seconds: f64, outcome: PhaseOutcome) {
        self.phases.push(PhaseRecord {
            name: phase.name(),
            limit_s: phase.limit_s(self.request.seconds),
            seconds: (seconds * 1_000.0).round() / 1_000.0,
            outcome,
        });
    }

    /// The phases that never ran, after the last one recorded.
    pub fn skip_rest(&mut self) {
        let done = self.phases.len();
        for phase in Phase::ALL.iter().skip(done) {
            self.phases.push(PhaseRecord {
                name: phase.name(),
                limit_s: phase.limit_s(self.request.seconds),
                seconds: 0.0,
                outcome: PhaseOutcome::Skipped,
            });
        }
    }

    /// The JSON text.
    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Writes the report into `folder` through a temporary file and a rename, so a reader never
    /// sees half a file.
    pub fn write(&self, folder: &Path) -> std::io::Result<()> {
        let temporary = folder.join(format!("{REPORT}.tmp"));
        fs::write(&temporary, self.json())?;
        fs::rename(&temporary, folder.join(REPORT)).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    }

    /// The summary for standard output: no path, device, LUID, index or log line.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "{}: exit {} ({})\n",
            self.run_id, self.exit_code, self.reason
        );
        for p in &self.phases {
            let outcome = match p.outcome {
                PhaseOutcome::Ok => "ok",
                PhaseOutcome::Refused => "refused",
                PhaseOutcome::Failed => "failed",
                PhaseOutcome::Timeout => "timed out",
                PhaseOutcome::Skipped => "skipped",
            };
            let _ = writeln!(text, "  {:<8} {:>7.1} s  {outcome}", p.name, p.seconds);
        }
        if let Some(a) = &self.adapter
            && let Some(matches) = a.matches
        {
            let said = if matches {
                "names the monitor's adapter"
            } else {
                "names another adapter"
            };
            let _ = writeln!(text, "adapter: FFmpeg's log {said}");
        }
        let c = self.readback_counts;
        let _ = writeln!(
            text,
            "read-back: {} verified, {} failed, {} unknown, {} set but not read back",
            c.verified, c.failed, c.unknown, c.not_read_back
        );
        if let Some(v) = &self.verify {
            let verdict = v.verdict.as_deref().unwrap_or("no report");
            match v.barcode_share {
                Some(share) => {
                    let _ = writeln!(
                        text,
                        "verify: {verdict} ({:.2}% of barcodes read)",
                        share * 100.0
                    );
                }
                None => {
                    let _ = writeln!(text, "verify: {verdict}");
                }
            }
        }
        let _ = writeln!(text, "files: {}", self.files.join(", "));
        text
    }
}

/// The last line of a child's standard error, for the report: printable ASCII only, at most 200
/// characters. It can still name what the child named, which is why the report stays local.
pub fn last_line(err: &str) -> Option<String> {
    let line: String = err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())?
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(200)
        .collect();
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked() -> Asked {
        Asked {
            profile: "capture",
            seconds: 30,
            output: "1920x1080@60".to_owned(),
            testbed_fps: 180,
            mode: "borderless",
            capture: "gfxcapture",
            max_framerate: 360,
            loglevel: "debug",
            finishing: false,
            voice: false,
        }
    }

    /// A report of a run whose every field holds a PC detail somewhere.
    fn full() -> Report {
        let mut r = Report::new(
            "harness-20261111-203000",
            "2026-11-11T20:30:00.000Z".to_owned(),
            asked(),
        );
        r.exit_code = exit::OK;
        r.reason = "the clip was judged".to_owned();
        r.adapter = Some(AdapterFacts {
            luid: "0x00000000000abcde".to_owned(),
            dxgi_index: 1,
            device: "abcd:1234".to_owned(),
            log_device: Some("abcd:1234".to_owned()),
            matches: Some(true),
        });
        r.ffmpeg.asset = Some("ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0.zip".to_owned());
        r.testbed = Some(ChildFacts {
            message: Some("testbed: monitor \\\\?\\DISPLAY#ABC1234#5&1 refused".to_owned()),
            ..ChildFacts::default()
        });
        for phase in Phase::ALL {
            r.phase(phase, 1.25, PhaseOutcome::Ok);
        }
        r.verify = Some(VerifyFacts {
            exit_code: Some(0),
            report: true,
            verdict: Some("pass".to_owned()),
            seconds: Some(12.5),
            barcode_share: Some(0.9995),
            failing: Vec::new(),
        });
        r.files = vec!["harness.json".to_owned(), "capture.mp4".to_owned()];
        r
    }

    #[test]
    fn the_summary_names_nothing_of_the_pc() {
        let text = full().summary();
        assert!(text.starts_with("harness-20261111-203000: exit 0 (the clip was judged)\n"));
        assert!(text.contains("  capture      1.2 s  ok\n"), "{text}");
        assert!(text.contains("adapter: FFmpeg's log names the monitor's adapter"));
        assert!(text.contains("verify: pass (99.95% of barcodes read)"));
        for detail in ["DISPLAY#", "abcd", "1234", "0x0000", "abcde", "lgpl-shared"] {
            assert!(!text.contains(detail), "{detail}: {text}");
        }
        let hex_pair = text
            .as_bytes()
            .windows(9)
            .any(|w| w[4] == b':' && w[..4].iter().chain(&w[5..]).all(u8::is_ascii_hexdigit));
        assert!(!hex_pair, "{text}");
    }

    #[test]
    fn the_report_round_trips_and_writes_whole() {
        let mut r = full();
        r.phases.truncate(2);
        r.skip_rest();
        assert_eq!(r.phases.len(), 6);
        assert_eq!(r.phases[5].outcome, PhaseOutcome::Skipped);
        let value: serde_json::Value = serde_json::from_str(&r.json()).unwrap();
        assert_eq!(value["kairo_harness"], 1);
        assert_eq!(value["adapter"]["match"], true);
        assert_eq!(value["phases"][3]["limit_s"], 60);
        let dir = std::env::temp_dir().join(format!("kairo-harness-report-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        r.write(&dir).unwrap();
        assert!(dir.join(REPORT).is_file() && !dir.join("harness.json.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_childs_last_line() {
        assert_eq!(last_line("a\nlast line\n\n"), Some("last line".to_owned()));
        assert_eq!(last_line("tab\there\u{7}"), Some("tabhere".to_owned()));
        assert_eq!(last_line(&"x".repeat(300)).map(|l| l.len()), Some(200));
        assert_eq!(last_line(" \n"), None);
    }
}
