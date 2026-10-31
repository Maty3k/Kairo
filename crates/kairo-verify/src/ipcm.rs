// SPDX-License-Identifier: GPL-3.0-or-later
//! A lossless H.264 stream of I_PCM macroblocks (docs/research/16-verify-testbed/README.md
//! §7.2): the encoder of the CI corpus, which needs no H.264 encoder at all. Every macroblock
//! carries its samples raw, so a decoder gives back the exact pictures, while the stream stays
//! ordinary High-profile H.264 that FFmpeg's muxer and decoders take. IDR I slices come every
//! `gop` frames and, between them, P slices whose macroblocks are all I_PCM too, so only the
//! IDRs are sync samples, as in Kairo's clips.
//!
//! The stream, Annex B: the SPS and PPS before every IDR ([`Ipcm::headers`], as AMF's
//! `-header_spacing` does), then one slice per picture. CAVLC (the expect file says
//! `cabac: false`), `pic_order_cnt_type` 2, one reference frame, the deblocking filter off, and a
//! VUI with BT.709 limited colour, square pixels, the frame rate and no reordering. A size that
//! isn't a multiple of 16 is coded larger and cropped. The level is nominal: the one the frame
//! size and rate need, which `video.params` expects. Raw macroblocks run far past that level's
//! bit-rate and minimum-compression limits (94.5 Mbit/s at 480x270, 60 fps), by design; FFmpeg
//! doesn't mind, but a strict or hardware decoder may.

use crate::decode::Picture;

/// An MSB-first bit writer for RBSPs.
#[derive(Debug, Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    /// `n` bits of `value`, most significant first.
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

    /// One bit.
    pub fn flag(&mut self, on: bool) -> &mut Self {
        self.u(1, u64::from(on))
    }

    /// Exp-Golomb, unsigned.
    pub fn ue(&mut self, value: u32) -> &mut Self {
        let v = u64::from(value) + 1;
        let len = 64 - v.leading_zeros();
        self.u(len - 1, 0).u(len, v)
    }

    /// Exp-Golomb, signed.
    pub fn se(&mut self, value: i32) -> &mut Self {
        let v = i64::from(value);
        let k = if v > 0 { 2 * v - 1 } else { -2 * v };
        self.ue(k as u32)
    }

    /// Zero bits up to the next byte.
    pub fn align_zero(&mut self) -> &mut Self {
        while !self.bits.is_multiple_of(8) {
            self.u(1, 0);
        }
        self
    }

    /// Whole bytes; the writer must be byte-aligned.
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        debug_assert!(self.bits.is_multiple_of(8));
        self.bytes.extend_from_slice(bytes);
        self.bits += 8 * bytes.len() as u32;
        self
    }

    /// The RBSP with its stop bit and alignment.
    pub fn finish(&mut self) -> Vec<u8> {
        self.u(1, 1);
        self.align_zero();
        self.bits = 0;
        std::mem::take(&mut self.bytes)
    }
}

/// A NAL unit: its header byte, then the RBSP with emulation prevention.
pub fn nal(header: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + rbsp.len() / 64 + 1);
    out.push(header);
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

/// An Annex B start code.
const START: [u8; 4] = [0, 0, 0, 1];

/// An I_PCM stream's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipcm {
    width: usize,
    height: usize,
    mbs_wide: usize,
    mbs_high: usize,
    fps: u32,
    gop: u32,
    level: u8,
}

