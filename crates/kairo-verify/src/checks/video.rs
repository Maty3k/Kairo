// SPDX-License-Identifier: GPL-3.0-or-later
//! The video checks (container.md §6 C3–C5, C7–C9, C13, C14): `colr`, the parameter sets, CFR,
//! no reordering, IDR at 0, the closed GOP, one encoder session and the bitrate.

use serde_json::json;

use super::{Ctx, Outcome};
use crate::h264::{self, Sps};
use crate::mp4::tracks::{Colr, SampleEntry};
use crate::profile::{self, Profile};
use crate::scan::{B_SLICE, Scan};

/// How many sample numbers a message lists.
const LIST: usize = 5;

/// Table A-1: (level_idc, MaxMBPS, MaxFS).
const LEVELS: [(u8, u64, u64); 20] = [
    (9, 1_485, 99),
    (10, 1_485, 99),
    (11, 3_000, 396),
    (12, 6_000, 396),
    (13, 11_880, 396),
    (20, 11_880, 396),
    (21, 19_800, 792),
    (22, 20_250, 1_620),
    (30, 40_500, 1_620),
    (31, 108_000, 3_600),
    (32, 216_000, 5_120),
    (40, 245_760, 8_192),
    (41, 245_760, 8_192),
    (42, 522_240, 8_704),
    (50, 589_824, 22_080),
    (51, 983_040, 36_864),
    (52, 2_073_600, 36_864),
    (60, 4_177_920, 139_264),
    (61, 8_355_840, 139_264),
    (62, 16_711_680, 139_264),
];

fn mbps(bits_per_second: u64) -> String {
    format!("{:.1} Mbps", bits_per_second as f64 / 1e6)
}

fn first_numbers(numbers: &[u64]) -> String {
    let shown: Vec<String> = numbers.iter().take(LIST).map(u64::to_string).collect();
    if numbers.len() > LIST {
        format!("[{}, …] ({})", shown.join(", "), numbers.len())
    } else {
        format!("[{}]", shown.join(", "))
    }
}

/// The scan, or a failure saying the slices couldn't be read.
fn scan_or_fail<'a>(ctx: &Ctx<'a>, outcome: &mut Outcome) -> Option<&'a Scan> {
    let scan = ctx.clip.scan.as_ref();
    if scan.is_none() {
        outcome
            .fails
            .push("the H.264 headers couldn't be read (see video.params)".to_owned());
    }
    scan
}

/// C3: exactly one `colr`, `nclx`, BT.709 limited range.
pub(crate) fn colr(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome {
        limit: json!({"colr": "nclx 1/1/1 full_range 0", "source": "M1:12, research 09:683–686, CB:498–500"}),
        detail: "colr nclx 1/1/1, limited range".to_owned(),
        ..Outcome::default()
    };
    let Some(SampleEntry::Visual { colr, .. }) = ctx.entry() else {
        outcome.fails.push("no visual sample entry".to_owned());
        return outcome;
    };
    let shown: Vec<String> = colr.iter().map(describe_colr).collect();
    outcome.value = json!({"colr": shown});
    match colr.as_slice() {
        [] => outcome.fails.push("no colr box".to_owned()),
        [one] => match *one {
            Colr::Nclx {
                primaries,
                transfer,
                matrix,
                full_range,
                reserved,
            } => {
                outcome.require(
                    (primaries, transfer, matrix, full_range) == (1, 1, 1, false),
                    || format!("colr {}; want nclx 1/1/1 full_range 0", describe_colr(one)),
                );
                outcome.prefer(reserved == 0, || {
                    format!("colr reserved bits {reserved:#x}, not 0")
                });
            }
            Colr::Other(kind) => outcome
                .fails
                .push(format!("colr of type {kind}; want nclx")),
        },
        many => outcome
            .fails
            .push(format!("{} colr boxes; want one", many.len())),
    }
    outcome
}

