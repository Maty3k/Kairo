// SPDX-License-Identifier: GPL-3.0-or-later
//! An MP4 as written (docs/research/16-verify-testbed/container.md §3 and §5.1; README §3.3).
//!
//! A verifier has to see the file as it is: box order, duplicates, unknown boxes, 32- or 64-bit
//! headers, overruns. So this is a small reader of its own, not a typed decoder that would tidy
//! those away. It reads the top-level box headers by positioned reads, then all of `moov`
//! (capped at [`MAX_MOOV_BYTES`]), and never the media in `mdat`: the H.264 checks fetch a few
//! bytes per NAL themselves. Every structural problem is recorded with its box path and offset
//! for check C0 instead of stopping the read, so the report can name all of them.

pub mod tracks;

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// The largest `moov` read: 64 MiB, far above a 2-minute 4-track clip's 0.5 MB.
pub const MAX_MOOV_BYTES: u64 = 64 << 20;
/// The deepest box nesting followed.
pub const MAX_DEPTH: usize = 16;
/// The most top-level boxes scanned.
pub const MAX_TOP_BOXES: usize = 100_000;
/// The most boxes read inside `moov`; a real 4-track Kairo moov holds about 100.
pub const MAX_MOOV_BOXES: usize = 100_000;

/// A four-character box type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FourCc(pub [u8; 4]);

impl FourCc {
    /// The type `t`, e.g. `FourCc::new(b"moov")`.
    pub const fn new(t: &[u8; 4]) -> Self {
        Self(*t)
    }
}

impl fmt::Display for FourCc {
    /// Printable bytes as they are, others as `\xNN` (e.g. `©nam` is `\xa9nam`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            if (0x20..0x7f).contains(&b) {
                write!(f, "{}", b as char)?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{self}'")
    }
}

/// Positioned reads from a file or from bytes.
pub trait Source {
    /// The length in bytes.
    fn size(&mut self) -> io::Result<u64>;
    /// Fills `buf` from `offset`, or fails.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl Source for &[u8] {
    fn size(&mut self) -> io::Result<u64> {
        Ok(<[u8]>::len(self) as u64)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let end = start
            .checked_add(buf.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        let bytes = self.get(start..end).ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(bytes);
        Ok(())
    }
}

impl Source for File {
    fn size(&mut self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)
    }
}

/// A structural problem, for check C0: where it is and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The box path, e.g. `moov/trak[2]/mdia/minf/stbl/stsz`.
    pub path: String,
    /// The file offset of the box or byte concerned.
    pub offset: u64,
    /// What is wrong.
    pub what: String,
}

/// A box in the tree: its type, place and children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxNode {
    /// Its type.
    pub kind: FourCc,
    /// The file offset of its header.
    pub offset: u64,
    /// Its header length: 8, 16 with a 64-bit size, plus 16 for `uuid`.
    pub header: u64,
    /// Its whole size, header included.
    pub size: u64,
    /// The boxes it holds, if it is a container this reader descends into.
    pub children: Vec<BoxNode>,
}

impl BoxNode {
    /// The first child of type `kind`.
    pub fn child(&self, kind: &[u8; 4]) -> Option<&BoxNode> {
        self.children.iter().find(|c| c.kind.0 == *kind)
    }

    /// Every child of type `kind`.
    pub fn children_of<'a>(&'a self, kind: &'a [u8; 4]) -> impl Iterator<Item = &'a BoxNode> {
        self.children.iter().filter(move |c| c.kind.0 == *kind)
    }

    /// The child at `path`, following the first match at each step.
    pub fn at(&self, path: &[&[u8; 4]]) -> Option<&BoxNode> {
        path.iter().try_fold(self, |node, kind| node.child(kind))
    }
}

/// Bytes read from a file, with the file offset of their first byte, and bounded accessors.
#[derive(Debug, Clone, Copy)]
pub struct Bytes<'a> {
    data: &'a [u8],
    base: u64,
}