impl Ipcm {
    /// A stream of `width` × `height` pictures (both even) at `fps`, an IDR every `gop` frames.
    pub fn new(width: usize, height: usize, fps: u32, gop: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(format!("{width}x{height}: I_PCM needs an even size"));
        }
        if fps == 0 || gop == 0 {
            return Err("the frame rate and the GOP must be at least 1".to_owned());
        }
        let (mbs_wide, mbs_high) = (width.div_ceil(16), height.div_ceil(16));
        // The level `video.params` expects for this size and rate.
        let level = u32::try_from(width)
            .ok()
            .zip(u32::try_from(height).ok())
            .and_then(|(w, h)| crate::checks::level_needed(w, h, (u64::from(fps), 1)))
            .ok_or_else(|| format!("{width}x{height} at {fps} fps is beyond H.264 level 6.2"))?;
        Ok(Self {
            width,
            height,
            mbs_wide,
            mbs_high,
            fps,
            gop,
            level,
        })
    }

    /// Whether picture `index` (from 0) is an IDR, which [`Ipcm::headers`] should precede.
    pub fn is_idr(&self, index: u64) -> bool {
        index.is_multiple_of(u64::from(self.gop))
    }

    /// The SPS and PPS, Annex B.
    pub fn headers(&self) -> Vec<u8> {
        let mut sps = BitWriter::default();
        sps.u(8, 100) // profile_idc: High
            .u(8, 0) // constraint flags
            .u(8, u64::from(self.level))
            .ue(0) // seq_parameter_set_id
            .ue(1) // chroma_format_idc: 4:2:0
            .ue(0) // bit_depth_luma_minus8
            .ue(0) // bit_depth_chroma_minus8
            .flag(false) // qpprime_y_zero_transform_bypass_flag
            .flag(false) // seq_scaling_matrix_present_flag
            .ue(0) // log2_max_frame_num_minus4: frame_num has 4 bits
            .ue(2) // pic_order_cnt_type: output order is decode order
            .ue(1) // max_num_ref_frames
            .flag(false) // gaps_in_frame_num_value_allowed_flag
            .ue((self.mbs_wide - 1) as u32)
            .ue((self.mbs_high - 1) as u32)
            .flag(true) // frame_mbs_only_flag
            .flag(true); // direct_8x8_inference_flag
        let (right, bottom) = (
            (self.mbs_wide * 16 - self.width) / 2,
            (self.mbs_high * 16 - self.height) / 2,
        );
        sps.flag(right > 0 || bottom > 0);
        if right > 0 || bottom > 0 {
            // Crop units are 2 pixels in 4:2:0 frames.
            sps.ue(0).ue(right as u32).ue(0).ue(bottom as u32);
        }
        sps.flag(true) // vui_parameters_present_flag
            .flag(true) // aspect_ratio_info_present_flag
            .u(8, 1) // aspect_ratio_idc: 1:1
            .flag(false) // overscan_info_present_flag
            .flag(true) // video_signal_type_present_flag
            .u(3, 5) // video_format: unspecified
            .flag(false) // video_full_range_flag: limited
            .flag(true) // colour_description_present_flag
            .u(8, 1) // colour_primaries: BT.709
            .u(8, 1) // transfer_characteristics: BT.709
            .u(8, 1) // matrix_coefficients: BT.709
            .flag(false) // chroma_loc_info_present_flag
            .flag(true) // timing_info_present_flag
            .u(32, 1) // num_units_in_tick
            .u(32, 2 * u64::from(self.fps)) // time_scale: two ticks a frame
            .flag(true) // fixed_frame_rate_flag
            .flag(false) // nal_hrd_parameters_present_flag
            .flag(false) // vcl_hrd_parameters_present_flag
            .flag(false) // pic_struct_present_flag
            .flag(true) // bitstream_restriction_flag
            .flag(true) // motion_vectors_over_pic_boundaries_flag
            .ue(0) // max_bytes_per_pic_denom
            .ue(0) // max_bits_per_mb_denom
            .ue(15) // log2_max_mv_length_horizontal
            .ue(15) // log2_max_mv_length_vertical
            .ue(0) // max_num_reorder_frames: none
            .ue(1); // max_dec_frame_buffering
        let mut pps = BitWriter::default();
        pps.ue(0) // pic_parameter_set_id
            .ue(0) // seq_parameter_set_id
            .flag(false) // entropy_coding_mode_flag: CAVLC
            .flag(false) // bottom_field_pic_order_in_frame_present_flag
            .ue(0) // num_slice_groups_minus1
            .ue(0) // num_ref_idx_l0_default_active_minus1
            .ue(0) // num_ref_idx_l1_default_active_minus1
            .flag(false) // weighted_pred_flag
            .u(2, 0) // weighted_bipred_idc
            .se(0) // pic_init_qp_minus26
            .se(0) // pic_init_qs_minus26
            .se(0) // chroma_qp_index_offset
            .flag(true) // deblocking_filter_control_present_flag
            .flag(false) // constrained_intra_pred_flag
            .flag(false); // redundant_pic_cnt_present_flag
        let mut out = START.to_vec();
        out.extend(nal(0x67, &sps.finish()));
        out.extend_from_slice(&START);
        out.extend(nal(0x68, &pps.finish()));
        out
    }

    /// Picture `index` (from 0) as one slice, Annex B.
    pub fn picture(&self, index: u64, picture: &Picture) -> Result<Vec<u8>, String> {
        if (picture.width, picture.height) != (self.width, self.height) {
            return Err(format!(
                "a {}x{} picture in a {}x{} stream",
                picture.width, picture.height, self.width, self.height
            ));
        }
        let position = index % u64::from(self.gop);
        let idr = self.is_idr(index);
        let mut w = BitWriter::default();
        w.ue(0) // first_mb_in_slice
            .ue(if idr { 7 } else { 5 }) // slice_type: I or P, all of the picture's slices alike
            .ue(0) // pic_parameter_set_id
            .u(4, position % 16); // frame_num: reference pictures since the IDR
        if idr {
            // idr_pic_id: two IDRs in a row must differ.
            w.ue((index / u64::from(self.gop) % 2) as u32);
            w.flag(false).flag(false); // no_output_of_prior_pics_flag, long_term_reference_flag
        } else {
            w.flag(false) // num_ref_idx_active_override_flag
                .flag(false) // ref_pic_list_modification_flag_l0
                .flag(false); // adaptive_ref_pic_marking_mode_flag: sliding window
        }
        w.se(0) // slice_qp_delta
            .ue(1); // disable_deblocking_filter_idc: off
        let cw = self.width.div_ceil(2);
        let ch = self.height.div_ceil(2);
        let mut samples = [0_u8; 384];
        for my in 0..self.mbs_high {
            for mx in 0..self.mbs_wide {
                if !idr {
                    w.ue(0); // mb_skip_run
                }
                w.ue(if idr { 25 } else { 30 }); // mb_type: I_PCM (25, or 5 + 25 in a P slice)
                w.align_zero(); // pcm_alignment_zero_bit
                // Samples past the picture's edge repeat it; they are cropped away.
                let at = |plane: &[u8], stride: usize, rows: usize, x: usize, y: usize| {
                    plane[y.min(rows - 1) * stride + x.min(stride - 1)]
                };
                for (i, s) in samples[..256].iter_mut().enumerate() {
                    *s = at(
                        &picture.y,
                        self.width,
                        self.height,
                        mx * 16 + i % 16,
                        my * 16 + i / 16,
                    );
                }
                for (i, s) in samples[256..320].iter_mut().enumerate() {
                    *s = at(&picture.cb, cw, ch, mx * 8 + i % 8, my * 8 + i / 8);
                }
                for (i, s) in samples[320..].iter_mut().enumerate() {
                    *s = at(&picture.cr, cw, ch, mx * 8 + i % 8, my * 8 + i / 8);
                }
                w.bytes(&samples);
            }
        }
        let header = if idr { 0x65 } else { 0x41 };
        let mut out = START.to_vec();
        out.extend(nal(header, &w.finish()));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h264::{self, ParamSets};

    /// Splits an Annex B stream into NAL units.
    fn nals(stream: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        let mut i = 0;
        let mut start = None;
        while i + 4 <= stream.len() {
            if stream[i..i + 4] == START {
                if let Some(s) = start {
                    out.push(&stream[s..i]);
                }
                start = Some(i + 4);
                i += 4;
            } else {
                i += 1;
            }
        }
        if let Some(s) = start {
            out.push(&stream[s..]);
        }
        out
    }

    fn picture(width: usize, height: usize, seed: u8) -> Picture {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let byte = |i: usize| (i as u8).wrapping_mul(31).wrapping_add(seed);
        Picture {
            width,
            height,
            y: (0..width * height).map(byte).collect(),
            cb: (0..cw * ch).map(|i| byte(i + 7)).collect(),
            cr: (0..cw * ch).map(|i| byte(i + 13)).collect(),
        }
    }

    #[test]
    fn the_parameter_sets_parse_as_kairo_reads_them() {
        let stream = Ipcm::new(480, 270, 60, 60).unwrap();
        let headers = stream.headers();
        let units = nals(&headers);
        assert_eq!(units.len(), 2);
        let sps = h264::parse_sps(units[0]).unwrap();
        assert_eq!((sps.profile_idc, sps.width, sps.height), (100, 480, 270));
        assert_eq!(sps.level_idc, 30);
        let vui = sps.vui.unwrap();
        assert_eq!(vui.colour, Some((1, 1, 1)));
        assert_eq!(vui.full_range, Some(false));
        let pps = h264::parse_pps(units[1]).unwrap();
        assert!(!pps.cabac);
        // Slices: I at 0, P after, I again at the GOP (the parser keeps slice_type mod 5).
        let mut params = ParamSets::default();
        params.add_sps(units[0]).unwrap();
        params.add_pps(units[1]).unwrap();
        for (index, slice_type) in [(0, 2), (1, 0), (59, 0), (60, 2)] {
            let frame = stream.picture(index, &picture(480, 270, 1)).unwrap();
            let unit = nals(&frame)[0];
            let header = h264::parse_slice_header(unit, &params).unwrap();
            assert_eq!(header.slice_type, slice_type, "{index}");
            assert_eq!(header.frame_num, (index % 60 % 16) as u32, "{index}");
        }
    }

    /// Reads this writer's streams back: every picture's samples, cropped. It parses the slice
    /// data itself, so it checks the layout independently of the writer.
    fn decode(stream: &[u8]) -> Vec<Picture> {
        let mut params = ParamSets::default();
        let mut pictures = Vec::new();
        for unit in nals(stream) {
            let kind = unit[0] & 0x1F;
            match kind {
                7 => {
                    params.add_sps(unit).unwrap();
                }
                8 => {
                    params.add_pps(unit).unwrap();
                }
                1 | 5 => {
                    let sps = params.sps_for_pps(0).unwrap().clone();
                    let (width, height) = (sps.width as usize, sps.height as usize);
                    let (mbs_wide, mbs_high) = (width.div_ceil(16), height.div_ceil(16));
                    let (coded_w, coded_h) = (mbs_wide * 16, mbs_high * 16);
                    let mut y = vec![0; coded_w * coded_h];
                    let mut cb = vec![0; coded_w * coded_h / 4];
                    let mut cr = vec![0; coded_w * coded_h / 4];
                    let rbsp = h264::unescape(&unit[1..]);
                    let mut bits = h264::Bits::new(&rbsp);
                    let idr = kind == 5;
                    // first_mb_in_slice, slice_type, pic_parameter_set_id, frame_num.
                    assert_eq!(bits.ue(), Some(0));
                    assert_eq!(bits.ue(), Some(if idr { 7 } else { 5 }));
                    assert_eq!(bits.ue(), Some(0));
                    bits.u(4).unwrap();
                    if idr {
                        bits.ue().unwrap();
                    }
                    // IDR: no_output_of_prior_pics, long_term_reference; P: the override and
                    // the two modification flags.
                    for _ in 0..if idr { 2 } else { 3 } {
                        assert_eq!(bits.flag(), Some(false));
                    }
                    assert_eq!(bits.se(), Some(0));
                    assert_eq!(bits.ue(), Some(1));
                    let mut offset = 0;
                    for my in 0..mbs_high {
                        for mx in 0..mbs_wide {
                            if !idr {
                                assert_eq!(bits.ue(), Some(0));
                            }
                            assert_eq!(bits.ue(), Some(if idr { 25 } else { 30 }));
                            while !bits.position().is_multiple_of(8) {
                                assert_eq!(bits.flag(), Some(false));
                            }
                            let start = offset + bits.position() / 8;
                            let samples = &rbsp[start..start + 384];
                            for i in 0..256 {
                                y[(my * 16 + i / 16) * coded_w + mx * 16 + i % 16] = samples[i];
                            }
                            for i in 0..64 {
                                let at = (my * 8 + i / 8) * (coded_w / 2) + mx * 8 + i % 8;
                                cb[at] = samples[256 + i];
                                cr[at] = samples[320 + i];
                            }
                            offset = start + 384;
                            bits = h264::Bits::new(&rbsp[offset..]);
                        }
                    }
                    // Only the stop bit is left.
                    assert_eq!(&rbsp[offset..], [0x80]);
                    let crop = |plane: &[u8], stride: usize, w: usize, h: usize| -> Vec<u8> {
                        (0..h)
                            .flat_map(|row| plane[row * stride..row * stride + w].to_vec())
                            .collect()
                    };
                    let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
                    pictures.push(Picture {
                        width,
                        height,
                        y: crop(&y, coded_w, width, height),
                        cb: crop(&cb, coded_w / 2, cw, ch),
                        cr: crop(&cr, coded_w / 2, cw, ch),
                    });
                }
                other => panic!("NAL type {other}"),
            }
        }
        pictures
    }

    #[test]
    fn pictures_come_back_exactly() {
        // 50x34 is coded 64x48 and cropped; GOPs of 2 and 3 mix IDR and P pictures.
        for gop in [2, 3] {
            let stream = Ipcm::new(50, 34, 60, gop).unwrap();
            let inputs: Vec<Picture> = (0..5).map(|i| picture(50, 34, i * 40)).collect();
            let mut bytes = Vec::new();
            for (index, p) in inputs.iter().enumerate() {
                if stream.is_idr(index as u64) {
                    bytes.extend(stream.headers());
                }
                bytes.extend(stream.picture(index as u64, p).unwrap());
            }
            assert_eq!(decode(&bytes), inputs, "GOP {gop}");
        }
    }

    #[test]
    fn the_samples_go_in_raw() {
        // One 16x16 macroblock: its 384 sample bytes sit, escaped, right after the header.
        let stream = Ipcm::new(16, 16, 60, 2).unwrap();
        let p = picture(16, 16, 9);
        let frame = stream.picture(0, &p).unwrap();
        let rbsp = h264::unescape(&nals(&frame)[0][1..]);
        let mut expected = p.y.clone();
        expected.extend(&p.cb);
        expected.extend(&p.cr);
        // The samples end one byte before the RBSP's end (the stop bit's byte).
        let end = rbsp.len() - 1;
        assert_eq!(&rbsp[end - 384..end], &expected[..]);
        assert_eq!(rbsp[end], 0x80);
    }

    #[test]
    fn sizes_and_levels() {
        assert!(Ipcm::new(481, 270, 60, 60).is_err());
        assert!(Ipcm::new(480, 270, 0, 60).is_err());
        assert_eq!(Ipcm::new(1920, 1080, 60, 60).unwrap().level, 42);
        assert_eq!(Ipcm::new(2560, 1440, 60, 60).unwrap().level, 51);
        assert!(Ipcm::new(16_384, 16_384, 60, 60).is_err());
        // A picture of another size is refused.
        let stream = Ipcm::new(32, 32, 60, 60).unwrap();
        assert!(stream.picture(0, &picture(16, 16, 0)).is_err());
    }

    #[test]
    fn emulation_prevention_escapes_zero_runs() {
        assert_eq!(
            nal(0x65, &[0, 0, 1, 0, 0, 0, 2]),
            [0x65, 0, 0, 3, 1, 0, 0, 3, 0, 2]
        );
        assert_eq!(h264::unescape(&nal(0x65, &[0, 0, 3, 0])[1..]), [0, 0, 3, 0]);
    }
}
