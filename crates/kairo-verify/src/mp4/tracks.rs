// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed views of the boxes the checks read (docs/research/16-verify-testbed/container.md §5.1,
//! §6): the movie and track headers, sample tables, sample entries, edit lists and titles.
//! Malformed payloads become [`Problem`]s for check C0, with the box path and offset; a table's
//! entry count is checked against its box's size before anything is allocated.

use super::{BoxNode, Bytes, FourCc, Mp4, Problem};

/// `mvhd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mvhd {
    /// Units per second.
    pub timescale: u32,
    /// The movie's length, in its timescale.
    pub duration: u64,
    /// The next free track id.
    pub next_track_id: u32,
}

/// `tkhd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tkhd {
    /// Flags: 1 enabled, 2 in movie, 4 in preview.
    pub flags: u32,
    /// The track id.
    pub track_id: u32,
    /// The track's length, in the movie's timescale.
    pub duration: u64,
    /// The alternate group: tracks in one group are alternatives.
    pub alternate_group: i16,
    /// Width, 16.16 fixed point.
    pub width: u32,
    /// Height, 16.16 fixed point.
    pub height: u32,
}

/// `mdhd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mdhd {
    /// Units per second.
    pub timescale: u32,
    /// The media's length, in its timescale.
    pub duration: u64,
    /// ISO 639-2/T language, packed.
    pub language: [u8; 3],
}

/// `hdlr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hdlr {
    /// `vide`, `soun`, …
    pub handler: FourCc,
    /// Its name, up to the first NUL.
    pub name: String,
}

/// One `elst` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    /// Length in the movie's timescale.
    pub segment_duration: u64,
    /// Where in the media it starts; −1 for an empty edit.
    pub media_time: i64,
    /// Rate, 16.16.
    pub rate: i32,
}

/// A `colr` box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colr {
    /// `nclx`: H.273 code points and the range flag.
    Nclx {
        /// Colour primaries.
        primaries: u16,
        /// Transfer characteristics.
        transfer: u16,
        /// Matrix coefficients.
        matrix: u16,
        /// Full range.
        full_range: bool,
        /// The 7 reserved bits after the flag (should be 0).
        reserved: u8,
    },
    /// Another colour type (`nclc`, `prof`, `rICC`) or a short box.
    Other(FourCc),
}

/// A sample entry in `stsd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleEntry {
    /// A visual entry (`avc1` and kin).
    Visual {
        /// Its type.
        kind: FourCc,
        /// Width in pixels.
        width: u16,
        /// Height in pixels.
        height: u16,
        /// Each `avcC` payload.
        avcc: Vec<Vec<u8>>,
        /// Each `colr`.
        colr: Vec<Colr>,
        /// `pasp` as (h_spacing, v_spacing).
        pasp: Option<(u32, u32)>,
    },
    /// An audio entry (`mp4a` and kin).
    Audio {
        /// Its type.
        kind: FourCc,
        /// Channels.
        channels: u16,
        /// The sample rate's integer part.
        sample_rate: u32,
        /// Each `esds` payload (after version and flags).
        esds: Vec<Vec<u8>>,
    },
    /// Anything else.
    Other(FourCc),
}

/// One `stsc` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRun {
    /// The first chunk of the run, from 1.
    pub first_chunk: u32,
    /// Samples in each of its chunks.
    pub samples_per_chunk: u32,
    /// Which sample entry, from 1.
    pub description_index: u32,
}

/// `dref`: its declared entry count and each entry's type and flags (1: the media is in this
/// file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dref {
    /// The entry count it declares.
    pub count: u32,
    /// Each entry box's type and flags.
    pub entries: Vec<(FourCc, u32)>,
}

/// A sample: where it is and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// The file offset of its first byte.
    pub offset: u64,
    /// Its size in bytes.
    pub size: u32,
    /// Its decode time, in the media timescale.
    pub dts: u64,
    /// Its duration.
    pub duration: u32,
    /// Its sample entry, from 1.
    pub description_index: u32,
}

