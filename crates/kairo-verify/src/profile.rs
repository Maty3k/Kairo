// SPDX-License-Identifier: GPL-3.0-or-later
//! Profiles: which checks run, which ones fail a clip, and the limits they use
//! (docs/research/16-verify-testbed/README.md §6.5; container.md §7). Every limit carries its
//! source, and the report repeats it.

use std::fmt;

use crate::expect::Expect;

/// What kind of clip is being checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Gate 1's synthetic clips.
    Test,
    /// FFmpeg CLI captures of the testbed: gate 2, later SP2.
    Capture,
    /// `capture` with cadence as warnings: the media-PR smoke.
    Smoke,
    /// The engine's saves (M3 on).
    Engine,
    /// A share copy (Quick share, Copy, share upload).
    Share,
    /// Copy for Discord.
    Discord,
    /// WhatsApp or phone.
    Whatsapp,
    /// TikTok, Shorts or Reels.
    Reels,
    /// "Original (all tracks)": a stream copy of a save.
    Original,
}

impl Profile {
    /// Every profile, in the order the help lists them.
    pub const ALL: [Profile; 9] = [
        Profile::Test,
        Profile::Capture,
        Profile::Smoke,
        Profile::Engine,
        Profile::Share,
        Profile::Discord,
        Profile::Whatsapp,
        Profile::Reels,
        Profile::Original,
    ];

