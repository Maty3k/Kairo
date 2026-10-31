// SPDX-License-Identifier: GPL-3.0-or-later
//! `container.structure`, `container.faststart` and `container.no_editlist` (container.md §6
//! C0–C2).

use serde_json::json;

use super::{Ctx, Outcome};
use crate::Clip;
use crate::mp4::tracks::{COUNTED, Movie, Track};
use crate::mp4::{Mp4, Problem};

/// How many misplaced chunks each track lists before it just counts them.
const MAX_SAMPLE_PROBLEMS: usize = 5;

/// The structure rules beyond what reading the boxes already found (C0): the top level, the
/// boxes each track must hold, the sample tables, `dref`, and where every sample lies.
pub(crate) fn structure_problems(mp4: &Mp4, movie: Option<&Movie>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut add = |path: &str, offset: u64, what: String| {
        problems.push(Problem {
            path: path.to_owned(),
            offset,
            what,
        });
    };
    let count = |kind: &[u8; 4]| mp4.top.iter().filter(|b| b.kind.0 == *kind).count();
    match mp4.top.first() {
        Some(first) if first.kind.0 == *b"ftyp" => {}
        Some(first) => add(
            "",
            first.offset,
            format!("the first box is {}, not ftyp", first.kind),
        ),
        None => add("", 0, "the file is empty".to_owned()),
    }
    let moovs = count(b"moov");
    if moovs != 1 {
        add(
            "",
            0,
            format!("{moovs} moov boxes; exactly one is required"),
        );
    }
    if count(b"mdat") == 0 {
        add("", 0, "no mdat box".to_owned());
    }
    for kind in [b"moof", b"mfra"] {
        if let Some(b) = mp4.top.iter().find(|b| b.kind.0 == *kind) {
            add("", b.offset, format!("a {} box: fragmented MP4", b.kind));
        }
    }
    if let Some(mvex) = mp4.moov.as_ref().and_then(|m| m.child(b"mvex")) {
        add(
            "moov/mvex",
            mvex.offset,
            "an mvex box: fragmented MP4".to_owned(),
        );
    }
    let Some(movie) = movie else {
        return problems;
    };
    if movie.mvhd_count != 1 {
        add(
            "moov",
            0,
            format!("{} mvhd boxes; exactly one is required", movie.mvhd_count),
        );
    }
    if movie.tracks.is_empty() {
        add("moov", 0, "no tracks".to_owned());
    }
    // Every mdat payload, clipped to the file, sorted by start.
    let mut mdats: Vec<(u64, u64)> = mp4
        .top
        .iter()
        .filter(|b| b.kind.0 == *b"mdat")
        .map(|b| {
            let end = b.offset.saturating_add(b.size).min(mp4.file_len);
            (b.offset.saturating_add(b.header), end)
        })
        .collect();
    mdats.sort_unstable();
    for track in &movie.tracks {
        track_problems(track, &mut add);
        // Every chunk inside an mdat payload: a chunk's samples are contiguous, so that is
        // every sample.
        let (mut outside, mut examples) = (0_usize, Vec::new());
        let walked = track.walk_chunks(|offset, first, count, _| {
            let bytes: u64 = track
                .sizes
                .get(first..first + count as usize)
                .unwrap_or_default()
                .iter()
                .map(|&size| u64::from(size))
                .sum();
            if !inside(&mdats, offset, bytes) {
                outside += 1;
                if examples.len() < MAX_SAMPLE_PROBLEMS {
                    examples.push((offset, bytes));
                }
            }
        });
        if let Err(what) = walked {
            add(&track.path, track.stbl_offset, what);
            continue;
        }
        for (offset, bytes) in examples {
            add(
                &track.path,
                offset,
                format!("a chunk of {bytes} bytes lies outside every mdat payload"),
            );
        }
        if outside > MAX_SAMPLE_PROBLEMS {
            add(
                &track.path,
                0,
                format!(
                    "{} more chunks outside every mdat payload",
                    outside - MAX_SAMPLE_PROBLEMS
                ),
            );
        }
    }
    problems
}

/// Whether `[offset, offset + size)` lies inside one of `mdats` (sorted payload ranges).
fn inside(mdats: &[(u64, u64)], offset: u64, size: u64) -> bool {
    let Some(end) = offset.checked_add(size) else {
        return false;
    };
    // The last range starting at or before `offset`.
    let at = mdats.partition_point(|&(start, _)| start <= offset);
    at > 0 && mdats.get(at - 1).is_some_and(|&(_, stop)| end <= stop)
}

