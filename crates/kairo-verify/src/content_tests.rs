// SPDX-License-Identifier: GPL-3.0-or-later
//! The content checks end to end (README §9's U tier): the builder's 2 s clip, 120 frames at
//! 60 fps, decoded by the synthetic decoder as a 180 fps testbed's frames, under `test` and
//! `engine`. A good clip passes every content check this build runs; each content defect
//! (drop, duplicate, judder, unreadable, torn, wrong run tag, wrong flash) fails its own.

use kairo_core::testsig::{Curve, Load, Matrix, Options, Patches, Pattern, Range, Size};

use crate::checks::Status;
use crate::decode::synthetic::{Damage, Sound, Synthetic, SyntheticAudio};
use crate::expect::Expect;
use crate::profile::{Plan, Profile};
use crate::testkit::Spec;
use crate::{Report, check_with};

const FRAMES: u64 = 120;

fn options() -> Options {
    Options {
        pattern: Pattern::Full,
        nominal_fps: 180,
        run_tag: 5,
        load: Load::Scroll,
        patches: Patches::Sdr,
    }
}

/// The tones of gate 1's layout on the builder's three tracks: the Mix carries the game beep and
/// the mic marker, the Game track the beep, the Mic track the marker (README §6.4).
const TONES: [&str; 3] = [
    r#"{"game":"present","voice":"absent","mic":"present","cue":"absent"}"#,
    r#"{"game":"present","voice":"absent","mic":"absent","cue":"absent"}"#,
    r#"{"game":"absent","voice":"absent","mic":"present","cue":"absent"}"#,
];

fn plan(profile: Profile, run_tag: u32) -> Plan {
    let [mix, game, mic] = TONES;
    let text = format!(
        r#"{{"kairo_expect":1,"pattern":1,"size":[320,180],"fps":60,"source_hz":180,"seconds":2,
            "run_tag":{run_tag},"tracks":[
            {{"title":"Mix","channels":2,"default":true,"tones":{mix}}},
            {{"title":"Game","channels":2,"default":false,"tones":{game}}},
            {{"title":"Mic","channels":1,"default":false,"tones":{mic}}}]}}"#
    );
    Plan::new(profile, false, Some(Expect::parse(&text).unwrap())).unwrap()
}

/// Where the builder's 60 fps clip shows testbed frame `n` under [`content`]'s mapping, in
/// seconds: frame k ≥ 1 at (328 + 256k) / 15360 shows 1000 + 3k.
fn shown_at(n: f64) -> f64 {
    (328.0 + 256.0 * (n - 1_000.0) / 3.0) / 15_360.0
}

/// Audio for `spec`'s tracks, as long as their packets: with `beeps`, the game beep at each
/// flash (testbed frame 180·m) in the Mix and Game tracks and the mic marker 250 ms later in the
/// Mix and Mic tracks, each `late` seconds late; silence otherwise.
fn audio(spec: &Spec, beeps: bool, late: f64) -> Vec<SyntheticAudio> {
    let flashes: Vec<f64> = (6..=7).map(|m| shown_at(180.0 * f64::from(m))).collect();
    spec.audio
        .iter()
        .map(|track| {
            let mut sounds = Vec::new();
            let (game, mic) = match track.title.as_str() {
                "Mix" => (true, true),
                "Game" => (true, false),
                "Mic" => (false, true),
                _ => (false, false),
            };
            for &t in flashes.iter().filter(|_| beeps) {
                if game {
                    sounds.push((Sound::Beep(1_000.0), t + late, 0.0));
                }
                if mic {
                    sounds.push((Sound::Beep(3_000.0), t + 0.25 + late, 0.0));
                }
            }
            SyntheticAudio {
                frames: track.deltas.len() * 1_024,
                sounds,
                noise_db: None,
            }
        })
        .collect()
}

/// Output frame k showing 180 fps content: testbed frame 3k, from a clip-start offset.
fn content() -> Vec<u64> {
    (0..FRAMES).map(|k| 1_000 + 3 * k).collect()
}

fn run(content: Vec<u64>, damage: Damage, options: Options, plan: &Plan) -> Report {
    let spec = Spec::good();
    let bytes = spec.build();
    let mut decoder = Synthetic::new(Size::new(320, 180).unwrap(), options, content, damage)
        .with_audio(audio(&spec, true, 0.0));
    check_with(&mut bytes.as_slice(), "clip.mp4", plan, Some(&mut decoder))
}

fn status(report: &Report, id: &str) -> Status {
    report
        .checks
        .iter()
        .find(|f| f.id == id)
        .map(|f| f.status)
        .unwrap_or_else(|| panic!("no {id}"))
}

/// The content checks this build runs that fail.
fn failing(report: &Report) -> Vec<&'static str> {
    report
        .checks
        .iter()
        .filter(|f| f.status == Status::Fail)
        .map(|f| f.id)
        .collect()
}

#[test]
fn a_good_clip_passes_every_content_check_this_build_runs() {
    let report = run(
        content(),
        Damage::default(),
        options(),
        &plan(Profile::Test, 5),
    );
    for id in [
        "decode.clean",
        "content.barcode",
        "cadence.drops_dups",
        "cadence.judder",
        "content.flash",
        "content.bars",
        "content.colour_matrix",
        "content.bars_lines",
        "content.levels",
        "build.lgpl",
    ] {
        assert_eq!(
            status(&report, id),
            Status::Pass,
            "{id}: {}",
            report.to_text()
        );
    }
    assert!(failing(&report).is_empty(), "{}", report.to_text());
    assert_eq!(report.decoder.kind, "synthetic");
    assert_eq!(
        report.summary.content,
        "120/120 barcodes, 0 skipped, 0 repeats, judder excess 0.0 pp, bars min 59.5 dB"
    );
    // The audio too: every check `test` runs passes, so the clip does.
    for id in [
        "av.offset",
        "av.inter_track",
        "audio.tone_snr",
        "audio.routing",
        "audio.cue",
    ] {
        assert_eq!(
            status(&report, id),
            Status::Pass,
            "{id}: {}",
            report.to_text()
        );
    }
    assert_eq!(report.verdict, crate::Verdict::Pass, "{}", report.to_text());
    // Noise and scaling don't stop the barcodes, cadence or flash. The bars are stricter:
    // ±6 codes of noise alone is about 37 dB, under the 40 dB a recording must keep.
    let damage = Damage {
        noise: 6,
        scale_from: Some(Size::new(480, 270).unwrap()),
        ..Damage::default()
    };
    let rough = run(content(), damage, options(), &plan(Profile::Test, 5));
    for id in [
        "decode.clean",
        "content.barcode",
        "cadence.drops_dups",
        "cadence.judder",
        "content.flash",
    ] {
        assert_eq!(
            status(&rough, id),
            Status::Pass,
            "{id}: {}",
            rough.to_text()
        );
    }
    assert_eq!(status(&rough, "content.bars"), Status::Fail);
}

