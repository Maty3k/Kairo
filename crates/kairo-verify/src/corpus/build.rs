// SPDX-License-Identifier: GPL-3.0-or-later
//! `corpus build` (docs/research/16-verify-testbed/README.md §7.1–7.3): every clip of one
//! encoder's corpus in a new folder.
//!
//! Each video variant is one encoder session, `synth`'s frames into FFmpeg and out to a video-only
//! MP4 with no audio lead. Each clip is then muxed from one of those with its four WAVs and
//! FFmpeg's `aac`, the video stream-copied, so the held first frame is applied once (§7.2); the
//! container defects are box edits of the finished good clip. FFmpeg runs in the folder and gets
//! names relative to it, so its logs, in `logs/`, name no other folder (CLAUDE.md). Hardware
//! encoder sessions run one at a time, at least 1.5 s apart (M1:94). An FFmpeg run fails the
//! build if it exits with an error or leaves an option unused, or if its log lacks the encoder,
//! the BT.709 limited output or, for AMF, the adapter it was given (never assumed: the log must
//! name the vendor:device FFmpeg opened, and it goes in the manifest). `bframes` falls back from
//! AMF to Media Foundation, and if neither makes B-frames it is listed as missing, its expect file
//! written for the Mac's clip to be copied in. FFmpeg is checked before the folder is made; the
//! video-only files and WAVs go at the end, or when a step fails; `manifest.json`, with every
//! clip's size and hash, is written last.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    Adapter, CALIBRATED, Calibration, Encoder, GOOD, LINES_FLOOR_CODES, MANIFEST, Make, Manifest,
    Missing, Recipe, VERSION, Video, fnv1a64, recipes,
};
use crate::profile::{Plan, Profile};
use crate::synth::{AudioTrack, Defect, Synth};

/// The least gap between two hardware encoder sessions (M1:94).
pub const SPACING: Duration = Duration::from_millis(1_500);

/// The colour every encoder's input frames are tagged with: BT.709 limited (README §7.2's
/// `SETPARAMS`), so FFmpeg converts nothing and the encoder and `colr` say BT.709.
const SETPARAMS: [&str; 2] = [
    "-vf",
    "setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv",
];

/// README §7.2's colour fix for an encoder whose VUI lacks BT.709 limited (`--finishing
/// h264_metadata`): an LGPL bitstream filter, applied to every encode and recorded in the
/// manifest's `finishing`.
pub const H264_METADATA: &str = "h264_metadata=video_full_range_flag=0:colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1";

/// What FFmpeg prints for an option no stream used (EXP-8: a warning, exit 0).
const UNUSED: &str = "has not been used for any stream";

/// Why a build stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    /// The exit code: 2 a bad `--out`, 3 a step failed, 4 a file couldn't be written.
    pub code: u8,
    /// What happened, naming files inside the corpus folder only.
    pub message: String,
}

impl BuildError {
    fn step(message: String) -> Self {
        Self { code: 3, message }
    }

