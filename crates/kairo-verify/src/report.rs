// SPDX-License-Identifier: GPL-3.0-or-later
//! The report (docs/research/16-verify-testbed/README.md §6.6; M1 contract: "Claude decides the
//! layout"): one JSON object per clip, `{"kairo_verify":1,…}`, and a text summary that fits one
//! screen. Reports name the file only, never its folder, and list `ilst` keys, never values
//! (CLAUDE.md).

use std::fmt::Write as _;

use serde::Serialize;

use crate::checks::{Finding, Status};

/// The report format's version.
pub const VERSION: u32 = 1;

/// The clip's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every required check passed.
    Pass,
    /// A required check failed.
    Fail,
    /// The clip couldn't be evaluated, or a required check couldn't run.
    Error,
}

impl std::fmt::Display for Verdict {
    /// `pass`, `fail` or `error`, as in the JSON.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::Error => "error",
        })
    }
}

impl Verdict {
    /// `kairo-verify check`'s exit code: 0 pass, 1 fail, 3 cannot evaluate.
    pub fn exit_code(self) -> u8 {
        match self {
            Verdict::Pass => 0,
            Verdict::Fail => 1,
            Verdict::Error => 3,
        }
    }
}

/// The file checked: its name, never its folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileInfo {
    /// The file name.
    pub name: String,
    /// Its length in bytes.
    pub bytes: u64,
}

/// The decoder used for the content checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Decoder {
    /// `linked`, `cli`, `synthetic` or `none`.
    pub kind: &'static str,
}

/// One line per part of the clip.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// The container.
    pub container: String,
    /// The video track.
    pub video: String,
    /// The audio tracks.
    pub audio: String,
    /// The content checks.
    pub content: String,
    /// `original` only: whether plain Copy may hand over this file as it is (PLAN:249).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shareable_original: Option<bool>,
}

/// How long each stage took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Timings {
    /// Reading the boxes and the NAL headers.
    pub parse: u64,
    /// Decoding the pictures, readings included.
    pub decode: u64,
    /// Running the checks.
    pub analysis: u64,
}

/// One clip's report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The format version: [`VERSION`].
    pub kairo_verify: u32,
    /// This tool's version.
    pub tool: &'static str,
    /// The test pattern's version, from the expect file.
    pub pattern: Option<u32>,
    /// The profile.
    pub profile: &'static str,
    /// `m3` under `--gate m3`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<&'static str>,
    /// The verdict.
    pub verdict: Verdict,
    /// Why the clip couldn't be evaluated, when it couldn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The file.
    pub file: FileInfo,
    /// The decoder.
    pub decoder: Decoder,
    /// One line per part.
    pub summary: Summary,
    /// Every check, in order.
    pub checks: Vec<Finding>,
    /// Stage times in milliseconds.
    pub timings_ms: Timings,
}

impl Report {
    /// The report as one line of JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"kairo_verify":1,"verdict":"error","error":"the report couldn't be serialized"}"#.to_owned()
        })
    }

    /// The text summary: a headline, one line per part, then what failed, warned or couldn't
    /// run.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let gate = self.gate.map(|g| format!(", gate {g}")).unwrap_or_default();
        let verdict = match self.verdict {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Error => "ERROR",
        };
        let _ = writeln!(
            out,
            "{verdict}  {} (profile {}{gate})",
            self.file.name, self.profile
        );
        if let Some(error) = &self.error {
            let _ = writeln!(out, "  cannot evaluate: {error}");
            return out;
        }
        for (label, line) in [
            ("container", &self.summary.container),
            ("video", &self.summary.video),
            ("audio", &self.summary.audio),
            ("content", &self.summary.content),
        ] {
            let _ = writeln!(out, "  {label:<10}{line}");
        }
        if let Some(shareable) = self.summary.shareable_original {
            let _ = writeln!(
                out,
                "  {:<10}{}",
                "copy",
                if shareable {
                    "the original may be handed over"
                } else {
                    "Copy makes a share copy first"
                }
            );
        }
        let mut lines = |status: Status, label: &str| {
            let picked: Vec<&Finding> = self.checks.iter().filter(|f| f.status == status).collect();
            match picked.as_slice() {
                [] => {}
                // Required checks that couldn't run all say the same thing: list them once.
                many if status == Status::Error => {
                    let ids: Vec<&str> = many.iter().map(|f| f.id).collect();
                    let _ = writeln!(out, "  {label:<10}{}: {}", ids.join(", "), many[0].detail);
                }
                many => {
                    for f in many {
                        let more = if f.events_total > 1 {
                            format!(" (+{} more)", f.events_total - 1)
                        } else {
                            String::new()
                        };
                        let _ = writeln!(out, "  {label:<10}{}: {}{more}", f.id, f.detail);
                    }
                }
            }
        };
        lines(Status::Fail, "FAILED");
        lines(Status::Error, "NOT RUN");
        lines(Status::Warn, "warning");
        out
    }
}