#[test]
fn each_content_defect_fails_its_own_check() {
    let mid = 60;
    // Name, content, damage, pattern options, the expect file's run tag, and what fails.
    type Defect = (
        &'static str,
        Vec<u64>,
        Damage,
        Options,
        u32,
        &'static [&'static str],
    );
    let defects: Vec<Defect> = vec![
        (
            "a dropped frame",
            (0..FRAMES)
                .map(|k| 1_000 + 3 * (k + u64::from(k >= mid)))
                .collect(),
            Damage::default(),
            options(),
            5,
            &["cadence.drops_dups"],
        ),
        (
            "a duplicated frame",
            (0..FRAMES)
                .map(|k| 1_000 + 3 * (k - u64::from(k >= mid)))
                .collect(),
            Damage::default(),
            options(),
            5,
            &["cadence.drops_dups"],
        ),
        (
            "judder: 90/s on 180 Hz under a 60 fps pacer",
            (0..FRAMES).map(|k| 1_000 + 3 * k - (k % 2)).collect(),
            Damage::default(),
            options(),
            5,
            &["cadence.judder"],
        ),
        (
            "an unreadable frame",
            content(),
            Damage {
                garble: vec![30],
                ..Damage::default()
            },
            options(),
            5,
            &["content.barcode"],
        ),
        (
            "a torn frame",
            content(),
            Damage {
                tear: vec![30],
                ..Damage::default()
            },
            options(),
            5,
            &["content.barcode"],
        ),
        (
            "another run's tag",
            content(),
            Damage::default(),
            options(),
            6,
            &["content.barcode"],
        ),
        (
            "a flash timed for 120 fps",
            content(),
            Damage::default(),
            Options {
                nominal_fps: 120,
                ..options()
            },
            5,
            &["content.flash"],
        ),
    ];
    for (name, content, damage, options, tag, fails) in defects {
        let report = run(
            content.clone(),
            damage.clone(),
            options,
            &plan(Profile::Test, tag),
        );
        assert_eq!(failing(&report), fails, "{name}: {}", report.to_text());
        // Engine fails the same, except the flash, which only warns there.
        let report = run(content, damage, options, &plan(Profile::Engine, tag));
        let engine: Vec<&str> = fails
            .iter()
            .copied()
            .filter(|&id| id != "content.flash")
            .collect();
        assert_eq!(
            failing(&report),
            engine,
            "{name} under engine: {}",
            report.to_text()
        );
    }
}

#[test]
fn capture_tolerates_what_test_does_not() {
    // One drop in 119 steps is 0.84%, over capture's 0.5%; one unreadable frame in 120 is
    // under 99.9% readable either way.
    let drop: Vec<u64> = (0..FRAMES)
        .map(|k| 1_000 + 3 * (k + u64::from(k >= 60)))
        .collect();
    let report = run(
        drop,
        Damage::default(),
        options(),
        &plan(Profile::Capture, 5),
    );
    assert_eq!(
        failing(&report),
        ["cadence.drops_dups"],
        "{}",
        report.to_text()
    );
    // Smoke makes cadence a warning.
    let drop: Vec<u64> = (0..FRAMES)
        .map(|k| 1_000 + 3 * (k + u64::from(k >= 60)))
        .collect();
    let smoke = run(drop, Damage::default(), options(), &plan(Profile::Smoke, 5));
    assert_eq!(
        status(&smoke, "cadence.drops_dups"),
        Status::Warn,
        "{}",
        smoke.to_text()
    );
}

#[test]
fn a_clip_without_the_pattern_skips_the_content_checks_in_engine() {
    // An engine save of 2 s with no pattern named: the content checks run only if the
    // pattern shows up.
    let text = r#"{"kairo_expect":1,"fps":60,"seconds":2,"tracks":[
        {"title":"Mix","channels":2,"default":true},
        {"title":"Game","channels":2,"default":false},
        {"title":"Mic","channels":1,"default":false}]}"#;
    let plan = Plan::new(Profile::Engine, false, Some(Expect::parse(text).unwrap())).unwrap();
    let motion = Options {
        pattern: Pattern::Motion,
        ..options()
    };
    let report = run(content(), Damage::default(), motion, &plan);
    assert_eq!(
        status(&report, "decode.clean"),
        Status::Pass,
        "{}",
        report.to_text()
    );
    for id in ["content.barcode", "cadence.drops_dups", "content.flash"] {
        assert_eq!(status(&report, id), Status::Skip, "{id}");
    }
    assert!(failing(&report).is_empty(), "{}", report.to_text());
    // With the pattern, engine runs them.
    let report = run(content(), Damage::default(), options(), &plan);
    assert_eq!(
        status(&report, "content.barcode"),
        Status::Pass,
        "{}",
        report.to_text()
    );
}

#[test]
fn a_decode_that_disagrees_with_the_samples_fails_decode_clean() {
    let short: Vec<u64> = content().into_iter().take(119).collect();
    let report = run(short, Damage::default(), options(), &plan(Profile::Test, 5));
    assert_eq!(failing(&report), ["decode.clean"], "{}", report.to_text());
    let finding = report
        .checks
        .iter()
        .find(|f| f.id == "decode.clean")
        .unwrap();
    assert_eq!(
        finding.detail,
        "119 pictures decoded from 120 video samples"
    );
}

/// The builder's good clip at `fps` for `seconds`, with the held first frame and enough AAC
/// packets for its length.
fn spec(fps: u32, seconds: u32) -> Spec {
    let frames = (fps * seconds) as usize;
    let tick = 15_360 / fps;
    let mut deltas = vec![tick; frames];
    deltas[0] = tick + 328;
    let packets = (seconds as usize * 48_000 + 1_024).div_ceil(1_024);
    Spec::with(|s| {
        s.deltas = deltas;
        s.pics = (0..frames)
            .map(|i| {
                if i % fps as usize == 0 {
                    crate::testkit::Pic::Idr
                } else {
                    crate::testkit::Pic::P
                }
            })
            .collect();
        for track in &mut s.audio {
            track.deltas = vec![1_024; packets];
        }
    })
}

