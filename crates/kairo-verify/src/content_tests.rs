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
fn synth_audio_defects_fail_their_own_checks() {
    // `synth`'s 2 s clip on the builder with research 16 §7.1's four tracks, its WAVs as AAC
    // decodes them (1,024 samples of priming first): the good clip passes `test`, and each audio
    // defect of §7.3 fails its `must_fail` and nothing else.
    use crate::synth::{AudioTrack, Codec, Defect, Synth};
    use crate::testkit::AudioSpec;

    const PACKETS: usize = 95;
    let spec = Spec::with(|s| {
        s.audio = AudioTrack::ALL
            .iter()
            .map(|&t| AudioSpec::new(t.title(), t.channels(), t == AudioTrack::Mix, PACKETS))
            .collect();
    });
    let bytes = spec.build();
    let good = Synth {
        format: "320x180@60".parse().unwrap(),
        source_hz: 180,
        seconds: 2,
        defect: None,
        hdr: None,
        codec: Codec::Nv12,
    };
    let plan = Plan::new(Profile::Test, false, Some(good.expect(true))).unwrap();
    let cases: [(Option<Defect>, &[&str]); 6] = [
        (None, &[]),
        (Some(Defect::Av50), &["av.offset"]),
        (Some(Defect::SwapGameVoice), &["audio.routing"]),
        (Some(Defect::Cue), &["audio.cue"]),
        (Some(Defect::Leak), &["audio.routing"]),
        (Some(Defect::Noisy), &["audio.tone_snr"]),
    ];
    for (defect, must_fail) in cases {
        let synth = Synth { defect, ..good };
        let tracks = AudioTrack::ALL
            .map(|track| {
                let mut audio = synth.track_audio(track);
                for sound in &mut audio.sounds {
                    sound.1 += 1_024.0 / 48_000.0;
                }
                audio.frames = PACKETS * 1_024;
                audio
            })
            .to_vec();
        let mut decoder = synth.source().unwrap().with_audio(tracks);
        let report = check_with(&mut bytes.as_slice(), "clip.mp4", &plan, Some(&mut decoder));
        let name = defect.map_or("good".to_owned(), |d| d.to_string());
        assert_eq!(failing(&report), must_fail, "{name}: {}", report.to_text());
        if defect.is_none() {
            assert_eq!(report.verdict, crate::Verdict::Pass, "{}", report.to_text());
        }
        // The corpus requires av50 to measure +50 ± 1 ms (README §7.4), and the good clip 0 ± 1.
        let late = if defect == Some(Defect::Av50) {
            50.0
        } else {
            0.0
        };
        let offsets = report
            .checks
            .iter()
            .find(|f| f.id == "av.offset")
            .and_then(|f| f.value.get("tracks")?.as_array().cloned())
            .unwrap();
        let measured: Vec<f64> = offsets
            .iter()
            .filter(|t| t["measured"] == true)
            .map(|t| t["median_ms"].as_f64().unwrap())
            .collect();
        assert!(!measured.is_empty(), "{name}");
        // Every track still timed beside a cue or noise: a defect may not blind the detector.
        if !matches!(defect, Some(Defect::SwapGameVoice)) {
            assert_eq!(measured.len(), 4, "{name}: {offsets:?}");
        }
        for median in measured {
            assert!((median - late).abs() < 1.0, "{name}: {median} ms");
        }
    }
}

