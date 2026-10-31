// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-verify mutate`: one box-level defect in a sound clip, for the bad clips no encoder can
//! be asked to make (docs/research/16-verify-testbed/README.md §6.1, §7.3): `moov` after the
//! media, an edit list, a wrong or missing `colr`, a second default track, one uneven frame.
//!
//! The clip must pass `container.structure`. Its `moov` is rebuilt from the box tree with every
//! size recomputed (a 64-bit size stays 64-bit), the other top-level boxes are copied as they
//! are (a size-0 box that is no longer last gets its real size), and every chunk offset in
//! `stco` and `co64` moves with the bytes it points at. A clip with other absolute file offsets
//! (`iloc`, `saio`, a top-level `meta`) is refused, since they wouldn't move. `perturb-stts`
//! writes `stts` in its merged form, as encoders do. So, for the clips FFmpeg writes, the output
//! differs from the input only in the change named; the tests prove it by comparing each output
//! byte for byte with the same defect built from scratch.

use std::fmt;
use std::io::{self, Write};

use crate::Clip;
use crate::mp4::{BoxNode, Bytes, FourCc, ReadError, Source, TopBox, children_start};

#[cfg(test)]
mod tests;

/// How many bytes a copied box is read in at a time.
const COPY_CHUNK: u64 = 1 << 20;

/// One change to a clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `moov` moves after the media; the other top-level boxes keep their order.
    UndoFaststart,
    /// An identity edit list, one edit as long as the track, in every track.
    AddElst,
    /// The video's `nclx` `colr` gets these values in place; the SPS's VUI is left alone.
    SetColr {
        /// `colour_primaries`.
        primaries: u16,
        /// `transfer_characteristics`.
        transfer: u16,
        /// `matrix_coefficients`.
        matrix: u16,
        /// `full_range_flag`.
        full_range: bool,
    },
    /// The video's `colr` boxes go.
    RemoveColr,
    /// The track with this title (in `hdlr` or `udta/name`) is enabled, as a default track is
    /// (`tkhd` flag 1).
    SetDefault(String),
    /// Video sample `sample` (counted from 1, as `stss` counts) lasts `ticks` longer and the
    /// next one `ticks` shorter, so the total stays the same.
    PerturbStts {
        /// The sample that gets longer.
        sample: u32,
        /// How many ticks move.
        ticks: u32,
    },
}

impl fmt::Display for Op {
    /// The operation as the command line writes it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndoFaststart => write!(f, "undo-faststart"),
            Self::AddElst => write!(f, "add-elst"),
            Self::SetColr {
                primaries,
                transfer,
                matrix,
                full_range,
            } => write!(
                f,
                "set-colr {primaries} {transfer} {matrix} {}",
                u8::from(*full_range)
            ),
            Self::RemoveColr => write!(f, "remove-colr"),
            Self::SetDefault(title) => write!(f, "set-default {title:?}"),
            Self::PerturbStts { sample, ticks } => write!(f, "perturb-stts {sample} {ticks}"),
        }
    }
}

/// Why no output was written.
#[derive(Debug)]
pub enum MutateError {
    /// The input couldn't be read.
    Read(io::Error),
    /// The input isn't a clip this can rebuild.
    Unsound(String),
    /// The change doesn't apply to this clip, e.g. there is no `colr` to remove.
    NotApplicable(String),
    /// The output couldn't be written.
    Write(io::Error),
}

impl MutateError {
    /// The command line's exit code: 1 doesn't apply, 3 the input, 4 the output.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::NotApplicable(_) => 1,
            Self::Read(_) | Self::Unsound(_) => 3,
            Self::Write(_) => 4,
        }
    }
}

impl fmt::Display for MutateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "can't be read: {error}"),
            Self::Unsound(what) => write!(f, "can't be rebuilt: {what}"),
            Self::NotApplicable(what) => write!(f, "the change doesn't apply: {what}"),
            Self::Write(error) => write!(f, "the output can't be written: {error}"),
        }
    }
}

impl std::error::Error for MutateError {}

