// SPDX-License-Identifier: GPL-3.0-or-later
//! `expect.json`: what a clip should hold (docs/research/16-verify-testbed/README.md §6.1).
//! `kairo-verify synth`, the harness and later the engine harness write it; `check --expect`
//! reads it. The structure checks use the size, rate, length and tracks; the content checks
//! (research 16's PRs 4 and 5) use the rest.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The `kairo_expect` version this build reads.
pub const VERSION: u32 = 1;

/// What a clip should hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// The format version: [`VERSION`].
    pub kairo_expect: u32,
    /// The test pattern's version (1), or `None` for a clip without one.
    #[serde(default)]
    pub pattern: Option<u32>,
    /// Width and height in pixels.
    #[serde(default)]
    pub size: Option<[u32; 2]>,
    /// Output frames per second.
    pub fps: u32,
    /// The testbed's refresh rate in Hz, if it was recorded from one.
    #[serde(default)]
    pub source_hz: Option<u32>,
    /// The clip's length in seconds.
    pub seconds: u32,
    /// The testbed run's tag in the barcode.
    #[serde(default)]
    pub run_tag: Option<u32>,
    /// For an HDR-sourced clip, how its HDR was mapped.
    #[serde(default)]
    pub hdr: Option<Hdr>,
    /// For a Precise-trim export, the first and last source frame numbers.
    #[serde(default)]
    pub trim: Option<Trim>,
    /// The audio tracks, in order (the video track comes first and isn't listed).
    pub tracks: Vec<Track>,
}

/// How an HDR source was mapped to SDR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hdr {
    /// `natural` (PQ + BT.2390) or `match` (Match SDR apps).
    pub curve: String,
    /// The source's peak luminance in nits.
    pub lw_nits: u32,
    /// The SDR white level in nits.
    pub sdr_white_nits: u32,
}

/// A Precise-trim export's frame range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trim {
    /// The first source frame number kept.
    #[serde(rename = "first_N")]
    pub first_n: u64,
    /// The last source frame number kept.
    #[serde(rename = "last_N")]
    pub last_n: u64,
}

/// An expected audio track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// Its title: `Mix`, `Game`, `Mic`, `Voice chat` or `Everything else`.
    pub title: String,
    /// 1 or 2.
    pub channels: u16,
    /// Whether it is the default (enabled) track.
    pub default: bool,
    /// Which test tones it should carry: `game`, `voice`, `mic`, `cue` → `present`, `absent`
    /// or `any`.
    #[serde(default)]
    pub tones: BTreeMap<String, Tone>,
}

/// Whether a tone should be in a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// It must be there.
    Present,
    /// It must not be there.
    Absent,
    /// Reported only.
    Any,
}

/// Why an `expect.json` was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectError {
    /// Not valid JSON for this format.
    Json(String),
    /// Another version.
    Version(u32),
    /// A value out of range.
    Value(String),
}

impl fmt::Display for ExpectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "expect file: {error}"),
            Self::Version(v) => write!(
                f,
                "expect file: kairo_expect {v}; this build reads {VERSION}"
            ),
            Self::Value(what) => write!(f, "expect file: {what}"),
        }
    }
}

impl std::error::Error for ExpectError {}

/// The tone names a track may list.
const TONES: [&str; 4] = ["game", "voice", "mic", "cue"];