fn describe_colr(colr: &Colr) -> String {
    match *colr {
        Colr::Nclx {
            primaries,
            transfer,
            matrix,
            full_range,
            ..
        } => format!(
            "nclx {primaries}/{transfer}/{matrix} full_range {}",
            u8::from(full_range)
        ),
        Colr::Other(kind) => kind.to_string(),
    }
}

/// The SPS fields C4 checks, for comparing an in-band SPS with the `avcC` one.
fn checked_fields(sps: &Sps) -> impl PartialEq + use<> {
    let vui = sps.vui.unwrap_or_default();
    (
        (sps.profile_idc, sps.level_idc, sps.chroma_format_idc),
        (sps.bit_depth_luma, sps.bit_depth_chroma, sps.frame_mbs_only),
        (sps.width, sps.height),
        (vui.colour, vui.full_range, vui.aspect_ratio_idc, vui.sar),
    )
}

/// The lowest level whose limits cover `width`×`height` at `fps` (Table A-1).
fn level_needed(width: u32, height: u32, fps: (u64, u64)) -> Option<u8> {
    let (w, h) = (
        u64::from(width.div_ceil(16)),
        u64::from(height.div_ceil(16)),
    );
    // u128: a hostile SPS can claim 2^28 macroblocks a side and T / D any rate.
    let frame = u128::from(w) * u128::from(h);
    LEVELS
        .iter()
        .filter(|&&(idc, _, _)| idc != 9)
        .find(|&&(_, max_mbps, max_fs)| {
            let (mbps, fs) = (u128::from(max_mbps), u128::from(max_fs));
            frame <= fs
                && frame * u128::from(fps.0) <= mbps * u128::from(fps.1)
                && u128::from(w.max(h)).pow(2) <= 8 * fs
        })
        .map(|&(idc, _, _)| idc)
}

