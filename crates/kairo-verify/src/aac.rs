// SPDX-License-Identifier: GPL-3.0-or-later
//! AAC configuration, never audio (docs/research/16-verify-testbed/container.md §4): the `esds`
//! descriptors (ISO/IEC 14496-1 §7.2.6) and the AudioSpecificConfig inside them (ISO/IEC
//! 14496-3 §1.6.2.1), including the backward-compatible SBR signal FFmpeg appends.

/// The sampling rates of `samplingFrequencyIndex` 0–12.
const RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// An AudioSpecificConfig, as far as the checks read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    /// `audioObjectType` as signalled: 2 is AAC LC, 5 SBR (HE-AAC), 29 PS.
    pub object_type: u32,
    /// The core's object type: after an explicit SBR or PS signal, the one that follows it;
    /// otherwise `object_type`.
    pub core_object_type: u32,
    /// The (core) sampling rate, from the index or written out.
    pub sample_rate: u32,
    /// `channelConfiguration`: 1 mono, 2 stereo.
    pub channels: u8,
    /// `frameLengthFlag`: 1 means 960-sample frames.
    pub frame_length_flag: bool,
    /// `dependsOnCoreCoder`.
    pub depends_on_core_coder: bool,
    /// The sync extension (0x2B7) if present: its object type and `sbrPresentFlag`.
    pub sync_extension: Option<(u32, bool)>,
}

/// The `esds` fields the checks read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Esds {
    /// `objectTypeIndication`: 0x40 is MPEG-4 audio.
    pub object_type_indication: u8,
    /// `streamType`: 5 is audio.
    pub stream_type: u8,
    /// `avgBitrate`, as the muxer wrote it.
    pub avg_bitrate: u32,
    /// The AudioSpecificConfig, if there is one and it parses.
    pub config: Option<AudioConfig>,
}

/// An MSB-first bit reader.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn u(&mut self, n: u32) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            value = (value << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(value)
    }

    fn left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    fn object_type(&mut self) -> Option<u32> {
        let t = self.u(5)?;
        if t == 31 {
            Some(32 + self.u(6)?)
        } else {
            Some(t)
        }
    }

    fn rate(&mut self) -> Option<u32> {
        match self.u(4)? {
            15 => self.u(24),
            i => RATES.get(i as usize).copied(),
        }
    }
}

/// Reads an AudioSpecificConfig. Object types other than the AAC family return what was read
/// before their own fields.
pub fn parse_audio_config(data: &[u8]) -> Option<AudioConfig> {
    let mut bits = Bits { data, pos: 0 };
    let object_type = bits.object_type()?;
    let sample_rate = bits.rate()?;
    let channels = bits.u(4)? as u8;
    // Explicit SBR or PS: the output rate, then the core's object type.
    let core_object_type = if object_type == 5 || object_type == 29 {
        bits.rate()?;
        bits.object_type()?
    } else {
        object_type
    };
    let mut config = AudioConfig {
        object_type,
        core_object_type,
        sample_rate,
        channels,
        frame_length_flag: false,
        depends_on_core_coder: false,
        sync_extension: None,
    };
    if !matches!(core_object_type, 1..=4 | 6 | 7 | 17 | 19..=23) {
        return Some(config);
    }
    // GASpecificConfig (ISO/IEC 14496-3 Table 4.1).
    config.frame_length_flag = bits.u(1)? == 1;
    config.depends_on_core_coder = bits.u(1)? == 1;
    if config.depends_on_core_coder {
        bits.u(14)?;
    }
    let extension_flag = bits.u(1)? == 1;
    if channels == 0 {
        // A program_config_element follows; Kairo never writes one.
        return Some(config);
    }
    if matches!(core_object_type, 6 | 20) {
        bits.u(3)?; // layerNr
    }
    if extension_flag {
        if core_object_type == 22 {
            bits.u(16)?; // numOfSubFrame, layer_length
        }
        if matches!(core_object_type, 17 | 19 | 20 | 23) {
            bits.u(3)?; // the three resilience flags
        }
        bits.u(1)?; // extensionFlag3
    }
    // Error-resilient types carry epConfig; 2 and 3 bring a config this reader doesn't model.
    if matches!(core_object_type, 17 | 19..=23) && bits.u(2)? >= 2 {
        return Some(config);
    }
    // The backward-compatible SBR signal (Table 1.15), only when SBR or PS wasn't signalled
    // explicitly: 11 bits of sync word 0x2B7, then an object type and for SBR its flag.
    if object_type != 5 && object_type != 29 && bits.left() >= 16 && bits.u(11)? == 0x2B7 {
        let extension_type = bits.object_type()?;
        let sbr = extension_type == 5 && bits.u(1)? == 1;
        config.sync_extension = Some((extension_type, sbr));
    }
    Some(config)
}

/// A descriptor's tag, its payload start and its end, from `at`.
fn descriptor(data: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let tag = *data.get(at)?;
    let mut len = 0_usize;
    let mut i = at + 1;
    // The size: up to four bytes of 7 bits, with the top bit meaning "more".
    for _ in 0..4 {
        let b = *data.get(i)?;
        i += 1;
        len = (len << 7) | usize::from(b & 0x7F);
        if b & 0x80 == 0 {
            let end = i.checked_add(len)?;
            return (end <= data.len()).then_some((tag, i, end));
        }
    }
    None
}