    /// Its name on the command line and in reports.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Test => "test",
            Profile::Capture => "capture",
            Profile::Smoke => "smoke",
            Profile::Engine => "engine",
            Profile::Share => "share",
            Profile::Discord => "discord",
            Profile::Whatsapp => "whatsapp",
            Profile::Reels => "reels",
            Profile::Original => "original",
        }
    }

    /// The profile called `name`.
    pub fn parse(name: &str) -> Option<Profile> {
        Profile::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Whether it is a re-encoded export with one audio track.
    pub fn is_share_export(self) -> bool {
        matches!(
            self,
            Profile::Share | Profile::Discord | Profile::Whatsapp | Profile::Reels
        )
    }

    /// Whether its clips are made from the testbed pattern and need `--expect`.
    pub fn needs_expect(self) -> bool {
        matches!(self, Profile::Test | Profile::Capture | Profile::Smoke)
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How a check counts in a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A violation fails the clip.
    Required,
    /// A violation is a warning.
    Warning,
    /// Not run.
    Off,
}

/// The checks that read the container and the H.264 and AAC headers, in report order
/// (container.md's C0–C14, renamed in README §6.2).
pub const STRUCTURE_CHECKS: [&str; 15] = [
    "container.structure",
    "container.faststart",
    "container.no_editlist",
    "video.colr",
    "video.params",
    "video.cfr",
    "av.start_alignment",
    "video.no_reorder",
    "video.idr_start",
    "video.gop",
    "tracks.layout",
    "audio.format",
    "duration",
    "video.single_session",
    "video.bitrate",
];

/// The export checks that need no decoding (README §6.5).
pub const EXPORT_CHECKS: [&str; 3] = ["export.size", "export.shape", "export.tags"];

/// The checks that need decoded pictures or audio (README §6.3), which come with research 16's
/// PRs 4 and 5. This build lists them so a profile that needs them can't pass without them.
pub const CONTENT_CHECKS: [&str; 19] = [
    "decode.clean",
    "content.barcode",
    "cadence.drops_dups",
    "cadence.judder",
    "content.flash",
    "content.bars",
    "content.colour_matrix",
    "content.bars_lines",
    "content.levels",
    "hdr.curve_levels",
    "source.stability",
    "av.offset",
    "av.offset_tight",
    "av.inter_track",
    "audio.tone_snr",
    "audio.routing",
    "audio.cue",
    "build.lgpl",
    "export.trim",
];

/// The checks `--gate m3` lets decide the verdict (README §6.5, PLAN:446).
pub const GATE_M3_CHECKS: [&str; 10] = [
    "container.structure",
    "duration",
    "video.cfr",
    "video.idr_start",
    "video.no_reorder",
    "container.faststart",
    "container.no_editlist",
    "video.colr",
    "tracks.layout",
    "av.offset",
];

/// One GOP in seconds: "a 1 s closed IDR GOP with `gop_size` = fps" (PLAN:166).
pub const GOP_SECONDS: u32 = 1;
/// `content.barcode`: the least share of frames with a readable barcode outside `test`, which
/// needs every frame (README §6.5, proposed).
pub const BARCODE_MIN_READABLE: f64 = 0.999;
/// A barcode read with a margin below this warns (README §4.6 step 4).
pub const BARCODE_MARGIN_WARN: f64 = 0.5;
/// `cadence.drops_dups`: the most (repeats + skipped) / steps above the ideal delivery's,
/// outside `test`, which allows none (PLAN:351, README §2 item 11, proposed).
pub const DROPS_DUPS_MAX_EXCESS: f64 = 0.005;
/// `engine`'s longest hold and largest step, in ticks (M3 task 4, core/cadence.rs).
pub const ENGINE_MAX_HOLD: (usize, i128) = (2, 2);
/// `cadence.judder`: the most judder steps above the ideal delivery's, as a share, outside
/// `test`, which allows none (README §2 item 12, proposed).
pub const JUDDER_MAX_EXCESS: f64 = 0.05;
/// How many first frames are searched for the pattern in a clip that needn't hold it
/// (README §6.1).
pub const PATTERN_PROBE_FRAMES: usize = 60;
/// `content.bars`: every frame's worst plane at least this many dB (PLAN:351).
pub const BARS_MIN_DB: f64 = 40.0;
/// The same for the share exports, proposed (README §2 item 14).
pub const EXPORT_BARS_MIN_DB: f64 = 30.0;
/// The ramp's Cb and Cr may stray this many codes from grey (README §6.3).
pub const RAMP_CHROMA_TOLERANCE: f64 = 2.0;
/// `content.colour_matrix`: another reference beating BT.709 limited by this many dB fails,
/// proposed (README §2 item 14).
pub const COLOUR_MATRIX_MARGIN_DB: f64 = 6.0;
/// `content.bars_lines`: the largest row or column residual, proposed (README §2 item 14).
pub const BARS_LINES_MAX_CODES: f64 = 4.0;
/// `content.levels` and `hdr.curve_levels`: ±2 codes (SP3, PLAN:430).
pub const LEVELS_TOLERANCE: f64 = 2.0;
/// `av.offset`: every beep within this many ms of its picture (PLAN:351).
pub const AV_MAX_MS: f64 = 40.0;
/// `av.offset_tight`: the median and the worst beep, in ms (M3 task 9, CB:1193).
pub const AV_TIGHT_MS: (f64, f64) = (10.0, 21.3);
/// `av.inter_track`: one beep on two tracks at most this many ms apart, proposed (README §6.3).
pub const AV_INTER_TRACK_MS: f64 = 0.5;
/// `audio.tone_snr`: a beep's power over its residual, proposed (signals.md §3.5).
pub const TONE_SNR_MIN_DB: f64 = 35.0;
/// `audio.tone_snr`: a beep's frequency within this many Hz of its tone's.
pub const TONE_HZ_TOLERANCE: f64 = 1.0;
/// `audio.routing`: present at this level or more in this share of the windows, absent at this
/// level or less in every one; a leak between, all proposed (README §2 item 16).
pub const ROUTING_PRESENT_DB: f64 = -60.0;
pub const ROUTING_PRESENT_SHARE: f64 = 0.9;
pub const ROUTING_ABSENT_DB: f64 = -70.0;
/// `audio.cue`: present at this NCC and level or more, possible from the second NCC,
/// proposed (README §6.3).
pub const CUE_PRESENT_NCC: f64 = 0.5;
pub const CUE_POSSIBLE_NCC: f64 = 0.2;
pub const CUE_MIN_DB: f64 = -70.0;
/// How much of each audio track is decoded and judged, in seconds: 10 minutes of (L+R)/2 is
/// 115 MB; longer clips are judged on their start (M13's hour-long files need streaming).
pub const AUDIO_MAX_SECONDS: u64 = 600;
/// The AAC lead the cut gives a clip, in samples: one priming packet plus up to one more
/// (PLAN:179, cookbook P8, core/cut.rs).
pub const LEAD_SAMPLES: (u64, u64) = (1024, 2047);
/// The peak a recording may reach in any 1 s window before a warning (CLAUDE.md, research 13).
pub const PEAK_WARN_BPS: u64 = 50_000_000;
/// Reels' average video bitrate cap (PLAN:245).
pub const REELS_AVERAGE_BPS: u64 = 25_000_000;
/// Copy for Discord's hard size cap (PLAN:243).
pub const DISCORD_MAX_BYTES: u64 = 19_000_000;
/// Reels' size cap, 300 MB read as 300,000,000 bytes (PLAN:245).
pub const REELS_MAX_BYTES: u64 = 300_000_000;
/// Reels' length, 3 s to 15 min (PLAN:245, research 06:156).
pub const REELS_SECONDS: (u64, u64) = (3, 900);
/// Reels' frame rates (README §6.5).
pub const REELS_FPS: (u64, u64) = (23, 60);
/// Reels' picture size, portrait (PLAN:245).
pub const REELS_SIZE: (u32, u32) = (1080, 1920);
/// The largest share copy: "H.264 ≤1080p60" read as within 1920×1080 in either orientation,
/// at most 60 fps (PLAN:246): (long side, short side, fps).
pub const SHARE_MAX: (u32, u32, u64) = (1920, 1080, 60);
/// WhatsApp's frames, proposed: at most 1080 lines at 30 fps (PLAN:244).
pub const WHATSAPP_MAX: (u32, u64) = (1080, 30);
/// WhatsApp's AAC bitrate, proposed (PLAN:244).
pub const WHATSAPP_AUDIO_BPS: u64 = 128_000;
/// The average a WhatsApp copy's audio may reach: 128 kbps plus 15%, since FFmpeg's native AAC
/// encoder at `-b:a 128k` averaged up to 137.7 kbps (+7.6%) on modulated noise (measured on the Mac), while a
/// 160 kbps encode stays above 159 kbps.
pub const WHATSAPP_AUDIO_LIMIT_BPS: u64 = WHATSAPP_AUDIO_BPS * 23 / 20;
/// The rungs Copy for Discord may land on: (shorter side, fps) (PLAN:243).
pub const DISCORD_LADDER: [(u32, u32); 5] =
    [(1080, 60), (1080, 30), (720, 60), (720, 30), (540, 30)];
/// The `ilst` keys a share copy may keep: the title and the encoder (PLAN:249).
pub const SHARE_TAGS: [&[u8; 4]; 2] = [b"\xa9nam", b"\xa9too"];

/// How a structure or export check counts in `profile`.
pub fn mode(profile: Profile, id: &str) -> Mode {
    use Mode::{Off, Required, Warning};
    let share = profile.is_share_export();
    match id {
        "container.no_editlist" if profile == Profile::Original => Warning,
        "av.start_alignment" | "duration" if profile == Profile::Original => Off,
        "audio.format" if matches!(profile, Profile::Capture | Profile::Smoke) => Off,
        "video.bitrate" if profile == Profile::Reels => Required,
        "video.bitrate" => Warning,
        "export.size" => match profile {
            Profile::Discord | Profile::Reels => Required,
            _ => Off,
        },
        "export.shape" | "export.tags" if share => Required,
        "export.shape" | "export.tags" => Off,
        _ => Required,
    }
}

/// How a content check counts in `profile`, before `--gate m3` (README §6.5).
pub fn content_mode(profile: Profile, id: &str, expect: Option<&Expect>) -> Mode {
    use Mode::{Off, Required, Warning};
    use Profile::{Capture, Engine, Original, Smoke, Test};
    let share = profile.is_share_export();
    let hdr = expect.is_some_and(|e| e.hdr.is_some());
    match id {
        // HDR-sourced engine clips: the bars are warnings until SP3 settles the curve's colour
        // (README §2 item 14).
        "content.bars" | "content.colour_matrix" if hdr && matches!(profile, Engine | Original) => {
            Warning
        }
        "decode.clean" | "content.bars" | "content.colour_matrix" | "build.lgpl" => Required,
        "content.barcode" | "content.levels" if share => Warning,
        "cadence.drops_dups" | "cadence.judder" | "source.stability" if profile == Smoke => Warning,
        "cadence.judder" if share => Warning,
        "content.flash" if profile == Test => Required,
        "content.flash" => Warning,
        "content.bars_lines" => match profile {
            Test => Required,
            _ if share => Off,
            _ => Warning,
        },
        "hdr.curve_levels" => match profile {
            Capture | Smoke => Off,
            _ if !hdr => Off,
            _ if share => Warning,
            _ => Required,
        },
        "source.stability" => match profile {
            Capture | Engine => Required,
            _ => Off,
        },
        "av.offset" | "audio.tone_snr" | "audio.routing" | "audio.cue" => match profile {
            Capture | Smoke => Off,
            _ => Required,
        },
        "av.offset_tight" if profile == Engine => Warning,
        "av.offset_tight" => Off,
        "av.inter_track" => match profile {
            Test | Engine | Original => Required,
            _ => Off,
        },
        "export.trim" if share && expect.is_some_and(|e| e.trim.is_some()) => Required,
        "export.trim" => Off,
        _ => Required,
    }
}

/// An audio track a clip should have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    /// Its title in `hdlr` and `udta/name`.
    pub title: String,
    /// Its channels.
    pub channels: u16,
    /// Whether it is the default (enabled) track.
    pub default: bool,
}

