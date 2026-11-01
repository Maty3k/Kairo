// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify synth` (docs/research/16-verify-testbed/README.md §6.1, §7.1–7.3): pattern v1
//! at a clip's size and rate for an encoder, as raw NV12 or as [`crate::ipcm`]'s lossless H.264
//! stream; the clip's audio tracks as 48 kHz WAVs; and the clip's `expect.json`.
//!
//! Output frame k shows testbed frame N_k = ⌊k · source_hz / fps⌋ (3k from a 180 Hz testbed at
//! 60 fps), as an ideal pacer sampling the testbed would, unless a defect changes that. Flash m
//! is output frame m · fps, at m s, and the game beep starts at m s in its WAV: the AAC
//! encoder's priming and the mux's held first frame then cancel (cookbook P8).

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Write};
use std::str::FromStr;

use kairo_core::testsig::{self, Curve, Load, Options, Patches, Pattern, Size, Tone};

use crate::decode::Picture;
use crate::decode::synthetic::{Damage, Sound, Synthetic, SyntheticAudio};
use crate::expect::{self, Expect, Hdr};
use crate::ipcm::Ipcm;

/// A clip's size and rate: `--preset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
}

impl FromStr for Format {
    type Err = String;

    /// `1080p60`, `1440p60` or `WxH@fps`.
    fn from_str(text: &str) -> Result<Self, String> {
        let (width, height, fps) = match text {
            "1080p60" => (1920, 1080, 60),
            "1440p60" => (2560, 1440, 60),
            _ => {
                let parse = || -> Option<(u32, u32, u32)> {
                    let (size, fps) = text.split_once('@')?;
                    let (width, height) = size.split_once('x')?;
                    Some((width.parse().ok()?, height.parse().ok()?, fps.parse().ok()?))
                };
                parse().ok_or_else(|| "--preset takes 1080p60, 1440p60 or WxH@fps".to_owned())?
            }
        };
        Ok(Self { width, height, fps })
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{} at {} fps", self.width, self.height, self.fps)
    }
}

/// A defect (README §7.3). The video defects change the frames, and a frame, where they take
/// one, defaults to [`Synth::defect_frame`]. The audio defects (`av50` on) change only the WAVs,
/// so their clips share the good clip's video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// `drop[@K]`: from output frame K on, every frame shows the next one's content.
    Drop(Option<u64>),
    /// `dup[@K]`: output frame K repeats frame K − 1's content, and the rest follow a frame late.
    Dup(Option<u64>),
    /// `judder`: N_k = r·k − d·(k mod 2) with r = source_hz / fps and d = ⌊r/4⌋ + 1, so steps
    /// alternate between r − d and r + d testbed frames, each one tick long but more than a
    /// quarter tick off (README §2 item 12): 3k − (k mod 2) at 180 Hz and 60 fps, steps of 11.1
    /// and 22.2 ms. Ratios of 2 and 4 have no such d, and are refused.
    Judder,
    /// `corrupt-bars`: 1 px luma lines 48 codes up every 16 px, both ways, inside the bars band,
    /// in every frame.
    CorruptBars,
    /// `bars-sparse`: the same lines 24 codes up every 64 px.
    BarsSparse,
    /// `unreadable`: both barcodes garbled in 5 frames from the defect frame on.
    Unreadable,
    /// `av50`: every WAV starts with [`AV50_SAMPLES`] of silence and loses as many at its end,
    /// so the sound plays 50 ms after its pictures.
    Av50,
    /// `swap-game-voice`: the Game WAV carries the voice beeps and the Voice chat WAV the game
    /// beeps, each under its own title: a swap the container can't see (both stereo).
    SwapGameVoice,
    /// `cue`: `testsig`'s placeholder cue in the Mix, at [`testsig::CUE_OFFSET_MS`] in every
    /// second.
    Cue,
    /// `leak`: the voice beep in the Game WAV too, [`LEAK_DB`] under its own level (−40 dBFS).
    Leak,
    /// `noisy`: white noise at [`NOISE_DBFS`] RMS in the Game WAV.
    Noisy,
}

impl Defect {
    /// Whether it changes only the WAVs.
    pub fn is_audio(self) -> bool {
        matches!(
            self,
            Self::Av50 | Self::SwapGameVoice | Self::Cue | Self::Leak | Self::Noisy
        )
    }
}

/// `av50`'s delay: 50 ms at 48 kHz.
pub const AV50_SAMPLES: usize = 2_400;

/// `leak`'s level against the voice beep's own, in dB.
pub const LEAK_DB: f64 = -10.0;

/// `noisy`'s level, in dBFS RMS: a tone SNR of about 22 dB, while the narrowband level where a
/// tone should be absent stays near −77 dBFS, so routing still passes (README §7.3).
pub const NOISE_DBFS: f64 = -55.0;

/// How many frames `unreadable` garbles.
pub const UNREADABLE_FRAMES: u64 = 5;

