// SPDX-License-Identifier: GPL-3.0-or-later
//! WAV files: `check --cue`'s PCM (docs/research/16-verify-testbed/README.md §6.3 `audio.cue`).
//! 16-, 24- and 32-bit integer PCM and 32-bit float, plain or WAVE_FORMAT_EXTENSIBLE.

/// A WAV file's samples.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    /// Samples per second.
    pub rate: u32,
    /// Channels.
    pub channels: u16,
    /// The samples, -1 to 1, interleaved.
    pub samples: Vec<f32>,
}

/// The biggest WAV `read` takes: a cue is under 300 ms (PLAN:213).
pub const MAX_BYTES: usize = 16 << 20;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Reads a WAV file's `fmt ` and `data` chunks. Messages never repeat the file's content.
pub fn read(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("larger than {} MiB", MAX_BYTES >> 20));
    }
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a WAV file".to_owned());
    }
    let (mut fmt, mut data) = (None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4).unwrap_or(0) as usize;
        let body = bytes
            .get(at + 8..at + 8 + size)
            .ok_or("a chunk runs past the end of the file")?;
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let (fmt, data) = (fmt.ok_or("no fmt chunk")?, data.ok_or("no data chunk")?);
    let (Some(tag), Some(channels), Some(rate), Some(bits)) = (
        u16_at(fmt, 0),
        u16_at(fmt, 2),
        u32_at(fmt, 4),
        u16_at(fmt, 14),
    ) else {
        return Err("a fmt chunk too short".to_owned());
    };
    // WAVE_FORMAT_EXTENSIBLE names the format in its sub-format GUID's first two bytes.
    let format = if tag == 0xFFFE {
        u16_at(fmt, 24).ok_or("an extensible fmt chunk too short")?
    } else {
        tag
    };
    if channels == 0 {
        return Err("no channels".to_owned());
    }
    let samples: Vec<f32> = match (format, bits) {
        (1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| f32::from(i16::from_le_bytes(b)) / 32_768.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[a, b, c]| (i32::from_le_bytes([0, a, b, c]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| (f64::from(i32::from_le_bytes(b)) / 2_147_483_648.0) as f32)
            .collect(),
        (3, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f32::from_le_bytes(b))
            .collect(),
        _ => return Err(format!("format {format} at {bits} bits isn't PCM or float")),
    };
    Ok(Wav {
        rate,
        channels,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file around `data`.
    fn wav(format: u16, channels: u16, bits: u16, data: &[u8], extensible: bool) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend((if extensible { 0xFFFE } else { format }).to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(48_000_u32.to_le_bytes());
        fmt.extend((48_000 * u32::from(channels) * u32::from(bits / 8)).to_le_bytes());
        fmt.extend((channels * bits / 8).to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        if extensible {
            fmt.extend(22_u16.to_le_bytes());
            fmt.extend(bits.to_le_bytes());
            fmt.extend(3_u32.to_le_bytes());
            fmt.extend(format.to_le_bytes());
            fmt.extend([0; 14]);
        }
        let mut out = b"RIFF".to_vec();
        out.extend(((4 + 8 + fmt.len() + 8 + data.len()) as u32).to_le_bytes());
        out.extend(b"WAVE");
        out.extend(b"fmt ");
        out.extend((fmt.len() as u32).to_le_bytes());
        out.extend(&fmt);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn reads_pcm_and_float() {
        let pcm16 = wav(1, 2, 16, &[0x00, 0x40, 0x00, 0xC0], false);
        assert_eq!(
            read(&pcm16).unwrap(),
            Wav {
                rate: 48_000,
                channels: 2,
                samples: vec![0.5, -0.5]
            }
        );
        let pcm24 = wav(1, 1, 24, &[0x00, 0x00, 0xC0], true);
        assert_eq!(read(&pcm24).unwrap().samples, [-0.5]);
        let pcm32 = wav(1, 1, 32, &0x4000_0000_i32.to_le_bytes(), false);
        assert_eq!(read(&pcm32).unwrap().samples, [0.5]);
        let float = wav(3, 1, 32, &0.25_f32.to_le_bytes(), true);
        assert_eq!(read(&float).unwrap().samples, [0.25]);
    }

    #[test]
    fn refuses_what_it_cant_read() {
        assert!(read(b"RIFF").is_err());
        assert!(read(&wav(2, 1, 4, &[0; 4], false)).is_err());
        let mut cut = wav(1, 1, 16, &[0; 8], false);
        cut.truncate(cut.len() - 4);
        assert!(read(&cut).is_err());
        assert!(read(&vec![0; MAX_BYTES + 1]).is_err());
    }
}