impl Role {
    fn new(title: &str, channels: u16, default: bool) -> Self {
        Self {
            title: title.to_owned(),
            channels,
            default,
        }
    }
}

/// The audio tracks a clip should have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// Exactly these, in this order.
    Exact(Vec<Role>),
    /// The engine's: Mix (default), Game and Mic, then optionally Voice chat and Everything
    /// else (PLAN:185–191).
    Engine,
    /// One stereo track, enabled, with any title: a share copy (PLAN:249).
    OneStereo,
}

/// The engine's tracks in order, with the two optional ones last (PLAN:185–191, CB:598).
pub fn engine_roles() -> [Role; 5] {
    [
        Role::new("Mix", 2, true),
        Role::new("Game", 2, false),
        Role::new("Mic", 1, false),
        Role::new("Voice chat", 2, false),
        Role::new("Everything else", 2, false),
    ]
}

/// The video length a clip should have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    /// This many frames, give or take `tolerance`.
    Frames {
        /// Expected frames.
        expected: u64,
        /// Allowed difference either way.
        tolerance: u64,
        /// Where the rule comes from.
        source: &'static str,
    },
    /// `seconds` at the clip's rate, within one GOP either way; below `seconds` warns, since
    /// the engine announces a shortened clip (PLAN:162, PLAN:446).
    WithinGop {
        /// The requested length.
        seconds: u64,
    },
    /// No length rule.
    Any,
}