impl<'a> Bytes<'a> {
    /// `data`, which starts at file offset `base`.
    pub fn new(data: &'a [u8], base: u64) -> Self {
        Self { data, base }
    }

    /// The payload of `node` (after its header), if it lies inside these bytes.
    pub fn payload(&self, node: &BoxNode) -> Option<Bytes<'a>> {
        let start = node
            .offset
            .checked_add(node.header)?
            .checked_sub(self.base)?;
        let end = node.offset.checked_add(node.size)?.checked_sub(self.base)?;
        let data = self
            .data
            .get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)?;
        Some(Bytes::new(data, node.offset + node.header))
    }

    /// The bytes as a slice.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// `len` bytes from `at`.
    pub fn get(&self, at: usize, len: usize) -> Option<&'a [u8]> {
        self.data.get(at..at.checked_add(len)?)
    }

    /// A big-endian u8 at `at`.
    pub fn u8(&self, at: usize) -> Option<u8> {
        self.data.get(at).copied()
    }

    /// A big-endian u16 at `at`.
    pub fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes(self.get(at, 2)?.try_into().ok()?))
    }

    /// A big-endian u24 at `at`.
    pub fn u24(&self, at: usize) -> Option<u32> {
        let b = self.get(at, 3)?;
        Some(u32::from_be_bytes([0, b[0], b[1], b[2]]))
    }

    /// A big-endian u32 at `at`.
    pub fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_be_bytes(self.get(at, 4)?.try_into().ok()?))
    }

    /// A big-endian u64 at `at`.
    pub fn u64(&self, at: usize) -> Option<u64> {
        Some(u64::from_be_bytes(self.get(at, 8)?.try_into().ok()?))
    }

    /// The file offset of byte `at`.
    pub fn offset_of(&self, at: usize) -> u64 {
        self.base.saturating_add(at as u64)
    }

    /// The length in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// A box header: type, header length and whole size (`None` for "to the end").
struct Header {
    kind: FourCc,
    header: u64,
    size: Option<u64>,
}

/// Reads a box header from the first bytes `head` of a box (at least 8; 16 for a 64-bit size,
/// 32 for a `uuid` with one).
fn header(head: &[u8]) -> Result<Header, String> {
    let small = head
        .get(0..4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or("box header cut short")?;
    let kind = head
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(FourCc)
        .ok_or("box header cut short")?;
    let (mut header, size) = match small {
        0 => (8, None),
        1 => {
            let large = head
                .get(8..16)
                .and_then(|b| b.try_into().ok())
                .map(u64::from_be_bytes)
                .ok_or("64-bit box size cut short")?;
            (16, Some(large))
        }
        n => (8, Some(u64::from(n))),
    };
    if kind.0 == *b"uuid" {
        header += 16;
    }
    if let Some(size) = size
        && size < header
    {
        return Err(format!(
            "{kind} box size {size} is below its header's {header} bytes"
        ));
    }
    Ok(Header { kind, header, size })
}

/// How many fixed bytes come before a box's children, or `None` if it holds no boxes this
/// reader follows.
pub(crate) fn children_start(kind: &[u8; 4], payload: &Bytes<'_>) -> Option<usize> {
    match kind {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"dinf" | b"edts" | b"udta" | b"mvex"
        | b"moof" | b"traf" | b"tref" | b"ilst" => Some(0),
        // Full boxes: version and flags first; `dref` and `stsd` then count their entries.
        // ISO's `meta` is a full box. QuickTime's has no version and flags, so its first
        // child's type (`hdlr`) sits at bytes 4..8, where an ISO meta has a child's size: a
        // size of "hdlr" would be 1.7 GB, so the peek can't misread an ISO meta.
        b"meta" => Some(if payload.get(4, 4) == Some(&b"hdlr"[..]) {
            0
        } else {
            4
        }),
        b"dref" | b"stsd" => Some(8),
        // Visual sample entries: 8 bytes of SampleEntry and 70 of VisualSampleEntry.
        b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"av01" | b"encv" => Some(78),
        // Audio sample entries: 8 + 20 bytes, plus QuickTime's version 1 or 2 fields.
        b"mp4a" | b"enca" => match payload.u16(8) {
            Some(1) => Some(28 + 16),
            Some(2) => Some(28 + 36),
            _ => Some(28),
        },
        _ => None,
    }
}

/// The state of one walk through `moov`: the problems found and the boxes read so far.
struct Walk<'p> {
    problems: &'p mut Vec<Problem>,
    boxes: usize,
    /// Set once the box budget is spent; the walk then stops everywhere.
    stopped: bool,
}

