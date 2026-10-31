// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify`: checks Kairo's clips (docs/research/16-verify-testbed/, M1).
//!
//! This version reads the container and the H.264 and AAC headers with its own parser and runs
//! the structure checks: moov first, no edit list, `colr`, CFR, start alignment, no reordering,
//! IDR at 0, the closed 1 s GOP, the track layout, AAC LC at 48 kHz, durations, one encoder
//! session, the bitrate, and the export limits that need no decoding (README §6.2, §6.5). The
//! checks that need decoded pictures and audio come with research 16's PRs 4 to 6. Until then,
//! a clip that should hold the testbed pattern (profiles `test`, `capture` and `smoke`, or an
//! expect file that names a pattern) ends in `error`, and so does `--gate m3`, whose list holds
//! A/V: such a clip can't pass on structure alone. Other `engine` and export checks, as for a
//! real game clip, are structure only.
//!
//! It needs no FFmpeg and no `ffprobe`, so it runs on the Mac, on Linux CI and, later, inside
//! the engine for its post-save check (PLAN:179) and in "Test my setup" (PLAN:271). Reports
//! name the file only, never its folder (CLAUDE.md).
//!
//! [`mutate`] makes the bad clips that are one box-level change from a good one (README §7.3):
//! moov after the media, an edit list, a wrong or missing `colr`, a second default track, and
//! one uneven frame.

#![forbid(unsafe_code)]

pub mod aac;
pub mod boxes;
pub mod checks;
pub mod cli;
pub mod expect;
pub mod h264;
pub mod mp4;
pub mod mutate;
pub mod profile;
pub mod report;
pub mod scan;
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;

use std::time::Instant;

use crate::mp4::tracks::{MAX_VIDEO_SAMPLES, Movie, Sample, SampleEntry};
use crate::mp4::{Mp4, Problem, ReadError, Source};
use crate::profile::Plan;
pub use crate::report::{Report, Verdict};

/// A clip as read: its boxes, tracks and samples, every structural problem (check
/// `container.structure`), and the NAL scan of its H.264 track.
#[derive(Debug, Clone)]
pub struct Clip {
    /// The boxes.
    pub mp4: Mp4,
    /// The movie, if there is exactly one `moov`.
    pub movie: Option<Movie>,
    /// Every structural problem.
    pub problems: Vec<Problem>,
    /// The H.264 track's samples (empty where its tables disagree). Other tracks are judged
    /// from their tables, so their samples are never laid out one by one.
    pub video_samples: Vec<Sample>,
    /// The index of the first `vide` track whose sample entry is `avc1`.
    pub video: Option<usize>,
    /// That track's `avcC`, if it parses.
    pub avcc: Option<h264::AvcC>,
    /// That track's NAL scan, run only when the structure is sound.
    pub scan: Option<scan::Scan>,
}

impl Clip {
    /// Reads `source`. Only an I/O failure is an error; problems with the file are recorded.
    pub fn read(source: &mut dyn Source) -> Result<Clip, ReadError> {
        let mp4 = Mp4::read(source)?;
        let mut problems = mp4.problems.clone();
        let movie = Movie::read(&mp4, &mut problems);
        problems.extend(checks::structure_problems(&mp4, movie.as_ref()));
        let video = movie.as_ref().and_then(|m| {
            m.tracks.iter().position(|t| {
                t.handler().is_some_and(|h| h.0 == *b"vide")
                    && matches!(t.entries.first(), Some(SampleEntry::Visual { kind, .. }) if kind.0 == *b"avc1")
            })
        });
        let mut video_samples = Vec::new();
        if let Some(track) = video.and_then(|v| movie.as_ref()?.tracks.get(v)) {
            if track.sizes.len() > MAX_VIDEO_SAMPLES {
                problems.push(Problem {
                    path: track.path.clone(),
                    offset: track.stbl_offset,
                    what: format!("more than {MAX_VIDEO_SAMPLES} video samples: too large"),
                });
            } else if let Ok(samples) = track.samples() {
                // A table disagreement is already a structure problem.
                video_samples = samples;
            }
        }
        let avcc = video
            .and_then(|v| movie.as_ref()?.tracks.get(v)?.entries.first())
            .and_then(|e| match e {
                SampleEntry::Visual { avcc, .. } => avcc.first().and_then(|a| h264::parse_avcc(a)),
                _ => None,
            });
        let scan = match (video, avcc.as_ref()) {
            (Some(_), Some(config)) if problems.is_empty() => {
                Some(scan::scan(source, &video_samples, config).map_err(ReadError::Io)?)
            }
            _ => None,
        };
        Ok(Clip {
            mp4,
            movie,
            problems,
            video_samples,
            video,
            avcc,
            scan,
        })
    }

    /// Whether it looks like an MP4 at all: an `ftyp` or a `moov` at the top level.
    pub fn looks_like_mp4(&self) -> bool {
        self.mp4
            .top
            .iter()
            .any(|b| matches!(&b.kind.0, b"ftyp" | b"moov"))
    }
}

/// Checks the clip in `source`, called `name` in the report, under `plan`.
pub fn check(source: &mut dyn Source, name: &str, plan: &Plan) -> Report {
    let started = Instant::now();
    let mut report = Report {
        kairo_verify: report::VERSION,
        tool: env!("CARGO_PKG_VERSION"),
        pattern: plan.expect.as_ref().and_then(|e| e.pattern),
        profile: plan.profile.name(),
        gate: plan.gate_m3.then_some("m3"),
        verdict: Verdict::Error,
        error: None,
        file: report::FileInfo {
            name: name.to_owned(),
            bytes: 0,
        },
        decoder: report::Decoder { kind: "none" },
        summary: report::Summary::default(),
        checks: Vec::new(),
        timings_ms: report::Timings::default(),
    };
    let clip = match Clip::read(source) {
        Ok(clip) => clip,
        Err(error) => {
            report.error = Some(format!("unreadable: {error}"));
            return report;
        }
    };
    let parsed = started.elapsed();
    report.file.bytes = clip.mp4.file_len;
    report.timings_ms.parse = parsed.as_millis() as u64;
    if !clip.looks_like_mp4() {
        report.error = Some("not an MP4: no ftyp or moov box".to_owned());
        return report;
    }
    if clip.problems.is_empty() && clip.video.is_none() {
        report.error = Some("no H.264 (avc1) video track".to_owned());
        return report;
    }
    report.checks = checks::run(&clip, plan, clip.video);
    report.verdict = checks::verdict(&report.checks);
    report.summary = checks::summarize(&clip, plan, clip.video, &report.checks);
    report.timings_ms.analysis = started.elapsed().saturating_sub(parsed).as_millis() as u64;
    report
}
