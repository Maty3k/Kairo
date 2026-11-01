// SPDX-License-Identifier: GPL-3.0-or-later
//! `corpus check` (docs/research/16-verify-testbed/README.md §7.4): every clip of a corpus
//! checked with `--profile test` and its expect file, then judged against the manifest.
//!
//! The good clips go first. A PC corpus (AMF or Media Foundation) sets its line-error limit
//! from them, max(4, 2 × their worst line error), and judges the bad clips with it; the Mac and
//! CI corpora keep 4 codes. Each clip's report goes next to it as `<clip>.report.json`, and the
//! judgement into `corpus-report.json`, with a summary that fits one screen.

use std::fmt;
use std::fs::{self, File};
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use super::{
    CALIBRATED, Encoder, Entry, Judged, LINES_FLOOR_CODES, MANIFEST, Manifest, Missing, REPORT,
    VERSION, calibrated_limit, judge,
};
use crate::Report;
use crate::decode::Decoder;
use crate::expect::Expect;
use crate::profile::{Plan, Profile};

/// Why a check stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError {
    /// The exit code: 3 the corpus can't be judged, 4 a report couldn't be written.
    pub code: u8,
    /// What happened, naming files inside the corpus folder only.
    pub message: String,
}

impl CheckError {
    fn evaluate(message: String) -> Self {
        Self { code: 3, message }
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CheckError {}

/// The line-error limit a corpus was judged with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Lines {
    /// The limit, in codes.
    pub limit_codes: f64,
    /// How it was set.
    pub source: String,
    /// The good clips' worst line error, where it set the limit.
    pub worst_good_codes: Option<f64>,
}

/// `corpus-report.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CorpusReport {
    /// The format version: [`VERSION`].
    pub kairo_corpus_report: u32,
    /// The manifest's encoder.
    pub encoder: String,
    /// The FFmpeg that built the corpus.
    pub ffmpeg: String,
    /// The decoder that checked it.
    pub decoder: &'static str,
    /// `content.bars_lines`' limit.
    pub lines: Lines,
    /// Whether every clip behaved.
    pub pass: bool,
    /// Each clip, in the manifest's order.
    pub clips: Vec<Judged>,
    /// The clips the encoder couldn't make.
    pub missing: Vec<Missing>,
}

/// A check's settings.
pub struct Check<'a> {
    /// The corpus folder.
    pub dir: &'a Path,
    /// The decoder for a clip.
    pub decoder: &'a dyn Fn(&Path) -> Box<dyn Decoder>,
}

/// A report's largest row or column residual, if `content.bars_lines` measured one.
fn line_error(report: &Report) -> Option<f64> {
    report
        .checks
        .iter()
        .find(|f| f.id == CALIBRATED)?
        .value
        .get("max_codes")?
        .as_f64()
}

