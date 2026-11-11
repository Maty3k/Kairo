// SPDX-License-Identifier: GPL-3.0-or-later
//! The read-back table (docs/research/16-verify-testbed/README.md §8.5, M1:27 "every option set
//! and read back"). The CLI can't call `av_opt_get`, and `h264_amf` ignores the result of almost
//! every property it sets, so each option is shown as what proves it: a line of FFmpeg's log,
//! a fact `kairo-verify` read from the bitstream, or nothing yet ("set, not read back" until M3
//! task 6). A failed log row stops the run (exit 4) before the clip is judged.

use serde::Serialize;
use serde_json::Value;

use crate::capture::Source;
use crate::ffmpeg_log::{LogCheck, Problem};

/// The most the 1 s peak may reach (CLAUDE.md: H.264 peak ≤50 Mbps).
pub const PEAK_LIMIT_BPS: u64 = 50_000_000;

/// How an option's effect was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// What it should do is there.
    Verified,
    /// What it should do isn't.
    Failed,
    /// The fact that would say wasn't there to read.
    Unknown,
    /// Nothing the CLI exposes shows it.
    NotReadBack,
}

/// Where the proof comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Proof {
    /// FFmpeg's log.
    Log,
    /// The clip, through `kairo-verify`'s report.
    Bitstream,
    /// Nowhere yet.
    None,
}

/// One option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    /// The option as FFmpeg was given it.
    pub option: String,
    /// The outcome.
    pub status: Status,
    /// Where the proof comes from.
    pub source: Proof,
    /// What was found: a check and value, a problem id, never a PC detail.
    pub evidence: String,
}

/// How many rows have each status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Counts {
    /// Verified.
    pub verified: usize,
    /// Failed.
    pub failed: usize,
    /// Unknown.
    pub unknown: usize,
    /// Set, not read back.
    pub not_read_back: usize,
}

/// Counts `rows` by status.
pub fn counts(rows: &[Row]) -> Counts {
    let mut c = Counts::default();
    for row in rows {
        match row.status {
            Status::Verified => c.verified += 1,
            Status::Failed => c.failed += 1,
            Status::Unknown => c.unknown += 1,
            Status::NotReadBack => c.not_read_back += 1,
        }
    }
    c
}

/// A check in `report`: its status and value.
fn finding<'a>(report: Option<&'a Value>, id: &str) -> Option<(&'a str, &'a Value)> {
    report?
        .get("checks")?
        .as_array()?
        .iter()
        .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|c| Some((c.get("status")?.as_str()?, c.get("value")?)))
}

/// A row read from the bitstream: `holds` says whether a value proves the option.
fn bitstream(
    report: Option<&Value>,
    option: &str,
    id: &str,
    what: &str,
    holds: impl Fn(&str, &Value) -> Option<bool>,
) -> Row {
    let (status, evidence) = match finding(report, id) {
        None => (Status::Unknown, format!("{id}: not in the report")),
        Some((check, value)) => match (check, holds(check, value)) {
            ("skip" | "error", _) | (_, None) => (Status::Unknown, format!("{id}: {check}")),
            ("fail", _) | (_, Some(false)) => (Status::Failed, format!("{id}: {check}, {what}")),
            (_, Some(true)) => (Status::Verified, format!("{id}: {check}, {what}")),
        },
    };
    Row {
        option: option.to_owned(),
        status,
        source: Proof::Bitstream,
        evidence,
    }
}

/// A row read from FFmpeg's log: failed if `log` has any of `problems`.
fn from_log(log: &LogCheck, option: &str, problems: &[Problem]) -> Row {
    let found: Vec<&str> = log
        .problems
        .iter()
        .filter(|p| problems.contains(p))
        .map(|p| p.id())
        .collect();
    Row {
        option: option.to_owned(),
        status: if found.is_empty() {
            Status::Verified
        } else {
            Status::Failed
        },
        source: Proof::Log,
        evidence: if found.is_empty() {
            "the log's lines are there".to_owned()
        } else {
            found.join(", ")
        },
    }
}

fn not_read_back(option: &str, evidence: &str) -> Row {
    Row {
        option: option.to_owned(),
        status: Status::NotReadBack,
        source: Proof::None,
        evidence: evidence.to_owned(),
    }
}