/// A plan for the clip `spec(fps, seconds)` makes, with the testbed's display at `source_hz`.
fn plan_for(profile: Profile, fps: u32, seconds: u32, source_hz: u32) -> Plan {
    let text = format!(
        r#"{{"kairo_expect":1,"pattern":1,"size":[320,180],"fps":{fps},"source_hz":{source_hz},
            "seconds":{seconds},"run_tag":5,"tracks":[
            {{"title":"Mix","channels":2,"default":true}},
            {{"title":"Game","channels":2,"default":false}},
            {{"title":"Mic","channels":1,"default":false}}]}}"#
    );
    Plan::new(profile, false, Some(Expect::parse(&text).unwrap())).unwrap()
}

fn run_spec(
    spec: &Spec,
    content: Vec<u64>,
    damage: Damage,
    options: Options,
    plan: &Plan,
) -> Report {
    let bytes = spec.build();
    let mut decoder = Synthetic::new(Size::new(320, 180).unwrap(), options, content, damage)
        .with_audio(audio(spec, false, 0.0));
    check_with(&mut bytes.as_slice(), "clip.mp4", plan, Some(&mut decoder))
}

/// An engine save whose expect file names no pattern: content runs only if it shows up.
fn engine_plan() -> Plan {
    let text = r#"{"kairo_expect":1,"fps":60,"seconds":2,"tracks":[
        {"title":"Mix","channels":2,"default":true},
        {"title":"Game","channels":2,"default":false},
        {"title":"Mic","channels":1,"default":false}]}"#;
    Plan::new(Profile::Engine, false, Some(Expect::parse(text).unwrap())).unwrap()
}

#[test]
fn stalled_and_backward_clips_fail_drops_dups() {
    // A capture that stops delivering, one that stops after frame 0, a last frame that shows
    // frame 1's picture again, a ring read from the wrong end, and a testbed restart (review
    // F2): no barcode line fits some of them, and none may pass.
    let cases: Vec<(&str, Vec<u64>)> = vec![
        ("frozen", vec![1_000; FRAMES as usize]),
        (
            "frozen after frame 0",
            (0..FRAMES)
                .map(|k| if k == 0 { 1_000 } else { 1_003 })
                .collect(),
        ),
        (
            "frame 1 again at the end",
            (0..FRAMES)
                .map(|k| {
                    if k == FRAMES - 1 {
                        1_003
                    } else {
                        1_000 + 3 * k
                    }
                })
                .collect(),
        ),
        (
            "swapped halves",
            (0..FRAMES)
                .map(|k| {
                    if k < 60 {
                        1_180 + 3 * k
                    } else {
                        1_000 + 3 * (k - 60)
                    }
                })
                .collect(),
        ),
        (
            "a restart",
            (0..FRAMES)
                .map(|k| if k < 60 { 1_000 + 3 * k } else { 3 * (k - 60) })
                .collect(),
        ),
    ];
    for (name, content) in cases {
        for plan in [
            engine_plan(),
            plan(Profile::Test, 5),
            plan(Profile::Capture, 5),
        ] {
            let report = run(content.clone(), Damage::default(), options(), &plan);
            assert_eq!(
                status(&report, "cadence.drops_dups"),
                Status::Fail,
                "{name} under {}: {}",
                plan.profile,
                report.to_text()
            );
            assert_eq!(report.verdict, crate::Verdict::Fail, "{name}");
        }
    }
}

#[test]
fn each_audio_defect_fails_its_own_check() {
    // README §7.3's audio defects on the builder's Mix, Game and Mic: each fails its own check
    // and nothing else.
    type Change = fn(&mut [SyntheticAudio]);
    let defects: [(&str, Change, &[&str]); 6] = [
        (
            "audio 50 ms late",
            |tracks| {
                for t in tracks {
                    for s in &mut t.sounds {
                        s.1 += 0.050;
                    }
                }
            },
            &["av.offset"],
        ),
        (
            "the Game and Mic tracks swapped",
            |tracks| {
                let (game, mic) = (tracks[1].sounds.clone(), tracks[2].sounds.clone());
                tracks[1].sounds = mic;
                tracks[2].sounds = game;
            },
            &["audio.routing"],
        ),
        (
            "the cue in the Mix",
            |tracks| {
                let at = shown_at(1_080.0) + 0.75;
                tracks[0].sounds.push((Sound::Cue, at, 0.0));
            },
            &["audio.cue"],
        ),
        (
            "the voice tone leaking into Game at −40 dBFS",
            |tracks| {
                let at = shown_at(1_080.0) + 0.5;
                tracks[1].sounds.push((Sound::Beep(2_000.0), at, -10.0));
            },
            &["audio.routing"],
        ),
        (
            "noise at −55 dBFS RMS in Game",
            |tracks| {
                tracks[1].noise_db = Some(-55.0);
            },
            &["audio.tone_snr"],
        ),
        (
            "Game 1 ms behind the Mix",
            |tracks| {
                for s in &mut tracks[1].sounds {
                    s.1 += 0.001;
                }
            },
            &["av.inter_track"],
        ),
    ];
    let spec = Spec::good();
    let bytes = spec.build();
    for (name, change, fails) in defects {
        let mut tracks = audio(&spec, true, 0.0);
        change(&mut tracks);
        let mut decoder = Synthetic::new(
            Size::new(320, 180).unwrap(),
            options(),
            content(),
            Damage::default(),
        )
        .with_audio(tracks);
        let report = check_with(
            &mut bytes.as_slice(),
            "clip.mp4",
            &plan(Profile::Test, 5),
            Some(&mut decoder),
        );
        assert_eq!(failing(&report), fails, "{name}: {}", report.to_text());
    }
}

/// The builder's clip with research 16 §7.1's four tracks, and its `test` plan.
fn four_tracks() -> (Spec, Plan) {
    use crate::testkit::AudioSpec;
    let spec = Spec::with(|s| {
        s.audio = vec![
            AudioSpec::new("Mix", 2, true, 95),
            AudioSpec::new("Game", 2, false, 95),
            AudioSpec::new("Mic", 1, false, 95),
            AudioSpec::new("Voice chat", 2, false, 95),
        ];
    });
    let tones = |game: &str, voice: &str, mic: &str| {
        format!(r#"{{"game":"{game}","voice":"{voice}","mic":"{mic}","cue":"absent"}}"#)
    };
    let text = format!(
        r#"{{"kairo_expect":1,"pattern":1,"size":[320,180],"fps":60,"source_hz":180,"seconds":2,
            "run_tag":5,"tracks":[
            {{"title":"Mix","channels":2,"default":true,"tones":{}}},
            {{"title":"Game","channels":2,"default":false,"tones":{}}},
            {{"title":"Mic","channels":1,"default":false,"tones":{}}},
            {{"title":"Voice chat","channels":2,"default":false,"tones":{}}}]}}"#,
        tones("present", "present", "present"),
        tones("present", "absent", "absent"),
        tones("absent", "absent", "present"),
        tones("absent", "present", "absent"),
    );
    let plan = Plan::new(Profile::Test, false, Some(Expect::parse(&text).unwrap())).unwrap();
    (spec, plan)
}