/// Writes the clip in `source`, with `op` applied, to `out`.
pub fn mutate(source: &mut dyn Source, op: &Op, out: &mut dyn Write) -> Result<(), MutateError> {
    rebuild(source, Some(op), out)
}

/// Writes the clip in `source` to `out` with `op` applied, or unchanged without one.
fn rebuild(
    source: &mut dyn Source,
    op: Option<&Op>,
    out: &mut dyn Write,
) -> Result<(), MutateError> {
    rebuild_with(source, out, |clip, moov, order| match op {
        Some(op) => apply(op, clip, moov, order),
        None => Ok(()),
    })
}

/// [`rebuild`] with any `edit` of the `moov` tree and the top-level order.
fn rebuild_with(
    source: &mut dyn Source,
    out: &mut dyn Write,
    edit: impl FnOnce(&Clip, &mut Node, &mut Vec<usize>) -> Result<(), MutateError>,
) -> Result<(), MutateError> {
    let clip = Clip::read(source).map_err(|e| match e {
        ReadError::Io(error) => MutateError::Read(error),
    })?;
    if let Some(problem) = clip.problems.first() {
        let place = if problem.path.is_empty() {
            "the top level".to_owned()
        } else {
            problem.path.clone()
        };
        return Err(MutateError::Unsound(format!(
            "{place}: {} (container.structure must pass)",
            problem.what
        )));
    }
    let tops = &clip.mp4.top;
    let (Some(tree), Some(moov_index)) = (
        clip.mp4.moov.as_ref(),
        tops.iter().position(|b| b.kind.0 == *b"moov"),
    ) else {
        return Err(MutateError::Unsound("no moov".to_owned()));
    };
    if let Some(top) = tops.iter().find(|b| b.kind.0 == *b"meta") {
        return Err(MutateError::Unsound(format!(
            "a top-level meta box at offset {} may hold file offsets this can't move",
            top.offset
        )));
    }
    let mut moov = Node::read(tree, &clip.mp4.moov_bytes())?;
    if let Some(kind) = moov.find_kind(&[b"iloc", b"saio"]) {
        return Err(MutateError::Unsound(format!(
            "a {kind} box holds file offsets this can't move"
        )));
    }
    let mut order: Vec<usize> = (0..tops.len()).collect();
    edit(&clip, &mut moov, &mut order)?;
    let placed = place(source, tops, &order, moov_index, &moov)?;
    for (table, width) in chunk_tables(&mut moov) {
        move_offsets(table, width, &placed)?;
    }
    let mut moov_bytes = Vec::new();
    moov.write(&mut moov_bytes);
    for p in &placed {
        if p.moov {
            out.write_all(&moov_bytes).map_err(MutateError::Write)?;
        } else if let Some(header) = &p.new_header {
            out.write_all(header).map_err(MutateError::Write)?;
            copy(
                source,
                p.old.offset + p.old.header,
                p.old.size - p.old.header,
                out,
            )?;
        } else {
            copy(source, p.old.offset, p.old.size, out)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The box tree as it will be written
// ---------------------------------------------------------------------------------------------

/// A box as it will be written: its type, the form of its header, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    kind: FourCc,
    /// A `uuid` box's extended type.
    uuid: Option<[u8; 16]>,
    /// Whether its size is written in 64 bits, as it was read.
    large: bool,
    body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Body {
    /// The payload as read.
    Raw(Vec<u8>),
    /// The fixed bytes before the children (a full box's version and flags, a sample entry's
    /// fields), then the children.
    Parent { fixed: Vec<u8>, children: Vec<Node> },
}

impl Node {
    /// `node` with its payload from `bytes`, read the way [`crate::mp4`] read it.
    fn read(node: &BoxNode, bytes: &Bytes<'_>) -> Result<Self, MutateError> {
        let unsound = || {
            MutateError::Unsound(format!(
                "the {} box at offset {} doesn't read back",
                node.kind, node.offset
            ))
        };
        let payload = bytes.payload(node).ok_or_else(unsound)?;
        let mut header = node.header;
        let uuid = if node.kind.0 == *b"uuid" {
            header -= 16;
            let at = node.offset + header - bytes.offset_of(0);
            let extended = usize::try_from(at)
                .ok()
                .and_then(|at| bytes.get(at, 16))
                .and_then(|b| <[u8; 16]>::try_from(b).ok())
                .ok_or_else(unsound)?;
            Some(extended)
        } else {
            None
        };
        let body = match children_start(&node.kind.0, &payload) {
            Some(skip) => {
                let fixed = payload.get(0, skip).ok_or_else(unsound)?.to_vec();
                // The reader found children that tile the rest of the payload exactly.
                let mut end = payload.offset_of(skip);
                let mut children = Vec::with_capacity(node.children.len());
                for child in &node.children {
                    if child.offset != end {
                        return Err(unsound());
                    }
                    end = end.saturating_add(child.size);
                    children.push(Node::read(child, bytes)?);
                }
                if end != payload.offset_of(payload.len()) {
                    return Err(unsound());
                }
                Body::Parent { fixed, children }
            }
            None => Body::Raw(payload.data().to_vec()),
        };
        Ok(Self {
            kind: node.kind,
            uuid,
            large: header == 16,
            body,
        })
    }

    /// A box that holds `payload`.
    fn raw(kind: &[u8; 4], payload: Vec<u8>) -> Self {
        Self {
            kind: FourCc(*kind),
            uuid: None,
            large: false,
            body: Body::Raw(payload),
        }
    }

    fn payload_len(&self) -> u64 {
        match &self.body {
            Body::Raw(bytes) => bytes.len() as u64,
            Body::Parent { fixed, children } => {
                fixed.len() as u64 + children.iter().map(Node::len).sum::<u64>()
            }
        }
    }

    /// The bytes before the payload, other than the size: the type and any extended type.
    fn extra_header(&self) -> u64 {
        8 + if self.uuid.is_some() { 16 } else { 0 }
    }

    /// Whether the size needs 64 bits.
    fn wide(&self, payload: u64) -> bool {
        self.large || self.extra_header() + payload > u64::from(u32::MAX)
    }

    /// Its whole size.
    fn len(&self) -> u64 {
        let payload = self.payload_len();
        payload + self.extra_header() + if self.wide(payload) { 8 } else { 0 }
    }

    fn write(&self, out: &mut Vec<u8>) {
        let payload = self.payload_len();
        let size = self.len();
        if self.wide(payload) {
            out.extend_from_slice(&1_u32.to_be_bytes());
            out.extend_from_slice(&self.kind.0);
            out.extend_from_slice(&size.to_be_bytes());
        } else {
            // `wide` is false, so the size fits.
            out.extend_from_slice(&(size as u32).to_be_bytes());
            out.extend_from_slice(&self.kind.0);
        }
        if let Some(extended) = &self.uuid {
            out.extend_from_slice(extended);
        }
        match &self.body {
            Body::Raw(bytes) => out.extend_from_slice(bytes),
            Body::Parent { fixed, children } => {
                out.extend_from_slice(fixed);
                for child in children {
                    child.write(out);
                }
            }
        }
    }

    /// The first box of one of these types in this tree, itself included.
    fn find_kind(&self, kinds: &[&[u8; 4]]) -> Option<FourCc> {
        if kinds.contains(&&self.kind.0) {
            return Some(self.kind);
        }
        self.children().iter().find_map(|c| c.find_kind(kinds))
    }

    fn children(&self) -> &[Node] {
        match &self.body {
            Body::Parent { children, .. } => children,
            Body::Raw(_) => &[],
        }
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        match &mut self.body {
            Body::Parent { children, .. } => Some(children),
            Body::Raw(_) => None,
        }
    }

    fn child_mut(&mut self, kind: &[u8; 4]) -> Option<&mut Node> {
        self.children_mut()?.iter_mut().find(|c| c.kind.0 == *kind)
    }

    /// The first box down `path`.
    fn at_mut(&mut self, path: &[&[u8; 4]]) -> Option<&mut Node> {
        path.iter()
            .try_fold(self, |node, kind| node.child_mut(kind))
    }

    fn raw_mut(&mut self) -> Option<&mut Vec<u8>> {
        match &mut self.body {
            Body::Raw(bytes) => Some(bytes),
            Body::Parent { .. } => None,
        }
    }

    /// The `trak` boxes, in file order: the same order as [`crate::mp4::tracks::Movie`]'s.
    fn traks_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.children_mut()
            .into_iter()
            .flatten()
            .filter(|c| c.kind.0 == *b"trak")
    }
}

// ---------------------------------------------------------------------------------------------
// The changes
// ---------------------------------------------------------------------------------------------

fn not_applicable(what: impl Into<String>) -> MutateError {
    MutateError::NotApplicable(what.into())
}

fn apply(op: &Op, clip: &Clip, moov: &mut Node, order: &mut Vec<usize>) -> Result<(), MutateError> {
    let unsound = |what: &str| MutateError::Unsound(what.to_owned());
    let tracks = clip
        .movie
        .as_ref()
        .map(|m| m.tracks.as_slice())
        .unwrap_or_default();
    match op {
        Op::UndoFaststart => {
            let top = &clip.mp4.top;
            let moov_at = top.iter().position(|b| b.kind.0 == *b"moov");
            let mdat_at = top.iter().position(|b| b.kind.0 == *b"mdat");
            let (Some(moov_at), Some(mdat_at)) = (moov_at, mdat_at) else {
                return Err(unsound("no moov or no mdat"));
            };
            if moov_at > mdat_at {
                return Err(not_applicable("the moov already follows the media"));
            }
            order.retain(|&i| i != moov_at);
            order.push(moov_at);
        }
        Op::AddElst => {
            for (k, trak) in moov.traks_mut().enumerate() {
                if trak.children().iter().any(|c| c.kind.0 == *b"edts") {
                    return Err(not_applicable(format!(
                        "track {} already has an edit list",
                        k + 1
                    )));
                }
                let duration = tracks
                    .get(k)
                    .and_then(|t| t.tkhd)
                    .map(|t| t.duration)
                    .ok_or_else(|| unsound("a track without tkhd"))?;
                let children = trak
                    .children_mut()
                    .ok_or_else(|| unsound("a trak that holds no boxes"))?;
                // Where ISO 14496-12 puts it: after tkhd and tref, before meta and mdia.
                let at = children
                    .iter()
                    .position(|c| matches!(&c.kind.0, b"meta" | b"mdia"))
                    .unwrap_or(children.len());
                children.insert(at, identity_edit(duration));
            }
        }
        Op::SetColr {
            primaries,
            transfer,
            matrix,
            full_range,
        } => {
            let (mut nclx, mut changed) = (0, 0);
            for entry in video_entries(moov, clip)? {
                for colr in entry.children_mut().into_iter().flatten() {
                    if colr.kind.0 != *b"colr" {
                        continue;
                    }
                    let Some(payload) = colr.raw_mut() else {
                        continue;
                    };
                    if payload.get(0..4) != Some(&b"nclx"[..]) || payload.len() < 11 {
                        continue;
                    }
                    let before = payload[4..11].to_vec();
                    payload[4..6].copy_from_slice(&primaries.to_be_bytes());
                    payload[6..8].copy_from_slice(&transfer.to_be_bytes());
                    payload[8..10].copy_from_slice(&matrix.to_be_bytes());
                    payload[10] = (u8::from(*full_range) << 7) | (payload[10] & 0x7f);
                    nclx += 1;
                    if payload[4..11] != before[..] {
                        changed += 1;
                    }
                }
            }
            if nclx == 0 {
                return Err(not_applicable("the video has no nclx colr box"));
            }
            if changed == 0 {
                return Err(not_applicable(format!(
                    "the video's colr already holds {primaries} {transfer} {matrix} {}",
                    u8::from(*full_range)
                )));
            }
        }
        Op::RemoveColr => {
            let mut removed = 0;
            for entry in video_entries(moov, clip)? {
                if let Some(children) = entry.children_mut() {
                    let before = children.len();
                    children.retain(|c| c.kind.0 != *b"colr");
                    removed += before - children.len();
                }
            }
            if removed == 0 {
                return Err(not_applicable("the video has no colr box"));
            }
        }
        Op::SetDefault(title) => {
            let titled: Vec<usize> = tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.hdlr.as_ref().is_some_and(|h| h.name == *title)
                        || t.udta_name.as_deref() == Some(title.as_str())
                })
                .map(|(k, _)| k)
                .collect();
            let k = match titled.as_slice() {
                [k] => *k,
                [] => return Err(not_applicable(format!("no track is titled {title:?}"))),
                many => {
                    return Err(not_applicable(format!(
                        "{} tracks are titled {title:?}",
                        many.len()
                    )));
                }
            };
            let tkhd = moov
                .traks_mut()
                .nth(k)
                .and_then(|t| t.child_mut(b"tkhd"))
                .and_then(Node::raw_mut)
                .filter(|p| p.len() >= 4)
                .ok_or_else(|| unsound("a track without tkhd"))?;
            // Flags are bytes 1 to 3; "enabled" is the lowest bit.
            if tkhd[3] & 1 == 1 {
                return Err(not_applicable(format!(
                    "the track titled {title:?} is already enabled"
                )));
            }
            tkhd[3] |= 1;
        }
        Op::PerturbStts { sample, ticks } => {
            let video = clip.video.ok_or_else(|| not_applicable("no H.264 track"))?;
            let stts = moov
                .traks_mut()
                .nth(video)
                .and_then(|t| t.at_mut(&[b"mdia", b"minf", b"stbl", b"stts"]))
                .and_then(Node::raw_mut)
                .ok_or_else(|| unsound("a video track without stts"))?;
            let runs = read_runs(stts).ok_or_else(|| unsound("an stts that doesn't parse"))?;
            let runs = perturb(&runs, *sample, *ticks).map_err(MutateError::NotApplicable)?;
            stts.truncate(4);
            stts.extend_from_slice(&(runs.len() as u32).to_be_bytes());
            for (count, delta) in runs {
                stts.extend_from_slice(&count.to_be_bytes());
                stts.extend_from_slice(&delta.to_be_bytes());
            }
        }
    }
    Ok(())
}

