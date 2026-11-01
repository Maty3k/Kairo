// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify corpus` (docs/research/16-verify-testbed/README.md §7): gate 1's good and bad
//! clips, made with one encoder and judged against their manifest.
//!
//! `corpus build` makes every clip of §7.1–7.3 in a new folder. Each video variant is one
//! encoder session into a video-only MP4; each clip is then muxed from one of them with its
//! WAVs and no encoder session, which applies cookbook P8's held first frame once (§7.2); the
//! container defects are [`crate::mutate`] edits of the finished good clip. `corpus check` runs
//! `check --profile test` on every clip with its expect file and applies §7.4's rules.

#[cfg(feature = "dev-tools")]
pub mod build;
#[cfg(feature = "dev-tools")]
pub mod check;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use kairo_core::testsig::Curve;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Verdict;
use crate::checks::{Finding, Status};
use crate::mutate::Op;
use crate::synth::{Codec, Defect, Format, HdrSource, Synth};

/// The manifest's file name in a corpus folder.
pub const MANIFEST: &str = "manifest.json";

/// `corpus check`'s report in a corpus folder.
pub const REPORT: &str = "corpus-report.json";

/// The `kairo_corpus` version this build writes and reads.
pub const VERSION: u32 = 1;

/// The first frame's hold every clip but `bframes` must have, in ticks at T = 15,360: an
/// audio lead of 1,024 samples (README §7.2, E = 328).
pub const HOLD_TICKS: i64 = 328;

/// The video timescale [`HOLD_TICKS`] is in: 256 ticks a frame at 60 fps.
pub const TIMESCALE: u32 = 15_360;

/// How close `av50` must read to +50 ms, in ms (README §7.4).
pub const AV_TOLERANCE_MS: f64 = 1.0;

/// The line-error limit the Mac and CI corpora keep, and the floor of the PC's calibration.
pub const LINES_FLOOR_CODES: f64 = crate::profile::BARS_LINES_MAX_CODES;

/// The check the PC corpus calibrates.
pub const CALIBRATED: &str = "content.bars_lines";

/// A corpus's manifest (README §7.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The format version: [`VERSION`].
    pub kairo_corpus: u32,
    /// The test pattern's version.
    pub pattern: u32,
    /// The encoder: `h264_amf`, `h264_mf`, `h264_videotoolbox` or `ipcm`.
    pub encoder: String,
    /// The FFmpeg that built it, from the first line of `ffmpeg -version`.
    pub ffmpeg: String,
    /// Steps applied to every encode, such as README §7.2's `h264_metadata` colour fix.
    pub finishing: Vec<String>,
    /// AMF's adapter: the index asked for and the vendor:device FFmpeg reported using.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<Adapter>,
    /// Per check, a limit the corpus sets for itself.
    pub calibration: BTreeMap<String, Calibration>,
    /// The clips, in the order they are judged.
    pub clips: Vec<Entry>,
    /// Clips this encoder couldn't make, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<Missing>,
}

/// The adapter an AMF corpus was encoded on (README §7.2): never assumed, always read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adapter {
    /// The DXGI index the builder asked for (`--hw-index`).
    pub index: u32,
    /// The vendor:device FFmpeg's log names, e.g. `1002:7550`.
    pub device: String,
}

/// A limit a corpus sets for itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    /// The limit in codes, or `None` until `corpus check` sets it from the good clips.
    pub limit_codes: Option<f64>,
    /// How it is set.
    pub source: String,
}

/// One clip in a manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The clip's file name; its expect file is the same name with `.expect.json` for `.mp4`.
    pub file: String,
    /// `pass` for a good clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
    /// The profile it is checked with: `test`.
    pub profile: String,
    /// A bad clip's one defect, as made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defect: Option<String>,
    /// The checks a bad clip must fail.
    #[serde(default)]
    pub must_fail: Vec<String>,
    /// The checks it may fail besides.
    #[serde(default)]
    pub may_fail: Vec<String>,
    /// The first frame's hold it must have, in ticks at [`TIMESCALE`]; `None` for `bframes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_ticks: Option<i64>,
    /// What `av.offset` must read on every track it measures, ± [`AV_TOLERANCE_MS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub av_offset_ms: Option<f64>,
    /// What `cadence.drops_dups` must count: [skipped, repeated] testbed frames (`drop1` and
    /// `dup1`), so the two can't stand in for each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skips_repeats: Option<[u64; 2]>,
    /// The encoder that made it, where it isn't the corpus's (`bframes` on `h264_mf`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
    /// The clip's size, as built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Its FNV-1a 64 hash, as built, in hex: a clip edited, swapped or copied over since is
    /// refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fnv1a64: Option<String>,
}