/// The four tracks' sounds at flashes 6 and 7: the game beep, the mic marker 250 ms after it
/// and the voice beep 500 ms after it, each on its own track and all of them on the Mix.
fn four_track_audio(spec: &Spec) -> Vec<SyntheticAudio> {
    let flashes: Vec<f64> = (6..=7).map(|m| shown_at(180.0 * f64::from(m))).collect();
    spec.audio
        .iter()
        .map(|track| {
            let (game, mic, voice) = match track.title.as_str() {
                "Mix" => (true, true, true),
                "Game" => (true, false, false),
                "Mic" => (false, true, false),
                _ => (false, false, true),
            };
            let mut sounds = Vec::new();
            for &t in &flashes {
                if game {
                    sounds.push((Sound::Beep(1_000.0), t, 0.0));
                }
                if mic {
                    sounds.push((Sound::Beep(3_000.0), t + 0.25, 0.0));
                }
                if voice {
                    sounds.push((Sound::Beep(2_000.0), t + 0.5, 0.0));
                }
            }
            SyntheticAudio {
                frames: track.deltas.len() * 1_024,
                sounds,
                noise_db: None,
            }
        })
        .collect()
}

/// `spec`'s clip under `plan`, its pictures [`content`]'s and its audio `tracks`.
fn run_audio(spec: &Spec, plan: &Plan, tracks: Vec<SyntheticAudio>) -> Report {
    let bytes = spec.build();
    let mut decoder = Synthetic::new(
        Size::new(320, 180).unwrap(),
        options(),
        content(),
        Damage::default(),
    )
    .with_audio(tracks);
    check_with(&mut bytes.as_slice(), "clip.mp4", plan, Some(&mut decoder))
}

/// Each track's `av.offset` entry.
fn offsets(report: &Report) -> Vec<serde_json::Value> {
    report
        .checks
        .iter()
        .find(|f| f.id == "av.offset")
        .and_then(|f| f.value.get("tracks")?.as_array().cloned())
        .unwrap_or_default()
}

#[test]
fn tracks_off_the_mix_by_more_than_five_ms_fail() {
    // Review F1: beeps pair by event however far apart, and a voice-only or mic-only track is
    // timed through the Mix, beep by beep.
    let (spec, plan) = four_tracks();
    let good = run_audio(&spec, &plan, four_track_audio(&spec));
    assert!(failing(&good).is_empty(), "{}", good.to_text());
    assert_eq!(good.verdict, crate::Verdict::Pass, "{}", good.to_text());
    for track in offsets(&good) {
        assert_eq!(track["measured"], true, "{track}");
    }
    let cases: [(&str, f64, &[&str]); 5] = [
        ("Game", 6.0e-3, &["av.inter_track"]),
        ("Game", 1_024.0 / 48_000.0, &["av.inter_track"]),
        ("Mic", 10.0e-3, &["av.inter_track"]),
        ("Voice chat", 30.0e-3, &["av.inter_track"]),
        ("Voice chat", 50.0e-3, &["av.offset", "av.inter_track"]),
    ];
    for (title, late, fails) in cases {
        let mut tracks = four_track_audio(&spec);
        let i = spec.audio.iter().position(|t| t.title == title).unwrap();
        for sound in &mut tracks[i].sounds {
            sound.1 += late;
        }
        let report = run_audio(&spec, &plan, tracks);
        assert_eq!(
            failing(&report),
            fails,
            "{title} {late} s late: {}",
            report.to_text()
        );
    }
}

#[test]
fn audio_far_from_its_pictures_fails_av_offset() {
    // Review F3: a beep more than 250 ms off is found up to half the flash period away, and a
    // flash without its beep on a track that carries the game beep fails too.
    let (spec, plan) = four_tracks();
    let flash7 = shown_at(1_260.0);
    type Change = fn(&mut Vec<SyntheticAudio>, f64);
    let cases: [(&str, Change, &str); 3] = [
        (
            "everything 300 ms late",
            |tracks, _| {
                for t in tracks {
                    for s in &mut t.sounds {
                        s.1 += 0.3;
                    }
                }
            },
            "flash 6",
        ),
        (
            "a 300 ms slip from flash 7 on",
            |tracks, from| {
                for t in tracks {
                    for s in t.sounds.iter_mut().filter(|s| s.1 >= from - 1e-6) {
                        s.1 += 0.3;
                    }
                }
            },
            "flash 7",
        ),
        (
            "Game's beep missing at flash 7",
            |tracks, from| tracks[1].sounds.retain(|s| s.1 < from - 1e-6),
            "no game beep within 500 ms of flash 7",
        ),
    ];
    for (name, change, says) in cases {
        let mut tracks = four_track_audio(&spec);
        change(&mut tracks, flash7);
        let report = run_audio(&spec, &plan, tracks);
        assert_eq!(
            failing(&report),
            ["av.offset"],
            "{name}: {}",
            report.to_text()
        );
        let text = report.to_text();
        assert!(text.contains(says), "{name}: {text}");
    }
}