/// `edts` holding one identity edit as long as the track (`tkhd`'s duration, in the movie's
/// timescale): media time 0, rate 1. Version 0 unless the length needs 64 bits.
fn identity_edit(duration: u64) -> Node {
    let mut elst = Vec::new();
    match u32::try_from(duration) {
        Ok(short) => {
            elst.extend_from_slice(&[0, 0, 0, 0]);
            elst.extend_from_slice(&1_u32.to_be_bytes());
            elst.extend_from_slice(&short.to_be_bytes());
            elst.extend_from_slice(&0_i32.to_be_bytes());
        }
        Err(_) => {
            elst.extend_from_slice(&[1, 0, 0, 0]);
            elst.extend_from_slice(&1_u32.to_be_bytes());
            elst.extend_from_slice(&duration.to_be_bytes());
            elst.extend_from_slice(&0_i64.to_be_bytes());
        }
    }
    elst.extend_from_slice(&1_i16.to_be_bytes());
    elst.extend_from_slice(&0_i16.to_be_bytes());
    Node {
        kind: FourCc(*b"edts"),
        uuid: None,
        large: false,
        body: Body::Parent {
            fixed: Vec::new(),
            children: vec![Node::raw(b"elst", elst)],
        },
    }
}

/// The sample entries of the H.264 track.
fn video_entries<'a>(moov: &'a mut Node, clip: &Clip) -> Result<&'a mut Vec<Node>, MutateError> {
    let video = clip.video.ok_or_else(|| not_applicable("no H.264 track"))?;
    moov.traks_mut()
        .nth(video)
        .and_then(|t| t.at_mut(&[b"mdia", b"minf", b"stbl", b"stsd"]))
        .and_then(Node::children_mut)
        .ok_or_else(|| MutateError::Unsound("a video track without stsd".to_owned()))
}