    fn write(name: &str, error: impl fmt::Display) -> Self {
        Self {
            code: 4,
            message: format!("{name}: {error}"),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BuildError {}

/// A build's settings.
#[derive(Debug, Clone, Copy)]
pub struct Build<'a> {
    /// The encoder.
    pub encoder: Encoder,
    /// The FFmpeg to spawn.
    pub ffmpeg: &'a Path,
    /// AMF's adapter: `-init_hw_device d3d11va=kd:N`. Required for AMF.
    pub hw_index: Option<u32>,
    /// The vendor:device AMF's log must name, as the harness resolves it from the LUID.
    pub hw_device: Option<&'a str>,
    /// Whether to apply [`H264_METADATA`] to every encode.
    pub finishing: bool,
    /// The corpus folder, which must not exist yet.
    pub out: &'a Path,
}

/// FFmpeg's arguments for one encoder session of `video` with `encoder`: `synth`'s frames on
/// stdin, out to [`Video::file`] (README §7.2's table).
pub fn encode_args(
    encoder: Encoder,
    video: Video,
    synth: &Synth,
    hw_index: Option<u32>,
    finishing: bool,
) -> Vec<String> {
    let level = if encoder == Encoder::Amf {
        "debug"
    } else {
        "verbose"
    };
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", level, "-n"]
        .map(String::from)
        .to_vec();
    let push = |args: &mut Vec<String>, words: &[&str]| {
        args.extend(words.iter().map(|w| (*w).to_owned()));
    };
    if encoder == Encoder::Amf {
        let device = match hw_index {
            Some(n) => format!("d3d11va=kd:{n}"),
            None => "d3d11va=kd".to_owned(),
        };
        push(&mut args, &["-init_hw_device", &device]);
    }
    let format = synth.format;
    let fps = format.fps.to_string();
    match encoder {
        Encoder::Ipcm => push(&mut args, &["-f", "h264", "-r", &fps, "-i", "pipe:0"]),
        _ => push(
            &mut args,
            &[
                "-f",
                "rawvideo",
                "-pix_fmt",
                "nv12",
                "-video_size",
                &format!("{}x{}", format.width, format.height),
                "-framerate",
                &fps,
                "-i",
                "pipe:0",
            ],
        ),
    }
    let (rate, peak) = video.rates();
    let gop = match video {
        Video::Gop2s => 2 * format.fps,
        _ => format.fps,
    }
    .to_string();
    let bf = if video == Video::Bframes { "2" } else { "0" };
    match encoder {
        Encoder::Amf => {
            push(&mut args, &SETPARAMS);
            push(
                &mut args,
                &[
                    "-pix_fmt",
                    "nv12",
                    "-c:v",
                    "h264_amf",
                    "-usage",
                    "transcoding",
                    "-quality",
                    "balanced",
                    "-rc",
                    "vbr_peak",
                    "-b:v",
                    rate,
                    "-maxrate",
                    peak,
                    "-bufsize",
                    peak,
                    "-g",
                    &gop,
                    "-vbaq",
                    "1",
                    "-preanalysis",
                    "0",
                    "-high_motion_quality_boost_enable",
                    "0",
                    "-preencode",
                    "0",
                    "-bf",
                    bf,
                    "-max_b_frames",
                    bf,
                    "-frame_skipping",
                    "0",
                    "-filler_data",
                    "0",
                    "-coder",
                    "cabac",
                    "-profile:v",
                    "high",
                    "-header_spacing",
                    &gop,
                    "-async_depth",
                    "4",
                ],
            );
        }
        Encoder::Mf => {
            // h264_mf has no named profiles, so High is its number, 100; without it, Media
            // Foundation encodes Baseline, which can't carry B-frames. `-hw_encoding 1` takes
            // the first hardware MFT, whose adapter FFmpeg doesn't name [Unverified on the PC].
            push(&mut args, &SETPARAMS);
            push(
                &mut args,
                &[
                    "-pix_fmt",
                    "nv12",
                    "-c:v",
                    "h264_mf",
                    "-hw_encoding",
                    "1",
                    "-b:v",
                    rate,
                    "-maxrate",
                    peak,
                    "-g",
                    &gop,
                    "-bf",
                    bf,
                    "-profile:v",
                    "100",
                ],
            );
        }
        Encoder::VideoToolbox => {
            push(&mut args, &SETPARAMS);
            push(
                &mut args,
                &[
                    "-c:v",
                    "h264_videotoolbox",
                    "-b:v",
                    rate,
                    "-maxrate",
                    peak,
                    "-g",
                    &gop,
                    "-bf",
                    bf,
                    "-profile:v",
                    "high",
                ],
            );
        }
        Encoder::Ipcm => push(
            &mut args,
            &[
                "-c:v",
                "copy",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
                "-color_range",
                "tv",
            ],
        ),
    }
    if finishing {
        push(&mut args, &["-bsf:v", H264_METADATA]);
    }
    push(
        &mut args,
        &[
            "-an",
            "-movflags",
            movflags(video),
            "-use_editlist",
            "0",
            &video.file(),
        ],
    );
    args
}

/// The MP4 flags: B-frames need negative composition offsets without an edit list.
fn movflags(video: Video) -> &'static str {
    if video == Video::Bframes {
        "+faststart+negative_cts_offsets"
    } else {
        "+faststart"
    }
}

/// A WAV's name in the corpus folder: `a-<audio>-<track>.wav`.
fn wav_name(audio: Option<Defect>, track: AudioTrack) -> String {
    let variant = audio.map_or("good".to_owned(), |d| d.to_string());
    format!("a-{variant}-{}", track.file())
}

/// FFmpeg's arguments for muxing `recipe`'s clip from `video` and the WAVs of `audio` (README
/// §7.2's step 2): the video stream-copied, each track AAC at its bitrate with its title, the Mix
/// the only default.
pub fn mux_args(recipe: &Recipe, video: Video, audio: Option<Defect>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "verbose",
        "-n",
        "-i",
    ]
    .map(String::from)
    .to_vec();
    args.push(video.file());
    for track in AudioTrack::ALL {
        args.push("-i".to_owned());
        args.push(wav_name(audio, track));
    }
    args.extend(["-map", "0:v"].map(String::from));
    for i in 1..=AudioTrack::ALL.len() {
        args.push("-map".to_owned());
        args.push(format!("{i}:a"));
    }
    args.extend(["-c:v", "copy", "-c:a", "aac"].map(String::from));
    for (i, track) in AudioTrack::ALL.iter().enumerate() {
        let title = track.title();
        args.extend([
            format!("-b:a:{i}"),
            format!("{}k", track.kbps()),
            format!("-metadata:s:a:{i}"),
            format!("title={title}"),
            format!("-metadata:s:a:{i}"),
            format!("handler_name={title}"),
            format!("-metadata:s:a:{i}"),
            "language=und".to_owned(),
            format!("-disposition:a:{i}"),
            if *track == AudioTrack::Mix {
                "default"
            } else {
                "0"
            }
            .to_owned(),
        ]);
    }
    args.extend(["-movflags", movflags(video), "-use_editlist", "0"].map(String::from));
    args.push(recipe.file());
    args
}