/// Reads an `esds` payload (after its version and flags).
pub fn parse_esds(data: &[u8]) -> Option<Esds> {
    // ES_Descriptor (tag 3).
    let (tag, start, end) = descriptor(data, 0)?;
    if tag != 3 {
        return None;
    }
    let es = data.get(start..end)?;
    let flags = *es.get(2)?;
    let mut at = 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += 1 + usize::from(*es.get(at)?);
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    // DecoderConfigDescriptor (tag 4).
    let (tag, start, end) = descriptor(es, at)?;
    if tag != 4 {
        return None;
    }
    let dc = es.get(start..end)?;
    let avg_bitrate = u32::from_be_bytes(dc.get(9..13)?.try_into().ok()?);
    // DecoderSpecificInfo (tag 5), if present.
    let config = descriptor(dc, 13)
        .filter(|&(tag, _, _)| tag == 5)
        .and_then(|(_, start, end)| parse_audio_config(dc.get(start..end)?));
    Some(Esds {
        object_type_indication: *dc.first()?,
        stream_type: dc.get(1)? >> 2,
        avg_bitrate,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffmpeg_stereo_and_mono_configs() {
        // From a real FFmpeg 8.0.1 clip (container.md §2, F10): AAC LC, 48 kHz, plus the
        // explicit "SBR absent" extension.
        let stereo = parse_audio_config(&[0x11, 0x90, 0x56, 0xE5, 0x00]).unwrap();
        assert_eq!(
            stereo,
            AudioConfig {
                object_type: 2,
                core_object_type: 2,
                sample_rate: 48_000,
                channels: 2,
                frame_length_flag: false,
                depends_on_core_coder: false,
                sync_extension: Some((5, false)),
            }
        );
        let mono = parse_audio_config(&[0x11, 0x88, 0x56, 0xE5, 0x00]).unwrap();
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.sync_extension, Some((5, false)));
        // Without the extension.
        let plain = parse_audio_config(&[0x11, 0x90]).unwrap();
        assert_eq!((plain.object_type, plain.sync_extension), (2, None));
    }

    #[test]
    fn reads_other_shapes() {
        // HE-AAC, explicit: AOT 5, 24 kHz core index 6, stereo, then 48 kHz and AOT 2.
        // 00101 0110 0010 | 0011 00010 | 0 0 0
        let he = parse_audio_config(&[0x2B, 0x11, 0x88, 0x00]).unwrap();
        assert_eq!(
            (he.object_type, he.core_object_type, he.sample_rate),
            (5, 2, 24_000)
        );
        // 960-sample frames: AOT 2, index 3, stereo, frameLengthFlag 1.
        let short = parse_audio_config(&[0x11, 0x94]).unwrap();
        assert!(short.frame_length_flag);
        // An explicit rate: index 15 then 24 bits of 48000.
        // 00010 1111 | 0000 0000 1011 1011 1000 0000 | 0010 | 000
        let explicit = parse_audio_config(&[0x17, 0x80, 0x5D, 0xC0, 0x10, 0x00]).unwrap();
        assert_eq!((explicit.sample_rate, explicit.channels), (48_000, 2));
        // SBR signalled present in the extension.
        let sbr = parse_audio_config(&[0x11, 0x90, 0x56, 0xE5, 0x80]).unwrap();
        assert_eq!(sbr.sync_extension, Some((5, true)));
        // extensionFlag set: extensionFlag3 comes first, then the sync extension still follows.
        // 00010 0011 0010 0 0 1 | 0 | 01010110111 00101 1
        let ext = parse_audio_config(&[0x11, 0x91, 0x2B, 0x72, 0xC0]).unwrap();
        assert_eq!(ext.sync_extension, Some((5, true)));
        // After explicit SBR there is no backward-compatible signal to read, whatever follows.
        let explicit = parse_audio_config(&[0x2B, 0x11, 0x88, 0x2B, 0x72, 0xC0]).unwrap();
        assert_eq!((explicit.object_type, explicit.sync_extension), (5, None));
        assert_eq!(parse_audio_config(&[0x11]), None);
    }

    #[test]
    fn reads_an_esds() {
        // ES_Descriptor(id 1) { DecoderConfig(0x40, audio, buffer 0, max 160000, avg 160000)
        //   { DecoderSpecificInfo 11 90 56 e5 00 } SLConfig { 2 } }, as movenc writes it.
        let esds = [
            0x03, 0x80, 0x80, 0x80, 0x25, 0x00, 0x01, 0x00, 0x04, 0x80, 0x80, 0x80, 0x17, 0x40,
            0x15, 0x00, 0x00, 0x00, 0x00, 0x02, 0x71, 0x00, 0x00, 0x02, 0x71, 0x00, 0x05, 0x80,
            0x80, 0x80, 0x05, 0x11, 0x90, 0x56, 0xE5, 0x00, 0x06, 0x80, 0x80, 0x80, 0x01, 0x02,
        ];
        let parsed = parse_esds(&esds).unwrap();
        assert_eq!(parsed.object_type_indication, 0x40);
        assert_eq!(parsed.stream_type, 5);
        assert_eq!(parsed.avg_bitrate, 160_000);
        assert_eq!(parsed.config.map(|c| c.object_type), Some(2));
        for cut in 0..esds.len() {
            let _ = parse_esds(&esds[..cut]);
        }
        assert_eq!(parse_esds(&esds[..20]), None);
    }
}