#[test]
fn audio_that_didnt_decode_is_an_error_not_silence() {
    // Review F2 and F6: a track the decoder can't give back leaves every audio check unable to
    // run, rather than failing routes it never got to show; a picture fault found meanwhile
    // still fails decode.clean, and with it the clip.
    struct Faulty(Synthetic, usize);
    impl crate::decode::Decoder for Faulty {
        fn kind(&self) -> &'static str {
            "synthetic"
        }
        fn video(
            &mut self,
            each: &mut dyn FnMut(usize, &crate::decode::Planes<'_>) -> std::ops::ControlFlow<()>,
        ) -> Result<crate::decode::DecodeStats, crate::decode::DecodeError> {
            let mut stats = self.0.video(each)?;
            stats.errors = self.1;
            stats.last_error = (self.1 > 0).then(|| "concealed".to_owned());
            Ok(stats)
        }
    }
    let spec = Spec::good();
    let bytes = spec.build();
    let pictures = || {
        Synthetic::new(
            Size::new(320, 180).unwrap(),
            options(),
            content(),
            Damage::default(),
        )
    };
    let plan = plan(Profile::Test, 5);
    let report = check_with(
        &mut bytes.as_slice(),
        "clip.mp4",
        &plan,
        Some(&mut Faulty(pictures(), 0)),
    );
    for id in [
        "decode.clean",
        "av.offset",
        "av.inter_track",
        "audio.tone_snr",
        "audio.routing",
        "audio.cue",
    ] {
        assert_eq!(
            status(&report, id),
            Status::Error,
            "{id}: {}",
            report.to_text()
        );
    }
    assert!(failing(&report).is_empty(), "{}", report.to_text());
    assert_eq!(report.verdict, crate::Verdict::Error);
    let report = check_with(
        &mut bytes.as_slice(),
        "clip.mp4",
        &plan,
        Some(&mut Faulty(pictures(), 1)),
    );
    assert_eq!(
        status(&report, "decode.clean"),
        Status::Fail,
        "{}",
        report.to_text()
    );
    assert_eq!(report.verdict, crate::Verdict::Fail);
    // Audio for two of the three tracks: the Mic track's absence is an error, not a missing
    // marker.
    let mut decoder = pictures().with_audio(audio(&spec, true, 0.0)[..2].to_vec());
    let report = check_with(&mut bytes.as_slice(), "clip.mp4", &plan, Some(&mut decoder));
    assert_eq!(
        status(&report, "audio.routing"),
        Status::Error,
        "{}",
        report.to_text()
    );
    assert!(failing(&report).is_empty(), "{}", report.to_text());
}

#[test]
fn routes_hold_for_saves_copies_and_short_tracks() {
    use crate::testkit::AudioSpec;
    // Review F4: room noise in the Mix only warns in an engine save and in its original copy.
    let spec = Spec::good();
    let mut tracks = audio(&spec, true, 0.0);
    tracks[0].noise_db = Some(-60.0);
    tracks[2].noise_db = Some(-60.0);
    for profile in [Profile::Engine, Profile::Original] {
        let plan = Plan::new(profile, false, None).unwrap();
        let report = run_audio(&spec, &plan, tracks.clone());
        assert_eq!(
            status(&report, "audio.routing"),
            Status::Pass,
            "{profile}: {}",
            report.to_text()
        );
        assert_eq!(
            status(&report, "audio.tone_snr"),
            Status::Warn,
            "{profile}: {}",
            report.to_text()
        );
    }
    // Review F5: without tones from an expect file, Everything else may carry the game (plan B
    // and Windows 10 hear it there).
    let spec = Spec::with(|s| {
        s.audio
            .push(AudioSpec::new("Everything else", 2, false, 95))
    });
    let mut tracks = audio(&spec, true, 0.0);
    tracks[3].sounds = tracks[1].sounds.clone();
    let engine = Plan::new(Profile::Engine, false, None).unwrap();
    let report = run_audio(&spec, &engine, tracks);
    assert_eq!(
        status(&report, "audio.routing"),
        Status::Pass,
        "{}",
        report.to_text()
    );
    // Review F7: a Game track that stops after 1 s of a 2 s copy: its second window is silence,
    // so its beep is only partly there, which warns.
    let spec = Spec::with(|s| s.audio[1] = AudioSpec::new("Game", 2, false, 47));
    let original = Plan::new(Profile::Original, false, None).unwrap();
    let report = run_audio(&spec, &original, audio(&spec, true, 0.0));
    assert_eq!(
        status(&report, "audio.routing"),
        Status::Warn,
        "{}",
        report.to_text()
    );
}

#[test]
fn a_clip_that_shows_the_pattern_is_held_to_every_required_check() {
    // Engine with no pattern named, but the pattern shows: the checks this build lacks are
    // errors, not skips (review F3).
    let report = run(content(), Damage::default(), options(), &engine_plan());
    assert_eq!(status(&report, "content.barcode"), Status::Pass);
    assert_eq!(status(&report, "content.bars"), Status::Pass);
    assert_eq!(status(&report, "av.offset"), Status::Pass);
    // source.stability reads the testbed log, which comes with research 16's PR 8.
    assert_eq!(status(&report, "source.stability"), Status::Error);
    assert_eq!(
        report.verdict,
        crate::Verdict::Error,
        "{}",
        report.to_text()
    );
}

#[test]
fn a_decoder_that_cannot_start_is_an_error_not_a_failure() {
    struct Missing;
    impl crate::decode::Decoder for Missing {
        fn kind(&self) -> &'static str {
            "cli"
        }
        fn video(
            &mut self,
            _: &mut dyn FnMut(usize, &crate::decode::Planes<'_>) -> std::ops::ControlFlow<()>,
        ) -> Result<crate::decode::DecodeStats, crate::decode::DecodeError> {
            Err(crate::decode::DecodeError::Start(
                "FFmpeg: No such file or directory".to_owned(),
            ))
        }
    }
    let bytes = Spec::good().build();
    let report = check_with(
        &mut bytes.as_slice(),
        "clip.mp4",
        &engine_plan(),
        Some(&mut Missing),
    );
    assert_eq!(status(&report, "decode.clean"), Status::Error);
    assert_eq!(
        report.verdict,
        crate::Verdict::Error,
        "{}",
        report.to_text()
    );
    assert!(
        report
            .summary
            .content
            .starts_with("not decoded: the decoder didn't start"),
        "{}",
        report.summary.content
    );
}

#[test]
fn testbeds_near_the_clip_rate_pass_capture() {
    // Every vblank delivered at a display near the clip's rate (review F1), and testbeds that
    // hold each frame for several vblanks: the shares match their ideal.
    // (display Hz, vblanks per testbed frame, clip fps)
    for (hz, every, fps) in [
        (75, 1, 60),
        (50, 1, 60),
        (144, 1, 120),
        (100, 1, 120),
        (180, 2, 60),
        (360, 5, 60),
        (180, 1, 60),
    ] {
        let testbed = hz / every;
        let frames = u64::from(2 * fps);
        let content: Vec<u64> = (0..frames)
            .map(|k| {
                let vblank = (k as f64 * f64::from(hz) / f64::from(fps) + 0.37).floor() as u64;
                1_000 + vblank / u64::from(every)
            })
            .collect();
        let flash = Options {
            nominal_fps: testbed,
            ..options()
        };
        let report = run_spec(
            &spec(fps, 2),
            content,
            Damage::default(),
            flash,
            &plan_for(Profile::Capture, fps, 2, hz),
        );
        for id in ["cadence.drops_dups", "cadence.judder", "content.flash"] {
            assert_eq!(
                status(&report, id),
                Status::Pass,
                "{testbed} fps on {hz} Hz at {fps}: {id}: {}",
                report.to_text()
            );
        }
        let rate = report
            .checks
            .iter()
            .find(|f| f.id == "cadence.drops_dups")
            .and_then(|f| f.value.get("source_fps")?.as_u64());
        assert_eq!(rate, Some(u64::from(testbed)), "{testbed} on {hz}");
    }
}