/// C4: the VUI agrees with `colr`, and the parameter sets are what Kairo encodes.
pub(crate) fn params(ctx: &Ctx<'_>) -> Outcome {
    let profile = ctx.plan.profile;
    let mut outcome = Outcome {
        limit: json!({
            "profile": "High (100)", "chroma": "4:2:0", "bit_depth": 8, "progressive": true,
            "entropy": "CABAC", "sar": "1:1", "vui_colour": "= colr",
            "source": "research 09:558, CB:486, research 13:801, container.md §6 C4"
        }),
        ..Outcome::default()
    };
    let Some(SampleEntry::Visual {
        width,
        height,
        avcc,
        colr,
        pasp,
        ..
    }) = ctx.entry()
    else {
        outcome.fails.push("no visual sample entry".to_owned());
        return outcome;
    };
    if avcc.len() != 1 {
        outcome
            .fails
            .push(format!("{} avcC boxes; want one", avcc.len()));
    }
    let Some(config) = ctx.clip.avcc.as_ref() else {
        outcome.fails.push("the avcC box doesn't parse".to_owned());
        return outcome;
    };
    let (Some(sps), Some(pps)) = (ctx.sps.as_ref(), ctx.pps.as_ref()) else {
        outcome
            .fails
            .push("the avcC SPS or PPS doesn't parse".to_owned());
        return outcome;
    };
    let vui = sps.vui.unwrap_or_default();
    // 1. The VUI's colour equals colr's.
    match vui.colour {
        Some((p, t, m)) => {
            let full = vui.full_range.unwrap_or(false);
            if let [
                Colr::Nclx {
                    primaries,
                    transfer,
                    matrix,
                    full_range,
                    ..
                },
            ] = colr.as_slice()
            {
                let same = (u16::from(p), u16::from(t), u16::from(m), full)
                    == (*primaries, *transfer, *matrix, *full_range);
                outcome.require(same, || {
                    format!(
                        "VUI colour {p}/{t}/{m} full_range {} differs from colr {primaries}/{transfer}/{matrix} full_range {}",
                        u8::from(full),
                        u8::from(*full_range)
                    )
                });
            }
        }
        None => {
            // h264_mf writes no colour VUI (CB:478): only the testbed profiles fail on it.
            let fail = matches!(profile, Profile::Test | Profile::Capture | Profile::Smoke);
            outcome.violation(fail, "the VUI has no colour description".to_owned());
        }
    }
    // 2. In-band SPS units match the avcC one.
    if let Some(scan) = ctx.clip.scan.as_ref() {
        for (index, raw) in &scan.differing_sps {
            let Some(other) = h264::parse_sps(raw) else {
                outcome.fails.push(format!(
                    "an in-band SPS at sample {} doesn't parse",
                    index + 1
                ));
                continue;
            };
            let base = ctx.clip.avcc.as_ref().and_then(|a| {
                a.sps
                    .iter()
                    .filter_map(|n| h264::parse_sps(n))
                    .find(|s| s.sps_id == other.sps_id)
            });
            match base {
                Some(base) if checked_fields(&base) == checked_fields(&other) => {
                    outcome.warns.push(format!(
                        "an in-band SPS at sample {} differs from avcC's in bytes only",
                        index + 1
                    ))
                }
                _ => outcome.fails.push(format!(
                    "an in-band SPS at sample {} differs from avcC's ({}x{}, profile {})",
                    index + 1,
                    other.width,
                    other.height,
                    other.profile_idc
                )),
            }
        }
        if scan.problem_count > 0 {
            for p in &scan.problems {
                outcome.fails.push(p.clone());
            }
            if scan.problem_count > scan.problems.len() {
                outcome.fails.push(format!(
                    "{} more NAL problems",
                    scan.problem_count - scan.problems.len()
                ));
            }
        }
        // x264 or x265 SEI: a GPL encoder made this (M1:12).
        for encoder in &scan.encoders {
            outcome.violation(
                profile == Profile::Test,
                format!("{encoder} SEI: made by a GPL encoder"),
            );
        }
    }
    // 3. High profile, with avcC agreeing.
    outcome.require(sps.profile_idc == 100, || {
        format!("profile {}; want High (100)", sps.profile_idc)
    });
    outcome.require(
        (config.profile, config.level) == (sps.profile_idc, sps.level_idc),
        || {
            format!(
                "avcC says profile {} level {}, the SPS {} {}",
                config.profile, config.level, sps.profile_idc, sps.level_idc
            )
        },
    );
    // 4. 4:2:0, 8-bit, progressive.
    outcome.require(sps.chroma_format_idc == 1, || {
        format!(
            "chroma_format_idc {}; want 1 (4:2:0)",
            sps.chroma_format_idc
        )
    });
    outcome.require((sps.bit_depth_luma, sps.bit_depth_chroma) == (8, 8), || {
        format!(
            "bit depth {}/{}; want 8",
            sps.bit_depth_luma, sps.bit_depth_chroma
        )
    });
    outcome.require(sps.frame_mbs_only, || {
        "interlaced coding (frame_mbs_only 0)".to_owned()
    });
    outcome.require(pps.cabac, || "CAVLC; want CABAC".to_owned());
    // 5. One size everywhere, even.
    let tkhd = ctx.video.tkhd.map(|t| (t.width, t.height));
    let entry_size = (u32::from(*width), u32::from(*height));
    outcome.require((sps.width, sps.height) == entry_size, || {
        format!(
            "the SPS codes {}x{}, the sample entry says {}x{}",
            sps.width, sps.height, width, height
        )
    });
    if let Some((tw, th)) = tkhd {
        let whole = tw & 0xFFFF == 0 && th & 0xFFFF == 0;
        outcome.require(whole && (tw >> 16, th >> 16) == entry_size, || {
            format!(
                "tkhd size {}x{} differs from {}x{}",
                f64::from(tw) / 65536.0,
                f64::from(th) / 65536.0,
                width,
                height
            )
        });
    }
    outcome.require(sps.width % 2 == 0 && sps.height % 2 == 0, || {
        format!("odd size {}x{}", sps.width, sps.height)
    });
    if let Some((w, h)) = ctx.plan.size {
        outcome.require((sps.width, sps.height) == (w, h), || {
            format!("{}x{}; the expect file says {w}x{h}", sps.width, sps.height)
        });
    }
    // 6. Square pixels.
    let sar_square = match (vui.aspect_ratio_idc, vui.sar) {
        (None | Some(0) | Some(1), _) => true,
        // H.264 E.2.1: a zero sar_width or sar_height means unspecified.
        (Some(255), Some((h, v))) => h == v || h == 0 || v == 0,
        _ => false,
    };
    outcome.require(sar_square, || {
        format!(
            "SAR {:?} {:?}; want unspecified or 1:1",
            vui.aspect_ratio_idc, vui.sar
        )
    });
    if let Some((h, v)) = pasp {
        outcome.require(h == v, || format!("pasp {h}:{v}; want 1:1"));
    }
    // A level below Table A-1's need only warns: VideoToolbox declares 3.1 for 720p60.
    if let Some(fps) = ctx.timing.fps
        && let Some(needed) = level_needed(sps.width, sps.height, fps)
    {
        outcome.prefer(sps.level_idc >= needed, || {
            format!(
                "level {} is below the {} that {}x{} at this rate needs",
                sps.level_idc, needed, sps.width, sps.height
            )
        });
    }
    outcome.value = json!({
        "profile": sps.profile_idc, "level": sps.level_idc, "size": [sps.width, sps.height],
        "chroma_format_idc": sps.chroma_format_idc, "bit_depth": sps.bit_depth_luma,
        "cabac": pps.cabac, "vui_colour": vui.colour.map(|(p, t, m)| [p, t, m]),
        "vui_full_range": vui.full_range,
        "inband_sps": ctx.clip.scan.as_ref().map(|s| s.inband_sps),
    });
    outcome.detail = format!(
        "High {}x{} level {}, 4:2:0 8-bit, CABAC",
        sps.width, sps.height, sps.level_idc
    );
    outcome
}