#[test]
fn a_clip_that_shows_the_pattern_is_held_to_every_required_check() {
    // Engine with no pattern named, but the pattern shows: the checks this build lacks are
    // errors, not skips (review F3).
    let report = run(content(), Damage::default(), options(), &engine_plan());
    assert_eq!(status(&report, "content.barcode"), Status::Pass);
    assert_eq!(status(&report, "content.bars"), Status::Pass);
    assert_eq!(status(&report, "av.offset"), Status::Pass);
    // source.stability needs the testbed's log (--testbed-log), which this check wasn't given.
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

/// Opt-in, as above: research 16 §7.2's two-step recipe with real AAC. `synth`'s I_PCM video,
/// stream-copied once into a video-only file, then muxed with each case's four WAVs, encoded by
/// FFmpeg's `aac`. The good clip passes every audio check with every track's A/V offset within a
/// millisecond (the decoded priming and the held first frame cancel, cookbook P8); each audio
/// defect of §7.3 fails its own check and nothing else, and `av50` measures +50 ± 1 ms; a track
/// one AAC packet late, or a Voice chat track 50 ms late, fails too (review F1).
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn aac_clips_from_synth_behave() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use crate::synth::{AudioTrack, Codec, Defect, Synth};

    let ffmpeg = ffmpeg();
    let dir = Scratch::new("aac");
    let good = Synth {
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
    good.write(&mut child.stdin.take().unwrap()).unwrap();
    assert!(child.wait().unwrap().success());
    let plan = Plan::new(Profile::Test, false, Some(good.expect(true))).unwrap();
    // (case, its defect, a track made late by some samples, what fails)
    type Case = (
        &'static str,
        Option<Defect>,
        Option<(AudioTrack, f64)>,
        &'static [&'static str],
    );
    let cases: [Case; 8] = [
        ("good", None, None, &[]),
        ("av50", Some(Defect::Av50), None, &["av.offset"]),
        (
            "swap-game-voice",
            Some(Defect::SwapGameVoice),
            None,
            &["audio.routing"],
        ),
        ("cue", Some(Defect::Cue), None, &["audio.cue"]),
        ("leak", Some(Defect::Leak), None, &["audio.routing"]),
        ("noisy", Some(Defect::Noisy), None, &["audio.tone_snr"]),
        (
            "game-late",
            None,
            Some((AudioTrack::Game, 1_024.0)),
            &["av.inter_track"],
        ),
        (
            "voice-late",
            None,
            Some((AudioTrack::Voice, 2_400.0)),
            &["av.offset", "av.inter_track"],
        ),
    ];
    for (name, defect, late, must_fail) in cases {
        let synth = Synth { defect, ..good };
        let folder = dir.0.join(name);
        std::fs::create_dir(&folder).unwrap();
        for track in AudioTrack::ALL {
            let mut audio = synth.track_audio(track);
            if let Some((_, samples)) = late.filter(|(t, _)| *t == track) {
                for sound in &mut audio.sounds {
                    sound.1 += samples / 48_000.0;
                }
            }
            let file = std::fs::File::create(folder.join(track.file())).unwrap();
            let mut file = std::io::BufWriter::new(file);
            crate::wav::write_float(&mut file, 48_000, track.channels(), &audio.render()).unwrap();
            file.flush().unwrap();
        }
        let mut mux = Command::new(&ffmpeg);
        mux.current_dir(&folder)
            .args(["-hide_banner", "-nostdin", "-v", "error", "-y"])
            .arg("-i")
            .arg(dir.0.join("v.mp4"));
        for track in AudioTrack::ALL {
            mux.args(["-i", track.file()]);
        }
        mux.args(["-map", "0:v", "-c:v", "copy", "-c:a", "aac"]);
        for (i, track) in AudioTrack::ALL.iter().enumerate() {
            let title = track.title();
            mux.args(["-map", &format!("{}:a", i + 1)])
                .arg(format!("-b:a:{i}"))
                .arg(format!("{}k", track.kbps()))
                .arg(format!("-metadata:s:a:{i}"))
                .arg(format!("title={title}"))
                .arg(format!("-metadata:s:a:{i}"))
                .arg(format!("handler_name={title}"))
                .arg(format!("-disposition:a:{i}"))
                .arg(if i == 0 { "default" } else { "0" });
        }
        mux.args(["-movflags", "+faststart", "-use_editlist", "0", "clip.mp4"]);
        assert!(mux.status().unwrap().success(), "{name}");
        let clip = folder.join("clip.mp4");
        let mut decoder = crate::decode::cli::Cli::new(&ffmpeg, &clip);
        let mut file = std::fs::File::open(&clip).unwrap();
        let report = check_with(&mut file, "clip.mp4", &plan, Some(&mut decoder));
        assert_eq!(failing(&report), must_fail, "{name}: {}", report.to_text());
        let tracks = offsets(&report);
        if name == "good" {
            assert_eq!(report.verdict, crate::Verdict::Pass, "{}", report.to_text());
        }
        // Every track still timed beside a cue or noise: a defect may not blind the detector.
        if name != "swap-game-voice" {
            assert_eq!(tracks.len(), 4);
            assert!(
                tracks.iter().all(|t| t["measured"] == true),
                "{name}: {tracks:?}"
            );
        }
        // What av.offset reads where every track moves together.
        let want = match (defect, late) {
            (_, Some(_)) => continue,
            (Some(Defect::Av50), None) => 50.0,
            _ => 0.0,
        };
        let measured: Vec<f64> = tracks
            .iter()
            .filter(|t| t["measured"] == true)
            .map(|t| t["median_ms"].as_f64().unwrap())
            .collect();
        assert!(!measured.is_empty(), "{name}");
        for median in measured {
            assert!((median - want).abs() < 1.0, "{name}: {median} ms");
        }
    }
}