/// `stts`'s (count, delta) runs, if its entry count matches its length.
fn read_runs(stts: &[u8]) -> Option<Vec<(u32, u32)>> {
    let count = u32::from_be_bytes(stts.get(4..8)?.try_into().ok()?) as usize;
    let entries = stts.get(8..)?;
    if entries.len() != count.checked_mul(8)? {
        return None;
    }
    let (runs, _) = entries.as_chunks::<8>();
    Some(
        runs.iter()
            .map(|e| {
                let [c0, c1, c2, c3, d0, d1, d2, d3] = *e;
                (
                    u32::from_be_bytes([c0, c1, c2, c3]),
                    u32::from_be_bytes([d0, d1, d2, d3]),
                )
            })
            .collect(),
    )
}

/// `runs` with sample `sample` (from 1) `ticks` longer and the next one `ticks` shorter,
/// written as `stts` usually is: runs of equal deltas merged, none empty.
fn perturb(runs: &[(u32, u32)], sample: u32, ticks: u32) -> Result<Vec<(u32, u32)>, String> {
    let total: u64 = runs.iter().map(|&(count, _)| u64::from(count)).sum();
    let (first, second) = (u64::from(sample), u64::from(sample) + 1);
    if sample == 0 || second > total {
        return Err(format!(
            "perturb-stts needs a sample from 1 to {}; the video has {total} samples",
            total.saturating_sub(1)
        ));
    }
    if ticks == 0 {
        return Err("perturb-stts needs at least 1 tick to move".to_owned());
    }
    let mut out: Vec<(u32, u32)> = Vec::new();
    let mut push = |count: u64, delta: u32| {
        // Counts come from one input run, so they fit.
        let count = count as u32;
        if count == 0 {
            return;
        }
        match out.last_mut() {
            Some((n, d)) if *d == delta => *n += count,
            _ => out.push((count, delta)),
        }
    };
    let mut start = 1_u64;
    for &(count, delta) in runs {
        let end = start + u64::from(count);
        let mut cursor = start;
        for target in [first, second] {
            if !(start..end).contains(&target) {
                continue;
            }
            let changed = if target == first {
                delta
                    .checked_add(ticks)
                    .ok_or_else(|| format!("sample {sample} would last more than 2³² ticks"))?
            } else {
                delta.checked_sub(ticks).filter(|&d| d > 0).ok_or_else(|| {
                    format!(
                        "sample {} lasts {delta} ticks, so it can't give {ticks}",
                        sample + 1
                    )
                })?
            };
            push(target - cursor, delta);
            push(1, changed);
            cursor = target + 1;
        }
        push(end - cursor, delta);
        start = end;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Placing the top-level boxes and moving the chunk offsets
// ---------------------------------------------------------------------------------------------

/// A top-level box in the output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Placed {
    /// The box as read.
    old: TopBox,
    /// Whether it is the rebuilt `moov`.
    moov: bool,
    /// Where it goes.
    offset: u64,
    /// Its header length there.
    header: u64,
    /// A new header, for a box whose size was 0 ("to the end of the file") and that is no
    /// longer last; `None` copies the box as it is.
    new_header: Option<Vec<u8>>,
}

/// Where each top-level box goes, in `order`, with `moov` rebuilt.
fn place(
    source: &mut dyn Source,
    tops: &[TopBox],
    order: &[usize],
    moov_index: usize,
    moov: &Node,
) -> Result<Vec<Placed>, MutateError> {
    let mut placed = Vec::with_capacity(order.len());
    let mut offset = 0_u64;
    for (position, &index) in order.iter().enumerate() {
        let old = *tops
            .get(index)
            .ok_or_else(|| MutateError::Unsound("a box went missing".to_owned()))?;
        let last = position + 1 == order.len();
        let (header, new_header, len) = if index == moov_index {
            (moov.len() - moov.payload_len(), None, moov.len())
        } else {
            let mut size_field = [0_u8; 4];
            source
                .read_at(old.offset, &mut size_field)
                .map_err(MutateError::Read)?;
            if u32::from_be_bytes(size_field) == 0 && !last {
                let header = to_sized(source, &old)?;
                let len = old.size - old.header + header.len() as u64;
                (header.len() as u64, Some(header), len)
            } else {
                (old.header, None, old.size)
            }
        };
        placed.push(Placed {
            old,
            moov: index == moov_index,
            offset,
            header,
            new_header,
        });
        offset = offset.saturating_add(len);
    }
    Ok(placed)
}

/// The header that gives `old`, a box whose size field was 0, its real size.
fn to_sized(source: &mut dyn Source, old: &TopBox) -> Result<Vec<u8>, MutateError> {
    let mut extended = vec![0_u8; usize::try_from(old.header - 8).unwrap_or(0)];
    source
        .read_at(old.offset + 8, &mut extended)
        .map_err(MutateError::Read)?;
    let mut header = Vec::with_capacity(32);
    match u32::try_from(old.size) {
        Ok(size) => header.extend_from_slice(&size.to_be_bytes()),
        Err(_) => header.extend_from_slice(&1_u32.to_be_bytes()),
    }
    header.extend_from_slice(&old.kind.0);
    if u32::try_from(old.size).is_err() {
        header.extend_from_slice(&(old.size + 8).to_be_bytes());
    }
    header.extend_from_slice(&extended);
    Ok(header)
}

/// The chunk offset tables, `stco` and `co64`, of every track, with their entry width.
fn chunk_tables(moov: &mut Node) -> Vec<(&mut Vec<u8>, usize)> {
    let mut tables = Vec::new();
    for trak in moov.traks_mut() {
        let Some(stbl) = trak.at_mut(&[b"mdia", b"minf", b"stbl"]) else {
            continue;
        };
        for child in stbl.children_mut().into_iter().flatten() {
            let width = match &child.kind.0 {
                b"stco" => 4,
                b"co64" => 8,
                _ => continue,
            };
            if let Some(payload) = child.raw_mut() {
                tables.push((payload, width));
            }
        }
    }
    tables
}

/// Moves every offset in one chunk offset table, of `width`-byte entries, to where its bytes
/// land.
fn move_offsets(table: &mut [u8], width: usize, placed: &[Placed]) -> Result<(), MutateError> {
    let unsound = || MutateError::Unsound("a chunk offset table that doesn't parse".to_owned());
    let count = table
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or_else(unsound)? as usize;
    let entries = table.get_mut(8..).ok_or_else(unsound)?;
    if Some(entries.len()) != count.checked_mul(width) {
        return Err(unsound());
    }
    for entry in entries.chunks_exact_mut(width) {
        let old = if width == 4 {
            u64::from(u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]))
        } else {
            u64::from_be_bytes([
                entry[0], entry[1], entry[2], entry[3], entry[4], entry[5], entry[6], entry[7],
            ])
        };
        let new = moved(old, placed)?;
        if width == 4 {
            let new = u32::try_from(new).map_err(|_| {
                not_applicable("the media would move past 4 GiB, which stco can't point at")
            })?;
            entry.copy_from_slice(&new.to_be_bytes());
        } else {
            entry.copy_from_slice(&new.to_be_bytes());
        }
    }
    Ok(())
}