impl Walk<'_> {
    fn problem(&mut self, path: &str, offset: u64, what: String) {
        self.problems.push(Problem {
            path: path.to_owned(),
            offset,
            what,
        });
    }
}

/// Parses the boxes in `bytes` (the payload of the box at `path`, from `start`) into `out`.
fn parse_children(
    bytes: &Bytes<'_>,
    start: usize,
    path: &str,
    depth: usize,
    out: &mut Vec<BoxNode>,
    walk: &mut Walk<'_>,
) {
    let mut at = start;
    // How many of each type so far, for the `[n]` in paths.
    let mut seen: BTreeMap<FourCc, usize> = BTreeMap::new();
    while at < bytes.len() && !walk.stopped {
        let offset = bytes.offset_of(at);
        walk.boxes += 1;
        if walk.boxes > MAX_MOOV_BOXES {
            walk.problem(
                path,
                offset,
                format!("more than {MAX_MOOV_BOXES} boxes in moov: too large"),
            );
            walk.stopped = true;
            return;
        }
        let rest = &bytes.data()[at..];
        let head = match header(&rest[..rest.len().min(32)]) {
            Ok(head) => head,
            Err(what) => {
                walk.problem(path, offset, what);
                return;
            }
        };
        let Some(size) = head.size else {
            walk.problem(
                path,
                offset,
                format!("{} box with size 0 inside a box", head.kind),
            );
            return;
        };
        if size > (rest.len() as u64) {
            let what = format!(
                "{} box of {size} bytes overruns its parent by {} bytes",
                head.kind,
                size - rest.len() as u64
            );
            walk.problem(path, offset, what);
            return;
        }
        let count = {
            let n = seen.entry(head.kind).or_insert(0);
            *n += 1;
            *n
        };
        let child_path = if path.is_empty() {
            head.kind.to_string()
        } else if count > 1 || matches!(&head.kind.0, b"trak") {
            format!("{path}/{}[{count}]", head.kind)
        } else {
            format!("{path}/{}", head.kind)
        };
        let mut node = BoxNode {
            kind: head.kind,
            offset,
            header: head.header,
            size,
            children: Vec::new(),
        };
        if let Some(payload) = bytes.payload(&node)
            && let Some(skip) = children_start(&head.kind.0, &payload)
        {
            if depth >= MAX_DEPTH {
                walk.problem(
                    &child_path,
                    offset,
                    format!("boxes nested deeper than {MAX_DEPTH}"),
                );
            } else if skip > payload.len() {
                let what = format!("{} box too short for its {skip} fixed bytes", head.kind);
                walk.problem(&child_path, offset, what);
            } else {
                parse_children(
                    &payload,
                    skip,
                    &child_path,
                    depth + 1,
                    &mut node.children,
                    walk,
                );
            }
        }
        out.push(node);
        // `size` fits in `rest`, which is a usize.
        at += size as usize;
    }
}

/// A box at the top level of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopBox {
    /// Its type.
    pub kind: FourCc,
    /// The file offset of its header.
    pub offset: u64,
    /// Its header length.
    pub header: u64,
    /// Its whole size.
    pub size: u64,
}

/// The file as read: its top-level boxes, the `moov` tree and its bytes, and every structural
/// problem met on the way.
#[derive(Debug, Clone)]
pub struct Mp4 {
    /// The file's length in bytes.
    pub file_len: u64,
    /// The top-level boxes, in file order.
    pub top: Vec<TopBox>,
    /// The `moov` box's tree, if exactly one was found and read.
    pub moov: Option<BoxNode>,
    /// The `moov` box's bytes, header included.
    moov_bytes: Vec<u8>,
    /// Structural problems.
    pub problems: Vec<Problem>,
}

