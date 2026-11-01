// SPDX-License-Identifier: GPL-3.0-or-later
//! `corpus check` (docs/research/16-verify-testbed/README.md §7.4): every clip of a corpus
//! checked with `--profile test` and its expect file, then judged.
//!
//! The rules come from this build's recipes, not the manifest: the manifest must list every
//! recipe's clip once, with the recipe's rules, and each clip must still have the size and hash
//! the build recorded, so a clip left out, loosened, swapped or edited is refused (exit 3). The
//! good clips go first. A PC corpus (AMF or Media Foundation) sets its line-error limit from
//! them, max(4, 2 × their worst line error), and checks them again under it if it moved; the
//! Mac and CI corpora keep 4 codes. A PC corpus whose `bframes` no encoder could make is
//! incomplete until the Mac's clip is copied in, and then that clip is judged by the Mac's
//! rules. Earlier reports are removed first; each clip's report goes next to it as
//! `<clip>.report.json`, and the judgement into `corpus-report.json` once every clip is done.

use std::fmt;
use std::fs::{self, File};
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use super::{
    Entry, Judged, LINES_FLOOR_CODES, MANIFEST, Manifest, Missing, REPORT, VERSION,
    calibrated_limit, expected, fnv1a64, judge,
};
use crate::Report;
use crate::decode::{DecodeError, Decoder};
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

    fn write(name: &str, error: impl fmt::Display) -> Self {
        Self {
            code: 4,
            message: format!("{name}: {error}"),
        }
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
    /// The kairo-verify that judged it.
    pub tool: &'static str,
    /// The manifest's encoder.
    pub encoder: String,
    /// The FFmpeg that built the corpus.
    pub ffmpeg: String,
    /// The decoder that checked it.
    pub decoder: &'static str,
    /// `content.bars_lines`' limit.
    pub lines: Lines,
    /// Whether every clip of the corpus is there: false while the PC's `bframes` waits for the
    /// Mac's clip.
    pub complete: bool,
    /// Whether every clip is there and behaves.
    pub pass: bool,
    /// Each clip judged, in the recipes' order.
    pub clips: Vec<Judged>,
    /// The clips the encoder couldn't make that aren't there yet.
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
        .find(|f| f.id == super::CALIBRATED)?
        .value
        .get("max_codes")?
        .as_f64()
}

