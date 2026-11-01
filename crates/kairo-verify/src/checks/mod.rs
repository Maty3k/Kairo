// SPDX-License-Identifier: GPL-3.0-or-later
//! The checks (docs/research/16-verify-testbed/README.md §6.2, container.md §6) and how a
//! profile weighs them. Each check measures and lists its violations as failures or warnings;
//! the profile's [`Mode`] then decides whether the clip fails. `container.structure` gates the
//! rest: when it fails, they report `skip`.

mod audio;
mod container;
mod content;
mod export;
mod source;
mod tracks;
mod video;

pub(crate) use video::level_needed;

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use crate::Clip;
use crate::h264::{self, Pps, Sps};
use crate::mp4::tracks::{Movie, Sample, SampleEntry, Track};
use crate::profile::{self, Content, Mode, Plan};
use crate::report::{Summary, Verdict};

pub(crate) use container::structure_problems;
pub use content::Decoded;
pub(crate) use content::decode;

/// Each audio track's channel count, as its sample entry gives it (at least 1), and its packet
/// count, in file order.
pub(crate) fn audio_tracks(clip: &Clip) -> Vec<(usize, usize)> {
    clip.movie
        .iter()
        .flat_map(|m| &m.tracks)
        .filter(|t| t.handler().is_some_and(|h| h.0 == *b"soun"))
        .map(|t| {
            let channels = match t.entries.first() {
                Some(SampleEntry::Audio { channels, .. }) => usize::from(*channels).max(1),
                _ => 1,
            };
            (channels, t.sizes.len())
        })
        .collect()
}

/// How many violations a finding lists; the count covers the rest.
pub const MAX_EVENTS: usize = 100;

/// A check's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Nothing wrong.
    Pass,
    /// A violation the profile counts.
    Fail,
    /// A violation the profile only reports.
    Warn,
    /// Not run, with the reason in `detail`.
    Skip,
    /// Required but unable to run.
    Error,
}

/// One check in the report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// Its id, e.g. `video.gop`.
    pub id: &'static str,
    /// The result.
    pub status: Status,
    /// Whether a failure here fails the clip.
    pub required: bool,
    /// One line: the first violation, the reason it didn't run, or what was measured.
    pub detail: String,
    /// What was measured.
    pub value: Value,
    /// What was expected, with its source.
    pub limit: Value,
    /// Every violation, failures first, up to [`MAX_EVENTS`].
    pub events: Vec<String>,
    /// How many violations there were in all.
    pub events_total: usize,
}

/// What a check found, before the profile weighs it.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    /// Violations that fail a required check.
    pub fails: Vec<String>,
    /// Violations that only ever warn.
    pub warns: Vec<String>,
    /// Set when the check couldn't apply, with the reason.
    pub skip: Option<String>,
    /// Set when the check applies but couldn't be computed, with the reason: an error when it
    /// is required (README §6.1), a skip otherwise.
    pub cannot: Option<String>,
    /// What was measured, in a few words.
    pub detail: String,
    /// What was measured, in full.
    pub value: Value,
    /// What was expected.
    pub limit: Value,
}

impl Outcome {
    /// A check that doesn't apply to this clip.
    pub fn skip(reason: impl Into<String>) -> Self {
        Self {
            skip: Some(reason.into()),
            ..Self::default()
        }
    }

    /// A check that applies but couldn't be computed.
    pub fn cannot(reason: impl Into<String>) -> Self {
        Self {
            cannot: Some(reason.into()),
            ..Self::default()
        }
    }

    /// Records a failure when `ok` is false.
    pub fn require(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if !ok {
            self.fails.push(what());
        }
    }

    /// Records a warning when `ok` is false.
    pub fn prefer(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if !ok {
            self.warns.push(what());
        }
    }

    /// Records a violation as a failure or a warning.
    pub fn violation(&mut self, fail: bool, what: String) {
        if fail {
            self.fails.push(what);
        } else {
            self.warns.push(what);
        }
    }
}