impl FromStr for Defect {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let usage = || "--defect takes drop[@K], dup[@K], judder, corrupt-bars, bars-sparse, unreadable, av50, swap-game-voice, cue, leak or noisy";
        let frame = |k: &str| k.parse::<u64>().map_err(|_| usage().to_owned());
        Ok(match text.split_once('@') {
            Some(("drop", k)) => Self::Drop(Some(frame(k)?)),
            Some(("dup", k)) => Self::Dup(Some(frame(k)?)),
            Some(_) => return Err(usage().to_owned()),
            None => match text {
                "drop" => Self::Drop(None),
                "dup" => Self::Dup(None),
                "judder" => Self::Judder,
                "corrupt-bars" => Self::CorruptBars,
                "bars-sparse" => Self::BarsSparse,
                "unreadable" => Self::Unreadable,
                "av50" => Self::Av50,
                "swap-game-voice" => Self::SwapGameVoice,
                "cue" => Self::Cue,
                "leak" => Self::Leak,
                "noisy" => Self::Noisy,
                _ => return Err(usage().to_owned()),
            },
        })
    }
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drop(None) => f.write_str("drop"),
            Self::Drop(Some(k)) => write!(f, "drop@{k}"),
            Self::Dup(None) => f.write_str("dup"),
            Self::Dup(Some(k)) => write!(f, "dup@{k}"),
            Self::Judder => f.write_str("judder"),
            Self::CorruptBars => f.write_str("corrupt-bars"),
            Self::BarsSparse => f.write_str("bars-sparse"),
            Self::Unreadable => f.write_str("unreadable"),
            Self::Av50 => f.write_str("av50"),
            Self::SwapGameVoice => f.write_str("swap-game-voice"),
            Self::Cue => f.write_str("cue"),
            Self::Leak => f.write_str("leak"),
            Self::Noisy => f.write_str("noisy"),
        }
    }
}

/// An audio track of a `synth` clip (README §7.1), in the clip's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTrack {
    /// The default track: every beep. No two overlap, so it peaks at −30 dBFS too.
    Mix,
    /// The game beep.
    Game,
    /// The mic marker, mono.
    Mic,
    /// The voice beep.
    Voice,
}

impl AudioTrack {
    /// Every track, in the clip's order.
    pub const ALL: [Self; 4] = [Self::Mix, Self::Game, Self::Mic, Self::Voice];

    /// Its title, which the mux writes as its name.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Mix => "Mix",
            Self::Game => "Game",
            Self::Mic => "Mic",
            Self::Voice => "Voice chat",
        }
    }

    /// Its WAV's name in `--wav-dir`.
    pub const fn file(self) -> &'static str {
        match self {
            Self::Mix => "mix.wav",
            Self::Game => "game.wav",
            Self::Mic => "mic.wav",
            Self::Voice => "voice.wav",
        }
    }

    /// 1 or 2.
    pub const fn channels(self) -> u16 {
        match self {
            Self::Mic => 1,
            _ => 2,
        }
    }

    /// Its AAC bitrate in the mux, in kbps.
    pub const fn kbps(self) -> u32 {
        match self {
            Self::Mic => 128,
            _ => 160,
        }
    }

    /// The beeps it carries in a good clip.
    pub const fn tones(self) -> &'static [Tone] {
        match self {
            Self::Mix => &[Tone::Game, Tone::Mic, Tone::Voice],
            Self::Game => &[Tone::Game],
            Self::Mic => &[Tone::Mic],
            Self::Voice => &[Tone::Voice],
        }
    }
}

/// A tone's name in an expect file.
fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Game => "game",
        Tone::Voice => "voice",
        Tone::Mic => "mic",
    }
}

/// The longest clip `synth` writes WAVs for: the checks decode no more audio than this.
pub const MAX_AUDIO_SECONDS: u32 = crate::profile::AUDIO_MAX_SECONDS as u32;

/// `--hdr`: the HDR source the patch row stands for. The patches show the SDR codes `curve`
/// should give; every other element keeps its SDR codes, so only the patch check is tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrSource {
    /// Natural (PQ + BT.2390) or Match SDR apps.
    pub curve: Curve,
    /// The display's peak, in nits.
    pub lw_nits: u32,
    /// The OS's SDR white, in nits.
    pub sdr_white_nits: u32,
}

/// What `synth` writes to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Raw NV12 frames, for an encoder.
    Nv12,
    /// The lossless I_PCM H.264 stream, Annex B, with an IDR every `gop` frames.
    Ipcm {
        /// Frames from one IDR to the next.
        gop: u32,
    },
}

/// Why `synth` stopped.
#[derive(Debug)]
pub enum SynthError {
    /// The settings can't be rendered.
    Settings(String),
    /// A frame couldn't be rendered.
    Render(String),
    /// Writing the frames failed, for instance because the encoder closed the pipe.
    Write(io::Error),
}

impl SynthError {
    /// The exit code: 2 for settings, 4 for the rest.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Settings(_) => 2,
            Self::Render(_) | Self::Write(_) => 4,
        }
    }
}

impl fmt::Display for SynthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(what) => f.write_str(what),
            Self::Render(what) => write!(f, "a frame couldn't be rendered: {what}"),
            Self::Write(error) => write!(f, "writing the frames failed: {error}"),
        }
    }
}

impl std::error::Error for SynthError {}

impl From<io::Error> for SynthError {
    fn from(error: io::Error) -> Self {
        Self::Write(error)
    }
}

/// The longest clip `synth` renders, in seconds.
pub const MAX_SECONDS: u32 = 3_600;

/// The most frames `synth` renders at once, one per thread.
pub const MAX_WORKERS: usize = 8;

/// A `synth` run's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Synth {
    /// The clip's size and rate.
    pub format: Format,
    /// The testbed's refresh rate in Hz.
    pub source_hz: u32,
    /// The clip's length.
    pub seconds: u32,
    /// The one defect, if any.
    pub defect: Option<Defect>,
    /// The HDR source the patches stand for, if any.
    pub hdr: Option<HdrSource>,
    /// What goes to stdout.
    pub codec: Codec,
}