#[test]
fn picture_defects_fail_the_bars_and_level_checks() {
    // README §7.3 and signals.md §2.4: each fault, what it must fail.
    let cases: Vec<(&str, Damage, &[&str])> = vec![
        (
            "BT.601 tagged BT.709",
            Damage {
                colour: Some((Matrix::Bt601, Range::Limited)),
                ..Damage::default()
            },
            &[
                "content.bars",
                "content.colour_matrix",
                "content.bars_lines",
            ],
        ),
        (
            "full range tagged limited",
            Damage {
                colour: Some((Matrix::Bt709, Range::Full)),
                ..Damage::default()
            },
            &[
                "content.bars",
                "content.colour_matrix",
                "content.bars_lines",
                "content.levels",
            ],
        ),
        (
            "Cb and Cr swapped",
            Damage {
                swap_chroma: true,
                ..Damage::default()
            },
            &["content.bars", "content.colour_matrix"],
        ),
        (
            "gridlines ±48 every 16 px (bars)",
            Damage {
                gridlines: Some((16, 48)),
                ..Damage::default()
            },
            &["content.bars", "content.bars_lines"],
        ),
        (
            "an unreadable frame whose bars are corrupt too",
            Damage {
                garble: vec![30],
                gridlines: Some((16, 48)),
                gridline_frames: vec![30],
                ..Damage::default()
            },
            &["content.barcode", "content.bars", "content.bars_lines"],
        ),
    ];
    for (name, damage, fails) in cases {
        let report = run(content(), damage, options(), &plan(Profile::Test, 5));
        assert_eq!(failing(&report), fails, "{name}: {}", report.to_text());
    }
    // +24 every 64 px shows as lines; its PSNR may pass or not (README §7.3 bars-sparse).
    let sparse = Damage {
        gridlines: Some((64, 24)),
        ..Damage::default()
    };
    let report = run(content(), sparse, options(), &plan(Profile::Test, 5));
    assert_eq!(
        status(&report, "content.bars_lines"),
        Status::Fail,
        "{}",
        report.to_text()
    );
}

#[test]
fn the_ramp_chroma_rule_only_warns_in_exports() {
    // Low export bitrates leave the ramp's chroma a few codes off grey (review F2 of PR 4b).
    let off = Damage {
        ramp_chroma: 3,
        ..Damage::default()
    };
    let test = run(content(), off.clone(), options(), &plan(Profile::Test, 5));
    assert_eq!(
        status(&test, "content.bars"),
        Status::Fail,
        "{}",
        test.to_text()
    );
    let share = run(content(), off, options(), &plan(Profile::Share, 5));
    assert_eq!(
        status(&share, "content.bars"),
        Status::Warn,
        "{}",
        share.to_text()
    );
}

#[test]
fn many_short_unreadable_runs_are_all_scored() {
    // 140 unreadable frames in runs of one, more than the queue for the barcode line holds:
    // each is scored as soon as its next neighbour reads (review F3 of PR 4b).
    let frames = 420_u64;
    let content: Vec<u64> = (0..frames).map(|k| 1_000 + 3 * k).collect();
    let garble = Damage {
        garble: (0..frames as usize).filter(|i| i % 3 == 1).collect(),
        ..Damage::default()
    };
    let report = run_spec(
        &spec(60, 7),
        content,
        garble,
        options(),
        &plan_for(Profile::Capture, 60, 7, 180),
    );
    let bars = report
        .checks
        .iter()
        .find(|f| f.id == "content.bars")
        .unwrap();
    assert_eq!(bars.status, Status::Pass, "{}", report.to_text());
    assert_eq!(bars.value.get("scored"), bars.value.get("frames"));
}

#[test]
fn hdr_patches_are_judged_against_the_expected_curve() {
    let plan_for = |curve: &str| {
        let text = format!(
            r#"{{"kairo_expect":1,"pattern":1,"size":[320,180],"fps":60,"source_hz":180,"seconds":2,
                "run_tag":5,"hdr":{{"curve":"{curve}","lw_nits":1000,"sdr_white_nits":240}},"tracks":[
                {{"title":"Mix","channels":2,"default":true}},
                {{"title":"Game","channels":2,"default":false}},
                {{"title":"Mic","channels":1,"default":false}}]}}"#
        );
        Plan::new(Profile::Test, false, Some(Expect::parse(&text).unwrap())).unwrap()
    };
    let shown = |curve| Options {
        patches: Patches::ExpectedSdr {
            curve,
            peak_nits: 1_000.0,
            sdr_white_nits: 240.0,
        },
        ..options()
    };
    let natural = run(
        content(),
        Damage::default(),
        shown(Curve::Natural),
        &plan_for("natural"),
    );
    assert_eq!(
        status(&natural, "hdr.curve_levels"),
        Status::Pass,
        "{}",
        natural.to_text()
    );
    assert_eq!(status(&natural, "content.levels"), Status::Skip);
    assert!(failing(&natural).is_empty(), "{}", natural.to_text());
    // A clip made with Match SDR apps fails the Natural expectation.
    let wrong = run(
        content(),
        Damage::default(),
        shown(Curve::MatchSdrApps),
        &plan_for("natural"),
    );
    assert_eq!(failing(&wrong), ["hdr.curve_levels"], "{}", wrong.to_text());
}

/// A scratch folder for the opt-in tests, removed when dropped, failures included.
#[cfg(feature = "dev-tools")]
struct Scratch(std::path::PathBuf);

#[cfg(feature = "dev-tools")]
impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kairo-verify-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

#[cfg(feature = "dev-tools")]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The FFmpeg the opt-in tests spawn. They run only with `--ignored`, so a missing one is an
/// error, not a pass.
#[cfg(feature = "dev-tools")]
fn ffmpeg() -> std::ffi::OsString {
    std::env::var_os("KAIRO_FFMPEG")
        .expect("set KAIRO_FFMPEG to an FFmpeg to spawn, e.g. /opt/homebrew/bin/ffmpeg")
}