impl Entry {
    /// Its expect file's name.
    pub fn expect_file(&self) -> String {
        format!("{}.expect.json", self.file.trim_end_matches(".mp4"))
    }

    /// Its report's name.
    pub fn report_file(&self) -> String {
        format!("{}.report.json", self.file.trim_end_matches(".mp4"))
    }

    /// Whether it is a good clip.
    pub fn is_good(&self) -> bool {
        self.expect.as_deref() == Some("pass")
    }
}

/// A clip the encoder couldn't make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Missing {
    /// The clip's file name.
    pub file: String,
    /// Why, and what stands in for it.
    pub why: String,
}

/// The encoders a corpus can be built with (README §7.2). Only these, `aac` and stream copies
/// are ever named to FFmpeg, so GPL encoders can't be used even by mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// AMD's AMF on the PC, from NV12 in system memory on the chosen adapter.
    Amf,
    /// Media Foundation, the PC's fallback for `bframes`.
    Mf,
    /// The Mac's VideoToolbox, standing in for AMF.
    VideoToolbox,
    /// CI's lossless I_PCM clips from [`crate::ipcm`], stream-copied.
    Ipcm,
}

impl FromStr for Encoder {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Ok(match text {
            "h264_amf" => Self::Amf,
            "h264_mf" => Self::Mf,
            "h264_videotoolbox" => Self::VideoToolbox,
            "ipcm" => Self::Ipcm,
            _ => {
                return Err(
                    "--encoder takes h264_amf, h264_mf, h264_videotoolbox or ipcm".to_owned(),
                );
            }
        })
    }
}

impl fmt::Display for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Amf => "h264_amf",
            Self::Mf => "h264_mf",
            Self::VideoToolbox => "h264_videotoolbox",
            Self::Ipcm => "ipcm",
        })
    }
}

impl Encoder {
    /// Whether it is a hardware encoder, whose sessions run one at a time ≥1.5 s apart.
    pub fn is_hardware(self) -> bool {
        matches!(self, Self::Amf | Self::Mf)
    }

    /// Whether the PC calibrates its line-error limit from the good clips (README §7.4).
    pub fn calibrates(self) -> bool {
        matches!(self, Self::Amf | Self::Mf)
    }

    /// The clips' length: 30 s, or 4 s for CI's lossless clips (README §7.2).
    pub fn seconds(self) -> u32 {
        match self {
            Self::Ipcm => 4,
            _ => 30,
        }
    }

    /// The `synth` settings for `video`.
    pub fn synth(self, video: Video) -> Synth {
        let (base, high) = match self {
            Self::Ipcm => {
                let small = Format {
                    width: 480,
                    height: 270,
                    fps: 60,
                };
                (small, small)
            }
            _ => (
                Format {
                    width: 1920,
                    height: 1080,
                    fps: 60,
                },
                Format {
                    width: 2560,
                    height: 1440,
                    fps: 60,
                },
            ),
        };
        let gop = match video {
            Video::Gop2s => 2 * base.fps,
            _ => base.fps,
        };
        let mut synth = Synth {
            format: base,
            source_hz: 180,
            seconds: self.seconds(),
            defect: None,
            hdr: None,
            codec: match self {
                Self::Ipcm => Codec::Ipcm { gop },
                _ => Codec::Nv12,
            },
        };
        match video {
            Video::High => {
                synth.format = high;
                synth.source_hz = 360;
            }
            Video::Hdr(curve) => {
                synth.hdr = Some(HdrSource {
                    curve,
                    lw_nits: 1_000,
                    sdr_white_nits: 240,
                });
            }
            Video::Defect(defect) => synth.defect = Some(defect),
            Video::Good | Video::Gop2s | Video::Bframes => {}
        }
        synth
    }
}

