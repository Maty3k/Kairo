// SPDX-License-Identifier: GPL-3.0-or-later
//! H.264 headers, never pictures (docs/research/16-verify-testbed/container.md §4; ITU-T H.264
//! §7.3 and Annex E): `avcC`, the SPS through its VUI, the head of the PPS, slice headers and
//! SEI payload types. Every reader returns `None` on a stream that ends early or holds an
//! impossible value; nothing here can panic on bad input.

use std::collections::BTreeMap;

/// An MSB-first bit reader over an RBSP (emulation prevention already removed).
#[derive(Debug, Clone)]
pub struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    /// A reader at the start of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// One bit.
    pub fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(u32::from(bit))
    }

    /// `n` bits (0–32) as an unsigned number.
    pub fn u(&mut self, n: u32) -> Option<u32> {
        if n > 32 {
            return None;
        }
        let mut value: u64 = 0;
        for _ in 0..n {
            value = (value << 1) | u64::from(self.bit()?);
        }
        u32::try_from(value).ok()
    }

    /// A flag.
    pub fn flag(&mut self) -> Option<bool> {
        self.bit().map(|b| b == 1)
    }

    /// An unsigned Exp-Golomb number (`ue(v)`), at most 32 leading zeros.
    pub fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = self.u(zeros)?;
        u32::try_from((1_u64 << zeros) - 1 + u64::from(rest)).ok()
    }

    /// A signed Exp-Golomb number (`se(v)`).
    pub fn se(&mut self) -> Option<i32> {
        let k = i64::from(self.ue()?);
        i32::try_from(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) }).ok()
    }
}

/// A NAL unit's payload with emulation-prevention bytes (`00 00 03`) removed.
pub fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &b in payload {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// A NAL unit's type (the low 5 bits of its first byte).
pub fn nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| b & 0x1F)
}

/// The VUI fields the checks read (H.264 E.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vui {
    /// `aspect_ratio_idc` if present (255: explicit `sar`).
    pub aspect_ratio_idc: Option<u8>,
    /// The explicit sample aspect ratio.
    pub sar: Option<(u16, u16)>,
    /// Whether `video_signal_type` is present.
    pub signal_type_present: bool,
    /// `video_full_range_flag`, if the signal type is present.
    pub full_range: Option<bool>,
    /// (primaries, transfer, matrix), if the colour description is present.
    pub colour: Option<(u8, u8, u8)>,
    /// (`num_units_in_tick`, `time_scale`, `fixed_frame_rate`), if present.
    pub timing: Option<(u32, u32, bool)>,
    /// `max_num_reorder_frames`, if `bitstream_restriction` is present.
    pub max_num_reorder_frames: Option<u32>,
    /// `max_dec_frame_buffering`, if `bitstream_restriction` is present.
    pub max_dec_frame_buffering: Option<u32>,
}

/// A sequence parameter set, read through its VUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sps {
    /// `profile_idc`: 100 is High.
    pub profile_idc: u8,
    /// The constraint flags and reserved bits.
    pub constraint_flags: u8,
    /// `level_idc`.
    pub level_idc: u8,
    /// `seq_parameter_set_id`.
    pub sps_id: u32,
    /// `chroma_format_idc`: 1 is 4:2:0.
    pub chroma_format_idc: u32,
    /// Whether the colour planes are coded separately (4:4:4 only).
    pub separate_colour_plane: bool,
    /// Luma bit depth.
    pub bit_depth_luma: u32,
    /// Chroma bit depth.
    pub bit_depth_chroma: u32,
    /// `log2_max_frame_num`.
    pub log2_max_frame_num: u32,
    /// `pic_order_cnt_type`.
    pub poc_type: u32,
    /// `log2_max_pic_order_cnt_lsb`, for POC type 0.
    pub log2_max_poc_lsb: Option<u32>,
    /// `max_num_ref_frames`.
    pub max_num_ref_frames: u32,
    /// `frame_mbs_only_flag`: progressive.
    pub frame_mbs_only: bool,
    /// The cropped width in pixels.
    pub width: u32,
    /// The cropped height in pixels.
    pub height: u32,
    /// The VUI, if present.
    pub vui: Option<Vui>,
}