/// The vendor:device of the d3d11va adapter FFmpeg opened, from its `Using device vvvv:dddd (…)`
/// line (hwcontext_d3d11va.c), lowercase. The encoder's own `Using device kd (type d3d11va)`
/// line doesn't match: only four hex digits, a colon and four more do.
pub fn adapter_device(log: &str) -> Option<String> {
    log.match_indices("Using device ").find_map(|(at, word)| {
        let rest = log.get(at + word.len()..)?;
        let pair = rest.get(..9)?;
        let hex = |s: &str| s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit());
        let (vendor, device) = pair.split_once(':')?;
        (hex(vendor) && hex(device) && rest.get(9..11) == Some(" ("))
            .then(|| pair.to_ascii_lowercase())
    })
}

/// What's wrong with an encoder session's log (README §7.2): an option no stream used, no
/// mapping to the encoder, an output that isn't BT.709 limited, and for AMF the device lines, the
/// adapter asked for and the vendor:device FFmpeg opened, which must be `hw_device` when given.
/// FFmpeg names the index before it looks the adapter up, and falls back to the default one
/// without naming a vendor:device if the index doesn't exist.
pub fn encode_log_problems(
    encoder: Encoder,
    log: &str,
    hw_index: Option<u32>,
    hw_device: Option<&str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    if log.contains(UNUSED) {
        problems.push("an option no stream used".to_owned());
    }
    let mapping = match encoder {
        Encoder::Amf => "-> h264 (h264_amf)",
        Encoder::Mf => "-> h264 (h264_mf)",
        Encoder::VideoToolbox => "-> h264 (h264_videotoolbox)",
        Encoder::Ipcm => "-> #0:0 (copy)",
    };
    if !log.contains(mapping) {
        problems.push(format!("no \"{mapping}\" mapping"));
    }
    // The output's video stream line, under `Output #0`.
    let output = log
        .split("Output #0")
        .nth(1)
        .and_then(|rest| rest.lines().find(|l| l.contains("Video: h264")));
    let pixels = if encoder == Encoder::Ipcm {
        "(tv, bt709"
    } else {
        "nv12(tv, bt709"
    };
    if !output.is_some_and(|line| line.contains(pixels)) {
        problems.push(format!("the output's video isn't \"{pixels}\""));
    }
    if encoder == Encoder::Amf {
        for line in [
            "Using device kd (type d3d11va) with h264_amf encoder.",
            "AMF via D3D11.",
        ] {
            if !log.contains(line) {
                problems.push(format!("no \"{line}\""));
            }
        }
        match hw_index {
            Some(n) if !log.contains(&format!("Selecting d3d11va adapter {n}")) => {
                problems.push(format!("no \"Selecting d3d11va adapter {n}\""));
            }
            Some(_) => {}
            None => problems.push("no adapter asked for (--hw-index)".to_owned()),
        }
        match (adapter_device(log), hw_device) {
            (None, _) => problems.push(
                "no \"Using device vvvv:dddd\": the adapter wasn't found, and FFmpeg fell back to the default one".to_owned(),
            ),
            (Some(found), Some(want)) if !found.eq_ignore_ascii_case(want) => {
                problems.push(format!("the adapter is {found}, not {want}"));
            }
            _ => {}
        }
    }
    problems
}

