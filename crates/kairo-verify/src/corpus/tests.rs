// SPDX-License-Identifier: GPL-3.0-or-later
//! The corpus's recipes, manifest and judging, without FFmpeg; and, opt-in, CI's lossless
//! corpus built and judged through Homebrew's FFmpeg.

use serde_json::json;

use super::*;
use crate::checks::Finding;
use crate::profile::{CONTENT_CHECKS, STRUCTURE_CHECKS};

#[test]
fn every_corpus_holds_research_16s_clips() {
    let gate = [
        "drop1",
        "dup1",
        "judder",
        "av50",
        "bars",
        "bframes",
        "nofaststart",
        "editlist",
        "colr",
        "swapped",
    ];
    let extras = [
        "nocolr",
        "two-defaults",
        "vfr",
        "gop2s",
        "bars-sparse",
        "unreadable",
        "cue",
        "leak",
        "noisy",
        "curve-swap",
    ];
    for encoder in [
        Encoder::Amf,
        Encoder::Mf,
        Encoder::VideoToolbox,
        Encoder::Ipcm,
    ] {
        let recipes = recipes(encoder);
        let names: Vec<&str> = recipes.iter().map(|r| r.name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{encoder}");
        for name in ["good-1080p60", "good-1440p60", "good-hdr-1080p60"]
            .iter()
            .chain(&gate)
            .chain(&extras)
        {
            // CI's lossless corpus has every defect but B-frames (README §9).
            let wanted = !(encoder == Encoder::Ipcm && *name == "bframes");
            assert_eq!(names.contains(name), wanted, "{encoder}: {name}");
        }
        // The good clips come first, so the PC calibrates before any bad clip is judged, and
        // the box edits come after the good clip they edit.
        assert_eq!(
            names[..3],
            ["good-1080p60", "good-1440p60", "good-hdr-1080p60"]
        );
        for recipe in &recipes {
            let known = |id: &&str| STRUCTURE_CHECKS.contains(id) || CONTENT_CHECKS.contains(id);
            assert!(recipe.must_fail.iter().all(known), "{}", recipe.name);
            assert!(recipe.may_fail.iter().all(known), "{}", recipe.name);
            let good = recipe.name.starts_with("good");
            assert_eq!(recipe.must_fail.is_empty(), good, "{}", recipe.name);
            let entry = recipe.entry(encoder);
            assert_eq!(entry.is_good(), good);
            assert_eq!(entry.profile, "test");
            assert_eq!(
                entry.hold_ticks,
                (recipe.name != "bframes").then_some(HOLD_TICKS),
                "{}",
                recipe.name
            );
            assert_eq!(
                entry.av_offset_ms,
                (recipe.name == "av50").then_some(50.0),
                "{}",
                recipe.name
            );
            assert_eq!(entry.expect_file(), format!("{}.expect.json", recipe.name));
        }
    }
    // The defect frame scales with the clip (README §7.3: 930 of 1,800, 150 of CI's 240).
    let defect = |encoder, name| {
        recipes(encoder)
            .into_iter()
            .find(|r| r.name == name)
            .and_then(|r| r.defect(encoder))
    };
    assert_eq!(
        defect(Encoder::VideoToolbox, "drop1").as_deref(),
        Some("drop@930")
    );
    assert_eq!(defect(Encoder::Ipcm, "dup1").as_deref(), Some("dup@150"));
    assert_eq!(defect(Encoder::Amf, "gop2s").as_deref(), Some("gop=120"));
    assert_eq!(
        defect(Encoder::Amf, "colr").as_deref(),
        Some("set-colr 6 6 6 0")
    );
    assert_eq!(
        defect(Encoder::Amf, "swapped").as_deref(),
        Some("swap-game-voice")
    );
    assert_eq!(defect(Encoder::Amf, "good-1080p60"), None);
}

#[test]
fn encoders_parse_and_size_their_clips() {
    for name in ["h264_amf", "h264_mf", "h264_videotoolbox", "ipcm"] {
        assert_eq!(
            name.parse::<Encoder>().map(|e| e.to_string()),
            Ok(name.to_owned())
        );
    }
    for bad in ["libx264", "libx265", "copy", "h264_nvenc", ""] {
        assert!(bad.parse::<Encoder>().is_err(), "{bad}");
    }
    let vt = Encoder::VideoToolbox;
    assert_eq!(
        (
            vt.synth(Video::Good).format.width,
            vt.synth(Video::Good).seconds
        ),
        (1920, 30)
    );
    let high = vt.synth(Video::High);
    assert_eq!((high.format.width, high.source_hz), (2560, 360));
    let ci = Encoder::Ipcm.synth(Video::Gop2s);
    assert_eq!((ci.format.width, ci.seconds), (480, 4));
    assert_eq!(ci.codec, crate::synth::Codec::Ipcm { gop: 120 });
    for video in [
        Video::Good,
        Video::High,
        Video::Hdr(Curve::Natural),
        Video::Defect(Defect::Judder),
        Video::Gop2s,
        Video::Bframes,
    ] {
        assert!(Encoder::Amf.synth(video).check().is_ok(), "{video:?}");
        assert!(Encoder::Ipcm.synth(video).check().is_ok(), "{video:?}");
    }
}

#[cfg(feature = "dev-tools")]
#[test]
fn encoder_sessions_use_research_16s_recipes() {
    use super::build::{encode_args, mux_args};

    let amf = Encoder::Amf;
    let line = |encoder: Encoder, video, hw| {
        encode_args(encoder, video, &encoder.synth(video), hw).join(" ")
    };
    // README §7.2's PC row, word for word.
    assert_eq!(
        line(amf, Video::Good, Some(1)),
        "-hide_banner -nostdin -loglevel debug -n -init_hw_device d3d11va=kd:1 \
         -f rawvideo -pix_fmt nv12 -video_size 1920x1080 -framerate 60 -i pipe:0 \
         -vf setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv \
         -pix_fmt nv12 -c:v h264_amf -usage transcoding -quality balanced -rc vbr_peak \
         -b:v 25M -maxrate 40M -bufsize 40M -g 60 -vbaq 1 -preanalysis 0 \
         -high_motion_quality_boost_enable 0 -preencode 0 -bf 0 -max_b_frames 0 \
         -frame_skipping 0 -filler_data 0 -coder cabac -profile:v high -header_spacing 60 \
         -async_depth 4 -an -movflags +faststart -use_editlist 0 v-good.mp4"
    );
    let high = line(amf, Video::High, None);
    assert!(
        high.contains("-init_hw_device d3d11va=kd -f rawvideo"),
        "{high}"
    );
    assert!(high.contains("-video_size 2560x1440"), "{high}");
    assert!(
        high.contains("-b:v 36M -maxrate 50M -bufsize 50M"),
        "{high}"
    );
    let bframes = line(amf, Video::Bframes, None);
    assert!(bframes.contains("-bf 2 -max_b_frames 2"), "{bframes}");
    assert!(
        bframes.contains("+faststart+negative_cts_offsets"),
        "{bframes}"
    );
    let gop = line(amf, Video::Gop2s, None);
    assert!(
        gop.contains("-g 120") && gop.contains("-header_spacing 120"),
        "{gop}"
    );
    assert_eq!(
        line(Encoder::VideoToolbox, Video::Good, None),
        "-hide_banner -nostdin -loglevel verbose -n \
         -f rawvideo -pix_fmt nv12 -video_size 1920x1080 -framerate 60 -i pipe:0 \
         -vf setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv \
         -c:v h264_videotoolbox -b:v 25M -maxrate 40M -g 60 -bf 0 -profile:v high \
         -an -movflags +faststart -use_editlist 0 v-good.mp4"
    );
    assert_eq!(
        line(Encoder::Ipcm, Video::Good, None),
        "-hide_banner -nostdin -loglevel verbose -n -f h264 -r 60 -i pipe:0 -c:v copy \
         -color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv \
         -an -movflags +faststart -use_editlist 0 v-good.mp4"
    );
    // Only the four encoders, AAC and stream copies are ever named (CLAUDE.md: no GPL code).
    for encoder in [
        Encoder::Amf,
        Encoder::Mf,
        Encoder::VideoToolbox,
        Encoder::Ipcm,
    ] {
        for recipe in recipes(encoder) {
            let (video, audio) = match recipe.make {
                Make::Mux { video, audio } => (video, audio),
                Make::Mutate(_) => continue,
            };
            let words = encode_args(encoder, video, &encoder.synth(video), None)
                .into_iter()
                .chain(mux_args(&recipe, video, audio));
            for word in words {
                assert!(
                    !word.contains("x264") && !word.contains("x265"),
                    "{encoder}: {word}"
                );
            }
        }
    }
    // The mux: four tracks with their titles, bitrates and the Mix alone the default.
    let swapped = recipes(Encoder::VideoToolbox)
        .into_iter()
        .find(|r| r.name == "swapped")
        .unwrap();
    let mux = mux_args(&swapped, Video::Good, Some(Defect::SwapGameVoice)).join(" ");
    for want in [
        "-i v-good.mp4 -i a-swap-game-voice-mix.wav -i a-swap-game-voice-game.wav",
        "-map 0:v -map 1:a -map 2:a -map 3:a -map 4:a -c:v copy -c:a aac",
        "-b:a:0 160k -metadata:s:a:0 title=Mix -metadata:s:a:0 handler_name=Mix \
         -metadata:s:a:0 language=und -disposition:a:0 default",
        "-b:a:2 128k -metadata:s:a:2 title=Mic",
        "-metadata:s:a:3 title=Voice chat",
        "-disposition:a:3 0 -movflags +faststart -use_editlist 0 swapped.mp4",
    ] {
        assert!(mux.contains(want), "{want} in {mux}");
    }
}

#[cfg(feature = "dev-tools")]
#[test]
fn a_relative_ffmpeg_is_found_from_the_callers_folder() {
    let build = |ffmpeg: &'static str| build::Build {
        encoder: Encoder::Ipcm,
        ffmpeg: std::path::Path::new(ffmpeg),
        hw_index: None,
        out: std::path::Path::new("corpus"),
    };
    let here = std::env::current_dir().unwrap();
    assert_eq!(build("bin/ffmpeg").program(), here.join("bin/ffmpeg"));
    assert_eq!(
        build("ffmpeg").program(),
        std::path::PathBuf::from("ffmpeg")
    );
    assert_eq!(
        build("/opt/homebrew/bin/ffmpeg").program(),
        std::path::PathBuf::from("/opt/homebrew/bin/ffmpeg")
    );
}