impl Check<'_> {
    /// Checks and judges every clip, telling `say` each step.
    pub fn run(&self, say: &mut dyn FnMut(&str)) -> Result<CorpusReport, CheckError> {
        let manifest = self.manifest()?;
        let encoder: Encoder = manifest
            .encoder
            .parse()
            .map_err(|_| CheckError::evaluate(format!("{MANIFEST}: an unknown encoder")))?;
        let calibration = manifest.calibration.get(CALIBRATED);
        let calibrate = calibration.is_some_and(|c| c.limit_codes.is_none());
        let mut limit = calibration.and_then(|c| c.limit_codes);
        say(&format!(
            "corpus check: {} clips from {encoder}, FFmpeg {}",
            manifest.clips.len(),
            manifest.ffmpeg
        ));
        let good: Vec<&Entry> = manifest.clips.iter().filter(|e| e.is_good()).collect();
        let bad: Vec<&Entry> = manifest.clips.iter().filter(|e| !e.is_good()).collect();
        let mut reports: Vec<(String, Report)> = Vec::new();
        for entry in &good {
            reports.push((entry.file.clone(), self.check_clip(entry, limit, say)?));
        }
        let mut worst_good = None;
        if calibrate {
            let worst = reports
                .iter()
                .filter_map(|(_, r)| line_error(r))
                .fold(0.0_f64, f64::max);
            let calibrated = calibrated_limit(worst);
            say(&format!(
                "  line errors: worst good clip {worst:.2} codes, so the limit is {calibrated:.2}"
            ));
            worst_good = Some(worst);
            limit = Some(calibrated);
            // A good clip over 4 codes failed under them; it is judged again under the new limit.
            if worst > LINES_FLOOR_CODES {
                reports.clear();
                for entry in &good {
                    reports.push((entry.file.clone(), self.check_clip(entry, limit, say)?));
                }
            }
        }
        for entry in &bad {
            reports.push((entry.file.clone(), self.check_clip(entry, limit, say)?));
        }
        let decoder = reports.first().map_or("none", |(_, r)| r.decoder.kind);
        let clips: Vec<Judged> = manifest
            .clips
            .iter()
            .filter_map(|entry| {
                let (_, report) = reports.iter().find(|(file, _)| *file == entry.file)?;
                Some(judge(entry, report.verdict, &report.checks))
            })
            .collect();
        let report = CorpusReport {
            kairo_corpus_report: VERSION,
            encoder: manifest.encoder.clone(),
            ffmpeg: manifest.ffmpeg.clone(),
            decoder,
            lines: Lines {
                limit_codes: limit.unwrap_or(LINES_FLOOR_CODES),
                source: calibration.map_or_else(
                    || "README §2 item 14 (proposed)".to_owned(),
                    |c| c.source.clone(),
                ),
                worst_good_codes: worst_good,
            },
            pass: clips.iter().all(|c| c.problems.is_empty()),
            clips,
            missing: manifest.missing.clone(),
        };
        let text = serde_json::to_string_pretty(&report).map_err(|e| CheckError {
            code: 4,
            message: format!("{REPORT}: {e}"),
        })?;
        fs::write(self.dir.join(REPORT), format!("{text}\n")).map_err(|e| CheckError {
            code: 4,
            message: format!("{REPORT}: {e}"),
        })?;
        Ok(report)
    }

    /// The manifest, checked.
    fn manifest(&self) -> Result<Manifest, CheckError> {
        let text = fs::read_to_string(self.dir.join(MANIFEST)).map_err(|e| {
            CheckError::evaluate(format!(
                "{MANIFEST}: {e}; corpus build writes it last, so the build didn't finish"
            ))
        })?;
        let manifest: Manifest = serde_json::from_str(&text)
            .map_err(|e| CheckError::evaluate(format!("{MANIFEST}: {e}")))?;
        if manifest.kairo_corpus != VERSION {
            return Err(CheckError::evaluate(format!(
                "{MANIFEST}: kairo_corpus {}; this build reads {VERSION}",
                manifest.kairo_corpus
            )));
        }
        // Clips are named by the manifest; a name that leaves the folder is refused.
        for entry in &manifest.clips {
            let plain = Path::new(&entry.file).file_name().and_then(|n| n.to_str());
            if plain != Some(entry.file.as_str()) || !entry.file.ends_with(".mp4") {
                return Err(CheckError::evaluate(format!(
                    "{MANIFEST}: a clip that isn't a .mp4 in the corpus folder"
                )));
            }
        }
        Ok(manifest)
    }

    /// One clip checked with `--profile test`, its expect file and the line-error `limit`; its
    /// report written next to it.
    fn check_clip(
        &self,
        entry: &Entry,
        limit: Option<f64>,
        say: &mut dyn FnMut(&str),
    ) -> Result<Report, CheckError> {
        let started = Instant::now();
        let expect_file = entry.expect_file();
        let text = fs::read_to_string(self.dir.join(&expect_file))
            .map_err(|e| CheckError::evaluate(format!("{expect_file}: {e}")))?;
        let expect = Expect::parse(&text)
            .map_err(|e| CheckError::evaluate(format!("{expect_file}: {e}")))?;
        let mut plan = Plan::new(Profile::Test, false, Some(expect))
            .map_err(|e| CheckError::evaluate(format!("{expect_file}: {e}")))?;
        plan.bars_lines_codes = limit;
        let path = self.dir.join(&entry.file);
        let mut file =
            File::open(&path).map_err(|e| CheckError::evaluate(format!("{}: {e}", entry.file)))?;
        let mut decoder = (self.decoder)(&path);
        let report = crate::check_with(&mut file, &entry.file, &plan, Some(decoder.as_mut()));
        let name = entry.report_file();
        fs::write(self.dir.join(&name), report.to_json() + "\n").map_err(|e| CheckError {
            code: 4,
            message: format!("{name}: {e}"),
        })?;
        say(&format!(
            "  {} {} in {:.1} s",
            entry.file,
            report.verdict,
            started.elapsed().as_secs_f64()
        ));
        Ok(report)
    }
}

impl CorpusReport {
    /// The summary: one line per clip, the missing clips, the line-error limit and the result.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "corpus {} (FFmpeg {}, decoder {})\n",
            self.encoder, self.ffmpeg, self.decoder
        );
        let width = self.clips.iter().map(|c| c.file.len()).max().unwrap_or(0);
        for clip in &self.clips {
            let mark = if clip.problems.is_empty() {
                "ok "
            } else {
                "BAD"
            };
            let failed = if clip.failed.is_empty() {
                clip.verdict.to_string()
            } else {
                format!("fail: {}", clip.failed.join(", "))
            };
            out += &format!("  {mark} {:width$}  {failed}", clip.file);
            if !clip.problems.is_empty() {
                out += &format!("  <- {}", clip.problems.join("; "));
            }
            out.push('\n');
        }
        for missing in &self.missing {
            out += &format!("  missing {}: {}\n", missing.file, missing.why);
        }
        out += &format!(
            "  line errors: limit {:.2} codes ({})\n",
            self.lines.limit_codes, self.lines.source
        );
        let bad = self.clips.iter().filter(|c| !c.problems.is_empty()).count();
        out += &if self.pass {
            format!("PASS: all {} clips behave\n", self.clips.len())
        } else {
            format!("FAIL: {bad} of {} clips misbehave\n", self.clips.len())
        };
        out
    }
}