/// What the checks share: the clip as read, its H.264 track and the plan.
pub(crate) struct Ctx<'a> {
    pub clip: &'a Clip,
    pub movie: &'a Movie,
    pub plan: &'a Plan,
    /// The H.264 track.
    pub video: &'a Track,
    /// Its samples.
    pub samples: &'a [Sample],
    /// Its SPS and PPS from `avcC`, if they parse.
    pub sps: Option<Sps>,
    pub pps: Option<Pps>,
    /// Its timing.
    pub timing: Timing,
}

/// The video timing every check agrees on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    /// The media timescale T.
    pub timescale: u64,
    /// One frame's ticks D: T / fps when the rate is known and divides T, else sample 2's.
    pub delta: Option<u64>,
    /// Frames per second, as T / D in lowest terms (numerator, denominator).
    pub fps: Option<(u64, u64)>,
    /// The first sample's ticks beyond D: the held first frame (cookbook P8).
    pub extension: Option<i128>,
    /// The rate the samples themselves show, whatever the plan expects.
    pub measured: Option<Measured>,
}

/// The frame rate read from the samples alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Measured {
    /// The most common duration among samples 2…N (the shorter one on a tie).
    pub delta: u64,
    /// T / that duration in lowest terms.
    pub fps: (u64, u64),
    /// How many of samples 2…N last anything else.
    pub others: usize,
}

impl Measured {
    fn from(timescale: u64, samples: &[Sample]) -> Option<Measured> {
        let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
        for s in samples.iter().skip(1) {
            *counts.entry(s.duration).or_insert(0) += 1;
        }
        // BTreeMap iterates shortest first, and max_by_key keeps the last maximum: reverse it
        // so the shortest of equally common durations wins.
        let (&delta, &count) = counts.iter().rev().max_by_key(|&(_, &n)| n)?;
        let delta = u64::from(delta);
        if delta == 0 || timescale == 0 {
            return None;
        }
        let g = gcd(timescale, delta);
        Some(Measured {
            delta,
            fps: (timescale / g, delta / g),
            others: samples.len() - 1 - count,
        })
    }
}

impl Timing {
    /// The frame rate when it is a whole number.
    pub fn whole_fps(&self) -> Option<u64> {
        self.fps.and_then(|(n, d)| (d == 1).then_some(n))
    }

    /// One GOP in frames, when the rate is a whole number.
    pub fn gop_frames(&self) -> Option<u64> {
        self.whole_fps()
            .map(|fps| fps * u64::from(profile::GOP_SECONDS))
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl<'a> Ctx<'a> {
    fn new(clip: &'a Clip, movie: &'a Movie, plan: &'a Plan, video: usize) -> Option<Self> {
        let track = movie.tracks.get(video)?;
        let samples = clip.video_samples.as_slice();
        let avcc = clip.avcc.as_ref();
        let sps = avcc
            .and_then(|a| a.sps.first())
            .and_then(|n| h264::parse_sps(n));
        let pps = avcc
            .and_then(|a| a.pps.first())
            .and_then(|n| h264::parse_pps(n));
        let timescale = track.mdhd.map_or(0, |m| u64::from(m.timescale));
        let delta = match plan.fps {
            Some(fps) if fps > 0 => {
                (timescale % u64::from(fps) == 0).then(|| timescale / u64::from(fps))
            }
            _ => samples
                .get(1)
                .or(samples.first())
                .map(|s| u64::from(s.duration)),
        }
        .filter(|&d| d > 0);
        let fps = delta.filter(|_| timescale > 0).map(|d| {
            let g = gcd(timescale, d);
            (timescale / g, d / g)
        });
        let extension = delta
            .zip(samples.first())
            .map(|(d, s)| i128::from(s.duration) - i128::from(d));
        let measured = Measured::from(timescale, samples);
        Some(Self {
            clip,
            movie,
            plan,
            video: track,
            samples,
            sps,
            pps,
            timing: Timing {
                timescale,
                delta,
                fps,
                extension,
                measured,
            },
        })
    }

    /// The audio tracks, in file order.
    pub fn audio(&self) -> impl Iterator<Item = (usize, &'a Track)> {
        self.movie
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler().is_some_and(|h| h.0 == *b"soun"))
    }

    /// The video sample entry.
    pub fn entry(&self) -> Option<&'a SampleEntry> {
        self.video.entries.first()
    }
}

