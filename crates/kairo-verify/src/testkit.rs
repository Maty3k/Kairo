// SPDX-License-Identifier: GPL-3.0-or-later
//! Test helpers (container.md §9.1): an H.264 bit writer and an MP4 builder that makes a good
//! Kairo-style clip by default, so each test can change one thing and see one check fail. No
//! clip files: everything is built in memory.

/// An MSB-first bit writer for RBSPs.
#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    pub fn u(&mut self, n: u32, value: u64) -> &mut Self {
        for i in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((value >> i) & 1) as u8;
            if let Some(last) = self.bytes.last_mut() {
                *last |= bit << (7 - self.bits % 8);
            }
            self.bits += 1;
        }
        self
    }

    pub fn flag(&mut self, on: bool) -> &mut Self {
        self.u(1, u64::from(on))
    }

    pub fn ue(&mut self, value: u32) -> &mut Self {
        let v = u64::from(value) + 1;
        let len = 64 - v.leading_zeros();
        self.u(len - 1, 0).u(len, v)
    }

    pub fn se(&mut self, value: i32) -> &mut Self {
        let v = i64::from(value);
        let k = if v > 0 { 2 * v - 1 } else { -2 * v };
        self.ue(k as u32)
    }

    /// The RBSP with its stop bit and alignment.
    pub fn finish(&mut self) -> Vec<u8> {
        self.u(1, 1);
        while !self.bits.is_multiple_of(8) {
            self.u(1, 0);
        }
        std::mem::take(&mut self.bytes)
    }
}

/// A NAL unit: its header byte, then the RBSP with emulation prevention.
pub fn nal(header: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![header];
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// The VUI fields the builder writes.
#[derive(Clone, Debug)]
pub struct VuiSpec {
    pub aspect: Option<(u8, u16, u16)>,
    pub signal_type: bool,
    pub full_range: bool,
    pub colour: Option<(u8, u8, u8)>,
    pub timing: Option<(u32, u32, bool)>,
    pub nal_hrd: bool,
    pub reorder: Option<u32>,
}

impl Default for VuiSpec {
    fn default() -> Self {
        Self {
            aspect: None,
            signal_type: true,
            full_range: false,
            colour: Some((1, 1, 1)),
            timing: None,
            nal_hrd: false,
            reorder: None,
        }
    }
}

/// An SPS.
#[derive(Clone, Debug)]
pub struct SpsSpec {
    pub profile: u8,
    pub level: u8,
    pub sps_id: u32,
    pub chroma: u32,
    pub bit_depth: u32,
    pub scaling_lists: bool,
    pub log2_max_frame_num: u32,
    pub poc_type: u32,
    pub log2_max_poc_lsb: u32,
    pub width: u32,
    pub height: u32,
    pub frame_mbs_only: bool,
    pub vui: Option<VuiSpec>,
    /// The first delta_scale of scaling list 0, when the lists are written.
    pub first_delta: i32,
}

impl Default for SpsSpec {
    fn default() -> Self {
        Self {
            profile: 100,
            level: 31,
            sps_id: 0,
            chroma: 1,
            bit_depth: 8,
            scaling_lists: false,
            log2_max_frame_num: 4,
            poc_type: 0,
            log2_max_poc_lsb: 6,
            width: 320,
            height: 180,
            frame_mbs_only: true,
            vui: Some(VuiSpec::default()),
            first_delta: 2,
        }
    }
}

pub fn sps(spec: &SpsSpec) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.u(8, u64::from(spec.profile))
        .u(8, 0)
        .u(8, u64::from(spec.level))
        .ue(spec.sps_id);
    if spec.profile == 100 {
        w.ue(spec.chroma)
            .ue(spec.bit_depth - 8)
            .ue(spec.bit_depth - 8)
            .flag(false);
        w.flag(spec.scaling_lists);
        if spec.scaling_lists {
            for i in 0..8 {
                // List 0 present with a few deltas, the rest absent.
                w.flag(i == 0);
                if i == 0 {
                    for delta in [
                        spec.first_delta,
                        -1,
                        0,
                        3,
                        -2,
                        1,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                    ] {
                        w.se(delta);
                    }
                }
            }
        }
    }
    w.ue(spec.log2_max_frame_num - 4).ue(spec.poc_type);
    if spec.poc_type == 0 {
        w.ue(spec.log2_max_poc_lsb - 4);
    }
    w.ue(1).flag(false);
    let fields = if spec.frame_mbs_only { 1 } else { 2 };
    let width_mbs = spec.width.div_ceil(16);
    let height_units = spec.height.div_ceil(16 * fields);
    w.ue(width_mbs - 1)
        .ue(height_units - 1)
        .flag(spec.frame_mbs_only);
    if !spec.frame_mbs_only {
        w.flag(false);
    }
    w.flag(true);
    let crop_right = (width_mbs * 16 - spec.width) / 2;
    let crop_bottom = (height_units * 16 * fields - spec.height) / (2 * fields);
    let crop = crop_right > 0 || crop_bottom > 0;
    w.flag(crop);
    if crop {
        w.ue(0).ue(crop_right).ue(0).ue(crop_bottom);
    }
    w.flag(spec.vui.is_some());
    if let Some(vui) = &spec.vui {
        w.flag(vui.aspect.is_some());
        if let Some((idc, h, v)) = vui.aspect {
            w.u(8, u64::from(idc));
            if idc == 255 {
                w.u(16, u64::from(h)).u(16, u64::from(v));
            }
        }
        w.flag(false).flag(vui.signal_type);
        if vui.signal_type {
            w.u(3, 5).flag(vui.full_range).flag(vui.colour.is_some());
            if let Some((p, t, m)) = vui.colour {
                w.u(8, u64::from(p)).u(8, u64::from(t)).u(8, u64::from(m));
            }
        }
        w.flag(false).flag(vui.timing.is_some());
        if let Some((units, scale, fixed)) = vui.timing {
            w.u(32, u64::from(units))
                .u(32, u64::from(scale))
                .flag(fixed);
        }
        w.flag(vui.nal_hrd);
        if vui.nal_hrd {
            // One CPB: bit rate and size scales, its values, then the four lengths.
            w.ue(0)
                .u(4, 2)
                .u(4, 3)
                .ue(1_000)
                .ue(2_000)
                .flag(false)
                .u(20, 0x5_A5A5);
        }
        w.flag(false);
        if vui.nal_hrd {
            w.flag(false);
        }
        w.flag(false).flag(vui.reorder.is_some());
        if let Some(reorder) = vui.reorder {
            w.flag(true)
                .ue(2)
                .ue(1)
                .ue(16)
                .ue(16)
                .ue(reorder)
                .ue(reorder + 1);
        }
    }
    nal(0x67, &w.finish())
}