/// Where the byte at `offset` lands: the same place in the payload of the box that holds it.
fn moved(offset: u64, placed: &[Placed]) -> Result<u64, MutateError> {
    placed
        .iter()
        .filter(|p| !p.moov)
        .find(|p| {
            // A chunk of no bytes may sit at the very end of a payload, as container.structure
            // allows; the next box's payload starts at least 8 bytes later.
            offset >= p.old.offset + p.old.header
                && offset <= p.old.offset.saturating_add(p.old.size)
        })
        .map(|p| p.offset + p.header + (offset - p.old.offset - p.old.header))
        .ok_or_else(|| {
            MutateError::Unsound(format!(
                "chunk offset {offset} points outside every media box"
            ))
        })
}

/// Copies `len` bytes from `offset` in `source` to `out`.
fn copy(
    source: &mut dyn Source,
    offset: u64,
    len: u64,
    out: &mut dyn Write,
) -> Result<(), MutateError> {
    let mut buf = vec![0_u8; usize::try_from(len.min(COPY_CHUNK)).unwrap_or(0)];
    let mut done = 0_u64;
    while done < len {
        let n = (len - done).min(COPY_CHUNK) as usize;
        source
            .read_at(offset + done, &mut buf[..n])
            .map_err(MutateError::Read)?;
        out.write_all(&buf[..n]).map_err(MutateError::Write)?;
        done += n as u64;
    }
    Ok(())
}