impl Expect {
    /// Parses and checks an `expect.json`.
    pub fn parse(text: &str) -> Result<Expect, ExpectError> {
        let expect: Expect =
            serde_json::from_str(text).map_err(|e| ExpectError::Json(e.to_string()))?;
        if expect.kairo_expect != VERSION {
            return Err(ExpectError::Version(expect.kairo_expect));
        }
        let bad = |what: String| Err(ExpectError::Value(what));
        if !(1..=1000).contains(&expect.fps) {
            return bad(format!("fps {} is not 1–1000", expect.fps));
        }
        if let Some(hz) = expect.source_hz
            && !(1..=1000).contains(&hz)
        {
            return bad(format!("source_hz {hz} is not 1–1000"));
        }
        if expect.seconds == 0 || expect.seconds > 24 * 3600 {
            return bad(format!("seconds {} is not 1–86400", expect.seconds));
        }
        if let Some([w, h]) = expect.size
            && (w == 0 || h == 0 || w > 16_384 || h > 16_384)
        {
            return bad(format!("size {w}x{h} is out of range"));
        }
        if expect.tracks.len() > 16 {
            return bad(format!("{} audio tracks; at most 16", expect.tracks.len()));
        }
        for track in &expect.tracks {
            if !(1..=2).contains(&track.channels) {
                return bad(format!(
                    "track {:?} has {} channels",
                    track.title, track.channels
                ));
            }
            if let Some(name) = track.tones.keys().find(|k| !TONES.contains(&k.as_str())) {
                return bad(format!(
                    "track {:?} lists an unknown tone {name:?}",
                    track.title
                ));
            }
        }
        Ok(expect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_documented_example() {
        // research 16 README §6.1.
        let text = r#"{"kairo_expect":1,"pattern":1,"size":[1920,1080],"fps":60,"source_hz":180,"seconds":30,"run_tag":0,
           "hdr":null,"trim":null,
           "tracks":[{"title":"Mix","channels":2,"default":true,
                      "tones":{"game":"present","voice":"present","mic":"present","cue":"absent"}},
                     {"title":"Game","channels":2,"default":false,
                      "tones":{"game":"present","voice":"absent","mic":"absent","cue":"absent"}}]}"#;
        let expect = Expect::parse(text).unwrap();
        assert_eq!(expect.size, Some([1920, 1080]));
        assert_eq!((expect.fps, expect.seconds), (60, 30));
        assert_eq!(expect.tracks.len(), 2);
        assert_eq!(expect.tracks[1].tones["voice"], Tone::Absent);
        let trim = r#"{"kairo_expect":1,"fps":30,"seconds":5,"tracks":[],"trim":{"first_N":10,"last_N":160}}"#;
        assert_eq!(
            Expect::parse(trim).unwrap().trim,
            Some(Trim {
                first_n: 10,
                last_n: 160
            })
        );
    }

    #[test]
    fn refuses_bad_files() {
        let base = |extra: &str| {
            format!(r#"{{"kairo_expect":1,"fps":60,"seconds":30,"tracks":[]{extra}}}"#)
        };
        assert!(Expect::parse(&base("")).is_ok());
        assert!(matches!(
            Expect::parse(&base(r#","extra":1"#)),
            Err(ExpectError::Json(_))
        ));
        assert!(matches!(
            Expect::parse(r#"{"kairo_expect":2,"fps":60,"seconds":30,"tracks":[]}"#),
            Err(ExpectError::Version(2))
        ));
        for bad in [
            r#"{"kairo_expect":1,"fps":0,"seconds":30,"tracks":[]}"#,
            r#"{"kairo_expect":1,"fps":60,"seconds":0,"tracks":[]}"#,
            r#"{"kairo_expect":1,"fps":60,"seconds":30,"source_hz":0,"tracks":[]}"#,
            r#"{"kairo_expect":1,"fps":60,"seconds":30,"source_hz":100000,"tracks":[]}"#,
            r#"{"kairo_expect":1,"fps":60,"seconds":30,"size":[0,1080],"tracks":[]}"#,
            r#"{"kairo_expect":1,"fps":60,"seconds":30,"tracks":[{"title":"Mic","channels":3,"default":false}]}"#,
            r#"{"kairo_expect":1,"fps":60,"seconds":30,"tracks":[{"title":"Mic","channels":1,"default":false,"tones":{"beep":"any"}}]}"#,
        ] {
            assert!(
                matches!(Expect::parse(bad), Err(ExpectError::Value(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            Expect::parse("not json"),
            Err(ExpectError::Json(_))
        ));
    }
}
