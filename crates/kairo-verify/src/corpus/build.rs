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
//! the BT.709 limited output or, for AMF, the d3d11va device lines. `bframes` falls back from AMF
//! to Media Foundation, and is listed as missing if neither makes B-frames. The video-only files
//! and the WAVs go at the end, and `manifest.json` is written last.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{
    CALIBRATED, Calibration, Encoder, GOOD, LINES_FLOOR_CODES, MANIFEST, Make, Manifest, Missing,
    Recipe, VERSION, Video, recipes,
};
use crate::checks::Status;
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
    /// AMF's adapter: `-init_hw_device d3d11va=kd:N`.
    pub hw_index: Option<u32>,
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
            push(&mut args, &SETPARAMS);
            push(
                &mut args,
                &[
                    "-pix_fmt",
                    "nv12",
                    "-c:v",
                    "h264_mf",
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

/// What's wrong with an encoder session's log (README §7.2): an option no stream used, no
/// mapping to the encoder, an output that isn't BT.709 limited, and for AMF the device lines and
/// the adapter.
pub fn encode_log_problems(encoder: Encoder, log: &str, hw_index: Option<u32>) -> Vec<String> {
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
        let mut lines = vec![
            "Using device kd (type d3d11va) with h264_amf encoder.".to_owned(),
            "AMF via D3D11.".to_owned(),
        ];
        if let Some(n) = hw_index {
            lines.push(format!("Selecting d3d11va adapter {n}"));
        }
        for line in lines {
            if !log.contains(&line) {
                problems.push(format!("no \"{line}\""));
            }
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
        if self.out.symlink_metadata().is_ok() {
            return Err(BuildError {
                code: 2,
                message: "the --out folder exists already; corpus build makes a new one".to_owned(),
            });
        }
        fs::create_dir(self.out).map_err(|e| BuildError::write("the --out folder", e))?;
        fs::create_dir(self.out.join("logs")).map_err(|e| BuildError::write("logs", e))?;
        let ffmpeg = self.ffmpeg_version()?;
        say(&format!(
            "corpus build: {} with FFmpeg {ffmpeg}",
            self.encoder
        ));
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
        for &video in &videos {
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
            self.encode(video, self.encoder, &mut last_session, say)?;
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
            self.write_wavs(audio)?;
        }
        let mut entries = Vec::new();
        for recipe in &recipes {
            match &recipe.make {
                Make::Mux { video, audio } => {
                    if *video == Video::Bframes && !made_by.contains_key(&video.name()) {
                        continue;
                    }
                    self.mux(recipe, *video, *audio, say)?;
                }
                Make::Mutate(op) => self.mutate(recipe, op, say)?,
            }
            self.write_expect(recipe)?;
            let mut entry = recipe.entry(self.encoder);
            if let Make::Mux { video, .. } = recipe.make
                && let Some(&encoder) = made_by.get(&video.name())
                && encoder != self.encoder
            {
                entry.encoder = Some(encoder.to_string());
            }
            entries.push(entry);
        }
        // The intermediates go; the clips, expect files and logs stay.
        for &video in &videos {
            let _ = fs::remove_file(self.out.join(video.file()));
        }
        for &audio in &audios {
            for track in AudioTrack::ALL {
                let _ = fs::remove_file(self.out.join(wav_name(audio, track)));
            }
        }
        let calibration = if self.encoder.calibrates() {
            Calibration {
                limit_codes: None,
                source: "max(4, 2 x worst good clip), set by corpus check (README §7.4)".to_owned(),
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
            finishing: Vec::new(),
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

    /// Waits until a hardware encoder's last session ended at least [`SPACING`] ago.
    fn space(&self, last_session: &Option<Instant>) {
        if let Some(last) = last_session.filter(|_| self.encoder.is_hardware()) {
            let since = last.elapsed();
            if since < SPACING {
                std::thread::sleep(SPACING - since);
            }
        }
    }

    /// One encoder session: `video`'s frames from `synth` into FFmpeg with `encoder`.
    fn encode(
        &self,
        video: Video,
        encoder: Encoder,
        last_session: &mut Option<Instant>,
        say: &mut dyn FnMut(&str),
    ) -> Result<(), BuildError> {
        let synth = self.encoder.synth(video);
        let log_name = format!("logs/{}-{encoder}.log", video.name());
        let log = File::create_new(self.out.join(&log_name))
            .map_err(|e| BuildError::write(&log_name, e))?;
        self.space(last_session);
        let started = Instant::now();
        let mut child = Command::new(self.program())
            .current_dir(self.out)
            .args(encode_args(encoder, video, &synth, self.hw_index))
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
        let problems = encode_log_problems(encoder, &text, self.hw_index);
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
        Ok(())
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
                Ok(()) if self.reorders(&file)? => return Ok(Ok(encoder)),
                Ok(()) => why.push(format!("{encoder} made no B-frames")),
                Err(error) if error.code == 3 => why.push(error.message),
                Err(error) => return Err(error),
            }
            let _ = fs::remove_file(&file);
        }
        let fallback = if self.encoder == Encoder::VideoToolbox {
            ""
        } else {
            "; copy the Mac's VideoToolbox clip in (README §7.3)"
        };
        Ok(Err(format!("{}{fallback}", why.join("; "))))
    }

    /// Whether a video-only file reorders its pictures: `video.no_reorder` fails on it.
    fn reorders(&self, path: &Path) -> Result<bool, BuildError> {
        let name = Video::Bframes.file();
        let mut file = File::open(path).map_err(|e| BuildError::step(format!("{name}: {e}")))?;
        let expect = self.encoder.synth(Video::Good).expect(false);
        let plan = Plan::new(Profile::Test, false, Some(expect))
            .map_err(|e| BuildError::step(format!("{name}: {e}")))?;
        let report = crate::check_with(&mut file, &name, &plan, None);
        Ok(report
            .checks
            .iter()
            .any(|f| f.id == "video.no_reorder" && f.status == Status::Fail))
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
