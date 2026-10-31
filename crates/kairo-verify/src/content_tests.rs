// SPDX-License-Identifier: GPL-3.0-or-later
//! The content checks end to end (README §9's U tier): the builder's 2 s clip, 120 frames at
//! 60 fps, decoded by the synthetic decoder as a 180 fps testbed's frames, under `test` and
//! `engine`. A good clip passes every content check this build runs; each content defect
//! (drop, duplicate, judder, unreadable, torn, wrong run tag, wrong flash) fails its own.

use kairo_core::testsig::{Load, Options, Patches, Pattern, Size};

use crate::checks::Status;
use crate::decode::synthetic::{Damage, Synthetic};
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

fn plan(profile: Profile, run_tag: u32) -> Plan {
    let text = format!(
        r#"{{"kairo_expect":1,"pattern":1,"size":[320,180],"fps":60,"source_hz":180,"seconds":2,
            "run_tag":{run_tag},"tracks":[
            {{"title":"Mix","channels":2,"default":true}},
            {{"title":"Game","channels":2,"default":false}},
            {{"title":"Mic","channels":1,"default":false}}]}}"#
    );
    Plan::new(profile, false, Some(Expect::parse(&text).unwrap())).unwrap()
}

/// Output frame k showing 180 fps content: testbed frame 3k, from a clip-start offset.
fn content() -> Vec<u64> {
    (0..FRAMES).map(|k| 1_000 + 3 * k).collect()
}

fn run(content: Vec<u64>, damage: Damage, options: Options, plan: &Plan) -> Report {
    let bytes = Spec::good().build();
    let mut decoder = Synthetic::new(Size::new(320, 180).unwrap(), options, content, damage);
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
        "120/120 barcodes, 0 skipped, 0 repeats, judder excess 0.0 pp"
    );
    // Bars, levels and audio come later: `test` can't pass yet.
    assert_eq!(status(&report, "content.bars"), Status::Error);
    // Noise and scaling don't change it.
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
    let mut decoder = Synthetic::new(Size::new(320, 180).unwrap(), options, content, damage);
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
fn a_clip_that_shows_the_pattern_is_held_to_every_required_check() {
    // Engine with no pattern named, but the pattern shows: the checks this build lacks are
    // errors, not skips (review F3).
    let report = run(content(), Damage::default(), options(), &engine_plan());
    assert_eq!(status(&report, "content.barcode"), Status::Pass);
    assert_eq!(status(&report, "content.bars"), Status::Error);
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

/// Encodes `count` synthetic frames at 640x360 with `h264_videotoolbox` into `dir/name`
/// (spawned, never libx264): a GOP of 60 at 8 Mbps, no edit list. The raw frames are BT.709
/// limited, and FFmpeg must be told so: untagged, it takes a picture this size for BT.601 and
/// converts it to the output's BT.709 before encoding (FFmpeg 8.0.1 on the Mac).
#[cfg(feature = "dev-tools")]
fn videotoolbox(
    ffmpeg: &std::ffi::OsStr,
    dir: &std::path::Path,
    name: &str,
    frames: &Synthetic,
    count: usize,
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
        .args(["-s", "640x360"])
        .args(tags)
        .args([
            "-r",
            "60",
            "-i",
            "-",
            "-c:v",
            "h264_videotoolbox",
            "-b:v",
            "8M",
        ])
        .args(["-g", "60"])
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
    let clip = videotoolbox(&ffmpeg, &dir.0, "vt.mp4", &frames, FRAMES as usize);
    let mut decoder = crate::decode::cli::Cli::new(&ffmpeg, &clip);
    let mut file = std::fs::File::open(&clip).unwrap();
    let report = check_with(&mut file, "vt.mp4", &vt_plan(), Some(&mut decoder));
    for id in [
        "decode.clean",
        "content.barcode",
        "cadence.drops_dups",
        "cadence.judder",
        "content.flash",
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
    let clip = videotoolbox(&ffmpeg, &dir.0, "corrupt.mp4", &frames, FRAMES as usize);
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