/// Whether the content checks have to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// The clip holds the testbed pattern: required content checks must run.
    Required,
    /// Only if the testbed pattern turns up in the first 60 frames (README §6.1).
    IfPattern,
}

/// A profile resolved against `--expect` and `--gate`: the expectations the checks use.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The profile.
    pub profile: Profile,
    /// `--gate m3`.
    pub gate_m3: bool,
    /// The frame rate, if known; otherwise the checks measure it.
    pub fps: Option<u32>,
    /// The picture size, if known.
    pub size: Option<(u32, u32)>,
    /// The audio tracks.
    pub layout: Layout,
    /// The length rule.
    pub length: Length,
    /// Whether content checks must run.
    pub content: Content,
    /// The expect file, if one was given.
    pub expect: Option<Expect>,
    /// `--av-tight`: `av.offset_tight` is required, not a warning.
    pub av_tight: bool,
    /// `--voice-included`: an export's single track should carry the voice tone.
    pub voice_included: bool,
    /// `--cue`: Kairo's cue at 48 kHz, (L+R)/2; testsig's placeholder when `None`.
    pub cue: Option<Vec<f32>>,
}

/// Why a profile, gate and expect file don't fit together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError(pub String);

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlanError {}

impl Plan {
    /// Resolves `profile` with `--gate m3` and an expect file.
    pub fn new(profile: Profile, gate_m3: bool, expect: Option<Expect>) -> Result<Plan, PlanError> {
        if profile.needs_expect() && expect.is_none() {
            return Err(PlanError(format!("--profile {profile} needs --expect")));
        }
        if gate_m3 && profile != Profile::Engine {
            return Err(PlanError("--gate m3 goes with --profile engine".to_owned()));
        }
        if profile != Profile::Test && expect.as_ref().is_some_and(|e| e.cabac == Some(false)) {
            return Err(PlanError(
                "cabac: false is for the test profile's lossless I_PCM clips (README §7.2); Kairo encodes CABAC"
                    .to_owned(),
            ));
        }
        let roles = |e: &Expect| {
            Layout::Exact(
                e.tracks
                    .iter()
                    .map(|t| Role::new(&t.title, t.channels, t.default))
                    .collect(),
            )
        };
        let frames = |e: &Expect, tolerance, source| Length::Frames {
            expected: u64::from(e.seconds) * u64::from(e.fps),
            tolerance,
            source,
        };
        let content =
            if profile.needs_expect() || expect.as_ref().is_some_and(|e| e.pattern.is_some()) {
                Content::Required
            } else {
                Content::IfPattern
            };
        let mut plan = Plan {
            profile,
            gate_m3,
            fps: expect.as_ref().map(|e| e.fps),
            size: expect.as_ref().and_then(|e| e.size).map(|[w, h]| (w, h)),
            layout: Layout::Engine,
            length: Length::Any,
            content,
            expect: None,
            av_tight: false,
            voice_included: false,
            cue: None,
        };
        match (profile, expect.as_ref()) {
            (Profile::Test, Some(e)) => {
                plan.layout = roles(e);
                plan.length = frames(e, 1, "expect ± 1 frame (README §6.5)");
            }
            (Profile::Capture | Profile::Smoke, Some(e)) => {
                plan.layout = roles(e);
                plan.length = frames(e, 2, "expect.seconds × fps ± 2 frames (README §6.5)");
            }
            (Profile::Engine, e) => {
                // PLAN:446's defaults: 30 s at 60 fps with Mix, Game and Mic.
                let seconds = e.map_or(30, |e| e.seconds);
                plan.fps = Some(e.map_or(60, |e| e.fps));
                plan.layout = match e {
                    Some(e) => roles(e),
                    None if gate_m3 => Layout::Exact(engine_roles()[..3].to_vec()),
                    None => Layout::Engine,
                };
                plan.length = Length::WithinGop {
                    seconds: u64::from(seconds),
                };
                if gate_m3 {
                    let gate = Layout::Exact(engine_roles()[..3].to_vec());
                    if seconds != 30 || plan.fps != Some(60) || plan.layout != gate {
                        return Err(PlanError(
                            "--gate m3 fixes 30 s, 60 fps and Mix, Game and Mic (PLAN:446); the expect file differs"
                                .to_owned(),
                        ));
                    }
                }
            }
            (Profile::Original, e) => {
                plan.layout = e.map_or(Layout::Engine, roles);
            }
            (_, e) => {
                // A share export: one stereo track; its length follows the source when known.
                plan.layout = Layout::OneStereo;
                if let Some(e) = e {
                    plan.length = frames(e, 1, "the source ± 1 frame (README §6.5)");
                }
            }
        }
        plan.expect = expect;
        Ok(plan)
    }