/// Weighs an outcome by `mode`.
fn weigh(id: &'static str, outcome: Outcome, mode: Mode) -> Finding {
    let required = mode == Mode::Required;
    let events_total = outcome.fails.len() + outcome.warns.len();
    let events: Vec<String> = outcome
        .fails
        .iter()
        .chain(&outcome.warns)
        .take(MAX_EVENTS)
        .cloned()
        .collect();
    let (status, detail) = if let Some(reason) = outcome.skip {
        (Status::Skip, reason)
    } else if let Some(reason) = outcome.cannot {
        (
            if required {
                Status::Error
            } else {
                Status::Skip
            },
            reason,
        )
    } else if let Some(first) = outcome.fails.first() {
        let status = if required { Status::Fail } else { Status::Warn };
        (status, first.clone())
    } else if let Some(first) = outcome.warns.first() {
        (Status::Warn, first.clone())
    } else {
        (Status::Pass, outcome.detail)
    };
    Finding {
        id,
        status,
        required,
        detail,
        value: outcome.value,
        limit: outcome.limit,
        events,
        events_total,
    }
}

fn skipped(id: &'static str, required: bool, reason: String) -> Finding {
    Finding {
        id,
        status: Status::Skip,
        required,
        detail: reason,
        value: Value::Null,
        limit: Value::Null,
        events: Vec::new(),
        events_total: 0,
    }
}

/// Runs every check on `clip` under `plan`, the content checks on `decoded` when a decoder ran.
/// `video` is the index of the H.264 track, which [`crate::check`] has made sure exists unless
/// the structure failed.
pub(crate) fn run(
    clip: &Clip,
    plan: &Plan,
    video: Option<usize>,
    decoded: Option<&Decoded>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let structure_ok = clip.problems.is_empty();
    let ctx = clip
        .movie
        .as_ref()
        .zip(video)
        .and_then(|(movie, v)| Ctx::new(clip, movie, plan, v));
    for id in profile::STRUCTURE_CHECKS
        .into_iter()
        .chain(profile::EXPORT_CHECKS)
    {
        let mode = plan.mode(id);
        let required = mode == Mode::Required;
        if mode == Mode::Off {
            findings.push(skipped(
                id,
                false,
                format!("not in the {} profile", plan.profile),
            ));
            continue;
        }
        if id == "container.structure" {
            findings.push(weigh(id, container::structure(clip), mode));
            continue;
        }
        let Some(ctx) = ctx.as_ref().filter(|_| structure_ok) else {
            findings.push(skipped(
                id,
                required,
                "container.structure failed".to_owned(),
            ));
            continue;
        };
        let outcome = match id {
            "container.faststart" => container::faststart(ctx),
            "container.no_editlist" => container::no_editlist(ctx),
            "video.colr" => video::colr(ctx),
            "video.params" => video::params(ctx),
            "video.cfr" => video::cfr(ctx),
            "av.start_alignment" => tracks::start_alignment(ctx),
            "video.no_reorder" => video::no_reorder(ctx),
            "video.idr_start" => video::idr_start(ctx),
            "video.gop" => video::gop(ctx),
            "tracks.layout" => tracks::layout(ctx),
            "audio.format" => tracks::audio_format(ctx),
            "duration" => tracks::duration(ctx),
            "video.single_session" => video::single_session(ctx),
            "video.bitrate" => video::bitrate(ctx),
            "export.size" => export::size(ctx),
            "export.shape" => export::shape(ctx),
            "export.tags" => export::tags(ctx),
            _ => Outcome::skip("unknown check"),
        };
        findings.push(weigh(id, outcome, mode));
    }
    findings.extend(content::findings(
        ctx.as_ref().filter(|_| structure_ok),
        plan,
        decoded,
    ));
    findings
}

/// Why a linked FFmpeg can't be used, if it can't: one of its libraries isn't LGPL (README §2
/// item 18).
pub(crate) fn not_lgpl(ffmpeg: &crate::decode::LinkedFfmpeg) -> Option<String> {
    (!ffmpeg.lgpl).then(|| format!("the linked FFmpeg is {}, not LGPL", ffmpeg.license))
}

