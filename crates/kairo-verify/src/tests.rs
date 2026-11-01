// SPDX-License-Identifier: GPL-3.0-or-later
//! Clip-level tests (container.md §9.1, README §9's U tier): a good clip built in memory passes
//! every structure check, and each single defect fails its own check and nothing else; the
//! parser never panics on cut or damaged files; the report and the command line say so.

use crate::checks::Status;
use crate::expect::Expect;
use crate::profile::{Plan, Profile, STRUCTURE_CHECKS};
use crate::testkit::{AudioSpec, ColrSpec, Pic, Spec, SpsSpec, VuiSpec};
use crate::{Report, Verdict, check};

fn expect_text(seconds: u32, tracks: &[(&str, u16, bool)]) -> String {
    let tracks: Vec<String> = tracks
        .iter()
        .map(|(t, c, d)| format!(r#"{{"title":"{t}","channels":{c},"default":{d}}}"#))
        .collect();
    format!(
        r#"{{"kairo_expect":1,"size":[320,180],"fps":60,"seconds":{seconds},"tracks":[{}]}}"#,
        tracks.join(",")
    )
}

const ROLES: [(&str, u16, bool); 3] = [("Mix", 2, true), ("Game", 2, false), ("Mic", 1, false)];

fn plan(profile: Profile, seconds: u32, tracks: &[(&str, u16, bool)]) -> Plan {
    let expect = Expect::parse(&expect_text(seconds, tracks)).unwrap();
    Plan::new(profile, false, Some(expect)).unwrap()
}

/// A plan from an expect file with this rate, length and size (or none).
fn plan_with(
    profile: Profile,
    fps: u32,
    seconds: u32,
    size: Option<(u32, u32)>,
    tracks: &[(&str, u16, bool)],
) -> Plan {
    let tracks: Vec<String> = tracks
        .iter()
        .map(|(t, c, d)| format!(r#"{{"title":"{t}","channels":{c},"default":{d}}}"#))
        .collect();
    let size = size.map_or(String::new(), |(w, h)| format!(r#""size":[{w},{h}],"#));
    let text = format!(
        r#"{{"kairo_expect":1,{size}"fps":{fps},"seconds":{seconds},"tracks":[{}]}}"#,
        tracks.join(",")
    );
    Plan::new(profile, false, Some(Expect::parse(&text).unwrap())).unwrap()
}

/// The engine profile with an expect file for the 2 s clip: content checks skip.
fn engine() -> Plan {
    plan(Profile::Engine, 2, &ROLES)
}

fn report(spec: &Spec, plan: &Plan) -> Report {
    let bytes = spec.build();
    check(&mut bytes.as_slice(), "clip.mp4", plan)
}

fn status(report: &Report, id: &str) -> Status {
    report
        .checks
        .iter()
        .find(|f| f.id == id)
        .map(|f| f.status)
        .unwrap_or_else(|| panic!("no {id}"))
}

fn failing(report: &Report) -> Vec<&'static str> {
    report
        .checks
        .iter()
        .filter(|f| f.status == Status::Fail)
        .map(|f| f.id)
        .collect()
}

/// Asserts that `spec` under `plan` fails exactly `ids`.
#[track_caller]
fn fails_only(spec: &Spec, plan: &Plan, ids: &[&str]) -> Report {
    let report = report(spec, plan);
    assert_eq!(failing(&report), ids, "{}", report.to_text());
    report
}

fn audio(packets: usize) -> Vec<AudioSpec> {
    vec![
        AudioSpec::new("Mix", 2, true, packets),
        AudioSpec::new("Game", 2, false, packets),
        AudioSpec::new("Mic", 1, false, packets),
    ]
}

// ---------------------------------------------------------------------------------------------
// Good clips
// ---------------------------------------------------------------------------------------------

#[test]
fn a_good_clip_passes_every_structure_check() {
    let report = report(&Spec::good(), &engine());
    for id in STRUCTURE_CHECKS {
        assert_eq!(
            status(&report, id),
            Status::Pass,
            "{id}: {}",
            report.to_text()
        );
    }
    assert_eq!(report.verdict, Verdict::Pass);
    assert_eq!(report.verdict.exit_code(), 0);
    assert!(
        report
            .to_text()
            .starts_with("PASS  clip.mp4 (profile engine)")
    );
    // In the test profile the content checks are required and this build can't run them.
    let test = report_for_test(&Spec::good());
    assert!(failing(&test).is_empty(), "{}", test.to_text());
    assert_eq!(test.verdict, Verdict::Error);
    assert_eq!(status(&test, "decode.clean"), Status::Error);
    assert_eq!(status(&test, "build.lgpl"), Status::Pass);
}

fn report_for_test(spec: &Spec) -> Report {
    report(spec, &plan(Profile::Test, 2, &ROLES))
}

#[test]
fn good_variants_pass_too() {
    let variants: Vec<(&str, Spec)> = vec![
        ("version-1 boxes", Spec::with(|s| s.version1 = true)),
        ("a 64-bit mdat", Spec::with(|s| s.largesize_mdat = true)),
        ("co64", Spec::with(|s| s.co64 = true)),
        (
            "a size-0 mdat at the end",
            Spec::with(|s| s.mdat_to_end = true),
        ),
        ("many chunks", Spec::with(|s| s.video_chunk = 7)),
        ("one chunk", Spec::with(|s| s.video_chunk = 1_000)),
        ("no pasp", Spec::with(|s| s.pasp = None)),
        (
            "a rich SPS",
            Spec::with(|s| {
                s.sps.scaling_lists = true;
                s.sps.vui = Some(VuiSpec {
                    aspect: Some((1, 0, 0)),
                    timing: Some((1, 120, true)),
                    nal_hrd: true,
                    reorder: Some(0),
                    ..VuiSpec::default()
                });
            }),
        ),
        (
            "big samples read NAL by NAL",
            Spec::with(|s| s.video_sample_bytes = 20_000),
        ),
    ];
    for (name, spec) in variants {
        let report = report(&spec, &engine());
        assert_eq!(
            report.verdict,
            Verdict::Pass,
            "{name}: {}",
            report.to_text()
        );
        assert!(
            report.checks.iter().all(|f| f.status != Status::Warn),
            "{name}: {}",
            report.to_text()
        );
    }
}

// ---------------------------------------------------------------------------------------------
// container.structure (C0)
// ---------------------------------------------------------------------------------------------

fn structure_fails(bytes: &[u8], why: &str) {
    let report = check(&mut &bytes[..], "clip.mp4", &engine());
    assert_eq!(
        status(&report, "container.structure"),
        Status::Fail,
        "{}",
        report.to_text()
    );
    let detail = &report.checks[0].events.join(" | ");
    assert!(detail.contains(why), "want {why:?} in {detail}");
    // Everything else waits for a sound structure.
    for id in &STRUCTURE_CHECKS[1..] {
        assert_eq!(status(&report, id), Status::Skip, "{id}");
    }
    assert_eq!(report.verdict, Verdict::Fail);
}

#[test]
fn structural_defects_fail_the_structure_check() {
    let good = Spec::good().build();
    structure_fails(&good[..good.len() - 10], "overruns the file");
    structure_fails(&Spec::with(|s| s.moof = true).build(), "fragmented");
    structure_fails(
        &Spec::with(|s| {
            s.deltas.pop();
        })
        .build(),
        "stsz has 120 samples but stts 119",
    );
    structure_fails(&Spec::with(|s| s.dref_flags = 0).build(), "self-contained");
    structure_fails(
        &Spec::with(|s| s.ctts = Some(vec![(119, 0)])).build(),
        "ctts covers 119",
    );
    structure_fails(
        &Spec::with(|s| s.stss = Some(vec![1, 61, 200])).build(),
        "stss is not strictly rising",
    );
    structure_fails(
        &Spec::with(|s| s.stss = Some(vec![61, 1])).build(),
        "stss is not strictly rising",
    );
    let mut free_first = crate::testkit::boxed(b"free", &[]);
    free_first.extend_from_slice(&good);
    structure_fails(&free_first, "the first box is free, not ftyp");
    // A second moov.
    let moov_at = good.windows(4).position(|w| w == b"moov").unwrap() - 4;
    let moov_len = u32::from_be_bytes(good[moov_at..moov_at + 4].try_into().unwrap()) as usize;
    let mut two = good.clone();
    two.extend_from_slice(&good[moov_at..moov_at + moov_len]);
    structure_fails(&two, "2 moov boxes");
    // A chunk offset past the end of the file.
    let mut far = good.clone();
    let stco = far.windows(4).position(|w| w == b"stco").unwrap();
    far[stco + 12..stco + 16].copy_from_slice(&0x00F0_0000_u32.to_be_bytes());
    structure_fails(&far, "outside every mdat payload");
}

#[test]
fn cut_and_damaged_files_never_panic_and_never_pass() {
    let good = Spec::good().build();
    let moov_at = good.windows(4).position(|w| w == b"moov").unwrap() - 4;
    let moov_end =
        moov_at + u32::from_be_bytes(good[moov_at..moov_at + 4].try_into().unwrap()) as usize;
    // Every cut inside the moov, and every 7th byte after it.
    let cuts = (0..moov_end).chain((moov_end..good.len()).step_by(7));
    for cut in cuts {
        let report = check(&mut &good[..cut], "cut.mp4", &engine());
        assert_ne!(report.verdict, Verdict::Pass, "cut at {cut}");
    }
    // Seeded byte flips anywhere: no panic, whatever the verdict.
    let mut x: u64 = 0x4B41_4952_4F00_0002;
    for _ in 0..3_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let mut damaged = good.clone();
        let at = (x % good.len() as u64) as usize;
        damaged[at] ^= (x >> 32) as u8 | 1;
        let _ = check(&mut damaged.as_slice(), "flip.mp4", &engine());
    }
}

#[test]
fn files_that_are_not_clips_cannot_be_evaluated() {
    for (bytes, why) in [
        (&b""[..], "not an MP4"),
        (&b"hello, this is not a video"[..], "not an MP4"),
    ] {
        let report = check(&mut &bytes[..], "x.mp4", &engine());
        assert_eq!(report.verdict, Verdict::Error);
        assert!(report.error.as_deref().unwrap_or("").contains(why));
        assert_eq!(report.verdict.exit_code(), 3);
        assert!(report.to_text().contains("cannot evaluate"));
    }
    // An HEVC-labelled track isn't H.264.
    let mut hevc = Spec::good().build();
    while let Some(at) = hevc.windows(4).position(|w| w == b"avc1") {
        hevc[at..at + 4].copy_from_slice(b"hvc1");
    }
    let report = check(&mut hevc.as_slice(), "x.mp4", &engine());
    assert_eq!(report.error.as_deref(), Some("no H.264 (avc1) video track"));
}

// ---------------------------------------------------------------------------------------------
// One defect, one check (C1–C14)
// ---------------------------------------------------------------------------------------------

#[test]
fn faststart_and_edit_lists() {
    fails_only(
        &Spec::with(|s| s.faststart = false),
        &engine(),
        &["container.faststart"],
    );
    fails_only(
        &Spec::with(|s| s.edts = true),
        &engine(),
        &["container.no_editlist"],
    );
    // In an original stream copy an edit list only warns (PLAN:237).
    let original = report(
        &Spec::with(|s| s.edts = true),
        &plan(Profile::Original, 2, &ROLES),
    );
    assert_eq!(status(&original, "container.no_editlist"), Status::Warn);
    assert_eq!(original.verdict, Verdict::Pass, "{}", original.to_text());
}

#[test]
fn colr_must_be_bt709_limited() {
    let e = engine();
    fails_only(&Spec::with(|s| s.colr = vec![]), &e, &["video.colr"]);
    fails_only(
        &Spec::with(|s| s.colr = vec![ColrSpec::Other(*b"nclc")]),
        &e,
        &["video.colr"],
    );
    fails_only(
        &Spec::with(|s| s.colr = vec![ColrSpec::Nclx(1, 1, 1, false, 0); 2]),
        &e,
        &["video.colr"],
    );
    // A wrong colr also disagrees with the VUI.
    fails_only(
        &Spec::with(|s| s.colr = vec![ColrSpec::Nclx(6, 6, 6, false, 0)]),
        &e,
        &["video.colr", "video.params"],
    );
    fails_only(
        &Spec::with(|s| s.colr = vec![ColrSpec::Nclx(1, 1, 1, true, 0)]),
        &e,
        &["video.colr", "video.params"],
    );
    let reserved = fails_only(
        &Spec::with(|s| s.colr = vec![ColrSpec::Nclx(1, 1, 1, false, 1)]),
        &e,
        &[],
    );
    assert_eq!(status(&reserved, "video.colr"), Status::Warn);
}

fn with_vui(change: impl FnOnce(&mut VuiSpec)) -> Spec {
    Spec::with(|s| {
        let mut vui = VuiSpec::default();
        change(&mut vui);
        s.sps.vui = Some(vui);
    })
}

#[test]
fn cavlc_passes_only_where_the_expect_file_says_so() {
    // README §7.2: the CI corpus's lossless I_PCM clips are CAVLC, and their expect file says so.
    let text = |cabac: &str| {
        format!(
            r#"{{"kairo_expect":1,"fps":60,"seconds":2,{cabac}"tracks":[
                {{"title":"Mix","channels":2,"default":true}},
                {{"title":"Game","channels":2,"default":false}},
                {{"title":"Mic","channels":1,"default":false}}]}}"#
        )
    };
    let lossless = |profile| {
        Plan::new(
            profile,
            false,
            Some(Expect::parse(&text(r#""cabac":false,"#)).unwrap()),
        )
    };
    let cavlc = Spec::with(|s| s.cabac = false);
    fails_only(&cavlc, &lossless(Profile::Test).unwrap(), &[]);
    let report = fails_only(
        &Spec::good(),
        &lossless(Profile::Test).unwrap(),
        &["video.params"],
    );
    let params = report.checks.iter().find(|f| f.id == "video.params");
    assert_eq!(params.map(|f| f.detail.as_str()), Some("CABAC; want CAVLC"));
    // Without it CAVLC is a defect, and only the test profile takes it.
    fails_only(&cavlc, &plan(Profile::Test, 2, &ROLES), &["video.params"]);
    for profile in [
        Profile::Capture,
        Profile::Smoke,
        Profile::Engine,
        Profile::Original,
        Profile::Share,
    ] {
        assert!(lossless(profile).is_err(), "{profile}");
    }
    let cabac = Expect::parse(&text(r#""cabac":true,"#)).unwrap();
    fails_only(
        &Spec::good(),
        &Plan::new(Profile::Capture, false, Some(cabac)).unwrap(),
        &[],
    );
}

#[test]
fn parameter_sets_must_be_what_kairo_encodes() {
    let e = engine();
    let params = ["video.params"];
    fails_only(&with_vui(|v| v.colour = Some((5, 5, 5))), &e, &params);
    fails_only(&with_vui(|v| v.aspect = Some((255, 4, 3))), &e, &params);
    fails_only(&with_vui(|v| v.reorder = Some(0)), &e, &[]);
    fails_only(&Spec::with(|s| s.sps.profile = 77), &e, &params);
    fails_only(&Spec::with(|s| s.cabac = false), &e, &params);
    fails_only(&Spec::with(|s| s.pasp = Some((4, 3))), &e, &params);
    fails_only(&Spec::with(|s| s.sps.width = 352), &e, &params);
    fails_only(
        &Spec::with(|s| s.tkhd_size = Some((321 << 16, 180 << 16))),
        &e,
        &params,
    );
    fails_only(&Spec::with(|s| s.nal_overrun_at = Some(5)), &e, &params);
    fails_only(&Spec::with(|s| s.sps.bit_depth = 10), &e, &params);
    // A VUI without colour fails the testbed profiles and warns elsewhere (h264_mf, CB:478).
    let no_colour = with_vui(|v| v.colour = None);
    let warned = fails_only(&no_colour, &e, &[]);
    assert_eq!(status(&warned, "video.params"), Status::Warn);
    assert_eq!(failing(&report_for_test(&no_colour)), ["video.params"]);
    // A level below Table A-1's need warns.
    let low = fails_only(&Spec::with(|s| s.sps.level = 13), &e, &[]);
    assert_eq!(status(&low, "video.params"), Status::Warn);
    // x264 SEI: a GPL encoder. Fails the test profile, warns elsewhere.
    let x264 = Spec::with(|s| s.x264_at = Some(0));
    assert_eq!(
        status(&fails_only(&x264, &e, &[]), "video.params"),
        Status::Warn
    );
    assert_eq!(failing(&report_for_test(&x264)), ["video.params"]);
    // An in-band SPS that differs in bytes only warns; one that changes the stream fails twice.
    let timing = SpsSpec {
        vui: Some(VuiSpec {
            timing: Some((1, 120, true)),
            ..VuiSpec::default()
        }),
        ..SpsSpec::default()
    };
    let bytes_only = Spec::with(|s| s.inband_sps_at = Some((60, crate::testkit::sps(&timing))));
    assert_eq!(
        status(&fails_only(&bytes_only, &e, &[]), "video.params"),
        Status::Warn
    );
    let bigger = SpsSpec {
        width: 640,
        height: 360,
        ..SpsSpec::default()
    };
    fails_only(
        &Spec::with(|s| s.inband_sps_at = Some((60, crate::testkit::sps(&bigger)))),
        &e,
        &["video.params", "video.single_session"],
    );
}

#[test]
fn every_frame_after_the_first_lasts_one_tick() {
    let e = engine();
    // A double tick at sample 100, with the audio still ending with the video.
    fails_only(
        &Spec::with(|s| {
            s.deltas[99] = 512;
            s.audio = audio(96);
        }),
        &e,
        &["video.cfr"],
    );
    let last_zero = fails_only(
        &Spec::with(|s| {
            s.deltas[119] = 0;
            s.audio = audio(94);
        }),
        &e,
        &["video.cfr"],
    );
    assert!(
        last_zero
            .checks
            .iter()
            .any(|f| f.detail.contains("samples [120]"))
    );
    // A timescale that 60 fps doesn't divide.
    let odd = report(
        &Spec::with(|s| {
            s.timescale = 1_000;
            s.deltas = vec![17; 120];
        }),
        &e,
    );
    assert_eq!(status(&odd, "video.cfr"), Status::Fail);
    assert!(
        odd.checks[5].detail.contains("not a multiple of 60 fps"),
        "{}",
        odd.to_text()
    );
}

#[test]
fn the_held_first_frame_matches_an_audio_lead_of_1024_to_2047_samples() {
    let with_lead = |ticks: u32, packets: usize| {
        Spec::with(|s| {
            s.deltas[0] = 256 + ticks;
            s.audio = audio(packets);
        })
    };
    let e = engine();
    // 328 ticks at T = 15,360 is ⌈1,024 × T / 48,000⌉; 656 is ⌈2,047 × T / 48,000⌉.
    fails_only(&with_lead(328, 95), &e, &[]);
    fails_only(&with_lead(656, 96), &e, &[]);
    fails_only(&with_lead(657, 96), &e, &["av.start_alignment"]);
    // Under one AAC frame: a truncated start. Fails recordings, warns for share exports.
    fails_only(&with_lead(327, 95), &e, &["av.start_alignment"]);
    fails_only(&with_lead(0, 94), &e, &["av.start_alignment"]);
    let one = [("Mix", 2, true)];
    let share = report(
        &Spec::with(|s| {
            s.deltas[0] = 256;
            s.audio = vec![AudioSpec::new("Mix", 2, true, 94)];
            s.ilst = vec![];
        }),
        &plan(Profile::Share, 2, &one),
    );
    assert_eq!(
        status(&share, "av.start_alignment"),
        Status::Warn,
        "{}",
        share.to_text()
    );
    // The first audio packet must be a whole frame (b09's 1,375).
    fails_only(
        &Spec::with(|s| s.audio[0].deltas[0] = 1_375),
        &e,
        &["av.start_alignment", "audio.format"],
    );
    // Video alone starts at once.
    let none: [(&str, u16, bool); 0] = [];
    let video_only = plan(Profile::Engine, 2, &none);
    fails_only(&with_lead(0, 0).tap(|s| s.audio.clear()), &video_only, &[]);
    fails_only(
        &with_lead(328, 0).tap(|s| s.audio.clear()),
        &video_only,
        &["av.start_alignment"],
    );
}

trait Tap {
    fn tap(self, change: impl FnOnce(&mut Self)) -> Self;
}

impl Tap for Spec {
    fn tap(mut self, change: impl FnOnce(&mut Self)) -> Self {
        change(&mut self);
        self
    }
}

#[test]
fn nothing_is_reordered() {
    let e = engine();
    let no_reorder = ["video.no_reorder"];
    fails_only(
        &Spec::with(|s| s.ctts = Some(vec![(120, 512)])),
        &e,
        &no_reorder,
    );
    fails_only(&Spec::with(|s| s.pics[5] = Pic::B), &e, &no_reorder);
    fails_only(&with_vui(|v| v.reorder = Some(1)), &e, &no_reorder);
    let regress = fails_only(
        &Spec::with(|s| s.poc_regress_at = Some(10)),
        &e,
        &no_reorder,
    );
    assert!(
        regress.to_text().contains("falls back at sample 11"),
        "{}",
        regress.to_text()
    );
    // The POC wraps every 32 frames here (6 bits, steps of 2); wrapping isn't regressing.
    fails_only(&Spec::with(|s| s.poc_step = 2), &e, &[]);
}

#[test]
fn the_first_frame_is_an_idr() {
    let e = engine();
    fails_only(
        &Spec::with(|s| s.pics[0] = Pic::P),
        &e,
        &["video.idr_start", "video.gop"],
    );
    fails_only(
        &Spec::with(|s| {
            s.pics[0] = Pic::I;
            s.stss = Some(vec![1, 61]);
        }),
        &e,
        &["video.idr_start", "video.gop"],
    );
    fails_only(&Spec::with(|s| s.first_mb = 5), &e, &["video.idr_start"]);
    fails_only(&Spec::with(|s| s.slice_pps = 1), &e, &["video.idr_start"]);
}

#[test]
fn the_gop_is_closed_and_one_second_long() {
    let e = engine();
    let gop = ["video.gop"];
    fails_only(&Spec::with(|s| s.pics[60] = Pic::P), &e, &gop);
    let open = fails_only(
        &Spec::with(|s| {
            s.pics[60] = Pic::I;
            s.stss = Some(vec![1, 61]);
        }),
        &e,
        &gop,
    );
    assert!(open.to_text().contains("aren't IDR"), "{}", open.to_text());
    fails_only(&Spec::with(|s| s.stss = Some(vec![1])), &e, &gop);
    fails_only(&Spec::with(|s| s.recovery_at = Some(30)), &e, &gop);
    // A recovery point on an IDR is harmless.
    fails_only(&Spec::with(|s| s.recovery_at = Some(60)), &e, &[]);
}

#[test]
fn tracks_follow_the_layout() {
    let e = engine();
    let layout = ["tracks.layout"];
    fails_only(&Spec::with(|s| s.audio.swap(0, 1)), &e, &layout);
    fails_only(&Spec::with(|s| s.audio[1].enabled = true), &e, &layout);
    fails_only(
        &Spec::with(|s| {
            for (k, a) in s.audio.iter_mut().enumerate() {
                a.group = k as i16 + 1;
            }
        }),
        &e,
        &layout,
    );
    let unnamed = fails_only(&Spec::with(|s| s.audio[2].udta_name = None), &e, &layout);
    assert!(unnamed.to_text().contains("has no udta/name title"));
    fails_only(
        &Spec::with(|s| {
            s.audio[1].channels = 1;
            s.audio[1].entry_channels = 1;
            s.audio[1].asc = vec![0x11, 0x88, 0x56, 0xE5, 0x00];
        }),
        &e,
        &layout,
    );
    fails_only(
        &Spec::with(|s| s.extra_handler = Some(*b"text")),
        &e,
        &layout,
    );
    fails_only(
        &Spec::with(|s| s.track_ids = Some(vec![1, 3, 2, 4])),
        &e,
        &layout,
    );
    fails_only(&Spec::with(|s| s.video_enabled = false), &e, &layout);
    fails_only(&Spec::with(|s| s.audio.truncate(2)), &e, &layout);
}

#[test]
fn engine_layouts_without_an_expect_file() {
    // Mix, Game and Mic, then Voice chat and Everything else if present (PLAN:185–191).
    let engine = Plan::new(Profile::Engine, false, None).unwrap();
    let thirty = |change: fn(&mut Spec)| {
        let mut spec = Spec::good();
        spec.pics = (0..1_800)
            .map(|i| if i % 60 == 0 { Pic::Idr } else { Pic::P })
            .collect();
        spec.deltas = vec![256; 1_800];
        spec.deltas[0] = 584;
        spec.audio = audio(1_408);
        change(&mut spec);
        spec
    };
    fails_only(&thirty(|_| {}), &engine, &[]);
    fails_only(
        &thirty(|s| s.audio.push(AudioSpec::new("Voice chat", 2, false, 1_408))),
        &engine,
        &[],
    );
    fails_only(
        &thirty(|s| {
            s.audio
                .push(AudioSpec::new("Everything else", 2, false, 1_408))
        }),
        &engine,
        &[],
    );
    fails_only(
        &thirty(|s| {
            s.audio.push(AudioSpec::new("Voice chat", 2, false, 1_408));
            s.audio
                .push(AudioSpec::new("Everything else", 2, false, 1_408));
        }),
        &engine,
        &[],
    );
    fails_only(
        &thirty(|s| s.audio.push(AudioSpec::new("Music", 2, false, 1_408))),
        &engine,
        &["tracks.layout"],
    );
}

#[test]
fn audio_is_aac_lc_at_48_khz() {
    let e = engine();
    let format = ["audio.format"];
    fails_only(
        &Spec::with(|s| s.audio[0].asc = vec![0x2B, 0x11, 0x88, 0x00]),
        &e,
        &format,
    );
    fails_only(
        &Spec::with(|s| s.audio[0].asc = vec![0x11, 0x90, 0x56, 0xE5, 0x80]),
        &e,
        &format,
    );
    fails_only(
        &Spec::with(|s| s.audio[0].asc = vec![0x11, 0x94]),
        &e,
        &format,
    );
    fails_only(&Spec::with(|s| s.audio[0].oti = 0x6B), &e, &format);
    fails_only(
        &Spec::with(|s| s.audio[2].asc = vec![0x11, 0x90, 0x56, 0xE5, 0x00]),
        &e,
        &format,
    );
    fails_only(&Spec::with(|s| s.audio[0].deltas[40] = 1_000), &e, &format);
    fails_only(
        &Spec::with(|s| {
            let a = &mut s.audio[0];
            a.rate = 44_100;
            a.timescale = 44_100;
            a.asc = vec![0x12, 0x10];
            a.deltas = vec![1024; 88];
        }),
        &e,
        &format,
    );
    // A short last packet is fine (the FFmpeg CLI ends that way).
    fails_only(&Spec::with(|s| s.audio[1].deltas.push(640)), &e, &[]);
}

#[test]
fn durations_agree_and_fit_the_profile() {
    let e = engine();
    let duration = ["duration"];
    fails_only(&Spec::with(|s| s.mdhd_extra = 5), &e, &duration);
    fails_only(&Spec::with(|s| s.tkhd_extra = 3), &e, &duration);
    fails_only(&Spec::with(|s| s.mvhd_extra = 5), &e, &duration);
    fails_only(
        &Spec::with(|s| s.audio[0].deltas.truncate(90)),
        &e,
        &duration,
    );
    let frames = |n: usize, packets: usize| {
        Spec::with(|s| {
            s.pics.truncate(n);
            s.deltas.truncate(n);
            s.audio = audio(packets);
        })
    };
    // test: 120 frames ± 1.
    let test = plan(Profile::Test, 2, &ROLES);
    assert_eq!(
        failing(&report(&frames(119, 94), &test)),
        Vec::<&str>::new()
    );
    assert_eq!(failing(&report(&frames(118, 94), &test)), duration);
    // engine: 120 frames ± 1 GOP, and short of 120 warns.
    let short = fails_only(&frames(61, 49), &e, &[]);
    assert_eq!(status(&short, "duration"), Status::Warn);
    fails_only(&frames(59, 48), &e, &duration);
}

#[test]
fn one_encoder_session_per_file() {
    let two = |s: &mut Spec| {
        s.extra_entry = true;
        s.last_chunk_entry = 2;
    };
    fails_only(&Spec::with(two), &engine(), &["video.single_session"]);
    // Either half alone: a second entry no chunk uses, or chunks naming an entry that isn't
    // there (which C0 doesn't check).
    fails_only(
        &Spec::with(|s| s.extra_entry = true),
        &engine(),
        &["video.single_session"],
    );
    fails_only(
        &Spec::with(|s| s.last_chunk_entry = 2),
        &engine(),
        &["video.single_session"],
    );
}

#[test]
fn bitrate_warns_above_50_mbps_and_reels_caps_the_average() {
    let e = engine();
    // 60 samples of 100,000 bytes a second is 48 Mbps; of 110,000, 52.8.
    fails_only(&Spec::with(|s| s.video_sample_bytes = 100_000), &e, &[]);
    let high = fails_only(&Spec::with(|s| s.video_sample_bytes = 110_000), &e, &[]);
    assert_eq!(status(&high, "video.bitrate"), Status::Warn);
    assert!(
        high.to_text().contains("peak 52.8 Mbps"),
        "{}",
        high.to_text()
    );
    let one = [("Mix", 2, true)];
    let reels = report(
        &Spec::with(|s| {
            s.video_sample_bytes = 60_000;
            s.audio = vec![AudioSpec::new("Mix", 2, true, 95)];
        }),
        &plan(Profile::Reels, 2, &one),
    );
    assert_eq!(
        status(&reels, "video.bitrate"),
        Status::Fail,
        "{}",
        reels.to_text()
    );
}

// ---------------------------------------------------------------------------------------------
// Exports, the M3 gate and the original
// ---------------------------------------------------------------------------------------------

fn share_spec() -> Spec {
    Spec::with(|s| s.audio = vec![AudioSpec::new("Mix", 2, true, 95)])
}

#[test]
fn share_exports_have_one_stereo_track_and_few_tags() {
    let one = [("Mix", 2, true)];
    let share = plan(Profile::Share, 2, &one);
    let ok = fails_only(&share_spec(), &share, &[]);
    assert_eq!(status(&ok, "export.shape"), Status::Pass);
    assert_eq!(status(&ok, "export.tags"), Status::Pass);
    assert_eq!(status(&ok, "export.size"), Status::Skip);
    fails_only(
        &share_spec().tap(|s| s.ilst.push(*b"desc")),
        &share,
        &["export.tags"],
    );
    fails_only(&Spec::good(), &share, &["tracks.layout"]);
    // Titles don't matter in a share copy (`-map_metadata -1` resets them).
    fails_only(
        &share_spec().tap(|s| s.audio[0].udta_name = None),
        &share,
        &[],
    );
    // Discord's ladder has no 180p; WhatsApp wants at most 30 fps; Reels wants 1080×1920.
    fails_only(
        &share_spec(),
        &plan(Profile::Discord, 2, &one),
        &["export.shape"],
    );
    fails_only(
        &share_spec(),
        &plan(Profile::Whatsapp, 2, &one),
        &["export.shape"],
    );
    fails_only(
        &share_spec(),
        &plan(Profile::Reels, 2, &one),
        &["export.shape"],
    );
}

#[test]
fn the_original_reports_whether_it_may_be_handed_over() {
    let original = plan(Profile::Original, 2, &ROLES);
    let good = report(&Spec::good(), &original);
    assert_eq!(good.summary.shareable_original, Some(true));
    assert_eq!(status(&good, "duration"), Status::Skip);
    let five = [
        ("Mix", 2, true),
        ("Game", 2, false),
        ("Mic", 1, false),
        ("Voice chat", 2, false),
    ];
    let voice = report(
        &Spec::with(|s| s.audio.push(AudioSpec::new("Voice chat", 2, false, 95))),
        &plan(Profile::Original, 2, &five),
    );
    assert_eq!(voice.verdict, Verdict::Pass, "{}", voice.to_text());
    assert_eq!(voice.summary.shareable_original, Some(false));
}

#[test]
fn the_m3_gate_decides_on_its_own_list() {
    let gate = Plan::new(Profile::Engine, true, None).unwrap();
    let thirty = |change: fn(&mut Spec)| {
        let mut spec = Spec::good();
        spec.pics = (0..1_800)
            .map(|i| if i % 60 == 0 { Pic::Idr } else { Pic::P })
            .collect();
        spec.deltas = vec![256; 1_800];
        spec.deltas[0] = 584;
        spec.audio = audio(1_408);
        change(&mut spec);
        spec
    };
    // Every structure check on the gate's list passes, but A/V is on the list too and this
    // build can't decode, so the gate can't be judged: error, never a pass on structure alone.
    let good = report(&thirty(|_| {}), &gate);
    assert!(failing(&good).is_empty(), "{}", good.to_text());
    assert_eq!(status(&good, "av.offset"), Status::Error);
    assert_eq!(good.verdict, Verdict::Error, "{}", good.to_text());
    assert_eq!(good.gate, Some("m3"));
    // A GOP fault is reported but can't fail the gate.
    let gop = report(&thirty(|s| s.pics[60] = Pic::P), &gate);
    assert_eq!(status(&gop, "video.gop"), Status::Warn);
    assert_eq!(gop.verdict, Verdict::Error);
    // A B-frame can.
    assert_eq!(
        report(&thirty(|s| s.pics[7] = Pic::B), &gate).verdict,
        Verdict::Fail
    );
    // So can the length: a 2 s clip against 30 s ± 1 GOP.
    assert_eq!(failing(&report(&Spec::good(), &gate)), ["duration"]);
}

// ---------------------------------------------------------------------------------------------
// Report and command line
// ---------------------------------------------------------------------------------------------

#[test]
fn the_report_is_versioned_json_naming_the_file_only() {
    let report = report(&Spec::good(), &engine());
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["kairo_verify"], 1);
    assert_eq!(json["profile"], "engine");
    assert_eq!(json["verdict"], "pass");
    assert_eq!(json["file"]["name"], "clip.mp4");
    assert_eq!(json["decoder"]["kind"], "none");
    let ids: Vec<&str> = json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(&ids[..15], STRUCTURE_CHECKS);
    assert_eq!(ids.len(), 15 + 3 + 19);
    assert!(json.get("gate").is_none());
    assert_eq!(
        json["summary"]["video"],
        "h264 High 320x180 CFR 60, 120 frames, peak 0.0 Mbps"
    );
    assert_eq!(
        json["summary"]["audio"],
        "Mix mp4a 2ch 48 kHz, Game mp4a 2ch 48 kHz, Mic mp4a 1ch 48 kHz"
    );
}

mod cli {
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A fresh folder under the temp folder, removed when dropped.
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("kairo-verify-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(args: &[&OsStr]) -> (u8, String, String) {
        let args: Vec<OsString> = args.iter().map(|a| a.to_os_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = crate::cli::run(&args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    #[test]
    fn mutate_writes_a_new_file_and_never_replaces_one() {
        let dir = Dir::new();
        let folder = dir.0.to_string_lossy().into_owned();
        let good = dir.file("good.mp4", &Spec::good().build());
        let out = dir.0.join("nocolr.mp4");
        let mutate = |change: &str, input: &PathBuf, output: &PathBuf| {
            let (code, said, err) = run(&[
                os("mutate"),
                os(change),
                input.as_os_str(),
                output.as_os_str(),
            ]);
            // No message names a folder (CLAUDE.md), and no partial file is left behind.
            assert!(
                !said.contains(&folder) && !err.contains(&folder),
                "{said}{err}"
            );
            let mut partial = output.as_os_str().to_owned();
            partial.push(".partial");
            assert!(!PathBuf::from(partial).exists());
            (code, said, err)
        };
        let (code, said, err) = mutate("remove-colr", &good, &out);
        assert_eq!((code, err.as_str()), (0, ""), "{said}");
        assert_eq!(said, "nocolr.mp4: good.mp4 with remove-colr\n");
        let written = std::fs::read(&out).unwrap();
        assert_eq!(written, Spec::with(|s| s.colr = Vec::new()).build());
        // An existing output is never replaced, the input included.
        for target in [&out, &good] {
            let before = std::fs::read(target).unwrap();
            let (code, _, err) = mutate("add-elst", &good, target);
            assert_eq!(code, 4, "{err}");
            assert!(err.contains("already exists"), "{err}");
            assert_eq!(std::fs::read(target).unwrap(), before);
        }
        // A change that doesn't apply exits 1 and leaves no file behind.
        let again = dir.0.join("again.mp4");
        let (code, _, err) = mutate("remove-colr", &out, &again);
        assert_eq!(code, 1, "{err}");
        assert!(
            err.starts_with("kairo-verify: nocolr.mp4: the change doesn't apply"),
            "{err}"
        );
        assert!(!again.exists());
        // An unreadable input exits 3, an output that names no file 2.
        let (code, _, err) = mutate("add-elst", &dir.0.join("missing.mp4"), &again);
        assert_eq!(code, 3, "{err}");
        assert!(err.starts_with("kairo-verify: missing.mp4: "), "{err}");
        assert!(!again.exists());
        let (code, _, err) = mutate("add-elst", &good, &dir.0.join(".."));
        assert_eq!(code, 2, "{err}");
    }

    #[test]
    fn check_writes_reports_and_exits_by_verdict() {
        let dir = Dir::new();
        let good = dir.file("good.mp4", &Spec::good().build());
        let bad = dir.file("bad.mp4", &Spec::with(|s| s.faststart = false).build());
        let expect = dir.file("expect.json", expect_text(2, &ROLES).as_bytes());
        let json = dir.0.join("report.json");
        // Structure only, on every system: Windows would otherwise decode with the linked FFmpeg,
        // and the builder's clips hold no real pictures.
        let engine = [
            os("--profile"),
            os("engine"),
            os("--expect"),
            expect.as_os_str(),
            os("--decoder"),
            os("none"),
        ];
        let (code, out, err) = run(&[
            &[os("check"), good.as_os_str()][..],
            &engine,
            &[os("--json"), json.as_os_str()],
        ]
        .concat());
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        assert!(out.starts_with("PASS  good.mp4 (profile engine)"), "{out}");
        let written = std::fs::read_to_string(&json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed["verdict"], "pass");
        // No folder appears anywhere.
        let folder = dir.0.to_string_lossy().into_owned();
        assert!(!out.contains(&folder) && !written.contains(&folder));
        let (code, out, _) = run(&[&[os("check"), bad.as_os_str()][..], &engine].concat());
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("FAILED    container.faststart"), "{out}");
        // JSON on stdout replaces the summary.
        let (code, out, _) = run(&[
            &[os("check"), good.as_os_str()][..],
            &engine,
            &[os("--json"), os("-")],
        ]
        .concat());
        assert_eq!(code, 0, "{out}");
        assert!(out.starts_with("{\"kairo_verify\":1,"), "{out}");
        // The test profile needs content checks, which can't run without a decoder: exit 3.
        let (code, out, _) = run(&[
            os("check"),
            good.as_os_str(),
            os("--profile"),
            os("test"),
            os("--expect"),
            expect.as_os_str(),
            os("--decoder"),
            os("none"),
        ]);
        assert_eq!(code, 3, "{out}");
        assert!(out.contains("NOT RUN"), "{out}");
    }

    #[test]
    fn usage_and_unreadable_inputs() {
        let dir = Dir::new();
        let text = dir.file("notes.mp4", b"not a clip");
        let missing = dir.0.join("missing.mp4");
        assert_eq!(run(&[]).0, 2);
        assert_eq!(run(&[os("check"), text.as_os_str()]).0, 2);
        assert_eq!(
            run(&[os("check"), text.as_os_str(), os("--profile"), os("test")]).0,
            2
        );
        let (code, out, _) = run(&[os("check"), text.as_os_str(), os("--profile"), os("engine")]);
        assert_eq!(code, 3);
        assert!(out.contains("not an MP4"), "{out}");
        let (code, _, err) = run(&[
            os("check"),
            missing.as_os_str(),
            os("--profile"),
            os("engine"),
        ]);
        assert_eq!(code, 3);
        assert!(err.starts_with("kairo-verify: missing.mp4: "), "{err}");
        assert!(!err.contains(&*dir.0.to_string_lossy()), "{err}");
        let bad_expect = dir.file("bad.json", b"{}");
        let (code, _, err) = run(&[
            os("check"),
            text.as_os_str(),
            os("--profile"),
            os("engine"),
            os("--expect"),
            bad_expect.as_os_str(),
        ]);
        assert_eq!(code, 2);
        assert!(
            err.starts_with("kairo-verify: bad.json: expect file:"),
            "{err}"
        );
        assert_eq!(run(&[os("--version")]).0, 0);
        // A report that can't be written is an internal error.
        let good = dir.file("good.mp4", &Spec::good().build());
        let nowhere = dir.0.join("no-such-folder").join("r.json");
        let (code, _, err) = run(&[
            os("check"),
            good.as_os_str(),
            os("--profile"),
            os("engine"),
            os("--json"),
            nowhere.as_os_str(),
        ]);
        assert_eq!(code, 4, "{err}");
    }

    #[test]
    fn usage_errors_never_name_a_folder() {
        for args in [
            [
                "check",
                "a.mp4",
                "/Users/someone/Videos/second.mp4",
                "--profile",
                "engine",
            ]
            .as_slice(),
            &["/Users/someone/Videos/clip.mp4", "--profile", "engine"],
            &[
                "check",
                "a.mp4",
                "--profile",
                "engine",
                "--C:\\Users\\someone\\x.mp4",
            ],
            &["check", "a.mp4", "--profile", "/Users/someone/p"],
            &[
                "check",
                "a.mp4",
                "--profile",
                "engine",
                "--json=/Users/someone/r.json",
            ],
        ] {
            let words: Vec<&OsStr> = args.iter().map(|a| os(a)).collect();
            let (code, _, err) = run(&words);
            assert_eq!(code, 2, "{args:?}");
            assert!(!err.contains("someone"), "{err}");
        }
        let (code, _, err) = run(&[
            os("check"),
            os("a.mp4"),
            os("--profile"),
            os("engine"),
            os("--testbed-log"),
            os("t.log"),
        ]);
        assert_eq!(code, 2);
        assert!(
            err.contains("--testbed-log comes with source.stability and the testbed and tone logs (research 16's PR 8)"),
            "{err}"
        );
    }

    #[test]
    fn unreadable_and_unnamed_clips_exit_3() {
        let dir = Dir::new();
        let missing = dir.0.join("missing.mp4");
        assert_eq!(run(&[os("boxes"), missing.as_os_str()]).0, 3);
        // A file name that isn't UTF-8 can't go in a report; nothing is opened.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let odd = dir.0.join(OsStr::from_bytes(b"\xff.mp4"));
            assert_eq!(
                run(&[os("check"), odd.as_os_str(), os("--profile"), os("engine")]).0,
                3
            );
            assert_eq!(run(&[os("boxes"), odd.as_os_str()]).0, 3);
        }
    }

    /// Runs `synth` with these options into `out`, the expect file at `expect`: (code, stderr).
    fn synth_to(
        words: &[&str],
        expect: &std::path::Path,
        out: &mut dyn std::io::Write,
    ) -> (u8, String) {
        let args: Vec<OsString> = std::iter::once("synth")
            .chain(words.iter().copied())
            .chain(["--expect"])
            .map(OsString::from)
            .chain([expect.as_os_str().to_os_string()])
            .collect();
        let mut err = Vec::new();
        let code = crate::cli::run_with(&args, out, &mut err, false);
        (code, String::from_utf8(err).unwrap())
    }

    #[test]
    fn synth_stops_cleanly_when_its_reader_goes_away() {
        // An encoder that exits early: the pipe breaks after about 1 MB (review of PR 4c).
        struct Closes(usize);
        impl std::io::Write for Closes {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.0 >= 1 << 20 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "closed",
                    ));
                }
                self.0 += buf.len();
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let dir = Dir::new();
        let expect = dir.0.join("e.json");
        let (code, err) = synth_to(
            &["--preset", "640x360@60", "--seconds", "1"],
            &expect,
            &mut Closes(0),
        );
        assert_eq!(code, 4, "{err}");
        assert!(err.contains("writing the frames failed"), "{err}");
        let mut partial = expect.as_os_str().to_owned();
        partial.push(".partial");
        assert!(!expect.exists() && !PathBuf::from(partial).exists());
    }

    #[test]
    fn synth_refuses_a_folder_as_its_expect_file() {
        let dir = Dir::new();
        let folder = dir.0.join("Alice");
        std::fs::create_dir(&folder).unwrap();
        let mut trailing = folder.as_os_str().to_owned();
        trailing.push("/");
        for path in [
            folder.clone(),
            PathBuf::from(trailing),
            dir.0.join("Bob").join(".."),
        ] {
            let (code, err) = synth_to(
                &["--preset", "64x36@60", "--seconds", "1"],
                &path,
                &mut Vec::new(),
            );
            assert_eq!(code, 2, "{path:?}: {err}");
            assert!(!err.contains("Alice") && !err.contains("Bob"), "{err}");
        }
        // A file in a folder that doesn't exist: refused before any frame.
        let mut out = Vec::new();
        let (code, err) = synth_to(
            &["--preset", "64x36@60", "--seconds", "1"],
            &dir.0.join("nowhere").join("e.json"),
            &mut out,
        );
        assert_eq!((code, out.len()), (4, 0), "{err}");
    }

    #[test]
    fn synth_writes_frames_then_the_expect_file_and_never_replaces_one() {
        let dir = Dir::new();
        let folder = dir.0.to_string_lossy().into_owned();
        let expect = dir.0.join("e.json");
        let synth = |terminal: bool| {
            let words = [
                "synth",
                "--preset",
                "64x36@60",
                "--seconds",
                "1",
                "--codec",
                "ipcm",
                "--gop",
                "30",
                "--defect",
                "dup@20",
                "--expect",
            ];
            let args: Vec<OsString> = words
                .iter()
                .map(OsString::from)
                .chain([expect.clone().into_os_string()])
                .collect();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = crate::cli::run_with(&args, &mut out, &mut err, terminal);
            let err = String::from_utf8(err).unwrap();
            // No message names a folder (CLAUDE.md), and no partial file is left behind.
            assert!(!err.contains(&folder), "{err}");
            let mut partial = expect.as_os_str().to_owned();
            partial.push(".partial");
            assert!(!PathBuf::from(partial).exists());
            (code, out, err)
        };
        // A terminal gets no frames, and no file is made.
        let (code, out, err) = synth(true);
        assert_eq!((code, out.len()), (2, 0), "{err}");
        assert!(!expect.exists());
        let (code, out, err) = synth(false);
        assert_eq!(code, 0, "{err}");
        assert_eq!(
            err,
            "kairo-verify: synth: 60 frames, 64x36 at 60 fps from a 180 Hz testbed, I_PCM H.264 \
             (GOP 30), defect dup@20; the expect file is e.json\n"
        );
        // An SPS and a PPS before each of the two IDRs (frames 0 and 30), and 58 P slices.
        let mut kinds = [0; 32];
        for (i, window) in out.windows(5).enumerate() {
            if window[..4] == [0, 0, 0, 1] {
                kinds[usize::from(out[i + 4] & 0x1F)] += 1;
            }
        }
        assert_eq!((kinds[7], kinds[8], kinds[5], kinds[1]), (2, 2, 2, 58));
        let written = Expect::parse(&std::fs::read_to_string(&expect).unwrap()).unwrap();
        assert_eq!(written.cabac, Some(false));
        assert_eq!(
            (written.fps, written.seconds, written.size),
            (60, 1, Some([64, 36]))
        );
        // Never over an existing file.
        let before = std::fs::read(&expect).unwrap();
        let (code, out, err) = synth(false);
        assert_eq!((code, out.len()), (4, 0), "{err}");
        assert!(err.contains("e.json: already exists"), "{err}");
        assert_eq!(std::fs::read(&expect).unwrap(), before);
    }

    /// Runs `synth` for a 2 s 64×36 clip with this defect, its WAVs into `wavs` and its expect
    /// file at `expect`, writing the frames to `out`: (code, stderr).
    fn synth_wavs(
        defect: &str,
        wavs: &std::path::Path,
        expect: &std::path::Path,
        out: &mut dyn std::io::Write,
    ) -> (u8, String) {
        let args: Vec<OsString> = [
            "synth",
            "--preset",
            "64x36@60",
            "--seconds",
            "2",
            "--defect",
            defect,
            "--wav-dir",
        ]
        .iter()
        .map(OsString::from)
        .chain([wavs.as_os_str().to_os_string(), OsString::from("--expect")])
        .chain([expect.as_os_str().to_os_string()])
        .collect();
        let mut err = Vec::new();
        let code = crate::cli::run_with(&args, out, &mut err, false);
        (code, String::from_utf8(err).unwrap())
    }

    /// The names in `dir`, sorted.
    fn names_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn synth_writes_the_wavs_after_the_frames_and_never_replaces_one() {
        let dir = Dir::new();
        let folder = dir.0.to_string_lossy().into_owned();
        let expect = dir.0.join("e.json");
        let frames = 120 * 64 * 36 * 3 / 2;
        let mut out = Vec::new();
        let (code, err) = synth_wavs("cue", &dir.0, &expect, &mut out);
        assert_eq!(code, 0, "{err}");
        assert!(!err.contains(&folder), "{err}");
        assert_eq!(out.len(), frames);
        assert!(
            err.ends_with("defect cue; the expect file is e.json; the WAVs are mix.wav, game.wav, mic.wav and voice.wav\n"),
            "{err}"
        );
        for (file, channels) in [
            ("mix.wav", 2),
            ("game.wav", 2),
            ("mic.wav", 1),
            ("voice.wav", 2),
        ] {
            let wav = crate::wav::read(&std::fs::read(dir.0.join(file)).unwrap()).unwrap();
            assert_eq!((wav.rate, wav.channels), (48_000, channels), "{file}");
            assert_eq!(wav.samples.len(), 96_000 * usize::from(channels), "{file}");
        }
        let written = Expect::parse(&std::fs::read_to_string(&expect).unwrap()).unwrap();
        assert_eq!(written.tracks.len(), 4);
        assert_eq!(
            names_in(&dir.0),
            ["e.json", "game.wav", "mic.wav", "mix.wav", "voice.wav"]
        );
        // A WAV that exists already stops the run before any frame, and nothing is written.
        std::fs::remove_file(&expect).unwrap();
        let mix = std::fs::read(dir.0.join("mix.wav")).unwrap();
        let mut out = Vec::new();
        let (code, err) = synth_wavs("leak", &dir.0, &expect, &mut out);
        assert_eq!((code, out.len()), (4, 0), "{err}");
        assert!(err.contains("mix.wav: already exists"), "{err}");
        assert!(!expect.exists());
        assert_eq!(std::fs::read(dir.0.join("mix.wav")).unwrap(), mix);
        // A WAV folder that doesn't exist, likewise, without naming it.
        let (code, err) = synth_wavs("leak", &dir.0.join("Carol"), &expect, &mut out);
        assert_eq!((code, out.len()), (4, 0), "{err}");
        assert!(!err.contains("Carol"), "{err}");
    }

    #[test]
    fn synth_reserves_its_files_before_the_frames_and_cleans_up_after_them() {
        // PR 5b's review: a leftover partial file, or an expect file named like a WAV, stops it
        // before any frame; a name taken while the frames go out removes the WAVs it made.
        let dir = Dir::new();
        let expect = dir.0.join("e.json");
        std::fs::write(dir.0.join("voice.wav.partial"), b"left over").unwrap();
        let mut out = Vec::new();
        let (code, err) = synth_wavs("noisy", &dir.0, &expect, &mut out);
        assert_eq!((code, out.len()), (4, 0), "{err}");
        assert!(err.contains("voice.wav.partial: "), "{err}");
        assert_eq!(names_in(&dir.0), ["voice.wav.partial"]);
        std::fs::remove_file(dir.0.join("voice.wav.partial")).unwrap();
        let (code, err) = synth_wavs("noisy", &dir.0, &dir.0.join("mic.wav"), &mut out);
        assert_eq!((code, out.len()), (2, 0), "{err}");
        assert!(err.contains("can't take a WAV's name"), "{err}");
        assert!(names_in(&dir.0).is_empty());
        // Another program takes voice.wav while the frames go out.
        struct Takes(std::path::PathBuf, usize);
        impl std::io::Write for Takes {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.1 == 0 {
                    std::fs::write(&self.0, b"someone else's")?;
                }
                self.1 += buf.len();
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut takes = Takes(dir.0.join("voice.wav"), 0);
        let (code, err) = synth_wavs("noisy", &dir.0, &expect, &mut takes);
        assert_eq!(code, 4, "{err}");
        assert_eq!(takes.1, 120 * 64 * 36 * 3 / 2);
        assert!(err.contains("voice.wav: "), "{err}");
        assert_eq!(names_in(&dir.0), ["voice.wav"]);
        assert_eq!(
            std::fs::read(dir.0.join("voice.wav")).unwrap(),
            b"someone else's"
        );
    }

    #[test]
    fn boxes_prints_the_tree_and_the_tracks() {
        let dir = Dir::new();
        let good = dir.file("good.mp4", &Spec::good().build());
        let (code, out, _) = run(&[os("boxes"), good.as_os_str()]);
        assert_eq!(code, 0);
        assert!(out.starts_with("good.mp4: "), "{out}");
        for want in [
            "moov @32 ",
            "  trak @",
            "track 1 vide \"Video\"",
            "track 2 soun \"Mix\" name \"Mix\"",
            "ilst keys: \\xa9too",
            "colr nclx 1/1/1/0",
        ] {
            assert!(out.contains(want), "{want} in\n{out}");
        }
        assert!(!out.contains("problem:"));
    }
}

// ---------------------------------------------------------------------------------------------
// H.264 headers
// ---------------------------------------------------------------------------------------------

mod headers {
    use crate::h264::{self, ParamSets, SliceError};
    use crate::testkit::{self, BitWriter, SpsSpec, VuiSpec};

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn golden_videotoolbox_sps() {
        // research 16 §4: a 720p VideoToolbox SPS made without setparams (VUI 2/2/1), as
        // FFmpeg's trace_headers decoded it.
        let sps = h264::parse_sps(&hex("2764001fac56805005ba6a04040204")).unwrap();
        assert_eq!(
            (sps.profile_idc, sps.constraint_flags, sps.level_idc),
            (100, 0, 31)
        );
        assert_eq!(
            (sps.sps_id, sps.chroma_format_idc, sps.bit_depth_luma),
            (0, 1, 8)
        );
        assert_eq!(
            (sps.log2_max_frame_num, sps.poc_type, sps.log2_max_poc_lsb),
            (5, 0, Some(6))
        );
        assert_eq!((sps.max_num_ref_frames, sps.frame_mbs_only), (1, true));
        assert_eq!((sps.width, sps.height), (1280, 720));
        let vui = sps.vui.unwrap();
        assert_eq!(vui.aspect_ratio_idc, None);
        assert_eq!((vui.full_range, vui.colour), (Some(false), Some((2, 2, 1))));
        assert_eq!((vui.timing, vui.max_num_reorder_frames), (None, None));
        // The same encoder with setparams: 1/1/1, and its PPS: CABAC.
        let fixed = h264::parse_sps(&hex("2764001fac56805005ba6a02020204")).unwrap();
        assert_eq!(fixed.vui.unwrap().colour, Some((1, 1, 1)));
        let pps = h264::parse_pps(&hex("28ee3cb0")).unwrap();
        assert_eq!((pps.pps_id, pps.sps_id, pps.cabac), (0, 0, true));
        // Its avcC, High-profile tail included.
        let avcc = h264::parse_avcc(&hex(
            "0164001fffe1000f2764001fac56805005ba6a0202020401000428ee3cb0fdf8f800",
        ))
        .unwrap();
        assert_eq!((avcc.profile, avcc.level, avcc.length_size), (100, 31, 4));
        assert_eq!((avcc.sps.len(), avcc.pps.len()), (1, 1));
    }

    #[test]
    fn written_sps_read_back() {
        let spec = SpsSpec {
            level: 42,
            scaling_lists: true,
            width: 1_920,
            height: 1_080,
            vui: Some(VuiSpec {
                aspect: Some((255, 4, 3)),
                full_range: true,
                colour: Some((9, 16, 9)),
                timing: Some((1, 120, true)),
                nal_hrd: true,
                reorder: Some(0),
                ..VuiSpec::default()
            }),
            ..SpsSpec::default()
        };
        let sps = h264::parse_sps(&testkit::sps(&spec)).unwrap();
        assert_eq!((sps.width, sps.height, sps.level_idc), (1_920, 1_080, 42));
        let vui = sps.vui.unwrap();
        assert_eq!((vui.aspect_ratio_idc, vui.sar), (Some(255), Some((4, 3))));
        assert_eq!((vui.full_range, vui.colour), (Some(true), Some((9, 16, 9))));
        assert_eq!(vui.timing, Some((1, 120, true)));
        assert_eq!(
            (vui.max_num_reorder_frames, vui.max_dec_frame_buffering),
            (Some(0), Some(1))
        );
        // Interlaced: the height counts field pairs and crops in units of 4 rows.
        let fields = SpsSpec {
            frame_mbs_only: false,
            ..SpsSpec::default()
        };
        let sps = h264::parse_sps(&testkit::sps(&fields)).unwrap();
        assert_eq!(
            (sps.width, sps.height, sps.frame_mbs_only),
            (320, 180, false)
        );
        // A main-profile SPS has no chroma fields.
        let main = h264::parse_sps(&testkit::sps(&SpsSpec {
            profile: 77,
            ..SpsSpec::default()
        }))
        .unwrap();
        assert_eq!((main.profile_idc, main.chroma_format_idc), (77, 1));
    }

    #[test]
    fn slices_and_their_parameter_sets() {
        let mut params = ParamSets::default();
        assert!(!params.has_both());
        params.add_sps(&testkit::sps(&SpsSpec::default())).unwrap();
        params.add_pps(&testkit::pps(0, 0, true)).unwrap();
        assert!(params.has_both());
        let idr = h264::parse_slice_header(&testkit::slice(true, 7, 0, 0, 0, 10), &params).unwrap();
        assert_eq!(
            (idr.slice_type, idr.idr_pic_id, idr.poc_lsb),
            (2, Some(0), Some((10, 6)))
        );
        let p = h264::parse_slice_header(&testkit::slice(false, 5, 30, 0, 9, 18), &params).unwrap();
        assert_eq!(
            (p.first_mb, p.slice_type, p.frame_num, p.idr_pic_id),
            (30, 0, 9, None)
        );
        assert_eq!(
            h264::parse_slice_header(&testkit::slice(false, 5, 0, 1, 0, 0), &params),
            Err(SliceError::UnknownPps(1))
        );
        assert_eq!(
            h264::parse_slice_header(&[0x67, 0x80], &params),
            Err(SliceError::Malformed)
        );
        assert_eq!(
            h264::parse_slice_header(&[0x41], &params),
            Err(SliceError::Malformed)
        );
    }

    #[test]
    fn sei_messages_and_escapes() {
        let x264 = h264::sei_messages(&testkit::x264_sei());
        assert_eq!(x264.len(), 1);
        assert_eq!((x264[0].payload_type, x264[0].encoder), (5, Some("x264")));
        // Type 300 written as 0xFF 0x2D, then a 1-byte payload.
        let long = h264::sei_messages(&[0x06, 0xFF, 0x2D, 0x01, 0x00, 0x80]);
        assert_eq!(long[0].payload_type, 300);
        assert!(h264::sei_messages(&[0x67, 0x05]).is_empty());
        assert_eq!(h264::unescape(&[0, 0, 3, 1, 0, 0, 3]), [0, 0, 1, 0, 0]);
        assert_eq!(h264::unescape(&[0, 3, 0, 0, 3, 3]), [0, 3, 0, 0, 3]);
        // The writer escapes exactly where the reader unescapes.
        let nal = testkit::nal(0x06, &[0, 0, 0, 0, 1, 0, 0, 2]);
        assert_eq!(nal, [0x06, 0, 0, 3, 0, 0, 3, 1, 0, 0, 3, 2]);
        assert_eq!(h264::unescape(&nal[1..]), [0, 0, 0, 0, 1, 0, 0, 2]);
    }

    #[test]
    fn exp_golomb_limits() {
        let mut w = BitWriter::default();
        w.ue(0)
            .ue(u32::MAX - 1)
            .se(-5)
            .se(i32::MAX)
            .u(32, 0)
            .u(1, 1);
        let bytes = w.finish();
        let mut bits = h264::Bits::new(&bytes);
        assert_eq!(bits.ue(), Some(0));
        assert_eq!(bits.ue(), Some(u32::MAX - 1));
        assert_eq!(bits.se(), Some(-5));
        assert_eq!(bits.se(), Some(i32::MAX));
        // 32 zeros and a one: more than ue(v) holds.
        assert_eq!(bits.ue(), None);
        assert_eq!(h264::Bits::new(&[]).ue(), None);
        assert_eq!(h264::Bits::new(&[0xFF]).u(33), None);
    }

    #[test]
    fn damaged_headers_never_panic() {
        let sps = testkit::sps(&SpsSpec {
            scaling_lists: true,
            vui: Some(VuiSpec {
                nal_hrd: true,
                timing: Some((1, 60, true)),
                reorder: Some(0),
                ..VuiSpec::default()
            }),
            ..SpsSpec::default()
        });
        for cut in 0..sps.len() {
            let _ = h264::parse_sps(&sps[..cut]);
        }
        let mut x: u32 = 0x1234_5678;
        for _ in 0..5_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let mut damaged = sps.clone();
            let at = 1 + (x as usize % (sps.len() - 1));
            damaged[at] ^= (x >> 24) as u8 | 1;
            let _ = h264::parse_sps(&damaged);
            let _ = h264::sei_messages(&damaged);
            let _ = h264::parse_avcc(&damaged);
        }
        assert_eq!(h264::parse_avcc(&[1, 100, 0, 31, 0xFE, 0xE1]), None);
        assert_eq!(
            h264::parse_avcc(&[1, 100, 0, 31, 0xFF, 0xE1, 0, 40, 0x67]),
            None
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Rules the first review found unpinned
// ---------------------------------------------------------------------------------------------

mod pinned {
    use super::*;
    use crate::checks;
    use crate::mp4::FourCc;
    use crate::profile::{self, EXPORT_CHECKS, Mode};
    use crate::testkit::{self, SpsSpec};
    use crate::{Clip, h264};

    /// The offsets of every `kind` box type field in `bytes`.
    fn types(bytes: &[u8], kind: &[u8; 4]) -> Vec<usize> {
        bytes
            .windows(4)
            .enumerate()
            .filter(|(_, w)| w == kind)
            .map(|(i, _)| i)
            .collect()
    }

    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn get_u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn more_structure_rules() {
        let good = Spec::good().build();
        // A chunk running one byte past the end of its mdat: Mic's last size plus one.
        let mut over = good.clone();
        let stsz = types(&over, b"stsz")[3];
        let count = get_u32(&over, stsz + 12) as usize;
        let last = stsz + 16 + 4 * (count - 1);
        let size = get_u32(&over, last) + 1;
        put_u32(&mut over, last, size);
        structure_fails(&over, "outside every mdat payload");
        // stsc must start at chunk 1.
        let mut chunk2 = good.clone();
        let stsc = types(&chunk2, b"stsc")[0];
        put_u32(&mut chunk2, stsc + 12, 2);
        structure_fails(&chunk2, "starts at chunk 2");
        // Sync samples listed twice.
        structure_fails(
            &Spec::with(|s| s.stss = Some(vec![1, 61, 61])).build(),
            "stss is not strictly rising",
        );
        // An mvex box: fragmented. (moov's own udta, the last before the mdat, renamed.)
        let mut mvex = good.clone();
        let mdat = types(&mvex, b"mdat")[0];
        let udta = *types(&mvex, b"udta").iter().rfind(|&&i| i < mdat).unwrap();
        mvex[udta..udta + 4].copy_from_slice(b"mvex");
        structure_fails(&mvex, "an mvex box: fragmented MP4");
        // No mdat, no mvhd, two stts.
        let renamed = |from: &[u8; 4], to: &[u8; 4]| {
            let mut bytes = good.clone();
            let at = types(&bytes, from)[0];
            bytes[at..at + 4].copy_from_slice(to);
            bytes
        };
        structure_fails(&renamed(b"mdat", b"free"), "no mdat box");
        structure_fails(&renamed(b"mvhd", b"free"), "0 mvhd boxes");
        structure_fails(&renamed(b"stss", b"stts"), "2 stts boxes");
        // An optional box twice.
        let stss = testkit::full(b"stss", 0, 0, &[0, 0, 0, 1, 0, 0, 0, 1]);
        structure_fails(
            &Spec::with(|s| s.extra_stbl_box = Some(stss.clone())).build(),
            "2 stss boxes",
        );
    }

    #[test]
    fn hostile_sizes_are_too_large_never_a_panic() {
        let good = Spec::good().build();
        // 65 tracks.
        let many = Spec::with(|s| {
            s.audio = (0..64)
                .map(|k| AudioSpec::new(&format!("T{k}"), 2, k == 0, 95))
                .collect();
        });
        structure_fails(&many.build(), "more than 64 tracks: too large");
        // A uniform stsz claiming more samples than all tracks may hold together, and one
        // claiming more bytes than the file has.
        let uniform = |size: u32, count: u32| {
            let mut bytes = good.clone();
            let stsz = types(&bytes, b"stsz")[3];
            put_u32(&mut bytes, stsz + 8, size);
            put_u32(&mut bytes, stsz + 12, count);
            bytes
        };
        structure_fails(
            &uniform(1, (1 << 25) + 1),
            "samples in all tracks: too large",
        );
        structure_fails(&uniform(1_000, 1_000_000), "more than the file holds");
        // More than 100,000 boxes in the moov.
        structure_fails(
            &Spec::with(|s| s.udta_filler = 100_001).build(),
            "more than 100000 boxes in moov: too large",
        );
        // Huge numbers in the tables: no overflow anywhere (debug builds would panic).
        let big = Spec::with(|s| {
            s.movie_timescale = u32::MAX;
            s.deltas = vec![0x7FFF_FFFF; 120];
        });
        let _ = report(&big, &plan(Profile::Share, 2, &[("Mix", 2, true)]));
        let wide = Spec::with(|s| {
            s.timescale = 4_294_967_291;
            s.deltas = vec![1; 120];
            s.sps.width = 1 << 30;
            s.sps.height = 1 << 30;
        });
        let _ = report(&wide, &Plan::new(Profile::Share, false, None).unwrap());
    }

    #[test]
    fn quicktime_meta_is_read() {
        let bytes = Spec::with(|s| s.qt_meta = true).build();
        let clip = Clip::read(&mut bytes.as_slice()).unwrap();
        assert!(clip.problems.is_empty(), "{:?}", clip.problems);
        assert_eq!(clip.movie.unwrap().ilst_keys, [FourCc(*b"\xa9too")]);
    }

    #[test]
    fn scaling_deltas_stay_in_range() {
        // H.264 7.4.2.1.1.1: -128..=127.
        for (delta, ok) in [
            (127, true),
            (-128, true),
            (128, false),
            (-129, false),
            (i32::MAX, false),
            (i32::MIN + 1, false),
        ] {
            let spec = SpsSpec {
                scaling_lists: true,
                first_delta: delta,
                ..SpsSpec::default()
            };
            assert_eq!(
                h264::parse_sps(&testkit::sps(&spec)).is_some(),
                ok,
                "{delta}"
            );
        }
        let wild = SpsSpec {
            scaling_lists: true,
            first_delta: i32::MAX,
            ..SpsSpec::default()
        };
        fails_only(
            &Spec::with(|s| s.inband_sps_at = Some((60, testkit::sps(&wild)))),
            &engine(),
            &["video.params"],
        );
    }

    #[test]
    fn aac_object_types_on_their_own() {
        let e = engine();
        // AAC Main and LTP at 48 kHz, and HE-AAC with a 48 kHz core: only the object type is
        // wrong.
        for asc in [
            vec![0x09, 0x90],
            vec![0x21, 0x90],
            vec![0x29, 0x90, 0x10, 0x00],
        ] {
            fails_only(
                &Spec::with(|s| s.audio[0].asc = asc.clone()),
                &e,
                &["audio.format"],
            );
        }
        fails_only(
            &Spec::with(|s| s.audio[1].deltas.push(1_500)),
            &e,
            &["audio.format", "duration"],
        );
        fails_only(
            &Spec::with(|s| s.audio[0].rate = 44_100),
            &e,
            &["audio.format"],
        );
        fails_only(
            &Spec::with(|s| s.audio[0].asc = vec![0x11, 0x92, 0x00, 0x00]),
            &e,
            &["audio.format"],
        );
        fails_only(
            &Spec::with(|s| s.audio[0].timescale = 44_100),
            &e,
            &["audio.format", "duration"],
        );
    }

    #[test]
    fn more_parameter_set_rules() {
        let e = engine();
        let no_size = plan_with(Profile::Engine, 60, 2, None, &ROLES);
        // 4:2:2 at a size that needs no cropping.
        fails_only(
            &Spec::with(|s| {
                s.sps.chroma = 2;
                s.sps.height = 176;
                s.height = 176;
            }),
            &no_size,
            &["video.params"],
        );
        // Interlaced coding (the slices then read wrong too, so only params is asserted).
        let fields = report(&Spec::with(|s| s.sps.frame_mbs_only = false), &e);
        let params = fields
            .checks
            .iter()
            .find(|f| f.id == "video.params")
            .unwrap();
        assert_eq!(params.status, Status::Fail);
        assert!(
            params.events.iter().any(|e| e.contains("interlaced")),
            "{:?}",
            params.events
        );
        fails_only(
            &Spec::with(|s| s.avcc_level = Some(40)),
            &e,
            &["video.params"],
        );
        fails_only(
            &Spec::good(),
            &plan_with(Profile::Engine, 60, 2, Some((640, 360)), &ROLES),
            &["video.params"],
        );
        assert_eq!(
            failing(&report_for_test(&Spec::with(|s| s.x265_at = Some(0)))),
            ["video.params"]
        );
    }

    #[test]
    fn golden_videotoolbox_sps_in_a_clip() {
        // README §9: VideoToolbox's 720p SPS made without setparams (VUI 2/2/1) passes C7 and
        // the SAR rule and fails C4's colour rule against colr 1/1/1/0.
        let vt = |sps: &str| {
            Spec::with(|s| {
                s.raw_sps = Some(hex(sps));
                s.frame_num_bits = 5;
                s.width = 1_280;
                s.height = 720;
            })
        };
        let no_size = plan_with(Profile::Engine, 60, 2, None, &ROLES);
        let r = report(&vt("2764001fac56805005ba6a04040204"), &no_size);
        assert_eq!(failing(&r), ["video.params"], "{}", r.to_text());
        assert_eq!(status(&r, "video.no_reorder"), Status::Pass);
        let params = r.checks.iter().find(|f| f.id == "video.params").unwrap();
        assert!(
            params.events.iter().any(|e| e.contains("VUI colour 2/2/1")),
            "{:?}",
            params.events
        );
        assert!(!params.events.iter().any(|e| e.contains("SAR")));
        // The same encoder with setparams (VUI 1/1/1): only its level warns (3.1 for 720p60).
        let fixed = report(&vt("2764001fac56805005ba6a02020204"), &no_size);
        assert!(failing(&fixed).is_empty(), "{}", fixed.to_text());
        assert_eq!(status(&fixed, "video.params"), Status::Warn);
    }

    #[test]
    fn duration_and_gop_bounds_are_exact() {
        let e = engine();
        fails_only(&Spec::with(|s| s.tkhd_extra = 1), &e, &[]);
        fails_only(&Spec::with(|s| s.tkhd_extra = 2), &e, &["duration"]);
        fails_only(&Spec::with(|s| s.mvhd_extra = 2), &e, &["duration"]);
        fails_only(&Spec::with(|s| s.mdhd_extra = 1), &e, &["duration"]);
        // 60 frames is exactly one GOP short of 120: a warning, not a failure.
        let short = Spec::with(|s| {
            s.pics.truncate(60);
            s.deltas.truncate(60);
            s.audio = audio(48);
        });
        assert_eq!(
            status(&fails_only(&short, &e, &[]), "duration"),
            Status::Warn
        );
        // The audio end window: 94 packets end 769 samples early, 96 end 1,279 late.
        fails_only(
            &Spec::with(|s| s.audio[1].deltas.truncate(94)),
            &e,
            &["duration"],
        );
        fails_only(
            &Spec::with(|s| s.audio[1].deltas.push(1_024)),
            &e,
            &["duration"],
        );
    }

    /// A one-track clip of `seconds` at `fps` and `width`×`height`.
    fn export_clip(fps: u32, seconds: usize, width: u16, height: u16) -> Spec {
        let delta = 15_360 / fps;
        let frames = fps as usize * seconds;
        // Audio runs to the video's end, rounded up to a whole packet.
        let end = (frames as u64 * u64::from(delta) + 328) * 48_000 / 15_360;
        let packets = (end / 1_024 + 1) as usize;
        Spec::with(|s| {
            s.width = width;
            s.height = height;
            s.sps.width = u32::from(width);
            s.sps.height = u32::from(height);
            s.sps.level = 51;
            s.pics = (0..frames)
                .map(|i| {
                    if i % fps as usize == 0 {
                        Pic::Idr
                    } else {
                        Pic::P
                    }
                })
                .collect();
            s.deltas = vec![delta; frames];
            s.deltas[0] = delta + 328;
            s.audio = vec![AudioSpec::new("Mix", 2, true, packets)];
        })
    }

    #[test]
    fn export_limits() {
        let one = [("Mix", 2, true)];
        // Discord: 19,000,000 bytes at most (540p30 is on the ladder).
        let big = export_clip(30, 2, 960, 540).tap(|s| s.video_sample_bytes = 330_000);
        fails_only(
            &big,
            &plan_with(Profile::Discord, 30, 2, Some((960, 540)), &one),
            &["export.size"],
        );
        // Share copies stay within 1920x1080, either orientation, at 60 fps.
        let share = plan_with(Profile::Share, 60, 2, None, &one);
        fails_only(&export_clip(60, 2, 2_560, 1_440), &share, &["export.shape"]);
        fails_only(&export_clip(60, 2, 1_080, 1_920), &share, &[]);
        // Reels: 23 to 60 fps, 3 s to 15 min.
        let reels = |fps, seconds| plan_with(Profile::Reels, fps, seconds, None, &one);
        fails_only(&export_clip(30, 4, 1_080, 1_920), &reels(30, 4), &[]);
        fails_only(
            &export_clip(20, 3, 1_080, 1_920),
            &reels(20, 3),
            &["export.shape"],
        );
        fails_only(
            &export_clip(30, 2, 1_080, 1_920),
            &reels(30, 2),
            &["export.shape"],
        );
        // A share copy's one track is stereo and enabled.
        let share2 = plan(Profile::Share, 2, &one);
        let mono = share_spec().tap(|s| {
            s.audio[0].channels = 1;
            s.audio[0].entry_channels = 1;
            s.audio[0].asc = vec![0x11, 0x88, 0x56, 0xE5, 0x00];
        });
        fails_only(&mono, &share2, &["tracks.layout"]);
        fails_only(
            &share_spec().tap(|s| s.audio[0].enabled = false),
            &share2,
            &["tracks.layout"],
        );
        // WhatsApp's audio: 128 kbps passes, 160 kbps doesn't.
        let whatsapp = plan_with(Profile::Whatsapp, 30, 2, None, &one);
        let at =
            |bytes: usize| export_clip(30, 2, 320, 180).tap(|s| s.audio[0].payload_bytes = bytes);
        fails_only(&at(341), &whatsapp, &[]);
        fails_only(&at(427), &whatsapp, &["export.shape"]);
    }

    #[test]
    fn more_single_rules() {
        let e = engine();
        // Only free, skip or wide may sit between ftyp and moov: split the 32-byte ftyp into
        // a 16-byte ftyp and a 16-byte `junk` box.
        let mut junk = Spec::good().build();
        put_u32(&mut junk, 0, 16);
        put_u32(&mut junk, 16, 16);
        junk[20..24].copy_from_slice(b"junk");
        let r = check(&mut junk.as_slice(), "clip.mp4", &e);
        assert_eq!(failing(&r), ["container.faststart"]);
        // Sample 1 can't be shorter than a frame; it then also starts too soon.
        fails_only(
            &Spec::with(|s| {
                s.deltas[0] = 200;
                s.audio = audio(94);
            }),
            &e,
            &["video.cfr", "av.start_alignment"],
        );
        // An equal picture order count is a fall back too.
        fails_only(&Spec::with(|s| s.poc_step = 0), &e, &["video.no_reorder"]);
        // stss that leaves out sample 1.
        fails_only(
            &Spec::with(|s| s.stss = Some(vec![61])),
            &e,
            &["video.idr_start", "video.gop"],
        );
        // Audio in alternate group 0; video in group 1; video not first.
        fails_only(
            &Spec::with(|s| {
                for a in &mut s.audio {
                    a.group = 0;
                }
            }),
            &e,
            &["tracks.layout"],
        );
        fails_only(&Spec::with(|s| s.video_group = 1), &e, &["tracks.layout"]);
        fails_only(&Spec::with(|s| s.video_last = true), &e, &["tracks.layout"]);
        // A zero SAR side means unspecified (H.264 E.2.1).
        fails_only(&with_vui(|v| v.aspect = Some((255, 0, 0))), &e, &[]);
        fails_only(&with_vui(|v| v.aspect = Some((255, 0, 1))), &e, &[]);
    }

    #[test]
    fn the_summary_shows_the_clips_own_rate() {
        // A 30 fps clip checked against a 60 fps expect file.
        let r = report(
            &export_clip(30, 2, 320, 180).tap(|s| s.audio = audio(95)),
            &engine(),
        );
        assert!(r.summary.video.contains("CFR 30,"), "{}", r.summary.video);
        assert_eq!(status(&r, "video.cfr"), Status::Fail);
        let vfr = report(
            &Spec::with(|s| {
                s.deltas[99] = 512;
                s.audio = audio(96);
            }),
            &engine(),
        );
        assert_eq!(
            vfr.summary.video,
            "h264 High 320x180 VFR (mostly 60 fps, 1 other durations), 120 frames, peak 0.0 Mbps"
        );
    }

    #[test]
    fn box_orders_are_collapsed() {
        let kinds = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            checks::box_order(&kinds("ftyp moov free mdat")),
            "ftyp moov free mdat"
        );
        let fragmented = format!("ftyp moov {} mfra", "moof mdat ".repeat(120).trim_end());
        assert_eq!(
            checks::box_order(&kinds(&fragmented)),
            "ftyp moov (moof mdat)×120 mfra"
        );
        assert_eq!(
            checks::box_order(&kinds("ftyp free free free mdat")),
            "ftyp free×3 mdat"
        );
        let distinct: Vec<String> = (0..40).map(|i| format!("b{i:03}")).collect();
        assert!(checks::box_order(&distinct).ends_with("… (28 more)"));
    }

    #[test]
    fn the_gate_list_is_pinned() {
        // README §6.5's list, written out here rather than read from GATE_M3_CHECKS.
        let list = [
            "container.structure",
            "container.faststart",
            "container.no_editlist",
            "video.colr",
            "video.cfr",
            "video.idr_start",
            "video.no_reorder",
            "tracks.layout",
            "duration",
        ];
        let gate = Plan::new(Profile::Engine, true, None).unwrap();
        for id in STRUCTURE_CHECKS.iter().chain(&EXPORT_CHECKS) {
            let want = if list.contains(id) {
                Mode::Required
            } else if profile::mode(Profile::Engine, id) == Mode::Off {
                Mode::Off
            } else {
                Mode::Warning
            };
            assert_eq!(gate.mode(id), want, "{id}");
        }
        assert_eq!(gate.content_mode("av.offset"), Mode::Required);
        assert_eq!(gate.content_mode("decode.clean"), Mode::Warning);
    }
}