/// Removes `name` from `dir` if it is there.
fn remove(dir: &Path, name: &str) -> Result<(), CheckError> {
    match fs::remove_file(dir.join(name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(CheckError::write(name, e)),
        _ => Ok(()),
    }
}

impl Check<'_> {
    /// Checks and judges every clip, telling `say` each step.
    pub fn run(&self, say: &mut dyn FnMut(&str)) -> Result<CorpusReport, CheckError> {
        // An earlier judgement goes first, so a run that stops leaves none behind.
        remove(self.dir, REPORT)?;
        // A non-LGPL FFmpeg linked into this process judges nothing, whichever decoder runs
        // (README §2 item 18): the corpus can't be judged.
        if let Some(refusal) = crate::decode::linked_ffmpeg()
            .as_ref()
            .and_then(crate::checks::not_lgpl)
        {
            return Err(CheckError::evaluate(refusal));
        }
        let manifest = self.manifest()?;
        let (encoder, entries) =
            expected(&manifest).map_err(|e| CheckError::evaluate(format!("{MANIFEST}: {e}")))?;
        for entry in &entries {
            remove(self.dir, &entry.report_file())?;
        }
        // A clip a PC encoder couldn't make counts once the Mac's is copied in.
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for entry in entries {
            let listed = manifest.clips.iter().any(|e| e.file == entry.file);
            if listed || self.dir.join(&entry.file).exists() {
                present.push((entry, !listed));
            } else if let Some(m) = manifest.missing.iter().find(|m| m.file == entry.file) {
                missing.push(m.clone());
            }
        }
        say(&format!(
            "corpus check: {} clips from {encoder}, FFmpeg {}",
            present.len(),
            manifest.ffmpeg
        ));
        for (entry, copied) in &present {
            if !copied {
                self.verify(entry)?;
            }
        }
        self.decoder_starts(&present)?;
        let good: Vec<&Entry> = present
            .iter()
            .filter(|(e, _)| e.is_good())
            .map(|(e, _)| e)
            .collect();
        let mut reports: Vec<(String, Report)> = Vec::new();
        for entry in &good {
            reports.push((entry.file.clone(), self.check_clip(entry, None, say)?));
        }
        let (limit, lines) = if encoder.calibrates() {
            let worst = reports
                .iter()
                .filter_map(|(_, r)| line_error(r))
                .fold(0.0_f64, f64::max);
            let calibrated = calibrated_limit(worst);
            say(&format!(
                "  line errors: worst good clip {worst:.2} codes, so the limit is {calibrated:.2}"
            ));
            if calibrated != LINES_FLOOR_CODES {
                // The good clips are judged, and reported, under the limit the bad ones get.
                reports.clear();
                for entry in &good {
                    reports.push((
                        entry.file.clone(),
                        self.check_clip(entry, Some(calibrated), say)?,
                    ));
                }
            }
            (
                Some(calibrated).filter(|&l| l != LINES_FLOOR_CODES),
                Lines {
                    limit_codes: calibrated,
                    source: "max(4, 2 × the worst good clip), the PC's calibration (README §7.4)"
                        .to_owned(),
                    worst_good_codes: Some(worst),
                },
            )
        } else {
            (
                None,
                Lines {
                    limit_codes: LINES_FLOOR_CODES,
                    source: "fixed: the Mac and CI corpora keep 4 codes (README §7.4)".to_owned(),
                    worst_good_codes: None,
                },
            )
        };
        for (entry, _) in present.iter().filter(|(e, _)| !e.is_good()) {
            reports.push((entry.file.clone(), self.check_clip(entry, limit, say)?));
        }
        let decoder = reports.first().map_or("none", |(_, r)| r.decoder.kind);
        let clips: Vec<Judged> = present
            .iter()
            .filter_map(|(entry, copied)| {
                let (_, report) = reports.iter().find(|(file, _)| *file == entry.file)?;
                Some(Judged {
                    copied_in: *copied,
                    ..judge(entry, report.verdict, &report.checks)
                })
            })
            .collect();
        let complete = missing.is_empty();
        let report = CorpusReport {
            kairo_corpus_report: VERSION,
            tool: concat!("kairo-verify ", env!("CARGO_PKG_VERSION")),
            encoder: manifest.encoder.clone(),
            ffmpeg: manifest.ffmpeg.clone(),
            decoder,
            lines,
            complete,
            pass: complete && clips.iter().all(|c| c.problems.is_empty()),
            clips,
            missing,
        };
        // Written under a temporary name and moved into place: there is a judgement only once
        // every clip is judged.
        let text =
            serde_json::to_string_pretty(&report).map_err(|e| CheckError::write(REPORT, e))?;
        let partial = format!("{REPORT}.partial");
        fs::write(self.dir.join(&partial), format!("{text}\n"))
            .map_err(|e| CheckError::write(&partial, e))?;
        fs::rename(self.dir.join(&partial), self.dir.join(REPORT))
            .map_err(|e| CheckError::write(REPORT, e))?;
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
        Ok(manifest)
    }

    /// Refuses a clip whose size or hash isn't what the build recorded.
    fn verify(&self, entry: &Entry) -> Result<(), CheckError> {
        let file = &entry.file;
        let (Some(bytes), Some(hash)) = (entry.bytes, entry.fnv1a64.as_deref()) else {
            return Err(CheckError::evaluate(format!(
                "{MANIFEST}: {file} has no size and hash; rebuild the corpus"
            )));
        };
        let mut clip = File::open(self.dir.join(file))
            .map_err(|e| CheckError::evaluate(format!("{file}: {e}")))?;
        let size = clip
            .metadata()
            .map_err(|e| CheckError::evaluate(format!("{file}: {e}")))?
            .len();
        let found = fnv1a64(&mut clip).map_err(|e| CheckError::evaluate(format!("{file}: {e}")))?;
        if size != bytes || found != hash {
            return Err(CheckError::evaluate(format!(
                "{file} isn't the clip corpus build made: changed, swapped or copied over since"
            )));
        }
        Ok(())
    }

    /// Refuses to judge with a decoder that can't decode the first good clip's first picture:
    /// a missing or unsuitable FFmpeg isn't a corpus that misbehaves.
    fn decoder_starts(&self, present: &[(Entry, bool)]) -> Result<(), CheckError> {
        let Some((first, _)) = present.iter().find(|(e, _)| e.is_good()) else {
            return Err(CheckError::evaluate(format!(
                "{MANIFEST}: no good clip to judge the others by"
            )));
        };
        let mut decoder = (self.decoder)(&self.dir.join(&first.file));
        match decoder.video(&mut |_, _| ControlFlow::Break(())) {
            Err(error @ DecodeError::Start(_)) => Err(CheckError::evaluate(format!(
                "the decoder didn't start on {}: {error}",
                first.file
            ))),
            _ => Ok(()),
        }
    }

    /// One clip checked with `--profile test`, its expect file and the line-error `limit`
    /// (`None`: the proposed 4 codes); its report written next to it.
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
        fs::write(self.dir.join(&name), report.to_json() + "\n")
            .map_err(|e| CheckError::write(&name, e))?;
        // A clip whose size and hash checked out can't be judged only for a reason outside it.
        if let Some(error) = &report.error {
            return Err(CheckError::evaluate(format!("{}: {error}", entry.file)));
        }
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
            let copied = if clip.copied_in { " (copied in)" } else { "" };
            out += &format!("  {mark} {:width$}  {failed}{copied}", clip.file);
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
        } else if bad > 0 {
            format!("FAIL: {bad} of {} clips misbehave\n", self.clips.len())
        } else {
            format!(
                "INCOMPLETE: {} clips behave, {} missing\n",
                self.clips.len(),
                self.missing.len()
            )
        };
        out
    }
}