/// The lines of FFmpeg 8.0.1's verbose log that the checks read, from a VideoToolbox encode
/// on the Mac.
#[cfg(feature = "dev-tools")]
const VT_LOG: &str = "Input #0, rawvideo, from 'pipe:0':
  Stream #0:0: Video: rawvideo, 1 reference frame (NV12 / 0x3231564E), nv12, 640x360, 165888 kb/s, 60 tbr, 60 tbn
Stream mapping:
  Stream #0:0 -> #0:0 (rawvideo (native) -> h264 (h264_videotoolbox))
Output #0, mp4, to 'v.mp4':
  Metadata:
    encoder         : Lavf62.3.100
  Stream #0:0: Video: h264, 1 reference frame (avc1 / 0x31637661), nv12(tv, bt709, progressive), 640x360, q=2-31, 25000 kb/s, 60 fps, 15360 tbn
";

#[cfg(feature = "dev-tools")]
#[test]
fn logs_show_the_encoder_and_its_colour() {
    use super::build::{encode_log_problems, mux_log_problems, version};

    assert!(encode_log_problems(Encoder::VideoToolbox, VT_LOG, None).is_empty());
    // An unused option, another encoder, or BT.601 output each stop the build.
    let unused = format!(
        "{VT_LOG}[out#0/mp4 @ 0x1] Codec AVOption usage (x) has not been used for any stream."
    );
    assert_eq!(
        encode_log_problems(Encoder::VideoToolbox, &unused, None),
        ["an option no stream used"]
    );
    // Another encoder's log: no AMF mapping and neither device line.
    assert_eq!(encode_log_problems(Encoder::Amf, VT_LOG, None).len(), 3);
    let bt601 = VT_LOG.replace("nv12(tv, bt709", "nv12(tv, smpte170m");
    assert_eq!(
        encode_log_problems(Encoder::VideoToolbox, &bt601, None),
        ["the output's video isn't \"nv12(tv, bt709\""]
    );
    // AMF's device lines, the adapter included (README §7.2).
    let amf = VT_LOG.replace("h264_videotoolbox", "h264_amf")
        + "[AVHWDeviceContext @ 0x1] Selecting d3d11va adapter 1\n\
           [h264_amf @ 0x2] Using device kd (type d3d11va) with h264_amf encoder.\n\
           [AVHWDeviceContext @ 0x3] AMF via D3D11.\n";
    assert!(encode_log_problems(Encoder::Amf, &amf, Some(1)).is_empty());
    assert_eq!(
        encode_log_problems(Encoder::Amf, &amf, Some(0)),
        ["no \"Selecting d3d11va adapter 0\""]
    );
    // CI's stream copy.
    let copy = "  Stream #0:0 -> #0:0 (copy)
Output #0, mp4, to 'v-good.mp4':
  Stream #0:0: Video: h264 (High), 1 reference frame (avc1 / 0x31637661), yuv420p(tv, bt709, progressive, left), 480x270
";
    assert!(encode_log_problems(Encoder::Ipcm, copy, None).is_empty());
    // A mux copies the video and encodes four tracks with FFmpeg's own AAC.
    let mux = "  Stream #0:0 -> #0:0 (copy)
  Stream #1:0 -> #0:1 (pcm_f32le (native) -> aac (native))
  Stream #2:0 -> #0:2 (pcm_f32le (native) -> aac (native))
  Stream #3:0 -> #0:3 (pcm_f32le (native) -> aac (native))
  Stream #4:0 -> #0:4 (pcm_f32le (native) -> aac (native))
";
    assert!(mux_log_problems(mux).is_empty());
    assert_eq!(
        mux_log_problems(&mux.replace("aac (native)", "aac (aac_at)")),
        ["0 tracks to FFmpeg's aac, not 4"]
    );
    assert_eq!(
        version("ffmpeg version 8.0.1 Copyright (c) 2000-2025 the FFmpeg developers").as_deref(),
        Some("8.0.1")
    );
    assert_eq!(
        version("ffmpeg version n9.0.2-17-g2a571b6068 Copyright (c)").as_deref(),
        Some("n9.0.2-17-g2a571b6068")
    );
    assert_eq!(version("ffprobe version 8.0.1"), None);
}