/// Opt-in: 3 kHz mic markers at sub-sample onsets through FFmpeg's `aac` at 96 and 160 kbps
/// come back on their own cycle (PR 5a's review dsp-3: AAC smears a 3 kHz rise by up to
/// 0.27 ms, more than half its 0.33 ms period).
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn three_khz_markers_keep_their_cycle_through_aac() {
    use std::process::Command;

    use crate::analyze::audio::{Detector, RATE};

    let ffmpeg = ffmpeg();
    let dir = Scratch::new("markers");
    // 200 markers a second apart, each a different fraction of a sample late.
    let onsets: Vec<f64> = (0..200)
        .map(|k| 2_400.0 + 48_000.0 * f64::from(k) + (f64::from(k) * 0.618_034).fract())
        .collect();
    let sounds = onsets
        .iter()
        .map(|&o| (Sound::Beep(3_000.0), o / RATE, 0.0))
        .collect();
    let mono = SyntheticAudio {
        frames: 200 * 48_000 + 4_800,
        sounds,
        noise_db: Some(-70.0),
    }
    .render();
    for (channels, kbps) in [(2_u16, "96k"), (2, "160k"), (1, "128k")] {
        let mut wav = Vec::new();
        crate::wav::write_float(&mut wav, 48_000, channels, &mono).unwrap();
        std::fs::write(dir.0.join("m.wav"), wav).unwrap();
        let encoded = Command::new(&ffmpeg)
            .current_dir(&dir.0)
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-y",
                "-i",
                "m.wav",
            ])
            .args(["-c:a", "aac", "-b:a", kbps, "-use_editlist", "0", "m.mp4"])
            .status()
            .unwrap();
        assert!(encoded.success());
        let decoded = Command::new(&ffmpeg)
            .current_dir(&dir.0)
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-ignore_editlist",
                "1",
            ])
            .args(["-c:a", "aac", "-i", "m.mp4", "-f", "f32le", "-"])
            .output()
            .unwrap();
        let pcm: Vec<f32> = decoded
            .stdout
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f32::from_le_bytes(b))
            .collect();
        let mut x = Vec::new();
        crate::analyze::audio::downmix(&pcm, usize::from(channels), &mut x);
        let detector = Detector::new(3_000.0);
        let (mut found, mut slips) = (0, Vec::new());
        for &o in &onsets {
            // The encoder's priming comes first.
            let truth = o + 1_024.0;
            if let Some(b) = detector.find(&x, truth, 100.0 * 48.0) {
                found += 1;
                let error_us = (b.at - truth) / RATE * 1e6;
                if error_us.abs() > 100.0 {
                    slips.push(error_us.round());
                }
            }
        }
        eprintln!(
            "{channels} ch {kbps}: {found} of {} found, slips {slips:?}",
            onsets.len()
        );
        assert!(slips.is_empty(), "{channels} ch {kbps}: {slips:?}");
        assert!(found >= 198, "{channels} ch {kbps}: {found}");
    }
}

/// QPC time, in 100 ns, when the testbed showed frame 1,000 in [`testbed_log`].
const LOG_BASE_HNS: i64 = 100_000_000;

