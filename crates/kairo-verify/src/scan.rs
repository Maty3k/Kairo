// SPDX-License-Identifier: GPL-3.0-or-later
//! The video track's NAL units, sample by sample (docs/research/16-verify-testbed/README.md
//! §3.3, container.md §4): each NAL's length and first [`HEAD_BYTES`] bytes, and whole SEI, SPS
//! and PPS units up to [`PARAM_BYTES`], by positioned reads. Never the slice data, and never the
//! audio. The checks for IDR at 0, the GOP, reordering, one session and the parameter sets read
//! the [`Scan`] this builds. That is one small read per NAL unit: fast from the page cache,
//! where the harness and the engine find a clip they just wrote (README §6.7), and slower from
//! a cold disk.

use std::io;

use crate::h264::{self, AvcC, ParamSets, SliceError};
use crate::mp4::Source;
use crate::mp4::tracks::Sample;

/// The bytes read from the start of each NAL unit: enough for any slice header's first fields
/// and an x264 or x265 user-data SEI's opening words.
pub const HEAD_BYTES: u32 = 64;
/// The longest SEI, SPS or PPS read whole.
pub const PARAM_BYTES: u32 = 4096;
/// How many problems and differing SPS units are kept as examples.
pub const MAX_EXAMPLES: usize = 8;

/// One video sample, as its NAL units describe it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame {
    /// VCL NAL units (types 1 and 5).
    pub slices: u16,
    /// Of which IDR (type 5).
    pub idr_slices: u16,
    /// The slice types seen, as bits `1 << (slice_type % 5)`.
    pub slice_types: u8,
    /// The first slice's `first_mb_in_slice`.
    pub first_mb: Option<u32>,
    /// Whether a slice named parameter sets that were not found.
    pub unknown_pps: bool,
    /// The picture order count (POC type 0), from the first slice.
    pub poc: Option<i64>,
    /// Whether it carries a recovery-point SEI (payload type 6).
    pub recovery_point: bool,
}

/// `slice_types` bit for B slices.
pub const B_SLICE: u8 = 1 << 1;
/// `slice_types` bit for I slices.
pub const I_SLICE: u8 = 1 << 2;

impl Frame {
    /// Whether every slice is IDR (and there is at least one).
    pub fn is_idr(&self) -> bool {
        self.slices > 0 && self.idr_slices == self.slices
    }

    /// Whether every slice is an I slice (and there is at least one).
    pub fn intra_only(&self) -> bool {
        self.slices > 0 && self.slice_types == I_SLICE
    }
}

/// What the scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    /// One entry per sample.
    pub frames: Vec<Frame>,
    /// The NAL unit types of sample 1, in order.
    pub first_types: Vec<u8>,
    /// In-band SPS units.
    pub inband_sps: usize,
    /// In-band PPS units.
    pub inband_pps: usize,
    /// In-band SPS units whose bytes differ from the `avcC` SPS with the same id, as (sample
    /// index from 0, bytes); the first [`MAX_EXAMPLES`] distinct ones.
    pub differing_sps: Vec<(usize, Vec<u8>)>,
    /// Encoders named by user-data SEI: `x264`, `x265`.
    pub encoders: Vec<&'static str>,
    /// Framing problems (a NAL overrunning its sample, an empty NAL): the first few.
    pub problems: Vec<String>,
    /// How many framing problems there were in all.
    pub problem_count: usize,
}

/// Reads the NAL units of every sample in `samples`, starting from the parameter sets in
/// `avcc`. Only an I/O failure is an error.
pub fn scan(source: &mut dyn Source, samples: &[Sample], avcc: &AvcC) -> io::Result<Scan> {
    let mut scanner = Scanner {
        params: ParamSets::default(),
        avcc_sps: avcc.sps.clone(),
        length_size: u32::from(avcc.length_size),
        out: Scan::default(),
        prev_poc: (0, 0),
    };
    for nal in &avcc.sps {
        scanner.params.add_sps(nal);
    }
    for nal in &avcc.pps {
        scanner.params.add_pps(nal);
    }
    for (index, sample) in samples.iter().enumerate() {
        let frame = scanner.sample(source, index, sample)?;
        scanner.out.frames.push(frame);
    }
    Ok(scanner.out)
}

/// `len` bytes at `at` inside `sample`, from the file.
fn part(source: &mut dyn Source, sample: &Sample, at: u32, len: u32) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; len as usize];
    source.read_at(sample.offset.saturating_add(u64::from(at)), &mut bytes)?;
    Ok(bytes)
}

struct Scanner {
    params: ParamSets,
    avcc_sps: Vec<Vec<u8>>,
    length_size: u32,
    out: Scan,
    /// The previous reference picture's (POC msb, lsb), H.264 8.2.1.1.
    prev_poc: (i64, i64),
}