pub fn pps(pps_id: u32, sps_id: u32, cabac: bool) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(pps_id)
        .ue(sps_id)
        .flag(cabac)
        .flag(false)
        .ue(0)
        .ue(0)
        .ue(0)
        .flag(false)
        .u(2, 0);
    nal(0x68, &w.finish())
}

/// A slice NAL: an IDR (type 5) or not (type 1), with the header fields the scan reads and a
/// few bytes of "slice data" (zeros, so emulation prevention is exercised).
pub fn slice(
    idr: bool,
    slice_type: u32,
    first_mb: u32,
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
) -> Vec<u8> {
    slice_with(idr, slice_type, first_mb, pps_id, frame_num, poc_lsb, 4)
}

/// [`slice`] for an SPS whose `frame_num` has `frame_num_bits` bits.
pub fn slice_with(
    idr: bool,
    slice_type: u32,
    first_mb: u32,
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    frame_num_bits: u32,
) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(first_mb)
        .ue(slice_type)
        .ue(pps_id)
        .u(frame_num_bits, u64::from(frame_num));
    if idr {
        w.ue(0);
    }
    w.u(6, u64::from(poc_lsb)).u(24, 0);
    let header = if idr { 0x65 } else { 0x41 };
    nal(header, &w.finish())
}

/// An SEI NAL with one message.
pub fn sei(payload_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut rbsp = vec![payload_type, payload.len() as u8];
    rbsp.extend_from_slice(payload);
    rbsp.push(0x80);
    nal(0x06, &rbsp)
}

/// A user-data SEI naming an encoder, the way x264 and x265 write one.
pub fn encoder_sei(text: &[u8]) -> Vec<u8> {
    let mut payload = vec![0xDC; 16];
    payload.extend_from_slice(text);
    sei(5, &payload)
}

/// An x264 user-data SEI.
pub fn x264_sei() -> Vec<u8> {
    encoder_sei(b"x264 - core 164 r3095")
}