/// A track: its boxes as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Its box path, e.g. `moov/trak[1]`.
    pub path: String,
    /// The file offset of its `stbl` (of the `trak` if it has none), for messages.
    pub stbl_offset: u64,
    /// `tkhd`.
    pub tkhd: Option<Tkhd>,
    /// `mdhd`.
    pub mdhd: Option<Mdhd>,
    /// `hdlr`.
    pub hdlr: Option<Hdlr>,
    /// Whether it has an `edts`.
    pub has_edts: bool,
    /// The `elst` entries.
    pub edits: Vec<Edit>,
    /// `udta/name`, up to the first NUL.
    pub udta_name: Option<String>,
    /// `mdia/minf/dinf/dref`.
    pub dref: Option<Dref>,
    /// The `stsd` entries.
    pub entries: Vec<SampleEntry>,
    /// `stts` as (count, delta) runs.
    pub stts: Vec<(u32, u32)>,
    /// `ctts` as (count, offset) runs, if present.
    pub ctts: Option<Vec<(u32, i64)>>,
    /// `stss` sample numbers (from 1), if present.
    pub stss: Option<Vec<u32>>,
    /// Each sample's size, from `stsz`.
    pub sizes: Vec<u32>,
    /// `stsc`.
    pub stsc: Vec<ChunkRun>,
    /// Chunk offsets, from `stco` or `co64`.
    pub chunks: Vec<u64>,
    /// Whether the chunk offsets came from `co64`.
    pub co64: bool,
    /// How many of each box C0 counts this track holds.
    pub counts: Vec<(FourCc, usize)>,
}

/// The movie: `mvhd`, its tracks and its `ilst` keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movie {
    /// `mvhd`.
    pub mvhd: Option<Mvhd>,
    /// The tracks, in file order.
    pub tracks: Vec<Track>,
    /// The keys in `moov/udta/meta/ilst`, in order.
    pub ilst_keys: Vec<FourCc>,
    /// How many `mvhd` boxes.
    pub mvhd_count: usize,
}

/// The most tracks read; Kairo writes at most six.
pub const MAX_TRACKS: usize = 64;
/// The most samples read in all tracks together: 2²⁵, about three times what an hour of a
/// 4-track Kairo recording holds. It bounds the size tables at 128 MiB.
pub const MAX_SAMPLES: u64 = 1 << 25;
/// The most video samples laid out one by one: 2²², about 19 hours at 60 fps.
pub const MAX_VIDEO_SAMPLES: usize = 1 << 22;

/// The boxes C0 counts in each track: exactly one of the first eleven (one of `stco`/`co64`)
/// and at most one of the rest.
pub const COUNTED: [&[u8; 4]; 15] = [
    b"tkhd", b"mdia", b"mdhd", b"hdlr", b"minf", b"stbl", b"stsd", b"stts", b"stsc", b"stsz",
    b"stco", b"co64", b"stss", b"ctts", b"edts",
];

impl Movie {
    /// Reads the movie from `mp4`'s `moov`, adding problems to `problems`.
    pub fn read(mp4: &Mp4, problems: &mut Vec<Problem>) -> Option<Movie> {
        let moov = mp4.moov.as_ref()?;
        let bytes = mp4.moov_bytes();
        let mut movie = Movie {
            mvhd: None,
            tracks: Vec::new(),
            ilst_keys: Vec::new(),
            mvhd_count: moov.children_of(b"mvhd").count(),
        };
        if let Some(node) = moov.child(b"mvhd") {
            movie.mvhd = read_mvhd(&bytes, node);
            if movie.mvhd.is_none() {
                problems.push(bad(node, "moov/mvhd", "mvhd too short"));
            }
        }
        if let Some(ilst) = moov.at(&[b"udta", b"meta", b"ilst"]) {
            movie.ilst_keys = ilst.children.iter().map(|c| c.kind).collect();
        }
        let mut budget = MAX_SAMPLES;
        for (index, trak) in moov.children_of(b"trak").enumerate() {
            let path = format!("moov/trak[{}]", index + 1);
            if index == MAX_TRACKS {
                problems.push(bad(
                    trak,
                    &path,
                    &format!("more than {MAX_TRACKS} tracks: too large"),
                ));
                break;
            }
            let track = read_track(&bytes, trak, &path, mp4.file_len, &mut budget, problems);
            movie.tracks.push(track);
        }
        Some(movie)
    }
}