/// Where a clip's pictures come from: one encoder session each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Video {
    /// `good-1080p60`'s recipe: a 180 Hz source at Recommended's 25/40 Mbps.
    Good,
    /// `good-1440p60`'s: a 360 Hz source at High's 36/50 Mbps.
    High,
    /// The HDR patches at this curve, for a 1,000-nit display with SDR white at 240 nits.
    Hdr(Curve),
    /// The good recipe with a video defect.
    Defect(Defect),
    /// A GOP of 2 s.
    Gop2s,
    /// B-frames.
    Bframes,
}

impl Video {
    /// Its video-only file's name.
    pub fn file(self) -> String {
        format!("v-{}.mp4", self.name())
    }

    /// A short name for files and messages.
    pub fn name(self) -> String {
        match self {
            Self::Good => "good".to_owned(),
            Self::High => "high".to_owned(),
            Self::Hdr(Curve::Natural) => "hdr-natural".to_owned(),
            Self::Hdr(Curve::MatchSdrApps) => "hdr-match".to_owned(),
            Self::Defect(defect) => defect.to_string().replace('@', "-at-"),
            Self::Gop2s => "gop2s".to_owned(),
            Self::Bframes => "bframes".to_owned(),
        }
    }

    /// Its kbps on a lossy encoder: Recommended's 25/40 Mbps, or High's 36/50 for 1440p.
    pub fn rates(self) -> (&'static str, &'static str) {
        match self {
            Self::High => ("36M", "50M"),
            _ => ("25M", "40M"),
        }
    }
}

/// How a clip is made.
#[derive(Debug, Clone, PartialEq)]
pub enum Make {
    /// `video` muxed with the WAVs, changed by an audio defect if one is given.
    Mux {
        /// The pictures.
        video: Video,
        /// The audio defect, if any.
        audio: Option<Defect>,
    },
    /// A box edit of the finished good clip.
    Mutate(Op),
}

/// One clip of a corpus (README §7.1, §7.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    /// Its file name, without `.mp4`.
    pub name: &'static str,
    /// How it is made.
    pub make: Make,
    /// What it should have been, for its expect file: the good, high or natural-HDR recipe.
    pub expect: Video,
    /// The checks it must fail; none for a good clip.
    pub must_fail: &'static [&'static str],
    /// The checks it may fail besides.
    pub may_fail: &'static [&'static str],
}

/// The good clip every bad one comes from.
pub const GOOD: &str = "good-1080p60";

/// VideoToolbox's `bframes` clip also fails these: its B-frames come with an irregular `stts`,
/// 57-frame GOPs and an audio lead that no longer matches (README §7.3 [Mac]).
const BFRAMES_MAY_FAIL: [&str; 3] = ["video.cfr", "video.gop", "av.start_alignment"];

impl Recipe {
    fn mux(name: &'static str, video: Video, audio: Option<Defect>) -> Self {
        Self {
            name,
            make: Make::Mux { video, audio },
            expect: Video::Good,
            must_fail: &[],
            may_fail: &[],
        }
    }