/// A finding with `status`, as a report lists it.
fn finding(id: &'static str, status: Status, value: serde_json::Value) -> Finding {
    Finding {
        id,
        status,
        required: true,
        detail: String::new(),
        value,
        limit: serde_json::Value::Null,
        events: Vec::new(),
        events_total: 0,
    }
}

/// A clip's findings: `av.start_alignment` holding `ticks`, `av.offset` reading `medians`, and
/// every id in `failed` failing.
fn findings(ticks: i64, medians: &[f64], failed: &[&'static str]) -> Vec<Finding> {
    let mut out = vec![
        finding(
            "av.start_alignment",
            Status::Pass,
            json!({"extension_ticks": ticks, "hold_ms": ticks as f64 * 1_000.0 / 15_360.0}),
        ),
        finding(
            "av.offset",
            Status::Pass,
            json!({"tracks": medians.iter().map(|m| json!({"measured": true, "median_ms": m}))
                .collect::<Vec<_>>()}),
        ),
    ];
    for &id in failed {
        match out.iter_mut().find(|f| f.id == id) {
            Some(f) => f.status = Status::Fail,
            None => out.push(finding(id, Status::Fail, serde_json::Value::Null)),
        }
    }
    out
}

#[test]
fn clips_are_judged_by_research_16s_rules() {
    let recipe = |name: &str| {
        recipes(Encoder::VideoToolbox)
            .into_iter()
            .find(|r| r.name == name)
            .unwrap()
            .entry(Encoder::VideoToolbox)
    };
    let problems =
        |entry: &Entry, verdict, checks: &[Finding]| judge(entry, verdict, checks).problems;
    let good = recipe("good-1080p60");
    assert!(problems(&good, Verdict::Pass, &findings(328, &[0.0], &[])).is_empty());
    assert_eq!(
        problems(&good, Verdict::Fail, &findings(328, &[0.0], &["video.gop"])),
        ["a good clip, but the verdict is fail", "video.gop failed"]
    );
    // The first frame held one tick short.
    assert_eq!(
        problems(&good, Verdict::Pass, &findings(327, &[0.0], &[])).len(),
        1
    );
    // A bad clip fails all of its must_fail, may fail its may_fail, and nothing else.
    let bars = recipe("bars");
    assert!(
        problems(
            &bars,
            Verdict::Fail,
            &findings(328, &[0.0], &["content.bars"])
        )
        .is_empty()
    );
    assert!(
        problems(
            &bars,
            Verdict::Fail,
            &findings(328, &[0.0], &["content.bars", "content.bars_lines"])
        )
        .is_empty()
    );
    assert_eq!(
        problems(
            &bars,
            Verdict::Fail,
            &findings(328, &[0.0], &["content.bars_lines"])
        ),
        ["content.bars didn't fail"]
    );
    assert_eq!(
        problems(
            &bars,
            Verdict::Fail,
            &findings(328, &[0.0], &["content.bars", "av.offset"])
        ),
        ["av.offset failed"]
    );
    // A check that couldn't run is a problem whatever else holds.
    let mut errored = findings(328, &[0.0], &["content.bars"]);
    errored.push(Finding {
        detail: "the decoder didn't start".to_owned(),
        ..finding("decode.clean", Status::Error, serde_json::Value::Null)
    });
    assert_eq!(
        problems(&bars, Verdict::Error, &errored),
        ["decode.clean couldn't run: the decoder didn't start"]
    );
    // av50 reads +50 ± 1 ms on every track it measures.
    let av50 = recipe("av50");
    assert!(
        problems(
            &av50,
            Verdict::Fail,
            &findings(328, &[49.98, 50.4], &["av.offset"])
        )
        .is_empty()
    );
    assert_eq!(
        problems(
            &av50,
            Verdict::Fail,
            &findings(328, &[49.98, 51.5], &["av.offset"])
        ),
        ["av.offset reads +51.50 ms; want +50 ± 1 ms"]
    );
    assert_eq!(
        problems(&av50, Verdict::Fail, &findings(328, &[], &["av.offset"])),
        ["av.offset measured no track"]
    );
    // bframes needn't hold 328 ticks.
    let bframes = recipe("bframes");
    assert!(
        problems(
            &bframes,
            Verdict::Fail,
            &findings(0, &[0.0], &["video.no_reorder", "video.cfr"])
        )
        .is_empty()
    );
}

#[test]
fn the_pc_calibrates_and_manifests_round_trip() {
    assert_eq!(calibrated_limit(0.0), 4.0);
    assert_eq!(calibrated_limit(1.9), 4.0);
    assert_eq!(calibrated_limit(3.0), 6.0);
    let manifest = Manifest {
        kairo_corpus: VERSION,
        pattern: 1,
        encoder: "ipcm".to_owned(),
        ffmpeg: "8.0.1".to_owned(),
        finishing: Vec::new(),
        calibration: [(
            CALIBRATED.to_owned(),
            Calibration {
                limit_codes: Some(4.0),
                source: "fixed".to_owned(),
            },
        )]
        .into(),
        clips: recipes(Encoder::Ipcm)
            .iter()
            .map(|r| r.entry(Encoder::Ipcm))
            .collect(),
        missing: Vec::new(),
    };
    let text = serde_json::to_string(&manifest).unwrap();
    assert!(
        text.starts_with(r#"{"kairo_corpus":1,"pattern":1,"encoder":"ipcm""#),
        "{text}"
    );
    assert!(text.contains(r#"{"file":"good-1080p60.mp4","expect":"pass","profile":"test""#));
    assert!(text.contains(r#""file":"drop1.mp4","profile":"test","defect":"drop@150","must_fail":["cadence.drops_dups"]"#));
    assert_eq!(serde_json::from_str::<Manifest>(&text).unwrap(), manifest);
    assert!(serde_json::from_str::<Manifest>(&text.replace("\"pattern\"", "\"patern\"")).is_err());
}

/// Opt-in: CI's lossless corpus (README §9's W tier) built with Homebrew's FFmpeg and judged
/// through the CLI decoder: every clip behaves.
#[cfg(feature = "dev-tools")]
#[test]
#[ignore = "needs KAIRO_FFMPEG"]
fn the_ci_corpus_builds_and_behaves() {
    let ffmpeg = std::path::PathBuf::from(
        std::env::var_os("KAIRO_FFMPEG")
            .expect("set KAIRO_FFMPEG to an FFmpeg to spawn, e.g. /opt/homebrew/bin/ffmpeg"),
    );
    let root = std::env::temp_dir().join(format!("kairo-verify-corpus-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let dir = root.join("ipcm");
    let build = build::Build {
        encoder: Encoder::Ipcm,
        ffmpeg: &ffmpeg,
        hw_index: None,
        out: &dir,
    };
    let mut lines = Vec::new();
    let manifest = build.run(&mut |line| lines.push(line.to_owned())).unwrap();
    assert_eq!(manifest.clips.len(), recipes(Encoder::Ipcm).len());
    // Only clips, expect files, logs and the manifest stay.
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            name == "logs"
                || name == MANIFEST
                || name.ends_with(".mp4")
                || name.ends_with(".expect.json"),
            "{name}"
        );
        assert!(!name.starts_with("v-") && !name.starts_with("a-"), "{name}");
    }
    let decoder = |clip: &std::path::Path| -> Box<dyn crate::decode::Decoder> {
        Box::new(crate::decode::cli::Cli::new(&ffmpeg, clip))
    };
    let check = check::Check {
        dir: &dir,
        decoder: &decoder,
    };
    let report = check.run(&mut |line| lines.push(line.to_owned())).unwrap();
    assert!(report.pass, "{}", report.to_text());
    assert!(dir.join(REPORT).exists());
    // A second build can't reuse the folder.
    assert_eq!(build.run(&mut |_| {}).map_err(|e| e.code), Err(2));
    std::fs::remove_dir_all(&root).unwrap();
}