    /// How a structure or export check counts, `--gate m3` included: under the gate only its
    /// list can fail the clip.
    pub fn mode(&self, id: &str) -> Mode {
        let mode = mode(self.profile, id);
        if self.gate_m3 && mode == Mode::Required && !GATE_M3_CHECKS.contains(&id) {
            Mode::Warning
        } else {
            mode
        }
    }

    /// How a content check counts, `--gate m3` included.
    pub fn content_mode(&self, id: &str) -> Mode {
        // --av-tight makes M3 task 9's pair a requirement, under --gate m3 too (README §6.5).
        if id == "av.offset_tight" && self.av_tight && self.profile == Profile::Engine {
            return Mode::Required;
        }
        let mode = content_mode(self.profile, id, self.expect.as_ref());
        if self.gate_m3 && mode == Mode::Required && !GATE_M3_CHECKS.contains(&id) {
            Mode::Warning
        } else {
            mode
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect(text: &str) -> Expect {
        Expect::parse(text).unwrap()
    }

    #[test]
    fn names_round_trip() {
        for p in Profile::ALL {
            assert_eq!(Profile::parse(p.name()), Some(p));
        }
        assert_eq!(Profile::parse("clip"), None);
    }

    #[test]
    fn resolves_profiles() {
        assert!(Plan::new(Profile::Test, false, None).is_err());
        assert!(Plan::new(Profile::Share, true, None).is_err());
        let engine = Plan::new(Profile::Engine, false, None).unwrap();
        assert_eq!(engine.fps, Some(60));
        assert_eq!(engine.length, Length::WithinGop { seconds: 30 });
        assert_eq!(engine.layout, Layout::Engine);
        assert_eq!(engine.content, Content::IfPattern);
        let gate = Plan::new(Profile::Engine, true, None).unwrap();
        assert_eq!(gate.layout, Layout::Exact(engine_roles()[..3].to_vec()));
        // Under the gate, a check off its list can only warn.
        assert_eq!(gate.mode("video.gop"), Mode::Warning);
        assert_eq!(gate.mode("video.colr"), Mode::Required);
        let e = expect(r#"{"kairo_expect":1,"pattern":1,"fps":60,"seconds":10,"tracks":[]}"#);
        let test = Plan::new(Profile::Test, false, Some(e.clone())).unwrap();
        assert_eq!(test.layout, Layout::Exact(vec![]));
        assert!(matches!(
            test.length,
            Length::Frames {
                expected: 600,
                tolerance: 1,
                ..
            }
        ));
        assert_eq!(test.content, Content::Required);
        assert!(Plan::new(Profile::Engine, true, Some(e)).is_err());
        let share = Plan::new(Profile::Share, false, None).unwrap();
        assert_eq!(
            (share.layout, share.length, share.fps),
            (Layout::OneStereo, Length::Any, None)
        );
    }

    #[test]
    fn modes_follow_the_table() {
        assert_eq!(
            mode(Profile::Original, "container.no_editlist"),
            Mode::Warning
        );
        assert_eq!(mode(Profile::Original, "duration"), Mode::Off);
        assert_eq!(mode(Profile::Capture, "audio.format"), Mode::Off);
        assert_eq!(mode(Profile::Reels, "video.bitrate"), Mode::Required);
        assert_eq!(mode(Profile::Engine, "video.bitrate"), Mode::Warning);
        assert_eq!(mode(Profile::Discord, "export.size"), Mode::Required);
        assert_eq!(mode(Profile::Share, "export.size"), Mode::Off);
        assert_eq!(mode(Profile::Engine, "export.shape"), Mode::Off);
        assert_eq!(content_mode(Profile::Capture, "av.offset", None), Mode::Off);
        assert_eq!(
            content_mode(Profile::Smoke, "cadence.judder", None),
            Mode::Warning
        );
        assert_eq!(
            content_mode(Profile::Test, "hdr.curve_levels", None),
            Mode::Off
        );
        // Exports judge HDR curves only for HDR-sourced clips, and as warnings; `original`, like
        // engine, keeps HDR-sourced bars as warnings until SP3 (review F4, F7 of PR 4b).
        let hdr = expect(
            r#"{"kairo_expect":1,"fps":60,"seconds":30,"tracks":[],
                "hdr":{"curve":"natural","lw_nits":1000,"sdr_white_nits":240}}"#,
        );
        assert_eq!(
            content_mode(Profile::Share, "hdr.curve_levels", None),
            Mode::Off
        );
        assert_eq!(
            content_mode(Profile::Share, "hdr.curve_levels", Some(&hdr)),
            Mode::Warning
        );
        for p in [Profile::Engine, Profile::Original] {
            assert_eq!(content_mode(p, "content.bars", Some(&hdr)), Mode::Warning);
            assert_eq!(content_mode(p, "content.bars", None), Mode::Required);
        }
        for p in Profile::ALL {
            for id in STRUCTURE_CHECKS.iter().chain(&EXPORT_CHECKS) {
                let _ = mode(p, id);
            }
        }
    }
}