/// C5: every sample after the first lasts exactly one tick; the first lasts at least one.
pub(crate) fn cfr(ctx: &Ctx<'_>) -> Outcome {
    let t = ctx.timing;
    let mut outcome = Outcome {
        limit: json!({"fps": ctx.plan.fps, "rule": "samples 2…N last T/fps; sample 1 at least that",
                      "source": "PLAN:446, PLAN:161, core/cut.rs:221"}),
        ..Outcome::default()
    };
    if ctx.samples.is_empty() {
        outcome.fails.push("no video samples".to_owned());
        return outcome;
    }
    let Some(delta) = t.delta else {
        let fps = ctx.plan.fps.unwrap_or(0);
        outcome.fails.push(format!(
            "timescale {} is not a multiple of {fps} fps",
            t.timescale
        ));
        return outcome;
    };
    let irregular: Vec<u64> = ctx
        .samples
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, s)| u64::from(s.duration) != delta)
        .map(|(i, _)| i as u64 + 1)
        .collect();
    outcome.require(irregular.is_empty(), || {
        let first = irregular
            .first()
            .and_then(|&n| ctx.samples.get(n as usize - 1));
        format!(
            "samples {} don't last {delta} ticks (sample {} lasts {})",
            first_numbers(&irregular),
            irregular.first().copied().unwrap_or(0),
            first.map_or(0, |s| s.duration)
        )
    });
    let extension = t.extension.unwrap_or(0);
    outcome.require(extension >= 0, || {
        format!(
            "sample 1 lasts {} ticks, under one frame ({delta})",
            i128::from(delta) + extension
        )
    });
    let fps = t.fps.map(|(n, d)| {
        if d == 1 {
            n.to_string()
        } else {
            format!("{n}/{d}")
        }
    });
    let measured = t.measured.map(|m| {
        let fps = if m.fps.1 == 1 {
            m.fps.0.to_string()
        } else {
            format!("{}/{}", m.fps.0, m.fps.1)
        };
        json!({"delta": m.delta, "fps": fps, "other_durations": m.others})
    });
    outcome.value = json!({"timescale": t.timescale, "delta": delta, "fps": fps, "measured": measured,
                           "first": ctx.samples.first().map(|s| s.duration), "irregular": irregular.len()});
    outcome.detail = format!(
        "CFR {} fps ({} / {delta}), sample 1 +{extension} ticks",
        fps.unwrap_or_default(),
        t.timescale
    );
    outcome
}