    fn fails(mut self, must: &'static [&'static str], may: &'static [&'static str]) -> Self {
        self.must_fail = must;
        self.may_fail = may;
        self
    }

    fn mutate(name: &'static str, op: Op, must: &'static [&'static str]) -> Self {
        Self {
            name,
            make: Make::Mutate(op),
            expect: Video::Good,
            must_fail: must,
            may_fail: &[],
        }
    }

    /// Its file name.
    pub fn file(&self) -> String {
        format!("{}.mp4", self.name)
    }

    /// The manifest's `defect` text, with the frame a video defect lands on.
    pub fn defect(&self, encoder: Encoder) -> Option<String> {
        match &self.make {
            Make::Mux {
                video: Video::Defect(defect),
                ..
            } => Some(match defect {
                Defect::Drop(None) => {
                    format!("drop@{}", encoder.synth(Video::Good).defect_frame())
                }
                Defect::Dup(None) => format!("dup@{}", encoder.synth(Video::Good).defect_frame()),
                other => other.to_string(),
            }),
            Make::Mux {
                audio: Some(defect),
                ..
            } => Some(defect.to_string()),
            Make::Mux {
                video: Video::Gop2s,
                ..
            } => Some(format!("gop={}", 2 * encoder.synth(Video::Good).format.fps)),
            Make::Mux {
                video: Video::Bframes,
                ..
            } => Some("bf=2".to_owned()),
            Make::Mux {
                video: Video::Hdr(Curve::MatchSdrApps),
                ..
            } => Some("hdr match, expected natural".to_owned()),
            Make::Mux { .. } => None,
            Make::Mutate(op) => Some(op.to_string()),
        }
    }

    /// Its manifest entry.
    pub fn entry(&self, encoder: Encoder) -> Entry {
        let bframes = matches!(
            self.make,
            Make::Mux {
                video: Video::Bframes,
                ..
            }
        );
        let good = self.must_fail.is_empty();
        let names = |ids: &[&str]| ids.iter().map(|&id| id.to_owned()).collect();
        Entry {
            file: self.file(),
            expect: good.then(|| "pass".to_owned()),
            profile: "test".to_owned(),
            defect: self.defect(encoder),
            must_fail: names(self.must_fail),
            may_fail: names(self.may_fail),
            hold_ticks: (!bframes).then_some(HOLD_TICKS),
            av_offset_ms: matches!(
                self.make,
                Make::Mux {
                    audio: Some(Defect::Av50),
                    ..
                }
            )
            .then_some(50.0),
            skips_repeats: match self.make {
                Make::Mux {
                    video: Video::Defect(Defect::Drop(_)),
                    ..
                } => Some([1, 0]),
                Make::Mux {
                    video: Video::Defect(Defect::Dup(_)),
                    ..
                } => Some([0, 1]),
                _ => None,
            },
            encoder: None,
            bytes: None,
            fnv1a64: None,
        }
    }

    /// Whether it is the B-frames clip, which the PC may fill from the Mac's corpus.
    pub fn is_bframes(&self) -> bool {
        matches!(
            self.make,
            Make::Mux {
                video: Video::Bframes,
                ..
            }
        )
    }
}

/// The FNV-1a 64 hash of what `reader` holds, in hex.
pub fn fnv1a64(reader: &mut dyn std::io::Read) -> std::io::Result<String> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        for &byte in &buffer[..n] {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(format!("{hash:016x}"))
}

/// The clips `manifest` must hold for its encoder, each with its recipe's entry: every recipe
/// exactly once, nothing else, and each entry's rules as this build's recipe gives them, so a
/// corpus with a clip left out, added or loosened, or built by another version, isn't judged as
/// complete. Only `bframes` on the PC may be missing instead. A manifest entry's `encoder` and
/// its size and hash are taken from it; everything else from the recipe.
pub fn expected(manifest: &Manifest) -> Result<(Encoder, Vec<Entry>), String> {
    let encoder: Encoder = manifest
        .encoder
        .parse()
        .map_err(|_| "an unknown encoder".to_owned())?;
    if manifest.pattern != 1 {
        return Err(format!(
            "pattern {}; this build makes pattern 1",
            manifest.pattern
        ));
    }
    let wanted = recipes(encoder);
    for entry in &manifest.clips {
        if !wanted.iter().any(|r| r.file() == entry.file) {
            return Err(format!("{}: not a clip of this corpus", entry.file));
        }
    }
    let mut out = Vec::new();
    for recipe in &wanted {
        let file = recipe.file();
        let listed: Vec<&Entry> = manifest.clips.iter().filter(|e| e.file == file).collect();
        let missing = manifest.missing.iter().any(|m| m.file == file);
        let mut want = recipe.entry(encoder);
        match listed.as_slice() {
            [] if missing && recipe.is_bframes() && encoder.calibrates() => {
                // The Mac's VideoToolbox clip stands in (README §7.3).
                want.encoder = Some(Encoder::VideoToolbox.to_string());
                want.may_fail = recipes(Encoder::VideoToolbox)
                    .iter()
                    .find(|r| r.is_bframes())
                    .map(|r| r.entry(Encoder::VideoToolbox).may_fail)
                    .unwrap_or_default();
            }
            [] => return Err(format!("{file}: missing from the manifest")),
            [entry] => {
                let mut rules = (*entry).clone();
                rules.encoder = want.encoder.clone();
                rules.bytes = None;
                rules.fnv1a64 = None;
                if rules != want {
                    return Err(format!(
                        "{file}: its manifest entry isn't this build's recipe; rebuild the corpus"
                    ));
                }
                let made_by_mf = entry.encoder.as_deref() == Some("h264_mf")
                    && recipe.is_bframes()
                    && encoder == Encoder::Amf;
                if entry.encoder.is_some() && !made_by_mf {
                    return Err(format!("{file}: made by another encoder"));
                }
                want.encoder = entry.encoder.clone();
                want.bytes = entry.bytes;
                want.fnv1a64 = entry.fnv1a64.clone();
            }
            _ => return Err(format!("{file}: listed twice")),
        }
        out.push(want);
    }
    Ok((encoder, out))
}