/// `build.lgpl` (README §2 item 18, §6.3): the FFmpeg linked into this process, if any, must be
/// LGPL, whichever decoder ran; a spawned one's licence binds nothing and is only reported.
/// `kind` is the decoder that ran, if one did, and `spawned` its licence. A linked FFmpeg that
/// isn't LGPL has already stopped the check before analysis, so here it can only be an `error`.
pub(crate) fn lgpl_finding(
    required: bool,
    linked: Option<&crate::decode::LinkedFfmpeg>,
    kind: Option<&'static str>,
    spawned: Option<&'static str>,
) -> Finding {
    let id = "build.lgpl";
    let base = || Finding {
        limit: json!({"licence": "LGPL", "source": "CLAUDE.md, M1:12, README §2 item 18"}),
        ..skipped(id, required, String::new())
    };
    match linked {
        Some(ffmpeg) => Finding {
            status: if ffmpeg.lgpl {
                Status::Pass
            } else if required {
                Status::Error
            } else {
                Status::Skip
            },
            detail: not_lgpl(ffmpeg).unwrap_or_else(|| match (kind, spawned) {
                (Some(kind), Some(licence)) if kind != "linked" => format!(
                    "linked and LGPL ({}); the spawned {kind} decoder's FFmpeg: {licence}",
                    ffmpeg.license
                ),
                _ => format!("linked and LGPL ({})", ffmpeg.license),
            }),
            value: json!({"linked": true, "decoder": kind, "licence": ffmpeg.license,
                          "libs": ffmpeg.libs, "config_sha256": ffmpeg.config_sha256,
                          "spawned_licence": spawned.filter(|_| kind != Some("linked"))}),
            ..base()
        },
        None => Finding {
            status: Status::Pass,
            detail: match (kind, spawned) {
                (Some(kind), Some(licence)) => {
                    format!("spawned, not linked: the {kind} decoder ({licence})")
                }
                (Some(kind), None) => format!("nothing linked: the {kind} decoder"),
                (None, _) => "nothing linked: no decoder ran".to_owned(),
            },
            value: json!({"linked": false, "decoder": kind, "licence": spawned}),
            ..base()
        },
    }
}

/// The content checks when no decoder ran: a required one is an `error`, so a profile that
/// needs them can't pass on structure alone (README §6.1).
fn content_findings(plan: &Plan) -> Vec<Finding> {
    let linked = crate::decode::linked_ffmpeg();
    profile::CONTENT_CHECKS
        .into_iter()
        .map(|id| {
            let mode = plan.content_mode(id);
            let required = mode == Mode::Required;
            if id == "build.lgpl" {
                return lgpl_finding(required, linked.as_ref(), None, None);
            }
            match mode {
                Mode::Off => skipped(id, false, format!("not in the {} profile", plan.profile)),
                // Under --gate m3, A/V is on the gate's own list: without a decoder the gate
                // can't be judged, whatever the clip holds.
                Mode::Required if plan.content == Content::Required || plan.gate_m3 => Finding {
                    status: Status::Error,
                    detail: "needs decoded frames or audio, and no decoder ran".to_owned(),
                    ..skipped(id, true, String::new())
                },
                _ => skipped(
                    id,
                    required,
                    "structure only: no decoder ran to look for the testbed pattern".to_owned(),
                ),
            }
        })
        .collect()
}

/// The top-level box types in order, with repeats collapsed and the list kept short, e.g.
/// `ftyp moov (moof mdat)×120 mfra`.
pub(crate) fn box_order(kinds: &[String]) -> String {
    const SHOWN: usize = 12;
    let at = |i: usize| kinds.get(i).map(String::as_str);
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < kinds.len() {
        // A pair that repeats: (a b)×n.
        let pair = at(i) != at(i + 1)
            && at(i) == at(i + 2)
            && at(i + 1) == at(i + 3)
            && at(i + 3).is_some();
        if pair {
            let mut n = 2;
            while at(i + 2 * n).is_some()
                && at(i + 2 * n) == at(i)
                && at(i + 2 * n + 1) == at(i + 1)
            {
                n += 1;
            }
            parts.push(format!("({} {})×{n}", kinds[i], kinds[i + 1]));
            i += 2 * n;
            continue;
        }
        let mut n = 1;
        while at(i + n) == at(i) {
            n += 1;
        }
        parts.push(if n > 1 {
            format!("{}×{n}", kinds[i])
        } else {
            kinds[i].clone()
        });
        i += n;
    }
    if parts.len() > SHOWN {
        let more = parts.len() - SHOWN;
        parts.truncate(SHOWN);
        parts.push(format!("… ({more} more)"));
    }
    parts.join(" ")
}