/// C7: pts = dts: no `ctts` offsets, no B slices, no reordering, rising POC.
pub(crate) fn no_reorder(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome {
        limit: json!({"ctts": "absent or 0", "b_slices": 0, "max_num_reorder_frames": 0,
                      "source": "PLAN:166, research 09:588–591, core/ring.rs"}),
        ..Outcome::default()
    };
    let nonzero = ctx
        .video
        .ctts
        .as_ref()
        .map_or(0, |c| c.iter().filter(|&&(_, o)| o != 0).count());
    outcome.require(nonzero == 0, || {
        format!("ctts has {nonzero} non-zero entries")
    });
    let reorder = ctx
        .sps
        .as_ref()
        .and_then(|s| s.vui)
        .and_then(|v| v.max_num_reorder_frames);
    outcome.require(reorder.unwrap_or(0) == 0, || {
        format!("max_num_reorder_frames {}", reorder.unwrap_or(0))
    });
    let Some(scan) = scan_or_fail(ctx, &mut outcome) else {
        return outcome;
    };
    let b: Vec<u64> = scan
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f.slice_types & B_SLICE != 0)
        .map(|(i, _)| i as u64 + 1)
        .collect();
    outcome.require(b.is_empty(), || {
        format!("B slices in samples {}", first_numbers(&b))
    });
    // POC must rise in decode order inside each IDR period; type 2 can't reorder at all.
    let mut regression = None;
    let mut last: Option<i64> = None;
    for (i, frame) in scan.frames.iter().enumerate() {
        if frame.is_idr() {
            last = None;
        }
        if let Some(poc) = frame.poc {
            if last.is_some_and(|l| poc <= l) && regression.is_none() {
                regression = Some(i + 1);
            }
            last = Some(poc);
        }
    }
    outcome.require(regression.is_none(), || {
        format!(
            "the picture order falls back at sample {}",
            regression.unwrap_or(0)
        )
    });
    let poc_type = ctx.sps.as_ref().map(|s| s.poc_type);
    if poc_type == Some(1) {
        outcome.detail = "no reordering (POC type 1 not followed)".to_owned();
    } else {
        outcome.detail = "pts = dts: no ctts, no B slices, rising POC".to_owned();
    }
    outcome.value = json!({"ctts_nonzero": nonzero, "b_samples": b.len(), "max_num_reorder_frames": reorder,
                           "poc_type": poc_type, "poc_regression": regression});
    outcome
}

/// C8: sample 1 is a sync sample made only of IDR I slices, starting at macroblock 0, with
/// parameter sets that resolve.
pub(crate) fn idr_start(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome {
        limit: json!({"sample_1": "sync, all IDR I slices, first_mb 0", "source": "PLAN:179, M1:12, research 09:969"}),
        detail: "sample 1 is an IDR".to_owned(),
        ..Outcome::default()
    };
    let stss_first = ctx.video.stss.as_ref().map(|s| s.first().copied());
    outcome.require(matches!(stss_first, None | Some(Some(1))), || {
        format!(
            "sample 1 isn't a sync sample (stss starts at {})",
            stss_first.flatten().unwrap_or(0)
        )
    });
    let Some(scan) = scan_or_fail(ctx, &mut outcome) else {
        return outcome;
    };
    outcome.value = json!({"stss_first": stss_first.flatten(), "nal_types": scan.first_types});
    let Some(first) = scan.frames.first() else {
        outcome.fails.push("no video samples".to_owned());
        return outcome;
    };
    outcome.require(first.is_idr(), || {
        format!("sample 1 isn't all IDR (NAL types {:?})", scan.first_types)
    });
    outcome.require(first.intra_only(), || {
        "sample 1 has slices other than I".to_owned()
    });
    outcome.require(first.first_mb == Some(0), || {
        format!(
            "sample 1's first slice starts at macroblock {}",
            first.first_mb.unwrap_or(0)
        )
    });
    outcome.require(!first.unknown_pps, || {
        "sample 1 names a parameter set that isn't there".to_owned()
    });
    outcome
}