/// Joins NAL units with 4-byte length prefixes.
pub fn sample(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// MP4
// ---------------------------------------------------------------------------------------------

pub fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

pub fn full(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![version];
    body.extend_from_slice(&flags.to_be_bytes()[1..]);
    body.extend_from_slice(payload);
    boxed(kind, &body)
}

fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

/// A `colr` box.
#[derive(Clone, Debug)]
pub enum ColrSpec {
    Nclx(u16, u16, u16, bool, u8),
    Other([u8; 4]),
    /// A colr payload as given.
    Payload(Vec<u8>),
}

fn colr(c: &ColrSpec) -> Vec<u8> {
    match c {
        ColrSpec::Nclx(p, t, m, full_range, reserved) => {
            let mut body = b"nclx".to_vec();
            for v in [p, t, m] {
                body.extend_from_slice(&v.to_be_bytes());
            }
            body.push((u8::from(*full_range) << 7) | reserved);
            boxed(b"colr", &body)
        }
        ColrSpec::Other(kind) => {
            let mut body = kind.to_vec();
            body.extend_from_slice(&[0; 6]);
            boxed(b"colr", &body)
        }
        ColrSpec::Payload(body) => boxed(b"colr", body),
    }
}

/// An audio track.
#[derive(Clone, Debug)]
pub struct AudioSpec {
    pub title: String,
    pub udta_name: Option<String>,
    pub channels: u16,
    pub entry_channels: u16,
    pub enabled: bool,
    pub group: i16,
    pub rate: u32,
    pub timescale: u32,
    pub oti: u8,
    pub asc: Vec<u8>,
    pub deltas: Vec<u32>,
    pub kind: [u8; 4],
    /// Bytes in each packet.
    pub payload_bytes: usize,
}

impl AudioSpec {
    pub fn new(title: &str, channels: u16, enabled: bool, packets: usize) -> Self {
        let asc = if channels == 1 {
            vec![0x11, 0x88, 0x56, 0xE5, 0x00]
        } else {
            vec![0x11, 0x90, 0x56, 0xE5, 0x00]
        };
        Self {
            title: title.to_owned(),
            udta_name: Some(title.to_owned()),
            channels,
            entry_channels: channels,
            enabled,
            group: 1,
            rate: 48_000,
            timescale: 48_000,
            oti: 0x40,
            asc,
            deltas: vec![1024; packets],
            kind: *b"mp4a",
            payload_bytes: 6,
        }
    }
}

/// What a video sample holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pic {
    /// IDR, I slice.
    Idr,
    /// Non-IDR I slice.
    I,
    /// P slice.
    P,
    /// B slice.
    B,
}

/// A whole clip.
#[derive(Clone, Debug)]
pub struct Spec {
    pub timescale: u32,
    pub deltas: Vec<u32>,
    pub pics: Vec<Pic>,
    pub width: u16,
    pub height: u16,
    pub sps: SpsSpec,
    pub cabac: bool,
    pub slice_pps: u32,
    pub first_mb: u32,
    pub colr: Vec<ColrSpec>,
    pub pasp: Option<(u32, u32)>,
    pub stss: Option<Vec<u32>>,
    pub ctts: Option<Vec<(u32, u32)>>,
    pub poc_step: u32,
    pub poc_regress_at: Option<usize>,
    pub x264_at: Option<usize>,
    pub recovery_at: Option<usize>,
    pub inband_sps_at: Option<(usize, Vec<u8>)>,
    pub nal_overrun_at: Option<usize>,
    pub video_sample_bytes: usize,
    pub video_chunk: usize,
    pub audio: Vec<AudioSpec>,
    pub faststart: bool,
    pub edts: bool,
    pub version1: bool,
    pub largesize_mdat: bool,
    pub mdat_to_end: bool,
    pub co64: bool,
    pub extra_handler: Option<[u8; 4]>,
    pub ilst: Vec<[u8; 4]>,
    pub moof: bool,
    pub video_enabled: bool,
    pub tkhd_size: Option<(u32, u32)>,
    pub mdhd_extra: u64,
    pub tkhd_extra: u64,
    pub mvhd_extra: u64,
    pub track_ids: Option<Vec<u32>>,
    pub dref_flags: u32,
    /// SPS bytes to put in avcC instead of [`sps`]'s.
    pub raw_sps: Option<Vec<u8>>,
    /// The bits of `frame_num` the slices write (the SPS's `log2_max_frame_num`).
    pub frame_num_bits: u32,
    /// avcC's level byte, if it should differ from the SPS's.
    pub avcc_level: Option<u8>,
    pub x265_at: Option<usize>,
    /// One more box at the end of the video `stbl`.
    pub extra_stbl_box: Option<Vec<u8>>,
    /// Put the video track after the audio tracks.
    pub video_last: bool,
    /// A second, identical video sample entry.
    pub extra_entry: bool,
    /// The sample entry the video's last chunk names.
    pub last_chunk_entry: u32,
    pub video_group: i16,
    /// `udta/meta` in QuickTime's layout, without version and flags.
    pub qt_meta: bool,
    pub movie_timescale: u32,
    /// Empty `free` boxes added to `moov/udta`.
    pub udta_filler: usize,
}