/// Every clip `encoder`'s corpus holds, good ones first: README §7.1's good clips, §7.3's gate
/// items and its extras. CI's lossless corpus has no `bframes` (README §9).
pub fn recipes(encoder: Encoder) -> Vec<Recipe> {
    use Recipe as R;
    let mut out = vec![
        R::mux(GOOD, Video::Good, None),
        R {
            expect: Video::High,
            ..R::mux("good-1440p60", Video::High, None)
        },
        R {
            expect: Video::Hdr(Curve::Natural),
            ..R::mux("good-hdr-1080p60", Video::Hdr(Curve::Natural), None)
        },
        R::mux("drop1", Video::Defect(Defect::Drop(None)), None)
            .fails(&["cadence.drops_dups"], &[]),
        R::mux("dup1", Video::Defect(Defect::Dup(None)), None).fails(&["cadence.drops_dups"], &[]),
        R::mux("judder", Video::Defect(Defect::Judder), None).fails(&["cadence.judder"], &[]),
        R::mux("av50", Video::Good, Some(Defect::Av50)).fails(&["av.offset"], &[]),
        R::mux("bars", Video::Defect(Defect::CorruptBars), None)
            .fails(&["content.bars"], &["content.bars_lines"]),
        R::mux("bframes", Video::Bframes, None).fails(&["video.no_reorder"], &BFRAMES_MAY_FAIL),
        R::mutate("nofaststart", Op::UndoFaststart, &["container.faststart"]),
        R::mutate("editlist", Op::AddElst, &["container.no_editlist"]),
        R::mutate(
            "colr",
            Op::SetColr {
                primaries: 6,
                transfer: 6,
                matrix: 6,
                full_range: false,
            },
            &["video.colr", "video.params"],
        ),
        R::mux("swapped", Video::Good, Some(Defect::SwapGameVoice)).fails(&["audio.routing"], &[]),
        R::mutate("nocolr", Op::RemoveColr, &["video.colr"]),
        R::mutate(
            "two-defaults",
            Op::SetDefault("Game".to_owned()),
            &["tracks.layout"],
        ),
        R::mutate(
            "vfr",
            Op::PerturbStts {
                sample: 100,
                ticks: 128,
            },
            &["video.cfr"],
        ),
        R::mux("gop2s", Video::Gop2s, None).fails(&["video.gop"], &[]),
        R::mux("bars-sparse", Video::Defect(Defect::BarsSparse), None)
            .fails(&["content.bars_lines"], &["content.bars"]),
        R::mux("unreadable", Video::Defect(Defect::Unreadable), None)
            .fails(&["content.barcode"], &[]),
        R::mux("cue", Video::Good, Some(Defect::Cue)).fails(&["audio.cue"], &[]),
        R::mux("leak", Video::Good, Some(Defect::Leak)).fails(&["audio.routing"], &[]),
        R::mux("noisy", Video::Good, Some(Defect::Noisy)).fails(&["audio.tone_snr"], &[]),
        R {
            expect: Video::Hdr(Curve::Natural),
            ..R::mux("curve-swap", Video::Hdr(Curve::MatchSdrApps), None)
                .fails(&["hdr.curve_levels"], &[])
        },
    ];
    if encoder == Encoder::Ipcm {
        out.retain(|r| r.name != "bframes");
    }
    out
}