/// C9: sync samples every second, every sync sample an IDR and every IDR a sync sample, no
/// recovery points: with C7, the closed-GOP test.
pub(crate) fn gop(ctx: &Ctx<'_>) -> Outcome {
    let gop = ctx.timing.gop_frames();
    let mut outcome = Outcome {
        limit: json!({"gop_frames": gop, "rule": "sync = IDR = {1, 1+G, …}", "source": "PLAN:166, CB:537"}),
        ..Outcome::default()
    };
    let Some(scan) = scan_or_fail(ctx, &mut outcome) else {
        return outcome;
    };
    let n = scan.frames.len() as u64;
    let sync: Vec<u64> = match &ctx.video.stss {
        Some(stss) => stss.iter().map(|&s| u64::from(s)).collect(),
        None => (1..=n).collect(),
    };
    let idr: Vec<u64> = scan
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f.is_idr())
        .map(|(i, _)| i as u64 + 1)
        .collect();
    match gop {
        Some(g) if g > 0 => {
            let want: Vec<u64> = (1..=n).step_by(g as usize).collect();
            outcome.require(sync == want, || {
                format!(
                    "sync samples {}; want every {g}: {}",
                    first_numbers(&sync),
                    first_numbers(&want)
                )
            });
        }
        _ => outcome
            .warns
            .push("the frame rate isn't whole, so the GOP length wasn't checked".to_owned()),
    }
    if sync != idr {
        let open: Vec<u64> = sync
            .iter()
            .copied()
            .filter(|s| !idr.contains(s))
            .filter(|&s| {
                scan.frames
                    .get(s as usize - 1)
                    .is_some_and(|f| f.intra_only())
            })
            .collect();
        if !open.is_empty() {
            outcome.fails.push(format!(
                "I pictures that aren't IDR are sync samples: {}",
                first_numbers(&open)
            ));
        }
        outcome.fails.push(format!(
            "IDR samples {} differ from sync samples {}",
            first_numbers(&idr),
            first_numbers(&sync)
        ));
    }
    let recovery: Vec<u64> = scan
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f.recovery_point && !f.is_idr())
        .map(|(i, _)| i as u64 + 1)
        .collect();
    outcome.require(recovery.is_empty(), || {
        format!(
            "recovery-point SEI on non-IDR samples {}",
            first_numbers(&recovery)
        )
    });
    outcome.value = json!({"gop_frames": gop, "sync": sync.len(), "idr": idr.len(),
                           "sync_first": sync.iter().take(LIST).collect::<Vec<_>>()});
    outcome.detail = format!(
        "closed GOP of {} frames, {} IDRs",
        gop.unwrap_or(0),
        idr.len()
    );
    outcome
}