/// A testbed log of frames `first..=last`, drawn at 180 fps on 180 Hz and shown on cadence from
/// [`LOG_BASE_HNS`] (frame `first` there), except `missed`, never shown and each with an `m`
/// line; beeps 6 and 7 at their flashes, played `aim` seconds after the flash showed; run `run`.
fn testbed_log(
    first: u64,
    last: u64,
    missed: &[u64],
    aim: f64,
    run: u8,
) -> kairo_core::testbed_log::TestbedLog {
    let shown = |n: u64| LOG_BASE_HNS + ((n - first) as f64 * 10_000_000.0 / 180.0).round() as i64;
    let mut text = format!(
        "# kairo-testbed 1\nh pattern=1 run={run} mode=borderless pace=vsync fps=180 F=180 size=320x180 refresh_mhz=180000 hdr=0 sdr_white_nits=- max_nits=- start_hns={LOG_BASE_HNS}\n"
    );
    for n in first..=last {
        let t = shown(n);
        if missed.contains(&n) {
            text.push_str(&format!("f {n} {t} {} -\nm {n} vblank\n", t - 20_000));
        } else {
            text.push_str(&format!("f {n} {t} {} {t}\n", t - 20_000));
        }
    }
    for m in 6..=7_u64 {
        let n_f = 180 * m;
        if (first..=last).contains(&n_f) {
            let t = shown(n_f);
            let played = t + (aim * 10_000_000.0).round() as i64;
            text.push_str(&format!("b {m} {n_f} {t} {played}\n"));
        }
    }
    kairo_core::testbed_log::TestbedLog::parse(&text).unwrap()
}

/// Each `av.offset` track's median, in ms, of the measured tracks.
fn medians(report: &Report) -> Vec<f64> {
    offsets(report)
        .iter()
        .filter(|t| t["measured"] == true)
        .map(|t| t["median_ms"].as_f64().unwrap())
        .collect()
}

#[test]
fn with_the_testbed_log_av_is_exact_and_the_source_is_judged() {
    // The testbed's beeps played 30 ms after their flashes showed, and the capture kept that.
    let aim = 0.030;
    let spec = Spec::good();
    let check = |plan: &Plan| {
        let bytes = spec.build();
        let mut decoder = Synthetic::new(
            Size::new(320, 180).unwrap(),
            options(),
            content(),
            Damage::default(),
        )
        .with_audio(audio(&spec, true, aim));
        check_with(&mut bytes.as_slice(), "clip.mp4", plan, Some(&mut decoder))
    };
    let mut plan = plan_for(Profile::Engine, 60, 2, 180);
    // Without the log the testbed's 30 ms reads as the capture's, and source.stability can't run.
    let blind = check(&plan);
    assert!(
        medians(&blind).iter().all(|m| (m - 30.0).abs() < 1.0),
        "{:?}",
        medians(&blind)
    );
    assert_eq!(status(&blind, "source.stability"), Status::Error);
    // With it, A/V is the capture's own: 0. The testbed missed nothing.
    plan.testbed_log = Some(testbed_log(1_000, 1_400, &[], aim, 5));
    let exact = check(&plan);
    let m = medians(&exact);
    assert_eq!(m.len(), 2, "{}", exact.to_text());
    assert!(m.iter().all(|m| m.abs() < 1.0), "{m:?}");
    assert_eq!(
        status(&exact, "source.stability"),
        Status::Pass,
        "{}",
        exact.to_text()
    );
    assert_eq!(status(&exact, "cadence.judder"), Status::Pass);
    let judder = exact
        .checks
        .iter()
        .find(|f| f.id == "cadence.judder")
        .unwrap();
    assert_eq!(judder.value["p99_from"], "the testbed log's display times");
}