/// How one clip fared against its manifest entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Judged {
    /// The clip's file name.
    pub file: String,
    /// `check`'s verdict.
    pub verdict: Verdict,
    /// The checks that failed.
    pub failed: Vec<&'static str>,
    /// What breaks §7.4's rules; empty when the clip behaves.
    pub problems: Vec<String>,
    /// Whether it was copied in from another corpus (the PC's `bframes` from the Mac's).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub copied_in: bool,
}

/// Judges one clip's findings against its entry (README §7.4): a good clip passes; a bad clip
/// fails every `must_fail` check and nothing outside `must_fail ∪ may_fail`; no check is left
/// unable to run; the first frame is held [`HOLD_TICKS`] at [`TIMESCALE`]; and `av50` reads
/// +50 ± 1 ms on every track it measures; and `drop1` and `dup1` count exactly their one skip or
/// repeat.
pub fn judge(entry: &Entry, verdict: Verdict, checks: &[Finding]) -> Judged {
    let failed: Vec<&'static str> = checks
        .iter()
        .filter(|f| f.status == Status::Fail)
        .map(|f| f.id)
        .collect();
    let mut problems = Vec::new();
    if entry.is_good() && verdict != Verdict::Pass {
        problems.push(format!("a good clip, but the verdict is {verdict}"));
    }
    for id in &entry.must_fail {
        if !failed.contains(&id.as_str()) {
            problems.push(format!("{id} didn't fail"));
        }
    }
    // An unexpected failure carries its first reason, so a corpus run's summary says why.
    for finding in checks.iter().filter(|f| f.status == Status::Fail) {
        if !entry
            .must_fail
            .iter()
            .chain(&entry.may_fail)
            .any(|m| m == finding.id)
        {
            problems.push(if finding.detail.is_empty() {
                format!("{} failed", finding.id)
            } else {
                format!("{} failed: {}", finding.id, finding.detail)
            });
        }
    }
    for finding in checks.iter().filter(|f| f.status == Status::Error) {
        problems.push(format!("{} couldn't run: {}", finding.id, finding.detail));
    }
    let value =
        |id: &str| -> Option<&Value> { checks.iter().find(|f| f.id == id).map(|f| &f.value) };
    if let Some(want) = entry.hold_ticks {
        let start = value("av.start_alignment");
        let ticks = start.and_then(|v| v.get("extension_ticks")?.as_i64());
        let ms = start.and_then(|v| v.get("hold_ms")?.as_f64());
        let at = |ticks: i64| ticks as f64 * 1_000.0 / f64::from(TIMESCALE);
        let held =
            matches!((ticks, ms), (Some(t), Some(ms)) if t == want && (ms - at(want)).abs() < 1e-6);
        if !held {
            problems.push(format!(
                "the first frame isn't held {want} ticks at T = {TIMESCALE} (E = {ticks:?})"
            ));
        }
    }
    if let Some(want) = entry.av_offset_ms {
        let medians: Vec<f64> = value("av.offset")
            .and_then(|v| v.get("tracks")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|t| t.get("measured") == Some(&Value::Bool(true)))
            .filter_map(|t| t.get("median_ms")?.as_f64())
            .collect();
        if medians.is_empty() {
            problems.push("av.offset measured no track".to_owned());
        }
        for median in medians {
            if (median - want).abs() > AV_TOLERANCE_MS {
                problems.push(format!(
                    "av.offset reads {median:+.2} ms; want {want:+} ± {AV_TOLERANCE_MS} ms"
                ));
            }
        }
    }
    if let Some([skips, repeats]) = entry.skips_repeats {
        let counts = value("cadence.drops_dups").map(|v| {
            (
                v.get("skipped").and_then(Value::as_u64),
                v.get("repeats").and_then(Value::as_u64),
            )
        });
        if counts != Some((Some(skips), Some(repeats))) {
            let (got_skips, got_repeats) = counts.unwrap_or_default();
            problems.push(format!(
                "cadence.drops_dups counts {got_skips:?} skipped and {got_repeats:?} repeated; want {skips} and {repeats}"
            ));
        }
    }
    Judged {
        file: entry.file.clone(),
        verdict,
        failed,
        problems,
        copied_in: false,
    }
}

/// The PC's line-error limit from its good clips' worst line error (README §7.4).
pub fn calibrated_limit(worst_good_codes: f64) -> f64 {
    LINES_FLOOR_CODES.max(2.0 * worst_good_codes)
}