/// C13: one encoder session: one sample entry per track, every chunk on entry 1, and no
/// in-band SPS that changes the stream.
pub(crate) fn single_session(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome {
        limit: json!({"sample_entries": 1, "source": "PLAN:162, CB:617, research 09:637"}),
        detail: "one sample entry per track".to_owned(),
        ..Outcome::default()
    };
    for (index, track) in ctx.movie.tracks.iter().enumerate() {
        outcome.require(track.entries.len() == 1, || {
            format!(
                "track {} has {} sample entries",
                index + 1,
                track.entries.len()
            )
        });
        outcome.require(track.stsc.iter().all(|r| r.description_index == 1), || {
            format!("track {} has chunks on another sample entry", index + 1)
        });
    }
    if let (Some(scan), Some(avcc)) = (ctx.clip.scan.as_ref(), ctx.clip.avcc.as_ref()) {
        for (index, raw) in &scan.differing_sps {
            let Some(other) = h264::parse_sps(raw) else {
                continue;
            };
            let base = avcc
                .sps
                .iter()
                .filter_map(|n| h264::parse_sps(n))
                .find(|s| s.sps_id == other.sps_id);
            let same = base.is_some_and(|b| checked_fields(&b) == checked_fields(&other));
            outcome.require(same, || {
                format!(
                    "an in-band SPS at sample {} starts another stream ({}x{})",
                    index + 1,
                    other.width,
                    other.height
                )
            });
        }
    }
    outcome
}

/// C14: the average and the peak over sliding 1 s windows.
pub(crate) fn bitrate(ctx: &Ctx<'_>) -> Outcome {
    let profile = ctx.plan.profile;
    let t = ctx.timing.timescale;
    let mut outcome = Outcome {
        limit: match profile {
            Profile::Reels => {
                json!({"average_bps": profile::REELS_AVERAGE_BPS, "source": "PLAN:245"})
            }
            _ => {
                json!({"peak_bps": profile::PEAK_WARN_BPS, "window_s": 1, "source": "CLAUDE.md, research 13:139"})
            }
        },
        ..Outcome::default()
    };
    let samples = ctx.samples;
    let total_bytes: u64 = samples.iter().map(|s| u64::from(s.size)).sum();
    let total_ticks: u64 = samples.iter().map(|s| u64::from(s.duration)).sum();
    if t == 0 || total_ticks == 0 {
        return Outcome::skip("no timed video samples");
    }
    let average =
        u64::try_from(u128::from(total_bytes) * 8 * u128::from(t) / u128::from(total_ticks))
            .unwrap_or(u64::MAX);
    // The most bytes in any window [dts_i, dts_i + T).
    let mut peak_bytes = 0_u64;
    let mut end = 0;
    let mut window = 0_u64;
    for (start, s) in samples.iter().enumerate() {
        while let Some(next) = samples.get(end).filter(|n| n.dts < s.dts.saturating_add(t)) {
            window += u64::from(next.size);
            end += 1;
        }
        peak_bytes = peak_bytes.max(window);
        window -= u64::from(s.size).min(window);
        if end == start {
            end += 1;
        }
    }
    let peak = peak_bytes * 8;
    if matches!(
        profile,
        Profile::Test | Profile::Capture | Profile::Smoke | Profile::Engine | Profile::Original
    ) {
        outcome.prefer(peak <= profile::PEAK_WARN_BPS, || {
            format!("peak {} in 1 s, above 50 Mbps", mbps(peak))
        });
    }
    if profile == Profile::Reels {
        outcome.require(average <= profile::REELS_AVERAGE_BPS, || {
            format!("average {}, above 25 Mbps", mbps(average))
        });
    }
    outcome.value = json!({"average_bps": average, "peak_bps": peak, "window_s": 1});
    outcome.detail = format!("average {}, peak {} in 1 s", mbps(average), mbps(peak));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_table_a1() {
        // 720p60 needs 3.2 (VideoToolbox declares 3.1), 1080p60 4.2, 1440p60 5.1.
        assert_eq!(level_needed(1280, 720, (60, 1)), Some(32));
        assert_eq!(level_needed(1920, 1080, (60, 1)), Some(42));
        assert_eq!(level_needed(2560, 1440, (60, 1)), Some(51));
        assert_eq!(level_needed(1080, 1920, (30, 1)), Some(40));
        assert_eq!(level_needed(16_384, 16_384, (120, 1)), None);
    }
}
