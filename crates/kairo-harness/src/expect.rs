// SPDX-License-Identifier: GPL-3.0-or-later
//! The capture's `expect.json` (docs/research/16-verify-testbed/README.md §6.1): what
//! `kairo-verify` should find in the clip. Gate 2 records video only (Stereo Mix stays off,
//! M1:187), so it lists no track.

use serde::Serialize;

/// The expect file's fields, in the order README §6.1 lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Expect {
    /// The format's version: 1.
    pub kairo_expect: u32,
    /// The test pattern: 1.
    pub pattern: u32,
    /// The clip's width and height.
    pub size: [u32; 2],
    /// The clip's frame rate.
    pub fps: u32,
    /// The testbed's monitor's refresh rate, rounded to whole Hz.
    pub source_hz: u64,
    /// How long the capture runs.
    pub seconds: u32,
    /// The testbed's run tag.
    pub run_tag: u8,
    /// No HDR: the harness refuses an HDR monitor.
    pub hdr: Option<()>,
    /// No trim.
    pub trim: Option<()>,
    /// No audio track.
    pub tracks: [(); 0],
}

impl Expect {
    /// The expect file of a `seconds` capture at `size` and `fps` of a testbed tagged `run_tag`
    /// on a monitor refreshing at `refresh_mhz`.
    pub fn new(size: (u32, u32), fps: u32, refresh_mhz: u64, seconds: u32, run_tag: u8) -> Expect {
        Expect {
            kairo_expect: 1,
            pattern: 1,
            size: [size.0, size.1],
            fps,
            source_hz: (refresh_mhz + 500) / 1_000,
            seconds,
            run_tag,
            hdr: None,
            trim: None,
            tracks: [],
        }
    }

    /// The file's text.
    pub fn json(&self) -> String {
        // Serializing plain fields can't fail.
        serde_json::to_string(self).unwrap_or_default() + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_expect_file_word_for_word() {
        let e = Expect::new((1920, 1080), 60, 179_960, 30, 77);
        assert_eq!(
            e.json(),
            "{\"kairo_expect\":1,\"pattern\":1,\"size\":[1920,1080],\"fps\":60,\"source_hz\":180,\"seconds\":30,\"run_tag\":77,\"hdr\":null,\"trim\":null,\"tracks\":[]}\n"
        );
        assert_eq!(Expect::new((2560, 1440), 60, 359_500, 10, 0).source_hz, 360);
        assert_eq!(Expect::new((2560, 1440), 60, 59_940, 10, 0).source_hz, 60);
    }
}