fn track_problems(track: &Track, add: &mut impl FnMut(&str, u64, String)) {
    let path = &track.path;
    let count = |kind: &[u8; 4]| {
        track
            .counts
            .iter()
            .find(|(k, _)| k.0 == *kind)
            .map_or(0, |&(_, n)| n)
    };
    for kind in &COUNTED[..10] {
        let n = count(kind);
        if n != 1 {
            add(
                path,
                0,
                format!(
                    "{n} {} boxes; exactly one is required",
                    crate::mp4::FourCc(**kind)
                ),
            );
        }
    }
    let offsets = count(b"stco") + count(b"co64");
    if offsets != 1 {
        add(
            path,
            0,
            format!("{offsets} stco/co64 boxes; exactly one is required"),
        );
    }
    for kind in &COUNTED[12..] {
        let n = count(kind);
        if n > 1 {
            add(
                path,
                0,
                format!(
                    "{n} {} boxes; at most one is allowed",
                    crate::mp4::FourCc(**kind)
                ),
            );
        }
    }
    let n = track.sizes.len() as u64;
    if let Some(ctts) = &track.ctts {
        let total: u64 = ctts.iter().map(|&(c, _)| u64::from(c)).sum();
        if total != n {
            add(
                path,
                0,
                format!("ctts covers {total} samples, stsz has {n}"),
            );
        }
    }
    if let Some(stss) = &track.stss {
        let rising = stss.windows(2).all(|w| w[0] < w[1]);
        let in_range = stss.iter().all(|&s| s >= 1 && u64::from(s) <= n);
        if !rising || !in_range {
            add(
                path,
                0,
                format!("stss is not strictly rising within 1..={n}"),
            );
        }
    }
    match &track.dref {
        Some(dref) => {
            let self_contained = dref.count == 1
                && dref.entries.len() == 1
                && dref
                    .entries
                    .iter()
                    .all(|(kind, flags)| kind.0 == *b"url " && flags & 1 == 1);
            if !self_contained {
                add(
                    path,
                    0,
                    "dref is not one self-contained url entry".to_owned(),
                );
            }
        }
        None => add(path, 0, "no dref".to_owned()),
    }
}

/// C0: every structural problem, from reading and from the rules above.
pub(crate) fn structure(clip: &Clip) -> Outcome {
    let mut outcome = Outcome {
        value: json!({"problems": clip.problems.len(), "boxes": clip.mp4.top.len()}),
        limit: json!({"problems": 0, "source": "PLAN:179, PLAN:271; container.md §6 C0"}),
        detail: format!(
            "{} top-level boxes, {} tracks, sample tables agree",
            clip.mp4.top.len(),
            clip.movie.as_ref().map_or(0, |m| m.tracks.len())
        ),
        ..Outcome::default()
    };
    for p in &clip.problems {
        let at = if p.path.is_empty() {
            format!("@{}", p.offset)
        } else {
            format!("{} @{}", p.path, p.offset)
        };
        outcome.fails.push(format!("{at}: {}", p.what));
    }
    outcome
}

/// C1: the `moov` comes before the first `mdat`, with only `free`, `skip` or `wide` between
/// `ftyp` and `moov`.
pub(crate) fn faststart(ctx: &Ctx<'_>) -> Outcome {
    let top = &ctx.clip.mp4.top;
    let order: Vec<String> = top.iter().map(|b| b.kind.to_string()).collect();
    let moov = top.iter().position(|b| b.kind.0 == *b"moov");
    let mdat = top.iter().position(|b| b.kind.0 == *b"mdat");
    let mut outcome = Outcome {
        value: json!({"order": super::box_order(&order), "moov_offset": moov.and_then(|i| top.get(i)).map(|b| b.offset)}),
        limit: json!({"moov": "before the first mdat", "source": "PLAN:179, research 06:132"}),
        detail: format!("moov first: {}", super::box_order(&order)),
        ..Outcome::default()
    };
    if let (Some(moov), Some(mdat)) = (moov, mdat) {
        outcome.require(moov < mdat, || {
            format!(
                "the moov comes after the mdat: {}",
                super::box_order(&order)
            )
        });
        let between = top.get(1..moov).unwrap_or_default();
        for b in between {
            outcome.require(matches!(&b.kind.0, b"free" | b"skip" | b"wide"), || {
                format!("a {} box between ftyp and moov", b.kind)
            });
        }
    }
    outcome
}

/// C2: no `edts` in any track; identity edits fail too.
pub(crate) fn no_editlist(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome {
        limit: json!({"edts": "none", "source": "PLAN:179, research 09:590, PLAN:237"}),
        detail: "no edit lists".to_owned(),
        ..Outcome::default()
    };
    let mut tracks = Vec::new();
    for (index, track) in ctx.movie.tracks.iter().enumerate() {
        if !track.has_edts {
            continue;
        }
        let edits: Vec<String> = track
            .edits
            .iter()
            .map(|e| {
                format!(
                    "segment {} media_time {} rate {}",
                    e.segment_duration,
                    e.media_time,
                    f64::from(e.rate) / 65536.0
                )
            })
            .collect();
        outcome.fails.push(format!(
            "track {} has an edit list: [{}]",
            index + 1,
            edits.join("; ")
        ));
        tracks.push(index + 1);
    }
    outcome.value = json!({"tracks_with_edts": tracks});
    outcome
}
