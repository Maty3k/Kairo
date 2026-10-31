// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify boxes`: the box tree and a line per track, for looking at a clip by hand
//! (README §6.1; it replaces docs/research/prototypes/scripts/mp4boxes.py). It prints `ilst`
//! keys, never their values (CLAUDE.md).

use std::fmt::Write as _;

use crate::Clip;
use crate::h264;
use crate::mp4::BoxNode;
use crate::mp4::tracks::{Colr, SampleEntry, Track};

/// How many `stts` runs and `stss` entries a track line shows.
const SHOWN: usize = 4;

fn node(out: &mut String, n: &BoxNode, depth: usize) {
    let large = if n.header >= 16 && n.kind.0 != *b"uuid" {
        " (64-bit size)"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "{:indent$}{} @{} {}{large}",
        "",
        n.kind,
        n.offset,
        n.size,
        indent = depth * 2
    );
    for child in &n.children {
        node(out, child, depth + 1);
    }
}

fn shown<T: std::fmt::Debug>(items: &[T]) -> String {
    let head: Vec<String> = items.iter().take(SHOWN).map(|i| format!("{i:?}")).collect();
    let more = if items.len() > SHOWN {
        format!(", … ({})", items.len())
    } else {
        String::new()
    };
    format!("[{}{more}]", head.join(", "))
}

fn track_line(index: usize, track: &Track, samples: usize) -> String {
    let mut line = format!("track {}", index + 1);
    if let Some(h) = &track.hdlr {
        let _ = write!(line, " {} {:?}", h.handler, h.name);
    }
    if let Some(name) = &track.udta_name {
        let _ = write!(line, " name {name:?}");
    }
    if let Some(t) = track.tkhd {
        let _ = write!(
            line,
            " | id {} flags {} group {}",
            t.track_id, t.flags, t.alternate_group
        );
    }
    if let Some(m) = track.mdhd {
        let _ = write!(
            line,
            " | {} Hz {} {}",
            m.timescale,
            m.duration,
            String::from_utf8_lossy(&m.language)
        );
    }
    for entry in &track.entries {
        match entry {
            SampleEntry::Visual {
                kind,
                width,
                height,
                avcc,
                colr,
                pasp,
            } => {
                let _ = write!(line, " | {kind} {width}x{height}");
                if let Some(config) = avcc.first().and_then(|a| h264::parse_avcc(a)) {
                    let _ = write!(
                        line,
                        " avcC {}@{} len {} {} SPS {} PPS",
                        config.profile,
                        config.level,
                        config.length_size,
                        config.sps.len(),
                        config.pps.len()
                    );
                }
                for c in colr {
                    match c {
                        Colr::Nclx {
                            primaries,
                            transfer,
                            matrix,
                            full_range,
                            ..
                        } => {
                            let _ = write!(
                                line,
                                " colr nclx {primaries}/{transfer}/{matrix}/{}",
                                u8::from(*full_range)
                            );
                        }
                        Colr::Other(kind) => {
                            let _ = write!(line, " colr {kind}");
                        }
                    }
                }
                if let Some((h, v)) = pasp {
                    let _ = write!(line, " pasp {h}:{v}");
                }
            }
            SampleEntry::Audio {
                kind,
                channels,
                sample_rate,
                ..
            } => {
                let _ = write!(line, " | {kind} {channels}ch {sample_rate} Hz");
            }
            SampleEntry::Other(kind) => {
                let _ = write!(line, " | {kind}");
            }
        }
    }
    let _ = write!(line, " | {samples} samples stts {}", shown(&track.stts));
    if let Some(stss) = &track.stss {
        let _ = write!(line, " stss {}", shown(stss));
    }
    if let Some(ctts) = &track.ctts {
        let _ = write!(line, " ctts {}", shown(ctts));
    }
    if track.has_edts {
        let edits: Vec<(u64, i64)> = track
            .edits
            .iter()
            .map(|e| (e.segment_duration, e.media_time))
            .collect();
        let _ = write!(line, " elst {}", shown(&edits));
    }
    line
}

/// The tree: top-level boxes, the `moov` tree, a line per track, the `ilst` keys and any
/// structural problems.
pub fn tree(clip: &Clip) -> String {
    let mut out = String::new();
    for top in &clip.mp4.top {
        match &clip.mp4.moov {
            Some(moov) if moov.offset == top.offset => node(&mut out, moov, 0),
            _ => {
                let large = if top.header >= 16 {
                    " (64-bit size)"
                } else {
                    ""
                };
                let _ = writeln!(out, "{} @{} {}{large}", top.kind, top.offset, top.size);
            }
        }
    }
    if let Some(movie) = &clip.movie {
        if let Some(m) = movie.mvhd {
            let _ = writeln!(
                out,
                "mvhd: {} Hz, duration {}, next track {}",
                m.timescale, m.duration, m.next_track_id
            );
        }
        for (index, track) in movie.tracks.iter().enumerate() {
            let samples = track.sizes.len();
            let _ = writeln!(out, "{}", track_line(index, track, samples));
        }
        if !movie.ilst_keys.is_empty() {
            let keys: Vec<String> = movie.ilst_keys.iter().map(ToString::to_string).collect();
            let _ = writeln!(out, "ilst keys: {}", keys.join(", "));
        }
    }
    for p in &clip.problems {
        let _ = writeln!(out, "problem: {} @{}: {}", p.path, p.offset, p.what);
    }
    out
}