fn bad(node: &BoxNode, path: &str, what: &str) -> Problem {
    Problem {
        path: path.to_owned(),
        offset: node.offset,
        what: what.to_owned(),
    }
}

/// Version, flags and the payload after them.
fn full<'a>(bytes: &Bytes<'a>, node: &BoxNode) -> Option<(u8, u32, Bytes<'a>)> {
    let payload = bytes.payload(node)?;
    let version = payload.u8(0)?;
    let flags = payload.u24(1)?;
    let rest = payload.get(4, payload.len().checked_sub(4)?)?;
    Some((version, flags, Bytes::new(rest, payload.offset_of(4))))
}

fn read_mvhd(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Mvhd> {
    let (version, _, p) = full(bytes, node)?;
    // Version 1's 64-bit times move everything after them 12 bytes.
    let (timescale, duration, next_at) = if version == 1 {
        (p.u32(16)?, p.u64(20)?, 104)
    } else {
        (p.u32(8)?, u64::from(p.u32(12)?), 92)
    };
    Some(Mvhd {
        timescale,
        duration,
        next_track_id: p.u32(next_at)?,
    })
}

fn read_tkhd(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Tkhd> {
    let (version, flags, p) = full(bytes, node)?;
    // Version 1 has 64-bit times, so everything after them moves 12 bytes.
    let (track_id, duration, shift) = if version == 1 {
        (p.u32(16)?, p.u64(24)?, 12)
    } else {
        (p.u32(8)?, u64::from(p.u32(16)?), 0)
    };
    Some(Tkhd {
        flags,
        track_id,
        duration,
        alternate_group: p.u16(30 + shift)? as i16,
        width: p.u32(72 + shift)?,
        height: p.u32(76 + shift)?,
    })
}

fn read_mdhd(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Mdhd> {
    let (version, _, p) = full(bytes, node)?;
    let (timescale, duration, lang_at) = if version == 1 {
        (p.u32(16)?, p.u64(20)?, 28)
    } else {
        (p.u32(8)?, u64::from(p.u32(12)?), 16)
    };
    let packed = p.u16(lang_at)?;
    let letter = |shift: u16| (((packed >> shift) & 0x1F) as u8).wrapping_add(0x60);
    Some(Mdhd {
        timescale,
        duration,
        language: [letter(10), letter(5), letter(0)],
    })
}

/// Text up to the first NUL, lossily.
fn text(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

fn read_hdlr(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Hdlr> {
    let (_, _, p) = full(bytes, node)?;
    let handler = FourCc(p.get(4, 4)?.try_into().ok()?);
    let name = p.get(20, p.len().checked_sub(20)?)?;
    Some(Hdlr {
        handler,
        name: text(name),
    })
}

/// A table's entries: `count` entries of `width` bytes after `skip` bytes, or `None` if they
/// don't fit the payload.
fn entries<'a>(p: &Bytes<'a>, skip: usize, count: u32, width: usize) -> Option<Vec<Bytes<'a>>> {
    let count = usize::try_from(count).ok()?;
    let need = count.checked_mul(width)?.checked_add(skip)?;
    if need > p.len() {
        return None;
    }
    Some(
        (0..count)
            .map(|i| {
                let at = skip + i * width;
                Bytes::new(&p.data()[at..at + width], p.offset_of(at))
            })
            .collect(),
    )
}

fn read_elst(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Vec<Edit>> {
    let (version, _, p) = full(bytes, node)?;
    let width = if version == 1 { 20 } else { 12 };
    entries(&p, 4, p.u32(0)?, width)?
        .iter()
        .map(|e| {
            Some(if version == 1 {
                Edit {
                    segment_duration: e.u64(0)?,
                    media_time: e.u64(8)? as i64,
                    rate: e.u32(16)? as i32,
                }
            } else {
                Edit {
                    segment_duration: u64::from(e.u32(0)?),
                    media_time: i64::from(e.u32(4)? as i32),
                    rate: e.u32(8)? as i32,
                }
            })
        })
        .collect()
}

fn read_pairs(bytes: &Bytes<'_>, node: &BoxNode) -> Option<(u8, Vec<(u32, u32)>)> {
    let (version, _, p) = full(bytes, node)?;
    let runs = entries(&p, 4, p.u32(0)?, 8)?
        .iter()
        .map(|e| Some((e.u32(0)?, e.u32(4)?)))
        .collect::<Option<Vec<_>>>()?;
    Some((version, runs))
}

fn read_stss(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Vec<u32>> {
    let (_, _, p) = full(bytes, node)?;
    entries(&p, 4, p.u32(0)?, 4)?
        .iter()
        .map(|e| e.u32(0))
        .collect()
}

/// `stsz`'s sizes, counted against `budget` (samples left for all tracks) before anything is
/// allocated.
fn read_stsz(
    bytes: &Bytes<'_>,
    node: &BoxNode,
    file_len: u64,
    budget: &mut u64,
) -> Result<Vec<u32>, String> {
    let malformed = || "stsz malformed or cut short".to_owned();
    let (_, _, p) = full(bytes, node).ok_or_else(malformed)?;
    let (uniform, count) = (
        p.u32(0).ok_or_else(malformed)?,
        p.u32(4).ok_or_else(malformed)?,
    );
    if u64::from(count) > *budget {
        return Err(format!(
            "more than {MAX_SAMPLES} samples in all tracks: too large"
        ));
    }
    let sizes = if uniform != 0 {
        // Every sample lies inside the file, which bounds a uniform table before it is expanded.
        if u64::from(count).saturating_mul(u64::from(uniform)) > file_len {
            return Err(format!(
                "stsz declares {count} samples of {uniform} bytes, more than the file holds"
            ));
        }
        vec![uniform; count as usize]
    } else {
        entries(&p, 8, count, 4)
            .ok_or_else(malformed)?
            .iter()
            .map(|e| e.u32(0))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(malformed)?
    };
    *budget -= u64::from(count);
    Ok(sizes)
}

fn read_stsc(bytes: &Bytes<'_>, node: &BoxNode) -> Option<Vec<ChunkRun>> {
    let (_, _, p) = full(bytes, node)?;
    entries(&p, 4, p.u32(0)?, 12)?
        .iter()
        .map(|e| {
            Some(ChunkRun {
                first_chunk: e.u32(0)?,
                samples_per_chunk: e.u32(4)?,
                description_index: e.u32(8)?,
            })
        })
        .collect()
}

fn read_chunks(bytes: &Bytes<'_>, node: &BoxNode, wide: bool) -> Option<Vec<u64>> {
    let (_, _, p) = full(bytes, node)?;
    let width = if wide { 8 } else { 4 };
    entries(&p, 4, p.u32(0)?, width)?
        .iter()
        .map(|e| {
            if wide {
                e.u64(0)
            } else {
                e.u32(0).map(u64::from)
            }
        })
        .collect()
}

fn read_colr(bytes: &Bytes<'_>, node: &BoxNode) -> Colr {
    let parse = || {
        let p = bytes.payload(node)?;
        let kind = FourCc(p.get(0, 4)?.try_into().ok()?);
        if kind.0 != *b"nclx" {
            return Some(Colr::Other(kind));
        }
        let flags = p.u8(10)?;
        Some(Colr::Nclx {
            primaries: p.u16(4)?,
            transfer: p.u16(6)?,
            matrix: p.u16(8)?,
            full_range: flags & 0x80 != 0,
            reserved: flags & 0x7F,
        })
    };
    parse().unwrap_or(Colr::Other(FourCc::new(b"????")))
}

fn read_entry(bytes: &Bytes<'_>, node: &BoxNode) -> SampleEntry {
    let Some(p) = bytes.payload(node) else {
        return SampleEntry::Other(node.kind);
    };
    let payload_of = |child: &BoxNode| bytes.payload(child).map(|b| b.data().to_vec());
    match &node.kind.0 {
        b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"av01" | b"encv" => SampleEntry::Visual {
            kind: node.kind,
            width: p.u16(24).unwrap_or(0),
            height: p.u16(26).unwrap_or(0),
            avcc: node.children_of(b"avcC").filter_map(payload_of).collect(),
            colr: node
                .children_of(b"colr")
                .map(|c| read_colr(bytes, c))
                .collect(),
            pasp: node
                .child(b"pasp")
                .and_then(|c| bytes.payload(c))
                .and_then(|b| Some((b.u32(0)?, b.u32(4)?))),
        },
        b"mp4a" | b"enca" => SampleEntry::Audio {
            kind: node.kind,
            channels: p.u16(16).unwrap_or(0),
            sample_rate: p.u32(24).map_or(0, |r| r >> 16),
            esds: node
                .children_of(b"esds")
                .filter_map(|c| full(bytes, c).map(|(_, _, rest)| rest.data().to_vec()))
                .collect(),
        },
        _ => SampleEntry::Other(node.kind),
    }
}

fn read_track(
    bytes: &Bytes<'_>,
    trak: &BoxNode,
    path: &str,
    file_len: u64,
    budget: &mut u64,
    problems: &mut Vec<Problem>,
) -> Track {
    let mdia = trak.child(b"mdia");
    let minf = mdia.and_then(|m| m.child(b"minf"));
    let stbl = minf.and_then(|m| m.child(b"stbl"));
    let mut counts: Vec<(FourCc, usize)> = Vec::new();
    for kind in COUNTED {
        let n = match &kind[..] {
            b"tkhd" | b"mdia" | b"edts" => trak.children_of(kind).count(),
            b"mdhd" | b"hdlr" | b"minf" => mdia.map_or(0, |m| m.children_of(kind).count()),
            b"stbl" => minf.map_or(0, |m| m.children_of(kind).count()),
            _ => stbl.map_or(0, |s| s.children_of(kind).count()),
        };
        counts.push((FourCc(*kind), n));
    }
    let mut track = Track {
        path: path.to_owned(),
        stbl_offset: stbl.map_or(trak.offset, |s| s.offset),
        tkhd: None,
        mdhd: None,
        hdlr: None,
        has_edts: trak.child(b"edts").is_some(),
        edits: Vec::new(),
        udta_name: None,
        dref: None,
        entries: Vec::new(),
        stts: Vec::new(),
        ctts: None,
        stss: None,
        sizes: Vec::new(),
        stsc: Vec::new(),
        chunks: Vec::new(),
        co64: false,
        counts,
    };
    // A problem found while `check` holds `problems`, pushed after it.
    let mut late = None;
    // `at` is the box's path inside the track, e.g. `mdia/minf/stbl/stsz`.
    let mut check = |node: &BoxNode, at: &str, ok: bool| {
        if !ok {
            let what = format!("{} malformed or cut short", node.kind);
            problems.push(bad(node, &format!("{path}/{at}"), &what));
        }
    };
    if let Some(node) = trak.child(b"tkhd") {
        track.tkhd = read_tkhd(bytes, node);
        check(node, "tkhd", track.tkhd.is_some());
    }
    if let Some(node) = mdia.and_then(|m| m.child(b"mdhd")) {
        track.mdhd = read_mdhd(bytes, node);
        check(node, "mdia/mdhd", track.mdhd.is_some());
    }
    if let Some(node) = mdia.and_then(|m| m.child(b"hdlr")) {
        track.hdlr = read_hdlr(bytes, node);
        check(node, "mdia/hdlr", track.hdlr.is_some());
    }
    if let Some(node) = trak.at(&[b"edts", b"elst"]) {
        let edits = read_elst(bytes, node);
        check(node, "edts/elst", edits.is_some());
        track.edits = edits.unwrap_or_default();
    }
    if let Some(node) = trak.at(&[b"udta", b"name"]) {
        track.udta_name = bytes.payload(node).map(|p| text(p.data()));
    }
    if let Some(node) = minf.and_then(|m| m.at(&[b"dinf", b"dref"])) {
        track.dref = full(bytes, node).and_then(|(_, _, p)| {
            let entries = node
                .children
                .iter()
                .map(|e| (e.kind, full(bytes, e).map_or(0, |(_, flags, _)| flags)))
                .collect();
            Some(Dref {
                count: p.u32(0)?,
                entries,
            })
        });
        check(node, "mdia/minf/dinf/dref", track.dref.is_some());
    }
    if let Some(stbl) = stbl {
        if let Some(stsd) = stbl.child(b"stsd") {
            track.entries = stsd.children.iter().map(|e| read_entry(bytes, e)).collect();
        }
        if let Some(node) = stbl.child(b"stts") {
            let runs = read_pairs(bytes, node);
            check(node, "mdia/minf/stbl/stts", runs.is_some());
            track.stts = runs.map(|(_, r)| r).unwrap_or_default();
        }
        if let Some(node) = stbl.child(b"ctts") {
            let runs = read_pairs(bytes, node);
            check(node, "mdia/minf/stbl/ctts", runs.is_some());
            // Version 0 offsets are unsigned in the standard but written as signed by muxers
            // that use negative offsets; version 1 is signed.
            track.ctts = runs.map(|(_, r)| {
                r.into_iter()
                    .map(|(count, offset)| (count, i64::from(offset as i32)))
                    .collect()
            });
        }
        if let Some(node) = stbl.child(b"stss") {
            track.stss = read_stss(bytes, node);
            check(node, "mdia/minf/stbl/stss", track.stss.is_some());
        }
        if let Some(node) = stbl.child(b"stsz") {
            match read_stsz(bytes, node, file_len, budget) {
                Ok(sizes) => track.sizes = sizes,
                Err(what) => late = Some(bad(node, &format!("{path}/mdia/minf/stbl/stsz"), &what)),
            }
        }
        if let Some(node) = stbl.child(b"stsc") {
            let runs = read_stsc(bytes, node);
            check(node, "mdia/minf/stbl/stsc", runs.is_some());
            track.stsc = runs.unwrap_or_default();
        }
        let (stco, co64) = (stbl.child(b"stco"), stbl.child(b"co64"));
        if let Some(node) = co64.or(stco) {
            let chunks = read_chunks(bytes, node, co64.is_some());
            let at = if co64.is_some() {
                "mdia/minf/stbl/co64"
            } else {
                "mdia/minf/stbl/stco"
            };
            check(node, at, chunks.is_some());
            track.chunks = chunks.unwrap_or_default();
            track.co64 = co64.is_some();
        }
    }
    problems.extend(late);
    track
}

impl Track {
    /// The handler type, if any.
    pub fn handler(&self) -> Option<FourCc> {
        self.hdlr.as_ref().map(|h| h.handler)
    }

    /// The number of samples per `stts`.
    pub fn stts_samples(&self) -> u64 {
        self.stts
            .iter()
            .fold(0_u64, |n, &(count, _)| n.saturating_add(u64::from(count)))
    }

    /// The sum of `stts` durations, in the media timescale.
    pub fn stts_duration(&self) -> u64 {
        self.stts.iter().fold(0_u64, |n, &(count, delta)| {
            n.saturating_add(u64::from(count) * u64::from(delta))
        })
    }

    /// The first sample's duration.
    pub fn first_delta(&self) -> Option<u32> {
        self.stts
            .iter()
            .find(|&&(count, _)| count > 0)
            .map(|&(_, delta)| delta)
    }

    /// Every sample's duration in order, from `stts` as it stands.
    pub fn deltas(&self) -> impl Iterator<Item = u32> + '_ {
        self.stts
            .iter()
            .flat_map(|&(count, delta)| std::iter::repeat_n(delta, count as usize))
    }

    /// The bytes of all samples.
    pub fn total_bytes(&self) -> u64 {
        self.sizes.iter().map(|&s| u64::from(s)).sum()
    }

    /// Walks the chunks that `stsc` lays out over the chunk offsets, calling `visit` with each
    /// chunk's file offset, its first sample's index, its sample count and its sample entry.
    /// Fails with a description when the tables disagree (container.md §6 C0): `stts` must
    /// count the samples `stsz` sizes, `stsc` must start at chunk 1 and rise, and its chunks
    /// must hold exactly those samples. Allocates nothing.
    pub fn walk_chunks(&self, mut visit: impl FnMut(u64, usize, u32, u32)) -> Result<(), String> {
        let n = self.sizes.len();
        if self.stts_samples() != n as u64 {
            return Err(format!(
                "stsz has {n} samples but stts {}",
                self.stts_samples()
            ));
        }
        let chunks = self.chunks.len() as u64;
        let mut laid = 0_usize;
        for (run_index, run) in self.stsc.iter().enumerate() {
            let next_first = self
                .stsc
                .get(run_index + 1)
                .map_or(chunks + 1, |r| u64::from(r.first_chunk));
            let first = u64::from(run.first_chunk);
            let previous = run_index
                .checked_sub(1)
                .and_then(|i| self.stsc.get(i))
                .map_or(0, |r| u64::from(r.first_chunk));
            let starts_wrong = if run_index == 0 {
                first != 1
            } else {
                first <= previous
            };
            if starts_wrong || next_first <= first || first > chunks {
                return Err(format!(
                    "stsc run {} starts at chunk {first}",
                    run_index + 1
                ));
            }
            for chunk in first..next_first {
                let offset = *self
                    .chunks
                    .get(chunk as usize - 1)
                    .ok_or_else(|| format!("stsc names chunk {chunk} of {chunks}"))?;
                let count = run.samples_per_chunk;
                if laid.saturating_add(count as usize) > n {
                    return Err(format!("stsc lays out more than stsz's {n} samples"));
                }
                visit(offset, laid, count, run.description_index);
                laid += count as usize;
            }
        }
        if laid != n {
            return Err(format!("stsc lays out {laid} samples, stsz has {n}"));
        }
        Ok(())
    }

    /// Every sample, laid out from `stsc`, the chunk offsets, the sizes and `stts`; only the
    /// video track needs this. Fails with a description when the tables disagree, or past
    /// [`MAX_VIDEO_SAMPLES`].
    pub fn samples(&self) -> Result<Vec<Sample>, String> {
        let n = self.sizes.len();
        if n > MAX_VIDEO_SAMPLES {
            return Err(format!(
                "more than {MAX_VIDEO_SAMPLES} video samples: too large"
            ));
        }
        let mut samples = Vec::with_capacity(n);
        let mut durations = self.deltas();
        let mut dts = 0_u64;
        self.walk_chunks(|offset, first, count, description_index| {
            let mut offset = offset;
            for index in first..first + count as usize {
                let size = self.sizes.get(index).copied().unwrap_or(0);
                let duration = durations.next().unwrap_or(0);
                samples.push(Sample {
                    offset,
                    size,
                    dts,
                    duration,
                    description_index,
                });
                offset = offset.saturating_add(u64::from(size));
                dts = dts.saturating_add(u64::from(duration));
            }
        })?;
        Ok(samples)
    }
}