impl Spec {
    /// A good 2 s clip: 320×180 H.264 High at 60 fps (GOP 60, frame 1 held 328 ticks) with Mix,
    /// Game and Mic, as an engine save would be.
    pub fn good() -> Self {
        let frames = 120;
        let mut deltas = vec![256; frames];
        deltas[0] = 256 + 328;
        Self {
            timescale: 15_360,
            deltas,
            pics: (0..frames)
                .map(|i| if i % 60 == 0 { Pic::Idr } else { Pic::P })
                .collect(),
            width: 320,
            height: 180,
            sps: SpsSpec::default(),
            cabac: true,
            slice_pps: 0,
            first_mb: 0,
            colr: vec![ColrSpec::Nclx(1, 1, 1, false, 0)],
            pasp: Some((1, 1)),
            stss: None,
            ctts: None,
            poc_step: 2,
            poc_regress_at: None,
            x264_at: None,
            recovery_at: None,
            inband_sps_at: None,
            nal_overrun_at: None,
            video_sample_bytes: 40,
            video_chunk: 50,
            audio: vec![
                AudioSpec::new("Mix", 2, true, 95),
                AudioSpec::new("Game", 2, false, 95),
                AudioSpec::new("Mic", 1, false, 95),
            ],
            faststart: true,
            edts: false,
            version1: false,
            largesize_mdat: false,
            mdat_to_end: false,
            co64: false,
            extra_handler: None,
            ilst: vec![*b"\xa9too"],
            moof: false,
            video_enabled: true,
            tkhd_size: None,
            mdhd_extra: 0,
            tkhd_extra: 0,
            mvhd_extra: 0,
            track_ids: None,
            dref_flags: 1,
            raw_sps: None,
            frame_num_bits: 4,
            avcc_level: None,
            x265_at: None,
            extra_stbl_box: None,
            video_last: false,
            extra_entry: false,
            last_chunk_entry: 1,
            video_group: 0,
            qt_meta: false,
            movie_timescale: 1_000,
            udta_filler: 0,
        }
    }

    /// The same with a change.
    pub fn with(change: impl FnOnce(&mut Spec)) -> Self {
        let mut spec = Self::good();
        change(&mut spec);
        spec
    }