/// Encodes `count` synthetic frames of `size` with `h264_videotoolbox` into `dir/name`
/// (spawned, never libx264): a GOP of 60, no B-frames, `rate` (`-b:v`, `-maxrate`), no edit
/// list. The raw frames are BT.709 limited, and FFmpeg must be told so: untagged, it takes a
/// picture this size for BT.601 and converts it to the output's BT.709 before encoding (FFmpeg
/// 8.0.1 on the Mac).
#[cfg(feature = "dev-tools")]
fn videotoolbox(
    ffmpeg: &std::ffi::OsStr,
    dir: &std::path::Path,
    name: &str,
    frames: &Synthetic,
    (count, size, rate): (usize, &str, [&str; 2]),
) -> std::path::PathBuf {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tags = [
        "-colorspace",
        "bt709",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-color_range",
        "tv",
    ];
    let mut child = Command::new(ffmpeg)
        .current_dir(dir)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "yuv420p"])
        .args(["-s", size])
        .args(tags)
        .args(["-r", "60", "-i", "-", "-c:v", "h264_videotoolbox"])
        .args(["-b:v", rate[0], "-maxrate", rate[1], "-g", "60", "-bf", "0"])
        .args(tags)
        .args(["-movflags", "+faststart", "-use_editlist", "0", "-an", name])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for k in 0..count {
        let picture = frames.frame(k).unwrap();
        for plane in [&picture.y, &picture.cb, &picture.cr] {
            stdin.write_all(plane).unwrap();
        }
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    dir.join(name)
}

/// The capture profile for the 640x360 clips.
#[cfg(feature = "dev-tools")]
fn vt_plan() -> Plan {
    let text = r#"{"kairo_expect":1,"pattern":1,"size":[640,360],"fps":60,"source_hz":180,
        "seconds":2,"run_tag":5,"tracks":[]}"#;
    Plan::new(Profile::Capture, false, Some(Expect::parse(text).unwrap())).unwrap()
}

/// Opt-in, on the Mac: `KAIRO_FFMPEG=/opt/homebrew/bin/ffmpeg cargo test -p kairo-verify --
/// --ignored`. Encodes the synthetic frames with `h264_videotoolbox` and checks the clip
/// through the CLI decoder: the barcodes, cadence and flash survive a real lossy encoder.
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn a_videotoolbox_clip_reads_back_through_the_cli_decoder() {
    let ffmpeg = ffmpeg();
    let dir = Scratch::new("vt");
    let frames = Synthetic::new(
        Size::new(640, 360).unwrap(),
        options(),
        content(),
        Damage::default(),
    );
    let clip = videotoolbox(
        &ffmpeg,
        &dir.0,
        "vt.mp4",
        &frames,
        (FRAMES as usize, "640x360", ["8M", "8M"]),
    );
    let mut decoder = crate::decode::cli::Cli::new(&ffmpeg, &clip);
    let mut file = std::fs::File::open(&clip).unwrap();
    let report = check_with(&mut file, "vt.mp4", &vt_plan(), Some(&mut decoder));
    for id in [
        "decode.clean",
        "content.barcode",
        "cadence.drops_dups",
        "cadence.judder",
        "content.flash",
        "content.bars",
        "content.colour_matrix",
        "content.levels",
    ] {
        assert_eq!(
            status(&report, id),
            Status::Pass,
            "{id}: {}",
            report.to_text()
        );
    }
    assert_eq!(report.decoder.kind, "cli");
}

/// Opt-in, as above: damage inside the second IDR's slice makes FFmpeg conceal, which it
/// reports only as a warning, and `decode.clean` must fail (review F1 of PR 4a).
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn a_corrupted_idr_fails_decode_clean() {
    let ffmpeg = ffmpeg();
    let dir = Scratch::new("vt-corrupt");
    let frames = Synthetic::new(
        Size::new(640, 360).unwrap(),
        options(),
        content(),
        Damage::default(),
    );
    let clip = videotoolbox(
        &ffmpeg,
        &dir.0,
        "corrupt.mp4",
        &frames,
        (FRAMES as usize, "640x360", ["8M", "8M"]),
    );
    let mut bytes = std::fs::read(&clip).unwrap();
    let parsed = crate::Clip::read(&mut bytes.as_slice()).unwrap();
    let idr = parsed.video_samples[60];
    let start = idr.offset as usize + 1_000;
    let end = idr.offset as usize + (idr.size as usize).min(10_000);
    assert!(end > start, "the IDR is only {} bytes", idr.size);
    for b in &mut bytes[start..end] {
        *b ^= 0x5A;
    }
    std::fs::write(&clip, &bytes).unwrap();
    let mut decoder = crate::decode::cli::Cli::new(&ffmpeg, &clip);
    let report = check_with(
        &mut bytes.as_slice(),
        "corrupt.mp4",
        &vt_plan(),
        Some(&mut decoder),
    );
    assert_eq!(
        status(&report, "decode.clean"),
        Status::Fail,
        "{}",
        report.to_text()
    );
}

/// Opt-in, as above: research 16 §7.2's Mac recipe at 1080p (25/40 Mbps) passes the `test`
/// profile's 4-code line rule, now that the columns where new content enters are left out
/// (review F1 of PR 4b: 4.6 codes at x = 0 before).
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn a_1080p_videotoolbox_clip_passes_the_line_rule() {
    let ffmpeg = ffmpeg();
    let dir = Scratch::new("vt-1080");
    let frames = Synthetic::new(
        Size::new(1920, 1080).unwrap(),
        options(),
        content(),
        Damage::default(),
    );
    let clip = videotoolbox(
        &ffmpeg,
        &dir.0,
        "vt1080.mp4",
        &frames,
        (FRAMES as usize, "1920x1080", ["25M", "40M"]),
    );
    let text = r#"{"kairo_expect":1,"pattern":1,"size":[1920,1080],"fps":60,"source_hz":180,
        "seconds":2,"run_tag":5,"tracks":[]}"#;
    let plan = Plan::new(Profile::Test, false, Some(Expect::parse(text).unwrap())).unwrap();
    let mut decoder = crate::decode::cli::Cli::new(&ffmpeg, &clip);
    let mut file = std::fs::File::open(&clip).unwrap();
    let report = check_with(&mut file, "vt1080.mp4", &plan, Some(&mut decoder));
    for id in ["content.bars", "content.bars_lines", "content.levels"] {
        assert_eq!(
            status(&report, id),
            Status::Pass,
            "{id}: {}",
            report.to_text()
        );
    }
}