#[test]
fn steps_at_testbed_misses_are_the_sources_not_the_clips() {
    // Every 20th output frame repeats the one before it, because the testbed never showed the
    // two frames after that one, holding it three vblanks, long enough for a 60 fps capture to
    // show it twice: 12 of 119 steps, far over engine's 0.5%.
    let content: Vec<u64> = (0..FRAMES)
        .map(|k| 1_000 + 3 * if k % 20 == 10 { k - 1 } else { k })
        .collect();
    let missed: Vec<u64> = (0..FRAMES)
        .filter(|k| k % 20 == 10)
        .flat_map(|k| [1_000 + 3 * (k - 1) + 1, 1_000 + 3 * (k - 1) + 2])
        .collect();
    let mut plan = plan_for(Profile::Engine, 60, 2, 180);
    let blind = run_spec(
        &Spec::good(),
        content.clone(),
        Damage::default(),
        options(),
        &plan,
    );
    assert_eq!(status(&blind, "cadence.drops_dups"), Status::Fail);
    plan.testbed_log = Some(testbed_log(1_000, 1_400, &missed, 0.0, 5));
    let report = run_spec(&Spec::good(), content, Damage::default(), options(), &plan);
    // The clip isn't blamed: its steps at the misses are set aside...
    assert_eq!(
        status(&report, "cadence.drops_dups"),
        Status::Pass,
        "{}",
        report.to_text()
    );
    let drops = report
        .checks
        .iter()
        .find(|f| f.id == "cadence.drops_dups")
        .unwrap();
    assert_eq!(drops.value["source_events"], 12, "{}", drops.detail);
    // ...and the testbed is: 12 of the clip's testbed frames missed, over 0.1%.
    let stability = report
        .checks
        .iter()
        .find(|f| f.id == "source.stability")
        .unwrap();
    assert_eq!(stability.status, Status::Fail);
    assert!(
        stability
            .detail
            .starts_with("rerun: the testbed itself missed 12"),
        "{}",
        stability.detail
    );
}

#[test]
fn another_runs_log_is_refused() {
    let mut plan = plan_for(Profile::Engine, 60, 2, 180);
    plan.testbed_log = Some(testbed_log(5_000, 5_400, &[], 0.0, 9));
    let report = run_spec(
        &Spec::good(),
        content(),
        Damage::default(),
        options(),
        &plan,
    );
    let stability = report
        .checks
        .iter()
        .find(|f| f.id == "source.stability")
        .unwrap();
    assert_eq!(stability.status, Status::Fail);
    assert!(
        stability.detail.contains("another run's log"),
        "{}",
        stability.detail
    );
    // The barcodes carry run tag 5, the log and the expect file disagree on it.
    let barcode = report
        .checks
        .iter()
        .find(|f| f.id == "content.barcode")
        .unwrap();
    assert_eq!(barcode.status, Status::Fail);
    assert!(
        barcode.detail.contains("run tag is 9, the expect file's 5"),
        "{}",
        barcode.detail
    );
}

#[test]
fn a_voice_track_is_timed_against_kairo_tones_log() {
    // The voice beeps at flash + 500 ms in the clip played `early` seconds earlier by
    // kairo-tone's log: the Voice chat track is that late against QPC, while it agrees with the
    // Mix. Up to half the 1 s voice period, the log's time is what it's judged by, never the Mix.
    for early in [0.020, 0.150, 0.300] {
        let (spec, mut plan) = four_tracks();
        plan.testbed_log = Some(testbed_log(1_000, 1_400, &[], 0.0, 5));
        // The clip's clock: frame n at shown_at(n), shown at QPC LOG_BASE + (n − 1000)/180 s.
        let clock = shown_at(1_000.0) - LOG_BASE_HNS as f64 / 1e7;
        let voices: String = (6..=7)
            .enumerate()
            .map(|(k, m)| {
                let at = shown_at(180.0 * f64::from(m)) + 0.5 - clock - early;
                let hns = (at * 1e7).round() as i64;
                format!("v {k} {hns} {hns}\n")
            })
            .collect();
        plan.voice_log = Some(
            kairo_core::testbed_log::ToneLog::parse(&format!(
                "# kairo-tone 1\nh run=5 hz=2000 offset_ms=500 dbfs=-30\n{voices}"
            ))
            .unwrap(),
        );
        let report = run_audio(&spec, &plan, four_track_audio(&spec));
        let voice = offsets(&report)
            .into_iter()
            .find(|t| t["track"] == "Voice chat" || t["title"] == "Voice chat")
            .unwrap_or_else(|| panic!("{}", report.to_text()));
        assert_eq!(
            voice["via"], "voice beeps, against kairo-tone's log",
            "{voice}"
        );
        let median = voice["median_ms"].as_f64().unwrap();
        assert!((median - early * 1_000.0).abs() < 1.0, "{early}: {voice}");
    }
}