    /// The video samples' bytes.
    fn video_samples(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut poc = 0_u32;
        for (i, pic) in self.pics.iter().enumerate() {
            let mut nals = Vec::new();
            if self.x264_at == Some(i) {
                nals.push(x264_sei());
            }
            if self.x265_at == Some(i) {
                nals.push(encoder_sei(b"x265 (build 199) - 3.6"));
            }
            if self.recovery_at == Some(i) {
                nals.push(sei(6, &[0x84]));
            }
            if let Some((at, raw)) = &self.inband_sps_at
                && *at == i
            {
                nals.push(raw.clone());
            }
            if *pic == Pic::Idr {
                poc = 0;
            }
            let lsb = if self.poc_regress_at == Some(i) {
                poc.saturating_sub(4)
            } else {
                poc
            } % 64;
            let (idr, slice_type) = match pic {
                Pic::Idr => (true, 7),
                Pic::I => (false, 7),
                Pic::P => (false, 5),
                Pic::B => (false, 6),
            };
            let first_mb = if i == 0 { self.first_mb } else { 0 };
            let frame_num = (i % (1 << self.frame_num_bits)) as u32;
            let mut s = slice_with(
                idr,
                slice_type,
                first_mb,
                self.slice_pps,
                frame_num,
                lsb,
                self.frame_num_bits,
            );
            // Pad the slice data to the sample size.
            s.resize(s.len().max(self.video_sample_bytes), 0x5A);
            nals.push(s);
            let mut bytes = sample(&nals);
            if self.nal_overrun_at == Some(i) {
                // The last NAL's length says 100 bytes more than there are.
                let at = bytes.len() - nals.last().map_or(0, Vec::len) - 4;
                let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) + 100;
                bytes[at..at + 4].copy_from_slice(&len.to_be_bytes());
            }
            out.push(bytes);
            poc += self.poc_step;
        }
        out
    }

    fn video_entry(&self, kind: &[u8; 4]) -> Vec<u8> {
        let mut body = vec![0; 6];
        body.extend_from_slice(&1_u16.to_be_bytes());
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(&self.width.to_be_bytes());
        body.extend_from_slice(&self.height.to_be_bytes());
        body.extend_from_slice(&0x0048_0000_u32.to_be_bytes());
        body.extend_from_slice(&0x0048_0000_u32.to_be_bytes());
        body.extend_from_slice(&[0; 4]);
        body.extend_from_slice(&1_u16.to_be_bytes());
        body.extend_from_slice(&[0; 32]);
        body.extend_from_slice(&0x0018_u16.to_be_bytes());
        body.extend_from_slice(&(-1_i16).to_be_bytes());
        let sps = self.raw_sps.clone().unwrap_or_else(|| sps(&self.sps));
        let pps = pps(0, 0, self.cabac);
        let profile = sps.get(1).copied().unwrap_or(self.sps.profile);
        let level = self
            .avcc_level
            .unwrap_or(sps.get(3).copied().unwrap_or(self.sps.level));
        let mut avcc = vec![1, profile, 0, level, 0xFF, 0xE1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&sps);
        avcc.push(1);
        avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&pps);
        body.extend(boxed(b"avcC", &avcc));
        for c in &self.colr {
            body.extend(colr(c));
        }
        if let Some((h, v)) = self.pasp {
            let mut p = h.to_be_bytes().to_vec();
            p.extend_from_slice(&v.to_be_bytes());
            body.extend(boxed(b"pasp", &p));
        }
        boxed(kind, &body)
    }

    fn audio_entry(a: &AudioSpec) -> Vec<u8> {
        let mut body = vec![0; 6];
        body.extend_from_slice(&1_u16.to_be_bytes());
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&a.entry_channels.to_be_bytes());
        body.extend_from_slice(&16_u16.to_be_bytes());
        body.extend_from_slice(&[0; 4]);
        body.extend_from_slice(&(a.rate << 16).to_be_bytes());
        // ES_Descriptor { DecoderConfigDescriptor { DecoderSpecificInfo }, SLConfig }.
        let mut dc = vec![a.oti, 0x15, 0, 0, 0];
        dc.extend_from_slice(&160_000_u32.to_be_bytes());
        dc.extend_from_slice(&160_000_u32.to_be_bytes());
        dc.extend_from_slice(&[0x05, 0x80, 0x80, 0x80, a.asc.len() as u8]);
        dc.extend_from_slice(&a.asc);
        let mut es = vec![0, 1, 0, 0x04, 0x80, 0x80, 0x80, dc.len() as u8];
        es.extend(dc);
        es.extend_from_slice(&[0x06, 0x80, 0x80, 0x80, 0x01, 0x02]);
        let mut esds = vec![0x03, 0x80, 0x80, 0x80, es.len() as u8];
        esds.extend(es);
        body.extend(full(b"esds", 0, 0, &esds));
        boxed(&a.kind, &body)
    }

    fn mvhd(&self, duration: u64, next: u32) -> Vec<u8> {
        let mut b = Vec::new();
        if self.version1 {
            b.extend_from_slice(&[0; 16]);
            b.extend_from_slice(&self.movie_timescale.to_be_bytes());
            b.extend_from_slice(&duration.to_be_bytes());
        } else {
            b.extend_from_slice(&[0; 8]);
            b.extend_from_slice(&self.movie_timescale.to_be_bytes());
            b.extend_from_slice(&(duration as u32).to_be_bytes());
        }
        b.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
        b.extend_from_slice(&0x0100_u16.to_be_bytes());
        b.extend_from_slice(&[0; 10]);
        b.extend_from_slice(&[0; 36]);
        b.extend_from_slice(&[0; 24]);
        b.extend_from_slice(&next.to_be_bytes());
        full(b"mvhd", u8::from(self.version1), 0, &b)
    }

    fn tkhd(&self, flags: u32, id: u32, duration: u64, group: i16, size: (u32, u32)) -> Vec<u8> {
        let mut b = Vec::new();
        if self.version1 {
            b.extend_from_slice(&[0; 16]);
            b.extend_from_slice(&id.to_be_bytes());
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(&duration.to_be_bytes());
        } else {
            b.extend_from_slice(&[0; 8]);
            b.extend_from_slice(&id.to_be_bytes());
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(&(duration as u32).to_be_bytes());
        }
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(&0_u16.to_be_bytes());
        b.extend_from_slice(&group.to_be_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&[0; 36]);
        b.extend_from_slice(&size.0.to_be_bytes());
        b.extend_from_slice(&size.1.to_be_bytes());
        full(b"tkhd", u8::from(self.version1), flags, &b)
    }

    fn mdhd(&self, timescale: u32, duration: u64) -> Vec<u8> {
        let mut b = Vec::new();
        if self.version1 {
            b.extend_from_slice(&[0; 16]);
            b.extend_from_slice(&timescale.to_be_bytes());
            b.extend_from_slice(&duration.to_be_bytes());
        } else {
            b.extend_from_slice(&[0; 8]);
            b.extend_from_slice(&timescale.to_be_bytes());
            b.extend_from_slice(&(duration as u32).to_be_bytes());
        }
        // "und": u, n, d minus 0x60, packed.
        b.extend_from_slice(&0x55C4_u16.to_be_bytes());
        b.extend_from_slice(&[0; 2]);
        full(b"mdhd", u8::from(self.version1), 0, &b)
    }

    fn hdlr(kind: &[u8; 4], name: &str) -> Vec<u8> {
        let mut b = vec![0; 4];
        b.extend_from_slice(kind);
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(name.as_bytes());
        b.push(0);
        full(b"hdlr", 0, 0, &b)
    }

    fn table<T: Copy>(
        kind: &[u8; 4],
        version: u8,
        rows: &[T],
        write: impl Fn(&mut Vec<u8>, T),
    ) -> Vec<u8> {
        let mut b = (rows.len() as u32).to_be_bytes().to_vec();
        for &row in rows {
            write(&mut b, row);
        }
        full(kind, version, 0, &b)
    }

    /// Runs of equal values.
    fn runs(values: &[u32]) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for &v in values {
            match out.last_mut() {
                Some((n, last)) if *last == v => *n += 1,
                _ => out.push((1, v)),
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn trak(
        &self,
        tkhd: Vec<u8>,
        handler: &[u8; 4],
        title: &str,
        udta: Option<&str>,
        timescale: u32,
        deltas: &[u32],
        entries: Vec<Vec<u8>>,
        sizes: &[usize],
        chunk: usize,
        offset: u64,
        video: bool,
    ) -> Vec<u8> {
        let duration: u64 = deltas.iter().map(|&d| u64::from(d)).sum::<u64>()
            + if video { self.mdhd_extra } else { 0 };
        let mut stbl = Vec::new();
        let mut stsd = (entries.len() as u32).to_be_bytes().to_vec();
        for e in &entries {
            stsd.extend_from_slice(e);
        }
        stbl.extend(full(b"stsd", 0, 0, &stsd));
        stbl.extend(Self::table(b"stts", 0, &Self::runs(deltas), |b, (n, d)| {
            b.extend_from_slice(&n.to_be_bytes());
            b.extend_from_slice(&d.to_be_bytes());
        }));
        if video {
            if let Some(ctts) = &self.ctts {
                stbl.extend(Self::table(b"ctts", 0, ctts, |b, (n, o)| {
                    b.extend_from_slice(&n.to_be_bytes());
                    b.extend_from_slice(&o.to_be_bytes());
                }));
            }
            let stss: Vec<u32> = self.stss.clone().unwrap_or_else(|| {
                self.pics
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| **p == Pic::Idr)
                    .map(|(i, _)| i as u32 + 1)
                    .collect()
            });
            stbl.extend(Self::table(b"stss", 0, &stss, |b, s| {
                b.extend_from_slice(&s.to_be_bytes())
            }));
        }
        // Chunks of `chunk` samples; stsc runs as compact as they come.
        let n = sizes.len();
        let chunks = n.div_ceil(chunk.max(1)).max(1);
        let mut stsc: Vec<(u32, u32, u32)> = Vec::new();
        for c in 0..chunks {
            let count = (n - c * chunk).min(chunk) as u32;
            let index = if video && c + 1 == chunks {
                self.last_chunk_entry
            } else {
                1
            };
            if stsc
                .last()
                .is_none_or(|&(_, last, idx)| last != count || idx != index)
            {
                stsc.push((c as u32 + 1, count, index));
            }
        }
        stbl.extend(Self::table(b"stsc", 0, &stsc, |b, (f, s, i)| {
            for v in [f, s, i] {
                b.extend_from_slice(&v.to_be_bytes());
            }
        }));
        let mut stsz = 0_u32.to_be_bytes().to_vec();
        stsz.extend_from_slice(&(n as u32).to_be_bytes());
        for &s in sizes {
            stsz.extend_from_slice(&(s as u32).to_be_bytes());
        }
        stbl.extend(full(b"stsz", 0, 0, &stsz));
        let mut offsets = Vec::new();
        let mut at = offset;
        for c in 0..chunks {
            offsets.push(at);
            at += sizes[c * chunk..((c + 1) * chunk).min(n)]
                .iter()
                .map(|&s| s as u64)
                .sum::<u64>();
        }
        if self.co64 {
            stbl.extend(Self::table(b"co64", 0, &offsets, |b, o| {
                b.extend_from_slice(&o.to_be_bytes())
            }));
        } else {
            stbl.extend(Self::table(b"stco", 0, &offsets, |b, o| {
                b.extend_from_slice(&(o as u32).to_be_bytes())
            }));
        }
        if video && let Some(extra) = &self.extra_stbl_box {
            stbl.extend_from_slice(extra);
        }
        let mhd = if video {
            full(b"vmhd", 0, 1, &[0; 8])
        } else {
            full(b"smhd", 0, 0, &[0; 4])
        };
        let dref = full(
            b"dref",
            0,
            0,
            &concat(&[
                1_u32.to_be_bytes().to_vec(),
                full(b"url ", 0, self.dref_flags, &[]),
            ]),
        );
        let minf = boxed(
            b"minf",
            &concat(&[mhd, boxed(b"dinf", &dref), boxed(b"stbl", &stbl)]),
        );
        let mdia = boxed(
            b"mdia",
            &concat(&[
                self.mdhd(timescale, duration),
                Self::hdlr(handler, title),
                minf,
            ]),
        );
        let mut parts = vec![tkhd];
        if self.edts {
            // One identity edit as long as the track, so only the edit list itself is wrong.
            let total: u64 = deltas.iter().map(|&d| u64::from(d)).sum();
            let segment =
                (total * u64::from(self.movie_timescale)).div_ceil(u64::from(timescale)) as u32;
            let elst = Self::table(b"elst", 0, &[(segment, 0_u32)], |b, (seg, media)| {
                b.extend_from_slice(&seg.to_be_bytes());
                b.extend_from_slice(&media.to_be_bytes());
                b.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
            });
            parts.push(boxed(b"edts", &elst));
        }
        parts.push(mdia);
        if let Some(name) = udta {
            parts.push(boxed(b"udta", &boxed(b"name", name.as_bytes())));
        }
        boxed(b"trak", &concat(&parts))
    }

    fn moov(&self, video: &[Vec<u8>], audio: &[Vec<Vec<u8>>], mdat_payload: u64) -> Vec<u8> {
        let mut traks = Vec::new();
        let ids: Vec<u32> = self.track_ids.clone().unwrap_or_else(|| {
            (1..=(1 + self.audio.len() as u32 + u32::from(self.extra_handler.is_some()))).collect()
        });
        let tk = |deltas: &[u32], timescale: u32| -> u64 {
            deltas
                .iter()
                .map(|&d| u64::from(d))
                .sum::<u64>()
                .saturating_mul(u64::from(self.movie_timescale))
                .div_ceil(u64::from(timescale))
        };
        // Video.
        let video_sizes: Vec<usize> = video.iter().map(Vec::len).collect();
        let video_ticks = tk(&self.deltas, self.timescale) + self.tkhd_extra;
        let size = self
            .tkhd_size
            .unwrap_or((u32::from(self.width) << 16, u32::from(self.height) << 16));
        let flags = if self.video_enabled { 3 } else { 2 };
        let mut entries = vec![self.video_entry(b"avc1")];
        if self.extra_entry {
            entries.push(self.video_entry(b"avc1"));
        }
        traks.push(self.trak(
            self.tkhd(flags, ids[0], video_ticks, self.video_group, size),
            b"vide",
            "Video",
            None,
            self.timescale,
            &self.deltas,
            entries,
            &video_sizes,
            self.video_chunk,
            mdat_payload,
            true,
        ));
        let mut at = mdat_payload + video_sizes.iter().map(|&s| s as u64).sum::<u64>();
        let mut longest = video_ticks;
        for (k, (a, samples)) in self.audio.iter().zip(audio).enumerate() {
            let sizes: Vec<usize> = samples.iter().map(Vec::len).collect();
            let ticks = tk(&a.deltas, a.timescale);
            longest = longest.max(ticks);
            traks.push(self.trak(
                self.tkhd(u32::from(a.enabled) | 2, ids[k + 1], ticks, a.group, (0, 0)),
                b"soun",
                &a.title,
                a.udta_name.as_deref(),
                a.timescale,
                &a.deltas,
                vec![Self::audio_entry(a)],
                &sizes,
                sizes.len().max(1),
                at,
                false,
            ));
            at += sizes.iter().map(|&s| s as u64).sum::<u64>();
        }
        if let Some(handler) = self.extra_handler {
            traks.push(self.trak(
                self.tkhd(2, ids[self.audio.len() + 1], 1_000, 0, (0, 0)),
                &handler,
                "Text",
                None,
                1_000,
                &[1_000],
                vec![boxed(b"tx3g", &[0; 8])],
                &[2],
                1,
                at,
                false,
            ));
        }
        if self.video_last {
            traks.rotate_left(1);
        }
        let next = ids.iter().copied().max().unwrap_or(0) + 1;
        let mut parts = vec![self.mvhd(longest + self.mvhd_extra, next)];
        parts.extend(traks);
        let mut udta = Vec::new();
        if !self.ilst.is_empty() {
            let items: Vec<u8> = self
                .ilst
                .iter()
                .flat_map(|k| boxed(k, &full(b"data", 0, 1, &[0, 0, 0, 0, b'x'])))
                .collect();
            let children = concat(&[Self::hdlr(b"mdir", ""), boxed(b"ilst", &items)]);
            udta.extend(if self.qt_meta {
                boxed(b"meta", &children)
            } else {
                full(b"meta", 0, 0, &children)
            });
        }
        for _ in 0..self.udta_filler {
            udta.extend(boxed(b"free", &[]));
        }
        if !udta.is_empty() {
            parts.push(boxed(b"udta", &udta));
        }
        boxed(b"moov", &concat(&parts))
    }

    /// The clip's bytes.
    pub fn build(&self) -> Vec<u8> {
        let video = self.video_samples();
        let audio: Vec<Vec<Vec<u8>>> = self
            .audio
            .iter()
            .map(|a| {
                a.deltas
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let mut packet = vec![0x21, i as u8, 0x5A, 0x00, 0x00, 0x03];
                        packet.resize(a.payload_bytes, 0x5A);
                        packet
                    })
                    .collect()
            })
            .collect();
        let mut payload = video.concat();
        for track in &audio {
            payload.extend(track.concat());
        }
        if self.extra_handler.is_some() {
            payload.extend_from_slice(&[0, 0]);
        }
        let ftyp = boxed(b"ftyp", b"isom\x00\x00\x02\x00isomiso2avc1mp41");
        let mdat_header = if self.largesize_mdat { 16 } else { 8 };
        let mdat = {
            let mut m = Vec::new();
            if self.largesize_mdat {
                m.extend_from_slice(&1_u32.to_be_bytes());
                m.extend_from_slice(b"mdat");
                m.extend_from_slice(&(payload.len() as u64 + 16).to_be_bytes());
            } else if self.mdat_to_end {
                m.extend_from_slice(&0_u32.to_be_bytes());
                m.extend_from_slice(b"mdat");
            } else {
                m.extend_from_slice(&(payload.len() as u32 + 8).to_be_bytes());
                m.extend_from_slice(b"mdat");
            }
            m.extend_from_slice(&payload);
            m
        };
        let free = boxed(b"free", &[]);
        // The moov's size doesn't depend on the offsets in it, so build it once to measure.
        let probe = self.moov(&video, &audio, 0);
        let mdat_payload = if self.faststart {
            (ftyp.len() + probe.len() + free.len() + mdat_header) as u64
        } else {
            (ftyp.len() + free.len() + mdat_header) as u64
        };
        let moov = self.moov(&video, &audio, mdat_payload);
        let mut out = ftyp;
        if self.faststart {
            out.extend(moov);
            out.extend(free);
            out.extend(mdat);
        } else {
            out.extend(free);
            if self.mdat_to_end {
                // A size-0 mdat must be last; this layout can't have one.
                out.extend(boxed(b"mdat", &payload));
            } else {
                out.extend(mdat);
            }
            out.extend(moov);
        }
        if self.moof {
            out.extend(boxed(b"moof", &boxed(b"mfhd", &[0; 8])));
        }
        out
    }
}