/// The table for a capture from `source`, whose log said `log`, of `seconds`, judged by
/// `report` (`kairo-verify`'s JSON, if there is one); `finishing`: `h264_metadata` was applied.
pub fn table(
    report: Option<&Value>,
    log: &LogCheck,
    source: Source,
    seconds: u32,
    finishing: bool,
) -> Vec<Row> {
    let int = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64);
    let mut rows = vec![
        from_log(
            log,
            "-c:v h264_amf",
            &[Problem::NoAmfDevice, Problem::NotMappedToAmf],
        ),
        from_log(
            log,
            "-init_hw_device d3d11va",
            &[
                Problem::NoAdapterSelected,
                Problem::NoAdapterDevice,
                Problem::AdapterMismatch,
            ],
        ),
    ];
    rows.push(match source {
        Source::Window { .. } => from_log(
            log,
            "-filter_hw_device kd",
            &[Problem::CaptureDeviceNotShared],
        ),
        Source::Desktop { .. } => {
            not_read_back("-filter_hw_device kd", "ddagrab names no device in the log")
        }
    });
    rows.extend([
        from_log(log, "AMF on D3D11", &[Problem::NoAmfD3d11]),
        from_log(log, "-pix_fmt nv12", &[Problem::NotNv12Bt709]),
        from_log(log, "no option unused", &[Problem::UnusedOption]),
        bitstream(
            report,
            "-profile:v high",
            "video.params",
            "profile 100",
            |_, v| Some(int(v, "profile")? == 100),
        ),
        bitstream(report, "-coder cabac", "video.params", "CABAC", |_, v| {
            v.get("cabac")?.as_bool()
        }),
        bitstream(report, "-g 60", "video.gop", "an IDR every 60", |s, _| {
            Some(s == "pass")
        }),
    ]);
    for option in ["-bf 0", "-max_b_frames 0"] {
        rows.push(bitstream(
            report,
            option,
            "video.no_reorder",
            "no B slice",
            |s, v| Some(s == "pass" && int(v, "b_samples")? == 0),
        ));
    }
    rows.extend([
        bitstream(
            report,
            "-header_spacing 60",
            "video.gop",
            "an SPS and a PPS with every IDR",
            |_, v| {
                let idr = int(v, "idr")?;
                Some(idr > 0 && int(v, "idr_with_params")? == idr)
            },
        ),
        bitstream(
            report,
            "-filler_data 0",
            "video.params",
            "no filler unit",
            |_, v| Some(int(v, "filler_nals")? == 0),
        ),
        bitstream(
            report,
            "-frame_skipping 0",
            "video.cfr",
            "one sample per tick",
            |s, v| Some(s == "pass" && int(v, "irregular")? == 0),
        ),
        bitstream(
            report,
            if finishing {
                "-bsf:v h264_metadata"
            } else {
                "setparams (VUI colour)"
            },
            "video.params",
            "VUI 1/1/1, limited",
            |_, v| {
                let colour = v.get("vui_colour")?.as_array()?;
                let bt709 = colour.len() == 3 && colour.iter().all(|c| c.as_u64() == Some(1));
                Some(bt709 && !v.get("vui_full_range")?.as_bool()?)
            },
        ),
        bitstream(
            report,
            "colr 1/1/1/0",
            "video.colr",
            "BT.709 limited",
            |s, _| Some(s == "pass"),
        ),
    ]);
    // The peak: verified under the limit; above it, the rate control's doing can't be told
    // from the content's, so unknown, not failed (README §12 risk 10's note).
    let peak = match finding(report, "video.bitrate") {
        Some((status, v)) if status != "skip" && status != "error" => int(v, "peak_bps"),
        _ => None,
    };
    rows.push(Row {
        option: "-rc vbr_peak -maxrate".to_owned(),
        status: match peak {
            Some(p) if p <= PEAK_LIMIT_BPS => Status::Verified,
            _ => Status::Unknown,
        },
        source: Proof::Bitstream,
        evidence: match peak {
            Some(p) => format!("video.bitrate: peak {:.1} Mbps in 1 s", p as f64 / 1e6),
            None => "video.bitrate: not read".to_owned(),
        },
    });
    rows.extend([
        bitstream(
            report,
            "-r 60 -fps_mode cfr",
            "video.cfr",
            "CFR 60",
            |s, _| Some(s == "pass"),
        ),
        bitstream(
            report,
            &format!("-t {seconds}"),
            "duration",
            "the length",
            |s, _| Some(s == "pass"),
        ),
        bitstream(
            report,
            "-movflags +faststart",
            "container.faststart",
            "moov first",
            |s, _| Some(s == "pass"),
        ),
        bitstream(
            report,
            "-use_editlist 0",
            "container.no_editlist",
            "no edit list",
            |s, _| Some(s == "pass"),
        ),
    ]);
    let average = finding(report, "video.bitrate").and_then(|(_, v)| int(v, "average_bps"));
    rows.push(not_read_back(
        "-b:v",
        &average.map_or("the average isn't the target's proof".to_owned(), |a| {
            format!(
                "average {:.1} Mbps (not proof of the target)",
                a as f64 / 1e6
            )
        }),
    ));
    for option in [
        "-bufsize",
        "-usage transcoding",
        "-quality balanced",
        "-vbaq 1",
        "-preanalysis 0",
        "-high_motion_quality_boost_enable 0",
        "-preencode 0",
        "-async_depth 4",
    ] {
        rows.push(not_read_back(option, "until M3 task 6 reads options back"));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(change: impl Fn(&mut Value)) -> Value {
        let mut r = json!({"kairo_verify": 1, "verdict": "pass", "checks": [
            {"id": "video.params", "status": "pass", "value": {"profile": 100, "cabac": true,
             "vui_colour": [1, 1, 1], "vui_full_range": false, "filler_nals": 0, "inband_pps": 30}},
            {"id": "video.gop", "status": "pass", "value": {"idr": 30, "idr_with_params": 30}},
            {"id": "video.no_reorder", "status": "pass", "value": {"b_samples": 0}},
            {"id": "video.cfr", "status": "pass", "value": {"irregular": 0}},
            {"id": "video.colr", "status": "pass", "value": {}},
            {"id": "video.bitrate", "status": "pass", "value": {"average_bps": 24_000_000_u64, "peak_bps": 39_000_000_u64}},
            {"id": "duration", "status": "pass", "value": {}},
            {"id": "container.faststart", "status": "pass", "value": {}},
            {"id": "container.no_editlist", "status": "pass", "value": {}}
        ]});
        change(&mut r);
        r
    }

    fn window() -> Source {
        Source::Window { hwnd: 1 }
    }

    fn status(rows: &[Row], option: &str) -> Status {
        rows.iter().find(|r| r.option == option).unwrap().status
    }

    #[test]
    fn a_good_capture_reads_back_every_option_it_can() {
        let r = report(|_| {});
        let rows = table(Some(&r), &LogCheck::default(), window(), 30, false);
        let c = counts(&rows);
        assert_eq!((c.failed, c.unknown), (0, 0), "{rows:#?}");
        assert_eq!(c.not_read_back, 9);
        assert_eq!(c.verified, rows.len() - 9);
        assert_eq!(status(&rows, "-t 30"), Status::Verified);
        // ddagrab names no device.
        let rows = table(
            Some(&r),
            &LogCheck::default(),
            Source::Desktop {
                output: 0,
                crop: (0, 0, 1, 1),
            },
            30,
            false,
        );
        assert_eq!(status(&rows, "-filter_hw_device kd"), Status::NotReadBack);
    }

    #[test]
    fn b_slices_and_a_missing_header_fail_their_rows() {
        let r = report(|r| {
            r["checks"][2]["status"] = json!("fail");
            r["checks"][2]["value"]["b_samples"] = json!(12);
            r["checks"][1]["value"]["idr_with_params"] = json!(29);
        });
        let rows = table(Some(&r), &LogCheck::default(), window(), 30, false);
        assert_eq!(status(&rows, "-bf 0"), Status::Failed);
        assert_eq!(status(&rows, "-max_b_frames 0"), Status::Failed);
        assert_eq!(status(&rows, "-header_spacing 60"), Status::Failed);
        assert_eq!(status(&rows, "-g 60"), Status::Verified);
    }

    #[test]
    fn missing_facts_are_unknown_and_a_high_peak_isnt_a_failure() {
        let rows = table(None, &LogCheck::default(), window(), 30, false);
        let c = counts(&rows);
        assert_eq!((c.verified, c.failed), (6, 0), "only the log rows");
        let r = report(|r| {
            r["checks"][0]["status"] = json!("error");
            r["checks"][5]["value"]["peak_bps"] = json!(55_000_000_u64);
        });
        let rows = table(Some(&r), &LogCheck::default(), window(), 30, true);
        assert_eq!(status(&rows, "-profile:v high"), Status::Unknown);
        assert_eq!(status(&rows, "-bsf:v h264_metadata"), Status::Unknown);
        assert_eq!(status(&rows, "-rc vbr_peak -maxrate"), Status::Unknown);
    }

    #[test]
    fn log_problems_fail_their_rows_by_id() {
        let log = LogCheck {
            problems: vec![Problem::AdapterMismatch, Problem::UnusedOption],
            ..LogCheck::default()
        };
        let rows = table(None, &log, window(), 30, false);
        let row = rows
            .iter()
            .find(|r| r.option == "-init_hw_device d3d11va")
            .unwrap();
        assert_eq!(
            (row.status, row.evidence.as_str()),
            (Status::Failed, "adapter_mismatch")
        );
        assert_eq!(status(&rows, "no option unused"), Status::Failed);
        assert_eq!(status(&rows, "-c:v h264_amf"), Status::Verified);
    }
}