impl Scanner {
    fn problem(&mut self, what: String) {
        self.out.problem_count += 1;
        if self.out.problems.len() < MAX_EXAMPLES {
            self.out.problems.push(what);
        }
    }

    /// One sample: per NAL, one read of its length and first [`HEAD_BYTES`] bytes, and one
    /// more for an SEI, SPS or PPS longer than that. The slice data is never read.
    fn sample(
        &mut self,
        source: &mut dyn Source,
        index: usize,
        sample: &Sample,
    ) -> io::Result<Frame> {
        let mut frame = Frame::default();
        let size = sample.size;
        let ls = self.length_size;
        let mut at = 0_u32;
        while at < size {
            let left = size - at;
            let chunk = part(source, sample, at, left.min(ls + HEAD_BYTES))?;
            let (Some(prefix), Some(&header)) = (chunk.get(..ls as usize), chunk.get(ls as usize))
            else {
                self.problem(format!("sample {}: NAL unit cut short", index + 1));
                break;
            };
            let len = prefix.iter().fold(0_u32, |n, &b| (n << 8) | u32::from(b));
            if len == 0 {
                self.problem(format!("sample {}: empty NAL unit", index + 1));
                at += ls;
                continue;
            }
            if len > left - ls {
                self.problem(format!(
                    "sample {}: NAL unit of {len} bytes overruns its sample by {} bytes",
                    index + 1,
                    len - (left - ls)
                ));
                break;
            }
            let kind = header & 0x1F;
            let nal = if matches!(kind, 6..=8) && len > HEAD_BYTES {
                part(source, sample, at + ls, len.min(PARAM_BYTES))?
            } else {
                let head = ls as usize..(ls + len.min(HEAD_BYTES)) as usize;
                chunk.get(head).unwrap_or_default().to_vec()
            };
            if index == 0 {
                self.out.first_types.push(kind);
            }
            self.nal(index, kind, len, &nal, &mut frame);
            at += ls + len;
        }
        Ok(frame)
    }

    fn nal(&mut self, index: usize, kind: u8, len: u32, nal: &[u8], frame: &mut Frame) {
        match kind {
            1 | 5 => {
                frame.slices = frame.slices.saturating_add(1);
                if kind == 5 {
                    frame.idr_slices = frame.idr_slices.saturating_add(1);
                }
                match h264::parse_slice_header(nal, &self.params) {
                    Ok(slice) => {
                        frame.slice_types |= 1 << slice.slice_type;
                        if frame.slices == 1 {
                            frame.first_mb = Some(slice.first_mb);
                            let reference = nal.first().is_some_and(|b| b & 0x60 != 0);
                            frame.poc = slice
                                .poc_lsb
                                .map(|(lsb, bits)| self.poc(kind == 5, reference, lsb, bits));
                        }
                    }
                    Err(SliceError::UnknownPps(_)) => frame.unknown_pps = true,
                    Err(SliceError::Malformed) => {
                        self.problem(format!("sample {}: unreadable slice header", index + 1));
                    }
                }
            }
            6 => {
                for message in h264::sei_messages(nal) {
                    frame.recovery_point |= message.payload_type == 6;
                    if let Some(encoder) = message.encoder
                        && !self.out.encoders.contains(&encoder)
                    {
                        self.out.encoders.push(encoder);
                    }
                }
            }
            7 => {
                self.out.inband_sps += 1;
                if len > PARAM_BYTES {
                    self.problem(format!(
                        "sample {}: SPS longer than {PARAM_BYTES} bytes",
                        index + 1
                    ));
                    return;
                }
                let matches_avcc = self.avcc_sps.iter().any(|known| known == nal);
                if !matches_avcc
                    && self.out.differing_sps.len() < MAX_EXAMPLES
                    && !self.out.differing_sps.iter().any(|(_, seen)| seen == nal)
                {
                    self.out.differing_sps.push((index, nal.to_vec()));
                }
                self.params.add_sps(nal);
            }
            8 => {
                self.out.inband_pps += 1;
                self.params.add_pps(nal);
            }
            _ => {}
        }
    }

    /// H.264 8.2.1.1: the POC from `pic_order_cnt_lsb`, carrying its msb across wraps.
    fn poc(&mut self, idr: bool, reference: bool, lsb: u32, bits: u32) -> i64 {
        if idr {
            self.prev_poc = (0, 0);
        }
        let (prev_msb, prev_lsb) = self.prev_poc;
        let max = 1_i64 << bits;
        let lsb = i64::from(lsb);
        let msb = if lsb < prev_lsb && prev_lsb - lsb >= max / 2 {
            prev_msb + max
        } else if lsb > prev_lsb && lsb - prev_lsb > max / 2 {
            prev_msb - max
        } else {
            prev_msb
        };
        if reference {
            self.prev_poc = (msb, lsb);
        }
        msb + lsb
    }
}