/// Writes `samples` (interleaved at `channels`) as a 48 kHz float WAV.
#[cfg(feature = "dev-tools")]
fn write_wav(path: &std::path::Path, channels: u16, samples: &[f32]) {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = b"RIFF".to_vec();
    out.extend((36 + data.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16_u32.to_le_bytes());
    out.extend(3_u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(48_000_u32.to_le_bytes());
    out.extend((48_000 * 4 * u32::from(channels)).to_le_bytes());
    out.extend((4 * channels).to_le_bytes());
    out.extend(32_u16.to_le_bytes());
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    std::fs::write(path, out).unwrap();
}

/// Opt-in, as above: research 16 §7.2's two-step recipe with real AAC. An I_PCM clip from
/// `synth`, stream-copied, then muxed with four tracks of beeps encoded by FFmpeg's `aac`: the
/// game beep at each whole second, the mic marker 250 ms after it and the voice beep 500 ms
/// after it. Every audio check passes, with every track's A/V offset within a millisecond: the
/// decoded priming and the held first frame cancel (cookbook P8). A track one AAC packet late,
/// or a Voice chat track 50 ms late, fails (review F1).
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn an_aac_clip_passes_the_audio_checks() {
    use std::process::{Command, Stdio};

    use crate::synth::{Codec, Synth};

    let ffmpeg = ffmpeg();
    let dir = Scratch::new("aac");
    let synth = Synth {
        format: "480x270@60".parse().unwrap(),
        source_hz: 180,
        seconds: 4,
        defect: None,
        hdr: None,
        codec: Codec::Ipcm { gop: 60 },
    };
    let mut child = Command::new(&ffmpeg)
        .current_dir(&dir.0)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-y"])
        .args(["-f", "h264", "-r", "60", "-i", "-", "-c:v", "copy"])
        .args(["-color_primaries", "bt709", "-color_trc", "bt709"])
        .args(["-colorspace", "bt709", "-color_range", "tv"])
        .args([
            "-an",
            "-movflags",
            "+faststart",
            "-use_editlist",
            "0",
            "v.mp4",
        ])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    synth.write(&mut child.stdin.take().unwrap()).unwrap();
    assert!(child.wait().unwrap().success());
    // (title, channels, game, mic, voice)
    let layout = [
        ("Mix", 2_u16, true, true, true),
        ("Game", 2, true, false, false),
        ("Mic", 1, false, true, false),
        ("Voice chat", 2, false, false, true),
    ];
    let tones = |game: &str, voice: &str, mic: &str| {
        format!(r#"{{"game":"{game}","voice":"{voice}","mic":"{mic}","cue":"absent"}}"#)
    };
    let text = format!(
        r#"{{"kairo_expect":1,"pattern":1,"size":[480,270],"fps":60,"source_hz":180,"seconds":4,
            "run_tag":0,"cabac":false,"tracks":[
            {{"title":"Mix","channels":2,"default":true,"tones":{}}},
            {{"title":"Game","channels":2,"default":false,"tones":{}}},
            {{"title":"Mic","channels":1,"default":false,"tones":{}}},
            {{"title":"Voice chat","channels":2,"default":false,"tones":{}}}]}}"#,
        tones("present", "present", "present"),
        tones("present", "absent", "absent"),
        tones("absent", "absent", "present"),
        tones("absent", "present", "absent"),
    );
    let plan = Plan::new(Profile::Test, false, Some(Expect::parse(&text).unwrap())).unwrap();
    // (case, the track that is late, by how many samples, what fails)
    let cases: [(&str, &str, f64, &[&str]); 3] = [
        ("good", "", 0.0, &[]),
        ("game-late", "Game", 1_024.0, &["av.inter_track"]),
        (
            "voice-late",
            "Voice chat",
            2_400.0,
            &["av.offset", "av.inter_track"],
        ),
    ];
    for (case, late_track, late, fails) in cases {
        for (title, channels, game, mic, voice) in layout {
            let shift = if title == late_track {
                late / 48_000.0
            } else {
                0.0
            };
            let mut sounds = Vec::new();
            for m in 0..4 {
                let t = f64::from(m) + shift;
                if game {
                    sounds.push((Sound::Beep(1_000.0), t, 0.0));
                }
                if mic {
                    sounds.push((Sound::Beep(3_000.0), t + 0.25, 0.0));
                }
                if voice {
                    sounds.push((Sound::Beep(2_000.0), t + 0.5, 0.0));
                }
            }
            let mono = SyntheticAudio {
                frames: 4 * 48_000,
                sounds,
                noise_db: None,
            }
            .render();
            let samples: Vec<f32> = mono
                .iter()
                .flat_map(|&v| std::iter::repeat_n(v, usize::from(channels)))
                .collect();
            write_wav(
                &dir.0.join(format!("{case}-{title}.wav")),
                channels,
                &samples,
            );
        }
        let clip_name = format!("{case}.mp4");
        let mut mux = Command::new(&ffmpeg);
        mux.current_dir(&dir.0).args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-i",
            "v.mp4",
        ]);
        for (title, ..) in layout {
            mux.arg("-i").arg(format!("{case}-{title}.wav"));
        }
        mux.args(["-map", "0:v"]);
        for (i, (title, ..)) in layout.iter().enumerate() {
            mux.args(["-map", &format!("{}:a", i + 1)]);
            mux.arg(format!("-metadata:s:a:{i}"))
                .arg(format!("title={title}"));
            mux.arg(format!("-metadata:s:a:{i}"))
                .arg(format!("handler_name={title}"));
            mux.arg(format!("-disposition:a:{i}"))
                .arg(if i == 0 { "default" } else { "0" });
        }
        mux.args([
            "-c:v", "copy", "-c:a", "aac", "-b:a:0", "160k", "-b:a:1", "160k",
        ])
        .args(["-b:a:2", "128k", "-b:a:3", "160k"])
        .args(["-movflags", "+faststart", "-use_editlist", "0"])
        .arg(&clip_name);
        assert!(mux.status().unwrap().success(), "{case}");
        let clip = dir.0.join(&clip_name);
        let mut decoder = crate::decode::cli::Cli::new(&ffmpeg, &clip);
        let mut file = std::fs::File::open(&clip).unwrap();
        let report = check_with(&mut file, &clip_name, &plan, Some(&mut decoder));
        assert_eq!(failing(&report), fails, "{case}: {}", report.to_text());
        if case != "good" {
            continue;
        }
        for id in [
            "decode.clean",
            "av.offset",
            "av.inter_track",
            "audio.tone_snr",
            "audio.routing",
            "audio.cue",
        ] {
            assert_eq!(
                status(&report, id),
                Status::Pass,
                "{id}: {}",
                report.to_text()
            );
        }
        let offsets = offsets(&report);
        assert_eq!(offsets.len(), 4);
        for track in &offsets {
            assert_eq!(track["measured"], true, "{track}");
            let median = track["median_ms"].as_f64().unwrap();
            assert!(median.abs() < 1.0, "{track}");
        }
    }
}