impl Synth {
    /// Checks that the settings can be rendered, and as what they ask for.
    pub fn check(&self) -> Result<(), SynthError> {
        let bad = |what: String| Err(SynthError::Settings(what));
        let Format { width, height, fps } = self.format;
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return bad(format!("{width}x{height}: the size must be even"));
        }
        if let Err(error) = Size::new(width, height) {
            return bad(error.to_string());
        }
        if !(1..=1_000).contains(&fps) || !(1..=1_000).contains(&self.source_hz) {
            return bad("the frame rate and --source-hz must be 1–1000".to_owned());
        }
        if !(1..=MAX_SECONDS).contains(&self.seconds) {
            return bad(format!("--seconds must be 1–{MAX_SECONDS}"));
        }
        // The test profile allows no repeat or skip, so the testbed's frames must divide evenly
        // into the clip's (README §6.5: 0 at test's source rates).
        if !self.source_hz.is_multiple_of(fps) {
            return bad(format!(
                "--source-hz must be a whole multiple of the frame rate; {} isn't of {fps}",
                self.source_hz
            ));
        }
        let frames = self.frames();
        match self.defect {
            Some(Defect::Drop(_) | Defect::Dup(_)) => {
                let k = self.defect_at();
                if k == 0 || k >= frames {
                    return bad(format!(
                        "the defect's frame {k} must be 1–{} in a clip of {frames} frames",
                        frames - 1
                    ));
                }
            }
            Some(Defect::Judder) => {
                let r = self.source_hz / fps;
                if r < 3 || r == 4 || ![30, 60, 120].contains(&fps) {
                    return bad(format!(
                        "judder needs 30, 60 or 120 fps (the pacer's rates) and --source-hz 3 or at least 5 times that; {} isn't for {fps} fps",
                        self.source_hz
                    ));
                }
            }
            Some(Defect::Unreadable) if self.defect_at() + UNREADABLE_FRAMES > frames => {
                return bad(format!(
                    "unreadable garbles frames {} to {}; the clip has {frames}",
                    self.defect_at(),
                    self.defect_at() + UNREADABLE_FRAMES - 1
                ));
            }
            _ => {}
        }
        if let Some(hdr) = self.hdr
            && ![hdr.lw_nits, hdr.sdr_white_nits]
                .iter()
                .all(|n| (1..=10_000).contains(n))
        {
            return bad("--lw and --sdr-white must be 1–10000 nits".to_owned());
        }
        if let Codec::Ipcm { gop } = self.codec {
            Ipcm::new(width as usize, height as usize, fps, gop).map_err(SynthError::Settings)?;
        }
        Ok(())
    }

    /// Checks that the WAVs can be written too: an audio defect needs nothing more, but a clip
    /// longer than the checks read is refused.
    pub fn check_audio(&self) -> Result<(), SynthError> {
        if self.seconds > MAX_AUDIO_SECONDS {
            return Err(SynthError::Settings(format!(
                "--wav-dir takes clips of up to {MAX_AUDIO_SECONDS} s, as much audio as the checks read"
            )));
        }
        Ok(())
    }

    /// The number of output frames.
    pub fn frames(&self) -> u64 {
        u64::from(self.seconds) * u64::from(self.format.fps)
    }

    /// Where a defect goes unless `@K` says otherwise: midway between two flashes near the
    /// middle, fps · ⌊frames / (2 · fps)⌋ + ⌊fps / 2⌋ (frame 930 of 1,800; 150 of CI's 240;
    /// README §7.3).
    pub fn defect_frame(&self) -> u64 {
        let fps = u64::from(self.format.fps);
        fps * (self.frames() / (2 * fps)) + fps / 2
    }

    /// This run's defect frame: `@K` or [`Synth::defect_frame`].
    fn defect_at(&self) -> u64 {
        match self.defect {
            Some(Defect::Drop(Some(k)) | Defect::Dup(Some(k))) => k,
            _ => self.defect_frame(),
        }
    }

    /// N_k, the testbed frame each output frame shows.
    pub fn content(&self) -> Vec<u64> {
        let (source, fps) = (u64::from(self.source_hz), u64::from(self.format.fps));
        let ideal = |k: u64| k * source / fps;
        let at = self.defect_at();
        (0..self.frames())
            .map(|k| match self.defect {
                Some(Defect::Drop(_)) if k >= at => ideal(k + 1),
                Some(Defect::Dup(_)) if k >= at => ideal(k.saturating_sub(1)),
                Some(Defect::Judder) => {
                    let r = source / fps;
                    r * k - (r / 4 + 1) * (k % 2)
                }
                _ => ideal(k),
            })
            .collect()
    }

    /// The pattern's options: the testbed's rate times the flash; the run tag is 0.
    fn options(&self) -> Options {
        Options {
            pattern: Pattern::Full,
            nominal_fps: self.source_hz,
            run_tag: 0,
            load: Load::Scroll,
            patches: match self.hdr {
                None => Patches::Sdr,
                Some(hdr) => Patches::ExpectedSdr {
                    curve: hdr.curve,
                    peak_nits: f64::from(hdr.lw_nits),
                    sdr_white_nits: f64::from(hdr.sdr_white_nits),
                },
            },
        }
    }

    /// The damage a defect paints.
    fn damage(&self) -> Damage {
        let at = self.defect_at();
        match self.defect {
            Some(Defect::CorruptBars) => Damage {
                gridlines: Some((16, 48)),
                ..Damage::default()
            },
            Some(Defect::BarsSparse) => Damage {
                gridlines: Some((64, 24)),
                ..Damage::default()
            },
            Some(Defect::Unreadable) => Damage {
                garble: (at..at + UNREADABLE_FRAMES).map(|k| k as usize).collect(),
                ..Damage::default()
            },
            _ => Damage::default(),
        }
    }

    /// The clip's `expect.json`, as the encoded clip should come out: with [`AudioTrack::ALL`]
    /// as a good clip carries them when its WAVs are muxed in (`audio`), otherwise video only.
    pub fn expect(&self, audio: bool) -> Expect {
        Expect {
            kairo_expect: expect::VERSION,
            pattern: Some(1),
            size: Some([self.format.width, self.format.height]),
            fps: self.format.fps,
            source_hz: Some(self.source_hz),
            seconds: self.seconds,
            run_tag: Some(0),
            hdr: self.hdr.map(|hdr| Hdr {
                curve: match hdr.curve {
                    Curve::Natural => expect::CURVES[0],
                    Curve::MatchSdrApps => expect::CURVES[1],
                }
                .to_owned(),
                lw_nits: hdr.lw_nits,
                sdr_white_nits: hdr.sdr_white_nits,
            }),
            trim: None,
            cabac: matches!(self.codec, Codec::Ipcm { .. }).then_some(false),
            tracks: if audio {
                AudioTrack::ALL
                    .iter()
                    .map(|&track| {
                        let mut tones: BTreeMap<String, expect::Tone> =
                            [Tone::Game, Tone::Voice, Tone::Mic]
                                .into_iter()
                                .map(|tone| {
                                    let want = if track.tones().contains(&tone) {
                                        expect::Tone::Present
                                    } else {
                                        expect::Tone::Absent
                                    };
                                    (tone_name(tone).to_owned(), want)
                                })
                                .collect();
                        tones.insert("cue".to_owned(), expect::Tone::Absent);
                        expect::Track {
                            title: track.title().to_owned(),
                            channels: track.channels(),
                            default: track == AudioTrack::Mix,
                            tones,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }
    }

    /// Track `track`'s sounds at 48 kHz, mono (each channel carries the same): in every second
    /// m, each beep the track carries at m s plus the beep's offset, changed by this run's audio
    /// defect.
    pub fn track_audio(&self, track: AudioTrack) -> SyntheticAudio {
        let rate = crate::analyze::audio::RATE;
        let late = match self.defect {
            Some(Defect::Av50) => AV50_SAMPLES as f64 / rate,
            _ => 0.0,
        };
        let tones = match (self.defect, track) {
            (Some(Defect::SwapGameVoice), AudioTrack::Game) => AudioTrack::Voice.tones(),
            (Some(Defect::SwapGameVoice), AudioTrack::Voice) => AudioTrack::Game.tones(),
            _ => track.tones(),
        };
        let at = |second: f64, ms: u32| second + f64::from(ms) / 1_000.0;
        let beep = |tone: Tone| Sound::Beep(f64::from(tone.hz()));
        let mut sounds = Vec::new();
        for m in 0..self.seconds {
            let second = f64::from(m) + late;
            for &tone in tones {
                sounds.push((beep(tone), at(second, tone.offset_ms()), 0.0));
            }
            match (self.defect, track) {
                (Some(Defect::Cue), AudioTrack::Mix) => {
                    sounds.push((Sound::Cue, at(second, testsig::CUE_OFFSET_MS), 0.0));
                }
                (Some(Defect::Leak), AudioTrack::Game) => {
                    let voice = Tone::Voice;
                    sounds.push((beep(voice), at(second, voice.offset_ms()), LEAK_DB));
                }
                _ => {}
            }
        }
        SyntheticAudio {
            frames: self.seconds as usize * rate as usize,
            sounds,
            noise_db: (self.defect == Some(Defect::Noisy) && track == AudioTrack::Game)
                .then_some(NOISE_DBFS),
        }
    }

    /// Track `track`'s WAV: 32-bit float at 48 kHz.
    pub fn write_wav(&self, track: AudioTrack, out: &mut dyn Write) -> io::Result<()> {
        if let Err(error) = self.check().and_then(|()| self.check_audio()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                error.to_string(),
            ));
        }
        let mono = self.track_audio(track).render();
        let mut out = io::BufWriter::with_capacity(1 << 20, out);
        crate::wav::write_float(&mut out, 48_000, track.channels(), &mono)?;
        out.flush()
    }

    /// The frames' renderer.
    pub fn source(&self) -> Result<Synthetic, SynthError> {
        let size = Size::new(self.format.width, self.format.height)
            .map_err(|e| SynthError::Settings(e.to_string()))?;
        Ok(Synthetic::new(
            size,
            self.options(),
            self.content(),
            self.damage(),
        ))
    }

    /// Renders every frame to `out`, as NV12 or I_PCM H.264. Frames are rendered and packed a
    /// batch at a time, one per core (at most [`MAX_WORKERS`]), and written in order.
    pub fn write(&self, out: &mut dyn Write) -> Result<(), SynthError> {
        self.check()?;
        let source = self.source()?;
        let Format { width, height, fps } = self.format;
        let stream = match self.codec {
            Codec::Nv12 => None,
            Codec::Ipcm { gop } => Some(
                Ipcm::new(width as usize, height as usize, fps, gop)
                    .map_err(SynthError::Settings)?,
            ),
        };
        // Frame `index`'s bytes: NV12, or its slice after the SPS and PPS on an IDR.
        let pack = |index: u64| -> Result<Vec<u8>, SynthError> {
            let picture = source
                .frame(index as usize)
                .map_err(|e| SynthError::Render(e.to_string()))?;
            let mut bytes = Vec::new();
            match &stream {
                Some(stream) => {
                    if stream.is_idr(index) {
                        bytes = stream.headers();
                    }
                    bytes.extend(
                        stream
                            .picture(index, &picture)
                            .map_err(SynthError::Render)?,
                    );
                }
                None => nv12(&picture, &mut bytes),
            }
            Ok(bytes)
        };
        let pack = &pack;
        let workers =
            std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_WORKERS)) as u64;
        let mut out = io::BufWriter::with_capacity(1 << 20, out);
        let frames = self.frames();
        let mut first = 0;
        while first < frames {
            let last = frames.min(first + workers);
            let batch: Vec<Result<Vec<u8>, SynthError>> = std::thread::scope(|scope| {
                // A thread the system can't make renders its frame here instead.
                let workers: Vec<_> = (first..last)
                    .map(|index| {
                        std::thread::Builder::new()
                            .spawn_scoped(scope, move || pack(index))
                            .map_err(|_| index)
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|worker| match worker {
                        Ok(worker) => worker.join().unwrap_or_else(|_| {
                            Err(SynthError::Render("a render thread panicked".to_owned()))
                        }),
                        Err(index) => pack(index),
                    })
                    .collect()
            });
            for bytes in batch {
                out.write_all(&bytes?)?;
            }
            first = last;
        }
        out.flush()?;
        Ok(())
    }
}