/// Why a file could not be read at all.
#[derive(Debug)]
pub enum ReadError {
    /// The system refused.
    Io(io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl Mp4 {
    /// Reads `source`'s boxes. Only an I/O failure on the file as a whole is an error; anything
    /// wrong with its structure becomes a [`Problem`].
    pub fn read(source: &mut dyn Source) -> Result<Self, ReadError> {
        let file_len = source.size().map_err(ReadError::Io)?;
        let mut file = Self {
            file_len,
            top: Vec::new(),
            moov: None,
            moov_bytes: Vec::new(),
            problems: Vec::new(),
        };
        let mut at = 0_u64;
        while at < file_len {
            if file.top.len() >= MAX_TOP_BOXES {
                file.problem("", at, format!("more than {MAX_TOP_BOXES} top-level boxes"));
                break;
            }
            let mut head = [0_u8; 32];
            let available = (file_len - at).min(head.len() as u64) as usize;
            source
                .read_at(at, &mut head[..available])
                .map_err(ReadError::Io)?;
            let parsed = match header(&head[..available]) {
                Ok(parsed) => parsed,
                Err(what) => {
                    file.problem("", at, what);
                    break;
                }
            };
            let size = parsed.size.unwrap_or(file_len - at);
            if size > file_len - at {
                file.problem(
                    &parsed.kind.to_string(),
                    at,
                    format!(
                        "{} box of {size} bytes overruns the file by {} bytes",
                        parsed.kind,
                        size - (file_len - at)
                    ),
                );
            }
            file.top.push(TopBox {
                kind: parsed.kind,
                offset: at,
                header: parsed.header,
                size,
            });
            at = at.saturating_add(size);
        }
        let moovs: Vec<TopBox> = file
            .top
            .iter()
            .filter(|b| b.kind.0 == *b"moov")
            .copied()
            .collect();
        if let [moov] = moovs.as_slice() {
            file.read_moov(source, *moov)?;
        }
        Ok(file)
    }

    fn problem(&mut self, path: &str, offset: u64, what: String) {
        self.problems.push(Problem {
            path: path.to_owned(),
            offset,
            what,
        });
    }

    fn read_moov(&mut self, source: &mut dyn Source, moov: TopBox) -> Result<(), ReadError> {
        if moov.size > MAX_MOOV_BYTES {
            self.problem(
                "moov",
                moov.offset,
                format!("moov larger than {MAX_MOOV_BYTES} bytes"),
            );
            return Ok(());
        }
        // An overrunning moov was already reported; read what the file holds.
        let len = moov.size.min(self.file_len - moov.offset) as usize;
        let mut bytes = vec![0_u8; len];
        source
            .read_at(moov.offset, &mut bytes)
            .map_err(ReadError::Io)?;
        let mut node = BoxNode {
            kind: moov.kind,
            offset: moov.offset,
            header: moov.header,
            size: moov.size,
            children: Vec::new(),
        };
        let payload_start = usize::try_from(moov.header).unwrap_or(usize::MAX);
        if payload_start <= bytes.len() {
            let mut problems = Vec::new();
            let payload = Bytes::new(&bytes[payload_start..], moov.offset + moov.header);
            let mut walk = Walk {
                problems: &mut problems,
                boxes: 0,
                stopped: false,
            };
            parse_children(&payload, 0, "moov", 1, &mut node.children, &mut walk);
            self.problems.extend(problems);
        }
        self.moov = Some(node);
        self.moov_bytes = bytes;
        Ok(())
    }

    /// The `moov` box's bytes, for reading the payloads of boxes in [`Mp4::moov`].
    pub fn moov_bytes(&self) -> Bytes<'_> {
        let base = self.moov.as_ref().map_or(0, |m| m.offset);
        Bytes::new(&self.moov_bytes, base)
    }
}