/// The profiles whose SPS carries the chroma format, bit depths and scaling matrices.
const HIGH_PROFILES: [u8; 13] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

fn skip_scaling_list(bits: &mut Bits<'_>, size: usize) -> Option<()> {
    let (mut last, mut next) = (8_i32, 8_i32);
    for _ in 0..size {
        if next != 0 {
            // H.264 7.4.2.1.1.1: delta_scale is -128..=127; FFmpeg refuses others too.
            let delta = bits.se()?;
            if !(-128..=127).contains(&delta) {
                return None;
            }
            next = (last + delta + 256).rem_euclid(256);
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

fn skip_hrd(bits: &mut Bits<'_>) -> Option<()> {
    let cpb_count = bits.ue()?.checked_add(1)?;
    if cpb_count > 32 {
        return None;
    }
    bits.u(4)?;
    bits.u(4)?;
    for _ in 0..cpb_count {
        bits.ue()?;
        bits.ue()?;
        bits.bit()?;
    }
    bits.u(20)?;
    Some(())
}

fn parse_vui(bits: &mut Bits<'_>) -> Option<Vui> {
    let mut vui = Vui::default();
    if bits.flag()? {
        let idc = bits.u(8)? as u8;
        vui.aspect_ratio_idc = Some(idc);
        if idc == 255 {
            vui.sar = Some((bits.u(16)? as u16, bits.u(16)? as u16));
        }
    }
    if bits.flag()? {
        bits.bit()?;
    }
    if bits.flag()? {
        vui.signal_type_present = true;
        bits.u(3)?;
        vui.full_range = Some(bits.flag()?);
        if bits.flag()? {
            vui.colour = Some((bits.u(8)? as u8, bits.u(8)? as u8, bits.u(8)? as u8));
        }
    }
    if bits.flag()? {
        bits.ue()?;
        bits.ue()?;
    }
    if bits.flag()? {
        vui.timing = Some((bits.u(32)?, bits.u(32)?, bits.flag()?));
    }
    let nal_hrd = bits.flag()?;
    if nal_hrd {
        skip_hrd(bits)?;
    }
    let vcl_hrd = bits.flag()?;
    if vcl_hrd {
        skip_hrd(bits)?;
    }
    if nal_hrd || vcl_hrd {
        bits.bit()?;
    }
    bits.bit()?;
    if bits.flag()? {
        bits.bit()?;
        for _ in 0..4 {
            bits.ue()?;
        }
        vui.max_num_reorder_frames = Some(bits.ue()?);
        vui.max_dec_frame_buffering = Some(bits.ue()?);
    }
    Some(vui)
}

/// Reads an SPS NAL unit (its header byte included).
pub fn parse_sps(nal: &[u8]) -> Option<Sps> {
    if nal_type(nal)? != 7 {
        return None;
    }
    let rbsp = unescape(nal.get(1..)?);
    let mut bits = Bits::new(&rbsp);
    let profile_idc = bits.u(8)? as u8;
    let constraint_flags = bits.u(8)? as u8;
    let level_idc = bits.u(8)? as u8;
    let sps_id = bits.ue()?;
    if sps_id > 31 {
        return None;
    }
    let (mut chroma_format_idc, mut separate_colour_plane) = (1, false);
    let (mut bit_depth_luma, mut bit_depth_chroma) = (8, 8);
    if HIGH_PROFILES.contains(&profile_idc) {
        chroma_format_idc = bits.ue()?;
        if chroma_format_idc > 3 {
            return None;
        }
        if chroma_format_idc == 3 {
            separate_colour_plane = bits.flag()?;
        }
        bit_depth_luma = bits.ue()?.checked_add(8)?;
        bit_depth_chroma = bits.ue()?.checked_add(8)?;
        bits.bit()?;
        if bits.flag()? {
            let lists = if chroma_format_idc == 3 { 12 } else { 8 };
            for i in 0..lists {
                if bits.flag()? {
                    skip_scaling_list(&mut bits, if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    let log2_max_frame_num = bits.ue()?.checked_add(4)?;
    let poc_type = bits.ue()?;
    let mut log2_max_poc_lsb = None;
    match poc_type {
        0 => log2_max_poc_lsb = Some(bits.ue()?.checked_add(4)?),
        1 => {
            bits.bit()?;
            bits.se()?;
            bits.se()?;
            let cycle = bits.ue()?;
            if cycle > 255 {
                return None;
            }
            for _ in 0..cycle {
                bits.se()?;
            }
        }
        2 => {}
        _ => return None,
    }
    let max_num_ref_frames = bits.ue()?;
    bits.bit()?;
    let width_mbs = u64::from(bits.ue()?) + 1;
    let height_map_units = u64::from(bits.ue()?) + 1;
    let frame_mbs_only = bits.flag()?;
    if !frame_mbs_only {
        bits.bit()?;
    }
    bits.bit()?;
    let mut crop = [0_u64; 4];
    if bits.flag()? {
        for c in &mut crop {
            *c = u64::from(bits.ue()?);
        }
    }
    let vui = if bits.flag()? {
        Some(parse_vui(&mut bits)?)
    } else {
        None
    };
    // Crop units (H.264 7.4.2.1.1): in chroma samples, doubled vertically for fields.
    let chroma_array_type = if separate_colour_plane {
        0
    } else {
        chroma_format_idc
    };
    let (sub_w, sub_h) = match chroma_array_type {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    let fields = if frame_mbs_only { 1 } else { 2 };
    let crop_x = if chroma_array_type == 0 { 1 } else { sub_w };
    let crop_y = if chroma_array_type == 0 {
        fields
    } else {
        sub_h * fields
    };
    let width = (width_mbs * 16).checked_sub(crop_x * (crop[0] + crop[1]))?;
    let height = (fields * height_map_units * 16).checked_sub(crop_y * (crop[2] + crop[3]))?;
    Some(Sps {
        profile_idc,
        constraint_flags,
        level_idc,
        sps_id,
        chroma_format_idc,
        separate_colour_plane,
        bit_depth_luma,
        bit_depth_chroma,
        log2_max_frame_num,
        poc_type,
        log2_max_poc_lsb,
        max_num_ref_frames,
        frame_mbs_only,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        vui,
    })
}

/// The head of a picture parameter set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pps {
    /// `pic_parameter_set_id`.
    pub pps_id: u32,
    /// `seq_parameter_set_id`.
    pub sps_id: u32,
    /// `entropy_coding_mode_flag`: CABAC.
    pub cabac: bool,
    /// `bottom_field_pic_order_in_frame_present_flag`.
    pub bottom_field_poc_present: bool,
}

/// Reads the head of a PPS NAL unit (its header byte included).
pub fn parse_pps(nal: &[u8]) -> Option<Pps> {
    if nal_type(nal)? != 8 {
        return None;
    }
    let rbsp = unescape(nal.get(1..)?);
    let mut bits = Bits::new(&rbsp);
    let pps_id = bits.ue()?;
    let sps_id = bits.ue()?;
    if pps_id > 255 || sps_id > 31 {
        return None;
    }
    Some(Pps {
        pps_id,
        sps_id,
        cabac: bits.flag()?,
        bottom_field_poc_present: bits.flag()?,
    })
}

/// The parameter sets seen so far, by id, with their bytes.
#[derive(Debug, Clone, Default)]
pub struct ParamSets {
    sps: BTreeMap<u32, (Sps, Vec<u8>)>,
    pps: BTreeMap<u32, (Pps, Vec<u8>)>,
}

impl ParamSets {
    /// Adds (or replaces) an SPS NAL unit; `None` if it doesn't parse.
    pub fn add_sps(&mut self, nal: &[u8]) -> Option<&Sps> {
        let sps = parse_sps(nal)?;
        let id = sps.sps_id;
        self.sps.insert(id, (sps, nal.to_vec()));
        self.sps.get(&id).map(|(sps, _)| sps)
    }

    /// Adds (or replaces) a PPS NAL unit; `None` if it doesn't parse.
    pub fn add_pps(&mut self, nal: &[u8]) -> Option<&Pps> {
        let pps = parse_pps(nal)?;
        let id = pps.pps_id;
        self.pps.insert(id, (pps, nal.to_vec()));
        self.pps.get(&id).map(|(pps, _)| pps)
    }

    /// The SPS with this id, and its bytes.
    pub fn sps(&self, id: u32) -> Option<(&Sps, &[u8])> {
        self.sps.get(&id).map(|(sps, raw)| (sps, raw.as_slice()))
    }

    /// The SPS a PPS id leads to.
    pub fn sps_for_pps(&self, pps_id: u32) -> Option<&Sps> {
        let (pps, _) = self.pps.get(&pps_id)?;
        self.sps.get(&pps.sps_id).map(|(sps, _)| sps)
    }

    /// Whether any SPS and any PPS are known.
    pub fn has_both(&self) -> bool {
        !self.sps.is_empty() && !self.pps.is_empty()
    }
}

/// The start of a slice header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceHeader {
    /// `first_mb_in_slice`.
    pub first_mb: u32,
    /// `slice_type` modulo 5: 0 P, 1 B, 2 I, 3 SP, 4 SI.
    pub slice_type: u32,
    /// `pic_parameter_set_id`.
    pub pps_id: u32,
    /// `frame_num`.
    pub frame_num: u32,
    /// Whether it codes a field.
    pub field: bool,
    /// `idr_pic_id`, in an IDR.
    pub idr_pic_id: Option<u32>,
    /// `pic_order_cnt_lsb`, for POC type 0, with `log2_max_pic_order_cnt_lsb`.
    pub poc_lsb: Option<(u32, u32)>,
}

/// Why a slice header couldn't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceError {
    /// Not a slice, cut short or impossible.
    Malformed,
    /// It names a PPS (or the PPS an SPS) that hasn't been seen.
    UnknownPps(u32),
}

impl std::fmt::Display for SliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "the slice header is cut short or impossible"),
            Self::UnknownPps(id) => write!(f, "the slice names PPS {id}, which hasn't been seen"),
        }
    }
}

impl std::error::Error for SliceError {}

/// Reads a slice header from the first bytes of a VCL NAL unit (types 1 and 5, header byte
/// included), with the parameter sets known so far.
pub fn parse_slice_header(nal_head: &[u8], params: &ParamSets) -> Result<SliceHeader, SliceError> {
    let kind = nal_type(nal_head).ok_or(SliceError::Malformed)?;
    if kind != 1 && kind != 5 {
        return Err(SliceError::Malformed);
    }
    let rbsp = unescape(nal_head.get(1..).unwrap_or_default());
    let mut bits = Bits::new(&rbsp);
    let start = (|| Some((bits.ue()?, bits.ue()?, bits.ue()?)))();
    let (first_mb, slice_type, pps_id) = start.ok_or(SliceError::Malformed)?;
    if slice_type > 9 {
        return Err(SliceError::Malformed);
    }
    let sps = params
        .sps_for_pps(pps_id)
        .ok_or(SliceError::UnknownPps(pps_id))?;
    let rest = (|| {
        if sps.separate_colour_plane {
            bits.u(2)?;
        }
        let frame_num = bits.u(sps.log2_max_frame_num)?;
        let mut field = false;
        if !sps.frame_mbs_only {
            field = bits.flag()?;
            if field {
                bits.bit()?;
            }
        }
        let idr_pic_id = if kind == 5 { Some(bits.ue()?) } else { None };
        let poc_lsb = match sps.log2_max_poc_lsb {
            Some(n) if sps.poc_type == 0 => Some((bits.u(n)?, n)),
            _ => None,
        };
        Some((frame_num, field, idr_pic_id, poc_lsb))
    })();
    let (frame_num, field, idr_pic_id, poc_lsb) = rest.ok_or(SliceError::Malformed)?;
    Ok(SliceHeader {
        first_mb,
        slice_type: slice_type % 5,
        pps_id,
        frame_num,
        field,
        idr_pic_id,
        poc_lsb,
    })
}

/// What an SEI message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeiMessage {
    /// Its payload type: 5 user data unregistered, 6 recovery point, …
    pub payload_type: u32,
    /// For user data from x264 or x265, which one.
    pub encoder: Option<&'static str>,
}

/// An SEI number: a run of 0xFF bytes plus a last byte, summed (H.264 7.3.2.3.1).
fn sei_number(rbsp: &[u8], at: &mut usize) -> Option<u32> {
    let mut value = 0_u32;
    loop {
        let b = *rbsp.get(*at)?;
        *at += 1;
        value = value.checked_add(u32::from(b))?;
        if b != 0xFF {
            return Some(value);
        }
    }
}

/// The messages in an SEI NAL unit (header byte included), as far as the bytes reach.
pub fn sei_messages(nal: &[u8]) -> Vec<SeiMessage> {
    let mut messages = Vec::new();
    if nal_type(nal) != Some(6) {
        return messages;
    }
    let rbsp = unescape(nal.get(1..).unwrap_or_default());
    let mut at = 0;
    // Each message: its type and size, then the payload; 0x80 is the stop bit.
    while rbsp.get(at).is_some_and(|&b| b != 0x80) {
        let (Some(payload_type), Some(size)) =
            (sei_number(&rbsp, &mut at), sei_number(&rbsp, &mut at))
        else {
            break;
        };
        let end = at.saturating_add(size as usize);
        let payload = rbsp.get(at..end.min(rbsp.len())).unwrap_or_default();
        // User data unregistered: a 16-byte UUID, then the encoder's text.
        let text = if payload_type == 5 {
            payload.get(16..).unwrap_or_default()
        } else {
            &[]
        };
        let encoder = if text.starts_with(b"x264 ") {
            Some("x264")
        } else if text.starts_with(b"x265 ") {
            Some("x265")
        } else {
            None
        };
        messages.push(SeiMessage {
            payload_type,
            encoder,
        });
        at = end;
    }
    messages
}

/// An `avcC` box's payload (ISO/IEC 14496-15 §5.3.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcC {
    /// `AVCProfileIndication`.
    pub profile: u8,
    /// `profile_compatibility`.
    pub compatibility: u8,
    /// `AVCLevelIndication`.
    pub level: u8,
    /// Bytes in each NAL length prefix: 1, 2 or 4.
    pub length_size: u8,
    /// The SPS NAL units.
    pub sps: Vec<Vec<u8>>,
    /// The PPS NAL units.
    pub pps: Vec<Vec<u8>>,
}

/// Reads an `avcC` payload.
pub fn parse_avcc(data: &[u8]) -> Option<AvcC> {
    let head = data.get(..6)?;
    if head[0] != 1 {
        return None;
    }
    let length_size = (head[4] & 0x03) + 1;
    if length_size == 3 {
        return None;
    }
    // A count, then that many NAL units, each behind a 16-bit length.
    let list = |count: usize, at: &mut usize| -> Option<Vec<Vec<u8>>> {
        (0..count)
            .map(|_| {
                let len = usize::from(u16::from_be_bytes(data.get(*at..*at + 2)?.try_into().ok()?));
                let nal = data.get(*at + 2..*at + 2 + len)?.to_vec();
                *at += 2 + len;
                Some(nal)
            })
            .collect()
    };
    let mut at = 6;
    let sps = list(usize::from(head[5] & 0x1F), &mut at)?;
    let pps_count = usize::from(*data.get(at)?);
    at += 1;
    let pps = list(pps_count, &mut at)?;
    Some(AvcC {
        profile: head[1],
        compatibility: head[2],
        level: head[3],
        length_size,
        sps,
        pps,
    })
}