impl fmt::Display for Synth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} frames, {} from a {} Hz testbed, ",
            self.frames(),
            self.format,
            self.source_hz
        )?;
        match self.codec {
            Codec::Nv12 => f.write_str("NV12")?,
            Codec::Ipcm { gop } => write!(f, "I_PCM H.264 (GOP {gop})")?,
        }
        if let Some(defect) = self.defect {
            write!(f, ", defect {defect}")?;
        }
        if let Some(hdr) = self.hdr {
            let curve = match hdr.curve {
                Curve::Natural => "natural",
                Curve::MatchSdrApps => "match",
            };
            write!(
                f,
                ", HDR {curve} {} / {} nits",
                hdr.lw_nits, hdr.sdr_white_nits
            )?;
        }
        Ok(())
    }
}

/// Packs `picture` as NV12 into `out`: the luma plane, then Cb and Cr interleaved.
fn nv12(picture: &Picture, out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&picture.y);
    for (&cb, &cr) in picture.cb.iter().zip(&picture.cr) {
        out.extend_from_slice(&[cb, cr]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(defect: Option<Defect>) -> Synth {
        Synth {
            format: "160x90@60".parse().unwrap(),
            source_hz: 180,
            seconds: 4,
            defect,
            hdr: None,
            codec: Codec::Nv12,
        }
    }

    #[test]
    fn presets_and_defects_parse() {
        assert_eq!(
            "1080p60".parse::<Format>(),
            Ok(Format {
                width: 1920,
                height: 1080,
                fps: 60
            })
        );
        assert_eq!("1440p60".parse::<Format>().map(|f| f.width), Ok(2560));
        assert_eq!(
            "480x270@60"
                .parse::<Format>()
                .map(|f| (f.width, f.height, f.fps)),
            Ok((480, 270, 60))
        );
        for bad in ["720p60", "480x270", "480@60", "x270@60", "480x270@", "4k"] {
            assert!(bad.parse::<Format>().is_err(), "{bad}");
        }
        for text in [
            "drop",
            "drop@930",
            "dup",
            "dup@7",
            "judder",
            "corrupt-bars",
            "bars-sparse",
            "unreadable",
            "av50",
            "swap-game-voice",
            "cue",
            "leak",
            "noisy",
        ] {
            let defect = text.parse::<Defect>();
            assert_eq!(defect.clone().map(|d| d.to_string()), Ok(text.to_owned()));
            let audio = ["av50", "swap-game-voice", "cue", "leak", "noisy"].contains(&text);
            assert_eq!(defect.map(Defect::is_audio), Ok(audio), "{text}");
        }
        for bad in ["drop@", "drop@x", "judder@3", "av50@3", "Drop", "swap", ""] {
            assert!(bad.parse::<Defect>().is_err(), "{bad}");
        }
    }

    #[test]
    fn defect_frames_sit_between_flashes() {
        let at = |seconds: u32, fps: u32| {
            Synth {
                seconds,
                format: Format {
                    width: 160,
                    height: 90,
                    fps,
                },
                ..synth(None)
            }
            .defect_frame()
        };
        // README §7.3: frame 930 of 1,800 and 150 of CI's 240.
        assert_eq!(at(30, 60), 930);
        assert_eq!(at(4, 60), 150);
        assert_eq!(at(1, 60), 30);
        assert_eq!(at(3, 30), 45);
    }

    #[test]
    fn content_follows_the_defect() {
        assert_eq!(synth(None).content()[..4], [0, 3, 6, 9]);
        let at = 150;
        let drop = synth(Some(Defect::Drop(None))).content();
        assert_eq!(drop[at - 1..at + 2], [447, 453, 456]);
        let dup = synth(Some(Defect::Dup(Some(10)))).content();
        assert_eq!(dup[9..12], [27, 27, 30]);
        // README §7.3's judder recipe at 180 Hz: 3k − (k mod 2).
        let judder = synth(Some(Defect::Judder)).content();
        assert_eq!(judder[..5], [0, 2, 6, 8, 12]);
        assert_eq!(synth(None).content().len(), 240);
    }

    #[test]
    fn judder_is_judder_at_every_ratio_it_takes() {
        // Every step one tick long and more than a quarter tick off, none repeated or skipped
        // (review of PR 4c: at 360 Hz, 3k − (k mod 2)'s steps weren't judder).
        for (source, fps) in [
            (180, 60),
            (300, 60),
            (360, 60),
            (480, 60),
            (180, 30),
            (360, 120),
        ] {
            let s = Synth {
                format: Format {
                    width: 160,
                    height: 90,
                    fps,
                },
                source_hz: source,
                ..synth(Some(Defect::Judder))
            };
            assert!(s.check().is_ok(), "{source}/{fps}");
            let numbers: Vec<Option<i64>> = s.content().iter().map(|&n| Some(n as i64)).collect();
            let counts = crate::analyze::cadence::count(&numbers, source, fps).unwrap();
            assert_eq!((counts.repeats, counts.skipped), (0, 0), "{source}/{fps}");
            assert_eq!(counts.judder, counts.steps, "{source}/{fps}");
        }
    }

    #[test]
    fn impossible_settings_are_refused() {
        let refused = |s: Synth| matches!(s.check(), Err(SynthError::Settings(_)));
        assert!(synth(None).check().is_ok());
        for defect in [
            Defect::Drop(Some(0)),
            Defect::Drop(Some(240)),
            Defect::Dup(Some(0)),
        ] {
            assert!(refused(synth(Some(defect))), "{defect}");
        }
        assert!(synth(Some(Defect::Drop(Some(239)))).check().is_ok());
        // Uneven rates: `test` allows no natural repeat or skip.
        assert!(refused(Synth {
            source_hz: 165,
            ..synth(None)
        }));
        // Judder needs a ratio with a usable offset, and a rate the pacer runs at.
        for (source, fps) in [(60, 60), (120, 60), (240, 60), (150, 50)] {
            assert!(
                refused(Synth {
                    format: Format {
                        width: 160,
                        height: 90,
                        fps
                    },
                    source_hz: source,
                    ..synth(Some(Defect::Judder))
                }),
                "{source}/{fps}"
            );
        }
        assert!(refused(Synth {
            format: "161x90@60".parse().unwrap(),
            ..synth(None)
        }));
        assert!(refused(Synth {
            seconds: 0,
            ..synth(None)
        }));
        assert!(refused(Synth {
            format: "160x90@1001".parse().unwrap(),
            ..synth(None)
        }));
        // One frame a second for 4 s: no room for 5 garbled frames from frame 2.
        assert!(refused(Synth {
            format: "160x90@1".parse().unwrap(),
            ..synth(Some(Defect::Unreadable))
        }));
        assert!(refused(Synth {
            hdr: Some(HdrSource {
                curve: Curve::Natural,
                lw_nits: 0,
                sdr_white_nits: 240
            }),
            ..synth(None)
        }));
        assert!(refused(Synth {
            codec: Codec::Ipcm { gop: 0 },
            ..synth(None)
        }));
    }

    #[test]
    fn the_expect_file_describes_the_clip() {
        let mut s = Synth {
            hdr: Some(HdrSource {
                curve: Curve::MatchSdrApps,
                lw_nits: 1_000,
                sdr_white_nits: 240,
            }),
            codec: Codec::Ipcm { gop: 60 },
            ..synth(None)
        };
        let text = serde_json::to_string(&s.expect(false)).unwrap();
        let expect = Expect::parse(&text).unwrap();
        assert_eq!(expect, s.expect(false));
        assert_eq!(
            (expect.size, expect.fps, expect.source_hz, expect.seconds),
            (Some([160, 90]), 60, Some(180), 4)
        );
        assert_eq!(expect.cabac, Some(false));
        assert_eq!(expect.hdr.map(|h| h.curve), Some("match".to_owned()));
        assert!(expect.tracks.is_empty());
        // NV12 leaves the entropy coder to the encoder: no `cabac` at all.
        s.codec = Codec::Nv12;
        assert!(
            !serde_json::to_string(&s.expect(false))
                .unwrap()
                .contains("cabac")
        );
        // With its WAVs, README §6.1's tracks: the Mix is the default and carries every beep.
        let expect = Expect::parse(&serde_json::to_string(&s.expect(true)).unwrap()).unwrap();
        let tracks: Vec<(&str, u16, bool, Vec<&str>)> = expect
            .tracks
            .iter()
            .map(|t| {
                let present = t
                    .tones
                    .iter()
                    .filter(|(_, want)| **want == expect::Tone::Present)
                    .map(|(name, _)| name.as_str())
                    .collect();
                (t.title.as_str(), t.channels, t.default, present)
            })
            .collect();
        assert_eq!(
            tracks,
            [
                ("Mix", 2, true, vec!["game", "mic", "voice"]),
                ("Game", 2, false, vec!["game"]),
                ("Mic", 1, false, vec!["mic"]),
                ("Voice chat", 2, false, vec!["voice"]),
            ]
        );
        assert!(
            expect
                .tracks
                .iter()
                .all(|t| t.tones.len() == 4 && t.tones.get("cue") == Some(&expect::Tone::Absent))
        );
    }

    #[test]
    fn audio_defects_leave_the_frames_alone() {
        let good = synth(None);
        for defect in [
            Defect::Av50,
            Defect::SwapGameVoice,
            Defect::Cue,
            Defect::Leak,
            Defect::Noisy,
        ] {
            let s = synth(Some(defect));
            assert!(s.check().is_ok(), "{defect}");
            assert_eq!(s.content(), good.content(), "{defect}");
            assert_eq!(s.damage(), good.damage(), "{defect}");
        }
    }

    #[test]
    fn the_wavs_follow_research_16s_layout() {
        // 2 s: in each second the game beep at 0, the mic marker at 250 ms and the voice beep at
        // 500 ms, each 100 ms long and on its own track, all of them on the Mix.
        let s = Synth {
            seconds: 2,
            ..synth(None)
        };
        let render = |s: &Synth, track| s.track_audio(track).render();
        let [mix, game, mic, voice] = AudioTrack::ALL.map(|t| render(&s, t));
        let sounding =
            |x: &[f32]| -> Vec<usize> { (0..x.len()).filter(|&n| x[n] != 0.0).collect() };
        let spans = |starts_ms: &[u32]| -> Vec<usize> {
            let mut n: Vec<usize> = starts_ms
                .iter()
                .flat_map(|&ms| {
                    let first = ms as usize * 48;
                    // Each burst's first sample is sin(0) = 0.
                    first + 1..first + 4_800
                })
                .collect();
            n.sort_unstable();
            n
        };
        assert_eq!(game.len(), 96_000);
        assert_eq!(sounding(&game), spans(&[0, 1_000]));
        assert_eq!(sounding(&mic), spans(&[250, 1_250]));
        assert_eq!(sounding(&voice), spans(&[500, 1_500]));
        for n in 0..mix.len() {
            assert_eq!(mix[n], game[n] + mic[n] + voice[n], "{n}");
        }
        let peak = game.iter().fold(0_f32, |p, v| p.max(v.abs()));
        assert!(
            (20.0 * f64::from(peak).log10() + 30.0).abs() < 0.01,
            "{peak}"
        );
        // av50: everything 2,400 samples later, the end cut.
        let av50 = Synth {
            defect: Some(Defect::Av50),
            ..s
        };
        for (track, good) in AudioTrack::ALL.into_iter().zip([&mix, &game, &mic, &voice]) {
            let late = render(&av50, track);
            assert_eq!(late.len(), good.len());
            assert!(late[..AV50_SAMPLES].iter().all(|&v| v == 0.0));
            for n in AV50_SAMPLES..late.len() {
                assert!((late[n] - good[n - AV50_SAMPLES]).abs() < 1e-7, "{n}");
            }
        }
        // swap-game-voice: the two WAVs trade places under their own titles.
        let swap = Synth {
            defect: Some(Defect::SwapGameVoice),
            ..s
        };
        assert_eq!(render(&swap, AudioTrack::Game), voice);
        assert_eq!(render(&swap, AudioTrack::Voice), game);
        assert_eq!(render(&swap, AudioTrack::Mix), mix);
        // cue: the placeholder in the Mix at 750 ms in each second, 220 ms long.
        let cue = Synth {
            defect: Some(Defect::Cue),
            ..s
        };
        let extra: Vec<f32> = render(&cue, AudioTrack::Mix)
            .iter()
            .zip(&mix)
            .map(|(a, b)| a - b)
            .collect();
        assert_eq!(
            sounding(&extra),
            [750, 1_750]
                .iter()
                .flat_map(|&ms| ms * 48 + 1..ms * 48 + 10_560)
                .filter(|&n| n != 750 * 48 + 4_800 && n != 1_750 * 48 + 4_800)
                .collect::<Vec<usize>>()
        );
        assert_eq!(render(&cue, AudioTrack::Game), game);
        // leak: the voice beep in Game at −40 dBFS.
        let leak = Synth {
            defect: Some(Defect::Leak),
            ..s
        };
        let extra: Vec<f32> = render(&leak, AudioTrack::Game)
            .iter()
            .zip(&game)
            .map(|(a, b)| a - b)
            .collect();
        assert_eq!(sounding(&extra), spans(&[500, 1_500]));
        let peak = extra.iter().fold(0_f32, |p, v| p.max(v.abs()));
        assert!(
            (20.0 * f64::from(peak).log10() + 40.0).abs() < 0.01,
            "{peak}"
        );
        // noisy: noise at −55 dBFS RMS in Game only.
        let noisy = Synth {
            defect: Some(Defect::Noisy),
            ..s
        };
        let noise: Vec<f64> = render(&noisy, AudioTrack::Game)
            .iter()
            .zip(&game)
            .map(|(a, b)| f64::from(a - b))
            .collect();
        let rms = (noise.iter().map(|v| v * v).sum::<f64>() / noise.len() as f64).sqrt();
        assert!((20.0 * rms.log10() + 55.0).abs() < 0.1, "{rms}");
        assert_eq!(render(&noisy, AudioTrack::Mix), mix);
    }

    #[test]
    fn the_wavs_are_float_at_48_khz() {
        let s = Synth {
            seconds: 1,
            ..synth(None)
        };
        for track in AudioTrack::ALL {
            let mut out = Vec::new();
            s.write_wav(track, &mut out).unwrap();
            let wav = crate::wav::read(&out).unwrap();
            assert_eq!((wav.rate, wav.channels), (48_000, track.channels()));
            let mono = s.track_audio(track).render();
            assert_eq!(
                wav.samples.len(),
                mono.len() * usize::from(track.channels())
            );
            for (frame, &want) in wav.samples.chunks(usize::from(track.channels())).zip(&mono) {
                assert!(frame.iter().all(|&v| v == want));
            }
        }
        let long = Synth {
            seconds: MAX_AUDIO_SECONDS + 1,
            ..s
        };
        assert!(long.check_audio().is_err());
        assert!(long.write_wav(AudioTrack::Mic, &mut Vec::new()).is_err());
    }

    #[test]
    fn nv12_frames_interleave_the_chroma() {
        let s = synth(None);
        let mut out = Vec::new();
        s.write(&mut out).unwrap();
        let frame = 160 * 90 * 3 / 2;
        assert_eq!(out.len(), 240 * frame);
        let picture = s.source().unwrap().frame(7).unwrap();
        let seventh = &out[7 * frame..8 * frame];
        assert_eq!(&seventh[..160 * 90], &picture.y[..]);
        let chroma = &seventh[160 * 90..];
        assert_eq!((chroma[0], chroma[1]), (picture.cb[0], picture.cr[0]));
        assert_eq!(
            (chroma[2 * 100], chroma[2 * 100 + 1]),
            (picture.cb[100], picture.cr[100])
        );
    }

    /// Opt-in, on the Mac: `KAIRO_FFMPEG=/opt/homebrew/bin/ffmpeg cargo test -p kairo-verify --
    /// --ignored`. Gate 1's CI corpus, video only (README §7.2–7.3): each I_PCM clip goes
    /// through FFmpeg's raw H.264 demuxer and MP4 muxer by stream copy, as the CI recipe does,
    /// and is checked under `test` through the CLI decoder. The good clip decodes to `synth`'s
    /// exact frames and fails nothing; each defect fails its own check and nothing outside its
    /// `may_fail`. The input `-r 60` matters: with `-framerate 60` alone, FFmpeg 8.0.1's CLI
    /// times the frames in whole microseconds and some samples last 20001 ticks of 1.2 MHz.
    #[cfg(feature = "dev-tools")]
    #[test]
    #[ignore = "needs KAIRO_FFMPEG"]
    fn the_ci_corpus_video_clips_behave_through_ffmpeg() {
        use std::ops::ControlFlow;
        use std::process::{Command, Stdio};

        use crate::checks::Status;
        use crate::decode::Decoder;
        use crate::decode::cli::Cli;
        use crate::profile::{Plan, Profile};

        // It runs only with --ignored, so a missing FFmpeg is an error, not a pass.
        let ffmpeg = std::env::var_os("KAIRO_FFMPEG")
            .expect("set KAIRO_FFMPEG to an FFmpeg to spawn, e.g. /opt/homebrew/bin/ffmpeg");
        let dir = std::env::temp_dir().join(format!("kairo-verify-synth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = Synth {
            format: "480x270@60".parse().unwrap(),
            source_hz: 180,
            seconds: 4,
            defect: None,
            hdr: None,
            codec: Codec::Ipcm { gop: 60 },
        };
        let defect = |d| Synth {
            defect: Some(d),
            ..good
        };
        let hdr = |curve| Synth {
            hdr: Some(HdrSource {
                curve,
                lw_nits: 1_000,
                sdr_white_nits: 240,
            }),
            ..good
        };
        let mut swapped = hdr(Curve::MatchSdrApps).expect(false);
        if let Some(h) = swapped.hdr.as_mut() {
            h.curve = "natural".to_owned();
        }
        // Name, settings, the expect file, must_fail, may_fail.
        type Case = (
            &'static str,
            Synth,
            Expect,
            &'static [&'static str],
            &'static [&'static str],
        );
        let cases: Vec<Case> = vec![
            ("good", good, good.expect(false), &[], &[]),
            (
                "drop1",
                defect(Defect::Drop(None)),
                good.expect(false),
                &["cadence.drops_dups"],
                &[],
            ),
            (
                "dup1",
                defect(Defect::Dup(None)),
                good.expect(false),
                &["cadence.drops_dups"],
                &[],
            ),
            (
                "judder",
                defect(Defect::Judder),
                good.expect(false),
                &["cadence.judder"],
                &[],
            ),
            (
                "bars",
                defect(Defect::CorruptBars),
                good.expect(false),
                &["content.bars"],
                &["content.bars_lines"],
            ),
            (
                "bars-sparse",
                defect(Defect::BarsSparse),
                good.expect(false),
                &["content.bars_lines"],
                &["content.bars"],
            ),
            (
                "unreadable",
                defect(Defect::Unreadable),
                good.expect(false),
                &["content.barcode"],
                &[],
            ),
            (
                "gop2s",
                Synth {
                    codec: Codec::Ipcm { gop: 120 },
                    ..good
                },
                good.expect(false),
                &["video.gop"],
                &[],
            ),
            (
                "good-hdr",
                hdr(Curve::Natural),
                hdr(Curve::Natural).expect(false),
                &[],
                &[],
            ),
            (
                "curve-swap",
                hdr(Curve::MatchSdrApps),
                swapped,
                &["hdr.curve_levels"],
                &[],
            ),
        ];
        for (name, synth, expect, must_fail, may_fail) in cases {
            let file = format!("{name}.mp4");
            let mut child = Command::new(&ffmpeg)
                .current_dir(&dir)
                .args(["-hide_banner", "-nostdin", "-v", "error", "-y"])
                .args(["-f", "h264", "-r", "60", "-i", "-", "-c:v", "copy"])
                .args(["-color_primaries", "bt709", "-color_trc", "bt709"])
                .args(["-colorspace", "bt709", "-color_range", "tv"])
                .args(["-an", "-movflags", "+faststart", "-use_editlist", "0"])
                .arg(&file)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = child.stdin.take().unwrap();
            synth.write(&mut stdin).unwrap();
            drop(stdin);
            assert!(child.wait().unwrap().success(), "{name}");
            let clip = dir.join(&file);
            let plan = Plan::new(Profile::Test, false, Some(expect)).unwrap();
            let mut decoder = Cli::new(&ffmpeg, &clip);
            let mut mp4 = std::fs::File::open(&clip).unwrap();
            let report = crate::check_with(&mut mp4, &file, &plan, Some(&mut decoder));
            let failed: Vec<&str> = report
                .checks
                .iter()
                .filter(|f| f.status == Status::Fail)
                .map(|f| f.id)
                .collect();
            for id in must_fail {
                assert!(
                    failed.contains(id),
                    "{name}: {id} passed\n{}",
                    report.to_text()
                );
            }
            for id in &failed {
                assert!(
                    must_fail.contains(id) || may_fail.contains(id),
                    "{name}: {id} failed\n{}",
                    report.to_text()
                );
            }
            if name == "good" {
                // Lossless: FFmpeg's decoder gives back exactly what was rendered.
                let source = synth.source().unwrap();
                let mut exact = 0;
                Cli::new(&ffmpeg, &clip)
                    .video(&mut |index, planes| {
                        let p = source.frame(index).unwrap();
                        let (w, cw) = (p.width, p.width / 2);
                        let same = (0..p.height).all(|r| planes.y_row(r) == &p.y[r * w..][..w])
                            && (0..p.height / 2).all(|r| {
                                planes.cb_row(r) == &p.cb[r * cw..][..cw]
                                    && planes.cr_row(r) == &p.cr[r * cw..][..cw]
                            });
                        exact += usize::from(same);
                        ControlFlow::Continue(())
                    })
                    .unwrap();
                assert_eq!(exact, 240);
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