/// The summary lines: container, video, audio and content.
pub(crate) fn summarize(
    clip: &Clip,
    plan: &Plan,
    video: Option<usize>,
    findings: &[Finding],
) -> Summary {
    let order: Vec<String> = clip.mp4.top.iter().map(|b| b.kind.to_string()).collect();
    let mut summary = Summary {
        container: format!(
            "{}; {} tracks",
            box_order(&order),
            clip.movie.as_ref().map_or(0, |m| m.tracks.len())
        ),
        content: content::summary(findings)
            .unwrap_or_else(|| "not checked: no decoder ran".to_owned()),
        ..Summary::default()
    };
    let Some(ctx) = clip
        .movie
        .as_ref()
        .zip(video)
        .and_then(|(movie, v)| Ctx::new(clip, movie, plan, v))
    else {
        return summary;
    };
    let value = |id: &str, key: &str| {
        findings
            .iter()
            .find(|f| f.id == id)
            .and_then(|f| f.value.get(key))
            .and_then(Value::as_u64)
    };
    // The rate the samples show, not the one the plan expects (video.cfr compares the two).
    let rate = |(n, d): (u64, u64)| {
        if d == 1 {
            n.to_string()
        } else {
            format!("{n}/{d}")
        }
    };
    let fps = match ctx.timing.measured {
        Some(m) if m.others == 0 => format!("CFR {}", rate(m.fps)),
        Some(m) => format!(
            "VFR (mostly {} fps, {} other durations)",
            rate(m.fps),
            m.others
        ),
        None => "no rate".to_owned(),
    };
    let size = ctx
        .sps
        .as_ref()
        .map_or("?".to_owned(), |s| format!("{}x{}", s.width, s.height));
    let profile = match ctx.sps.as_ref().map(|s| s.profile_idc) {
        Some(100) => "High".to_owned(),
        Some(p) => format!("profile {p}"),
        None => "?".to_owned(),
    };
    let peak = value("video.bitrate", "peak_bps")
        .map(|b| format!(", peak {:.1} Mbps", b as f64 / 1e6))
        .unwrap_or_default();
    summary.video = format!(
        "h264 {profile} {size} {fps}, {} frames{peak}",
        ctx.samples.len()
    );
    let audio: Vec<String> = ctx
        .audio()
        .map(|(_, t)| {
            let title = t.hdlr.as_ref().map_or("?", |h| h.name.as_str());
            match t.entries.first() {
                Some(SampleEntry::Audio {
                    kind,
                    channels,
                    sample_rate,
                    ..
                }) => {
                    format!("{title} {kind} {channels}ch {} kHz", sample_rate / 1000)
                }
                _ => format!("{title} (no audio entry)"),
            }
        })
        .collect();
    summary.audio = if audio.is_empty() {
        "none".to_owned()
    } else {
        audio.join(", ")
    };
    if plan.profile == profile::Profile::Original {
        // PLAN:249: faststart, H.264 + AAC, and no voice chat.
        let faststart = findings
            .iter()
            .any(|f| f.id == "container.faststart" && f.status == Status::Pass);
        let all_aac = ctx.audio().all(|(_, t)| matches!(t.entries.first(), Some(SampleEntry::Audio { kind, .. }) if kind.0 == *b"mp4a"));
        let voice = ctx.audio().any(|(_, t)| {
            t.hdlr.as_ref().is_some_and(|h| h.name == "Voice chat")
                || t.udta_name.as_deref() == Some("Voice chat")
        });
        summary.shareable_original = Some(faststart && all_aac && !voice);
    }
    summary
}