/// What's wrong with a mux's log: an option no stream used, a video that isn't a stream copy,
/// or a track that isn't AAC from FFmpeg's own encoder.
pub fn mux_log_problems(log: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if log.contains(UNUSED) {
        problems.push("an option no stream used".to_owned());
    }
    if !log.contains("Stream #0:0 -> #0:0 (copy)") {
        problems.push("the video isn't a stream copy".to_owned());
    }
    let aac = log.matches("-> aac (native)").count();
    if aac != AudioTrack::ALL.len() {
        problems.push(format!(
            "{aac} tracks to FFmpeg's aac, not {}",
            AudioTrack::ALL.len()
        ));
    }
    problems
}

/// The version from `ffmpeg -version`'s first line: `ffmpeg version 8.0.1 Copyright …` gives
/// `8.0.1`.
pub fn version(first_line: &str) -> Option<String> {
    let rest = first_line.strip_prefix("ffmpeg version ")?;
    rest.split_whitespace()
        .next()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Whether a video-only file's pictures are reordered: `video.no_reorder`'s count of non-zero
/// `ctts` entries or B-slice samples, not just its verdict, which a VUI field alone can fail.
fn holds_b_frames(report: &crate::Report) -> bool {
    let value = report
        .checks
        .iter()
        .find(|f| f.id == "video.no_reorder")
        .map(|f| &f.value);
    let count = |key: &str| value.and_then(|v| v.get(key)).and_then(Value::as_u64);
    count("ctts_nonzero").unwrap_or(0) > 0 || count("b_samples").unwrap_or(0) > 0
}

/// What a build has made so far that goes if a step fails: the intermediates and the output of
/// the step under way. Clips already finished, their expect files and the logs stay.
#[derive(Default)]
struct Made {
    intermediates: Vec<String>,
    current: Option<String>,
}

impl Build<'_> {
    /// The FFmpeg to spawn: a relative path with a folder in it is taken from the caller's
    /// folder, not the corpus folder FFmpeg runs in; a bare name is looked up on the `PATH`.
    pub(crate) fn program(&self) -> PathBuf {
        if self.ffmpeg.is_relative() && self.ffmpeg.components().count() > 1 {
            std::path::absolute(self.ffmpeg).unwrap_or_else(|_| self.ffmpeg.to_path_buf())
        } else {
            self.ffmpeg.to_path_buf()
        }
    }

    /// Builds the corpus, telling `say` each step, and returns its manifest.
    pub fn run(&self, say: &mut dyn FnMut(&str)) -> Result<Manifest, BuildError> {
        // The B-frames probe reads clips through `check_with`, which a non-LGPL linked FFmpeg
        // stops (README §2 item 18).
        if let Some(refusal) = crate::decode::linked_ffmpeg()
            .as_ref()
            .and_then(crate::checks::not_lgpl)
        {
            return Err(BuildError::step(refusal));
        }
        if self.out.symlink_metadata().is_ok() {
            return Err(BuildError {
                code: 2,
                message: "the --out folder exists already; corpus build makes a new one".to_owned(),
            });
        }
        let parent = self
            .out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !parent.is_dir() {
            return Err(BuildError::write(
                "the --out folder",
                "the folder it goes in doesn't exist",
            ));
        }
        if self.encoder == Encoder::Amf && self.hw_index.is_none() {
            return Err(BuildError {
                code: 2,
                message: "h264_amf needs --hw-index N: the adapter is chosen, never assumed"
                    .to_owned(),
            });
        }
        // FFmpeg, and the filter `--finishing` names, before anything is made.
        let ffmpeg = self.ffmpeg_version()?;
        if self.finishing {
            self.has_h264_metadata()?;
        }
        fs::create_dir(self.out).map_err(|e| BuildError::write("the --out folder", e))?;
        fs::create_dir(self.out.join("logs")).map_err(|e| BuildError::write("logs", e))?;
        say(&format!(
            "corpus build: {} with FFmpeg {ffmpeg}",
            self.encoder
        ));
        let mut made = Made::default();
        let result = self.make(ffmpeg, &mut made, say);
        // The intermediates go either way; on a failure, so does the step's own output.
        let leftovers = made
            .intermediates
            .iter()
            .chain(made.current.iter().filter(|_| result.is_err()));
        for name in leftovers {
            let _ = fs::remove_file(self.out.join(name));
        }
        result
    }

    /// Every clip, its expect file and the manifest.
    fn make(
        &self,
        ffmpeg: String,
        made: &mut Made,
        say: &mut dyn FnMut(&str),
    ) -> Result<Manifest, BuildError> {
        let started = Instant::now();
        let recipes = recipes(self.encoder);
        let mut last_session: Option<Instant> = None;
        // Each video variant once, in the recipes' order.
        let mut videos: Vec<Video> = Vec::new();
        for recipe in &recipes {
            if let Make::Mux { video, .. } = recipe.make
                && !videos.contains(&video)
            {
                videos.push(video);
            }
        }
        let mut made_by: BTreeMap<String, Encoder> = BTreeMap::new();
        let mut missing: Vec<Missing> = Vec::new();
        let mut adapter = None;
        for &video in &videos {
            made.intermediates.push(video.file());
            if video == Video::Bframes {
                match self.bframes(&mut last_session, say)? {
                    Ok(encoder) => {
                        made_by.insert(video.name(), encoder);
                    }
                    // The Mac's clip is the one the PC falls back on, so it must exist.
                    Err(why) if self.encoder == Encoder::VideoToolbox => {
                        return Err(BuildError::step(format!("bframes.mp4: {why}")));
                    }
                    Err(why) => missing.push(Missing {
                        file: "bframes.mp4".to_owned(),
                        why,
                    }),
                }
                continue;
            }
            let device = self.encode(video, self.encoder, &mut last_session, say)?;
            adapter = adapter.or(device);
        }
        // Each set of WAVs once.
        let mut audios: Vec<Option<Defect>> = Vec::new();
        for recipe in &recipes {
            if let Make::Mux { audio, .. } = recipe.make
                && !audios.contains(&audio)
            {
                audios.push(audio);
            }
        }
        for &audio in &audios {
            for track in AudioTrack::ALL {
                made.intermediates.push(wav_name(audio, track));
            }
            self.write_wavs(audio)?;
        }
        let mut entries = Vec::new();
        for recipe in &recipes {
            // A clip no encoder could make still gets its expect file, for the Mac's clip.
            self.write_expect(recipe)?;
            let skipped = recipe.is_bframes() && !made_by.contains_key(&Video::Bframes.name());
            if skipped {
                continue;
            }
            made.current = Some(recipe.file());
            match &recipe.make {
                Make::Mux { video, audio } => self.mux(recipe, *video, *audio, say)?,
                Make::Mutate(op) => self.mutate(recipe, op, say)?,
            }
            made.current = None;
            let mut entry = recipe.entry(self.encoder);
            if let Make::Mux { video, .. } = recipe.make
                && let Some(&encoder) = made_by.get(&video.name())
                && encoder != self.encoder
            {
                entry.encoder = Some(encoder.to_string());
            }
            let mut clip = File::open(self.out.join(recipe.file()))
                .map_err(|e| BuildError::step(format!("{}: {e}", recipe.file())))?;
            let bytes = clip
                .metadata()
                .map_err(|e| BuildError::step(format!("{}: {e}", recipe.file())))?
                .len();
            entry.bytes = Some(bytes);
            entry.fnv1a64 = Some(
                fnv1a64(&mut clip)
                    .map_err(|e| BuildError::step(format!("{}: {e}", recipe.file())))?,
            );
            entries.push(entry);
        }
        let calibration = if self.encoder.calibrates() {
            Calibration {
                limit_codes: None,
                source: "max(4, 2 x worst good clip), set by corpus check into corpus-report.json (README §7.4)"
                    .to_owned(),
            }
        } else {
            Calibration {
                limit_codes: Some(LINES_FLOOR_CODES),
                source: "fixed: the Mac and CI corpora keep 4 codes (README §7.4)".to_owned(),
            }
        };
        let manifest = Manifest {
            kairo_corpus: VERSION,
            pattern: 1,
            encoder: self.encoder.to_string(),
            ffmpeg,
            finishing: if self.finishing {
                vec![format!("-bsf:v {H264_METADATA}")]
            } else {
                Vec::new()
            },
            adapter: self
                .hw_index
                .zip(adapter)
                .map(|(index, device)| Adapter { index, device }),
            calibration: [(CALIBRATED.to_owned(), calibration)].into(),
            clips: entries,
            missing,
        };
        let text =
            serde_json::to_string_pretty(&manifest).map_err(|e| BuildError::write(MANIFEST, e))?;
        self.write_new(MANIFEST, |file| {
            file.write_all(format!("{text}\n").as_bytes())
        })?;
        say(&format!(
            "corpus build: {} clips in {:.0} s; manifest.json written",
            manifest.clips.len(),
            started.elapsed().as_secs_f64()
        ));
        Ok(manifest)
    }

    /// The first line of `ffmpeg -version`, as a version.
    fn ffmpeg_version(&self) -> Result<String, BuildError> {
        let output = Command::new(self.program())
            .arg("-version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| BuildError::step(format!("FFmpeg didn't start: {e}")))?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines()
            .next()
            .and_then(version)
            .ok_or_else(|| BuildError::step("FFmpeg's -version gave no version".to_owned()))
    }

    /// Refuses `--finishing` with an FFmpeg that lacks the `h264_metadata` filter.
    fn has_h264_metadata(&self) -> Result<(), BuildError> {
        let output = Command::new(self.program())
            .args(["-hide_banner", "-h", "bsf=h264_metadata"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| BuildError::step(format!("FFmpeg didn't start: {e}")))?;
        let text = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && text.contains("h264_metadata") {
            Ok(())
        } else {
            Err(BuildError::step(
                "this FFmpeg has no h264_metadata filter for --finishing".to_owned(),
            ))
        }
    }

    /// Waits until a hardware encoder's last session ended at least [`SPACING`] ago.
    fn space(&self, last_session: &Option<Instant>) {
        if let Some(last) = last_session.filter(|_| self.encoder.is_hardware()) {
            let since = last.elapsed();
            if since < SPACING {
                std::thread::sleep(SPACING - since);
            }
        }
    }

    /// One encoder session: `video`'s frames from `synth` into FFmpeg with `encoder`. Returns
    /// the vendor:device an AMF session opened.
    fn encode(
        &self,
        video: Video,
        encoder: Encoder,
        last_session: &mut Option<Instant>,
        say: &mut dyn FnMut(&str),
    ) -> Result<Option<String>, BuildError> {
        let synth = self.encoder.synth(video);
        let log_name = format!("logs/{}-{encoder}.log", video.name());
        let log = File::create_new(self.out.join(&log_name))
            .map_err(|e| BuildError::write(&log_name, e))?;
        self.space(last_session);
        let started = Instant::now();
        let mut child = Command::new(self.program())
            .current_dir(self.out)
            .args(encode_args(
                encoder,
                video,
                &synth,
                self.hw_index,
                self.finishing,
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .map_err(|e| BuildError::step(format!("FFmpeg didn't start: {e}")))?;
        let written = match child.stdin.take() {
            Some(mut stdin) => synth.write(&mut stdin),
            None => Ok(()),
        };
        let status = child
            .wait()
            .map_err(|e| BuildError::step(format!("{}: {e}", video.file())))?;
        *last_session = Some(Instant::now());
        // FFmpeg's own failure explains a closed pipe, so it comes first.
        if !status.success() {
            return Err(BuildError::step(format!(
                "{}: FFmpeg ended with {status}; see {log_name}",
                video.file()
            )));
        }
        written.map_err(|e| BuildError::step(format!("{}: {e}", video.file())))?;
        let text = fs::read(self.out.join(&log_name))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|e| BuildError::step(format!("{log_name}: {e}")))?;
        let problems = encode_log_problems(encoder, &text, self.hw_index, self.hw_device);
        if !problems.is_empty() {
            return Err(BuildError::step(format!(
                "{}: {}; see {log_name}",
                video.file(),
                problems.join("; ")
            )));
        }
        say(&format!(
            "  {} ({encoder}, {} frames) in {:.1} s",
            video.file(),
            synth.frames(),
            started.elapsed().as_secs_f64()
        ));
        Ok((encoder == Encoder::Amf)
            .then(|| adapter_device(&text))
            .flatten())
    }

    /// The `bframes` video: AMF, then Media Foundation on the PC, each kept only if it holds
    /// B-frames. `Ok(Err(why))` when no encoder made them.
    fn bframes(
        &self,
        last_session: &mut Option<Instant>,
        say: &mut dyn FnMut(&str),
    ) -> Result<Result<Encoder, String>, BuildError> {
        let chain = match self.encoder {
            Encoder::Amf => vec![Encoder::Amf, Encoder::Mf],
            other => vec![other],
        };
        let mut why = Vec::new();
        for encoder in chain {
            let file = self.out.join(Video::Bframes.file());
            match self.encode(Video::Bframes, encoder, last_session, say) {
                Ok(_) if self.reorders(&file)? => return Ok(Ok(encoder)),
                Ok(_) => why.push(format!("{encoder} made no B-frames")),
                Err(error) if error.code == 3 => why.push(error.message),
                Err(error) => return Err(error),
            }
            let _ = fs::remove_file(&file);
        }
        let fallback = if self.encoder == Encoder::VideoToolbox {
            ""
        } else {
            "; copy the Mac's VideoToolbox bframes.mp4 in (README §7.3)"
        };
        Ok(Err(format!("{}{fallback}", why.join("; "))))
    }

    /// Whether a video-only file holds B-frames.
    fn reorders(&self, path: &Path) -> Result<bool, BuildError> {
        let name = Video::Bframes.file();
        let mut file = File::open(path).map_err(|e| BuildError::step(format!("{name}: {e}")))?;
        let expect = self.encoder.synth(Video::Good).expect(false);
        let plan = Plan::new(Profile::Test, false, Some(expect))
            .map_err(|e| BuildError::step(format!("{name}: {e}")))?;
        let report = crate::check_with(&mut file, &name, &plan, None);
        if let Some(error) = &report.error {
            return Err(BuildError::step(format!("{name}: {error}")));
        }
        Ok(holds_b_frames(&report))
    }

    /// The four WAVs of `audio`: the good clip's tracks with this audio defect, if any.
    fn write_wavs(&self, audio: Option<Defect>) -> Result<(), BuildError> {
        let synth = Synth {
            defect: audio,
            ..self.encoder.synth(Video::Good)
        };
        for track in AudioTrack::ALL {
            let name = wav_name(audio, track);
            self.write_new(&name, |file| synth.write_wav(track, file))?;
        }
        Ok(())
    }

    /// Muxes `recipe`'s clip from `video` and the WAVs of `audio`, with no encoder session for
    /// the pictures.
    fn mux(
        &self,
        recipe: &Recipe,
        video: Video,
        audio: Option<Defect>,
        say: &mut dyn FnMut(&str),
    ) -> Result<(), BuildError> {
        let log_name = format!("logs/mux-{}.log", recipe.name);
        let log = File::create_new(self.out.join(&log_name))
            .map_err(|e| BuildError::write(&log_name, e))?;
        let status = Command::new(self.program())
            .current_dir(self.out)
            .args(mux_args(recipe, video, audio))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .status()
            .map_err(|e| BuildError::step(format!("FFmpeg didn't start: {e}")))?;
        if !status.success() {
            return Err(BuildError::step(format!(
                "{}: FFmpeg ended with {status}; see {log_name}",
                recipe.file()
            )));
        }
        let text = fs::read(self.out.join(&log_name))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|e| BuildError::step(format!("{log_name}: {e}")))?;
        let problems = mux_log_problems(&text);
        if !problems.is_empty() {
            return Err(BuildError::step(format!(
                "{}: {}; see {log_name}",
                recipe.file(),
                problems.join("; ")
            )));
        }
        say(&format!("  {}", recipe.file()));
        Ok(())
    }

    /// `recipe`'s clip as a box edit of the finished good clip.
    fn mutate(
        &self,
        recipe: &Recipe,
        op: &crate::mutate::Op,
        say: &mut dyn FnMut(&str),
    ) -> Result<(), BuildError> {
        let good = format!("{GOOD}.mp4");
        let mut source = File::open(self.out.join(&good))
            .map_err(|e| BuildError::step(format!("{good}: {e}")))?;
        let mut result = Ok(());
        self.write_new(&recipe.file(), |file| {
            let mut writer = BufWriter::new(file);
            result = crate::mutate::mutate(&mut source, op, &mut writer);
            writer.flush()
        })?;
        result.map_err(|e| BuildError::step(format!("{}: {e}", recipe.file())))?;
        say(&format!("  {} ({op})", recipe.file()));
        Ok(())
    }

    /// `recipe`'s expect file: what the clip should have been.
    fn write_expect(&self, recipe: &Recipe) -> Result<(), BuildError> {
        let name = format!("{}.expect.json", recipe.name);
        let expect = self.encoder.synth(recipe.expect).expect(true);
        let text =
            serde_json::to_string_pretty(&expect).map_err(|e| BuildError::write(&name, e))?;
        self.write_new(&name, |file| file.write_all(format!("{text}\n").as_bytes()))
    }

    /// Writes a new file in the corpus folder through `write`.
    fn write_new(
        &self,
        name: &str,
        write: impl FnOnce(&mut File) -> std::io::Result<()>,
    ) -> Result<(), BuildError> {
        let path: PathBuf = self.out.join(name);
        let mut file = File::create_new(&path).map_err(|e| BuildError::write(name, e))?;
        write(&mut file)
            .and_then(|()| file.sync_all())
            .map_err(|e| BuildError::write(name, e))
    }
}