/// The verdict: `fail` if any required check fails, else `error` if any required check
/// couldn't run, else `pass` (README §6.6).
pub fn verdict(findings: &[Finding]) -> Verdict {
    if findings
        .iter()
        .any(|f| f.required && f.status == Status::Fail)
    {
        Verdict::Fail
    } else if findings.iter().any(|f| f.status == Status::Error) {
        Verdict::Error
    } else {
        Verdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Libs, LinkedFfmpeg};

    fn ffmpeg(license: &'static str) -> LinkedFfmpeg {
        LinkedFfmpeg {
            libs: Libs {
                avutil: "60.8.100".to_owned(),
                avcodec: "62.11.100".to_owned(),
                avformat: "62.3.100".to_owned(),
            },
            config_sha256: "ab".repeat(32),
            license,
            lgpl: license.starts_with("LGPL"),
        }
    }

    #[test]
    fn build_lgpl_judges_a_linked_ffmpeg_whichever_decoder_ran() {
        // Linked and LGPL: a pass that names the build, with any decoder or none.
        let lgpl = ffmpeg("LGPL version 3 or later");
        for (kind, spawned) in [
            (Some("linked"), Some("LGPL version 3 or later")),
            (Some("cli"), Some("GPL version 3 or later")),
            (None, None),
        ] {
            let f = lgpl_finding(true, Some(&lgpl), kind, spawned);
            assert_eq!(f.status, Status::Pass, "{kind:?}");
            assert!(
                f.detail
                    .starts_with("linked and LGPL (LGPL version 3 or later)"),
                "{}",
                f.detail
            );
            assert_eq!(f.value["linked"], true);
            assert_eq!(f.value["libs"]["avcodec"], "62.11.100");
            assert_eq!(f.value["config_sha256"].as_str().map(str::len), Some(64));
        }
        // A spawned FFmpeg beside the linked one is still reported (README §2 item 18).
        let both = lgpl_finding(
            true,
            Some(&lgpl),
            Some("cli"),
            Some("GPL version 3 or later"),
        );
        assert!(
            both.detail
                .ends_with("the spawned cli decoder's FFmpeg: GPL version 3 or later")
        );
        assert_eq!(both.value["spawned_licence"], "GPL version 3 or later");
        // A GPL or non-free build can't judge clips: an error, whichever decoder ran.
        let gpl = ffmpeg("GPL version 3 or later");
        assert_eq!(
            not_lgpl(&gpl).as_deref(),
            Some("the linked FFmpeg is GPL version 3 or later, not LGPL")
        );
        assert_eq!(not_lgpl(&lgpl), None);
        for kind in [Some("linked"), Some("cli"), None] {
            let f = lgpl_finding(true, Some(&gpl), kind, None);
            assert_eq!(f.status, Status::Error, "{kind:?}");
            assert_eq!(f.value["linked"], true);
        }
        assert_eq!(
            lgpl_finding(false, Some(&gpl), None, None).status,
            Status::Skip
        );
        // Nothing linked (the Mac and Linux): a spawned FFmpeg's licence is only reported.
        let spawned = lgpl_finding(true, None, Some("cli"), Some("GPL version 3 or later"));
        assert_eq!(spawned.status, Status::Pass);
        assert_eq!(
            spawned.detail,
            "spawned, not linked: the cli decoder (GPL version 3 or later)"
        );
        assert_eq!(spawned.value["linked"], false);
        let none = lgpl_finding(true, None, None, None);
        assert_eq!(none.status, Status::Pass);
        assert_eq!(none.detail, "nothing linked: no decoder ran");
        let synthetic = lgpl_finding(true, None, Some("synthetic"), None);
        assert_eq!(synthetic.detail, "nothing linked: the synthetic decoder");
    }

    #[test]
    fn findings_list_at_most_100_events_and_count_them_all() {
        let outcome = Outcome {
            fails: (0..150).map(|i| format!("fail {i}")).collect(),
            warns: vec!["a warning".to_owned()],
            ..Outcome::default()
        };
        let finding = weigh("x", outcome, Mode::Required);
        assert_eq!(
            (finding.events.len(), finding.events_total),
            (MAX_EVENTS, 151)
        );
        assert_eq!(
            (finding.status, finding.detail.as_str()),
            (Status::Fail, "fail 0")
        );
        // A warning-only check turns its failures into warnings.
        let soft = weigh(
            "x",
            Outcome {
                fails: vec!["f".into()],
                ..Outcome::default()
            },
            Mode::Warning,
        );
        assert_eq!((soft.status, soft.required), (Status::Warn, false));
    }
}
