// SPDX-License-Identifier: GPL-3.0-or-later
//! Job files and job results of the M0 contract (docs/PLAN.md §3.1).
//!
//! `\Kairo\Run` starts `kairo-devrun run --job C:\kairo\jobs\next.json` in the owner's session.
//! The job runs with a time limit and, if asked, stops at the owner's first input. Its result
//! goes to `C:\kairo\logs\<id>.result.json` and its output to `<id>.out.log` and `<id>.err.log`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Longest job id; ids match `^[A-Za-z0-9_-]{1,64}$`.
pub const MAX_JOB_ID_LEN: usize = 64;
/// Longest time limit `kairo-devrun` accepts for one job: 24 hours.
pub const MAX_JOB_SECONDS: u64 = 24 * 60 * 60;
/// Exit code given to a job's processes when `kairo-devrun` stops them.
pub const ABORT_EXIT_CODE: u32 = 1;

/// Whether a console program gets a console window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    /// Console programs get their own console window.
    Visible,
    /// No console window (`CREATE_NO_WINDOW`). GUI programs still show their own windows.
    #[default]
    Hidden,
}

/// A job file, e.g. `C:\kairo\jobs\next.json`.
///
/// `id`, `cmd` and `max_seconds` are required. Missing optional fields take the safe choice:
/// `cwd` null (the layout root), `abort_on_input` true, `window` hidden. Unknown fields are
/// rejected, so a misspelt field fails loudly instead of being ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobFile {
    /// Names the result and log files; matches `^[A-Za-z0-9_-]{1,64}$`.
    pub id: String,
    /// Program and arguments, e.g. `["C:\\kairo\\bin\\current\\kairo-hello.exe"]`.
    pub cmd: Vec<String>,
    /// Working folder; null means the layout root (`C:\kairo`).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Hard limit in seconds (1 to 86,400); then every process of the job is stopped.
    pub max_seconds: u64,
    /// Stop the job at the owner's first keyboard or mouse input after it starts.
    #[serde(default = "default_abort_on_input")]
    pub abort_on_input: bool,
    /// Console window for console programs.
    #[serde(default)]
    pub window: WindowMode,
}

fn default_abort_on_input() -> bool {
    true
}

/// Why a job file was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// The file is neither UTF-8 nor UTF-16 with a byte-order mark.
    Encoding,
    /// The text is not a job object; the message comes from the JSON parser.
    Json(String),
    /// `id` does not match `^[A-Za-z0-9_-]{1,64}$`.
    BadId,
    /// `cmd` is empty or its program is an empty string.
    NoProgram,
    /// `max_seconds` is outside 1 to 86,400.
    BadMaxSeconds(u64),
    /// `cwd` is an empty string.
    EmptyCwd,
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoding => f.write_str("the job file is not UTF-8 or UTF-16 text"),
            Self::Json(message) => write!(f, "the job file is not a valid job: {message}"),
            Self::BadId => f.write_str("`id` must match ^[A-Za-z0-9_-]{1,64}$"),
            Self::NoProgram => f.write_str("`cmd` must start with a program"),
            Self::BadMaxSeconds(seconds) => {
                write!(
                    f,
                    "`max_seconds` must be 1 to {MAX_JOB_SECONDS}, not {seconds}"
                )
            }
            Self::EmptyCwd => f.write_str("`cwd` must be null or a folder"),
        }
    }
}

impl std::error::Error for JobError {}

impl JobFile {
    /// Checks the rules serde cannot express.
    pub fn validate(&self) -> Result<(), JobError> {
        if !is_valid_job_id(&self.id) {
            return Err(JobError::BadId);
        }
        if self.cmd.first().is_none_or(|program| program.is_empty()) {
            return Err(JobError::NoProgram);
        }
        if self.max_seconds == 0 || self.max_seconds > MAX_JOB_SECONDS {
            return Err(JobError::BadMaxSeconds(self.max_seconds));
        }
        if self.cwd.as_deref() == Some("") {
            return Err(JobError::EmptyCwd);
        }
        Ok(())
    }
}

/// True if `id` matches `^[A-Za-z0-9_-]{1,64}$`, which also keeps it safe in file names.
pub fn is_valid_job_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_JOB_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Parses and validates a job file.
///
/// Accepts UTF-8 with or without a byte-order mark and UTF-16 with one, which covers what
/// Windows PowerShell 5.1 writes with `Set-Content -Encoding utf8` (BOM) or `Out-File` (UTF-16).
pub fn parse_job(bytes: &[u8]) -> Result<JobFile, JobError> {
    let text = decode_text(bytes).ok_or(JobError::Encoding)?;
    let job: JobFile =
        serde_json::from_str(&text).map_err(|error| JobError::Json(error.to_string()))?;
    job.validate()?;
    Ok(job)
}

/// The job's id if it is valid, even when the rest of the file is not, so `kairo-devrun` can
/// still report the failure in the result file the caller is waiting for.
pub fn peek_job_id(bytes: &[u8]) -> Option<String> {
    let text = decode_text(bytes)?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let id = value.get("id")?.as_str()?;
    is_valid_job_id(id).then(|| id.to_owned())
}

/// Decodes UTF-8 (with or without a byte-order mark) or UTF-16 LE/BE with a byte-order mark.
pub fn decode_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).ok();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    String::from_utf8(bytes.to_vec()).ok()
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    let (pairs, odd_byte) = bytes.as_chunks::<2>();
    if !odd_byte.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().map(|pair| unit(*pair)).collect();
    String::from_utf16(&units).ok()
}

/// True if the owner gave input after the job started: the last-input time moved.
///
/// Both values are `GetLastInputInfo` tick counts. If the start value is unknown, any later
/// reading counts as input (stop rather than risk disturbing the owner).
pub fn input_since(at_start: Option<u32>, now: Option<u32>) -> bool {
    match (at_start, now) {
        (Some(start), Some(now)) => start != now,
        (None, Some(_)) => true,
        (_, None) => false,
    }
}

/// Why a job ended: `reason` in the result file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// Every process of the job exited by itself.
    Exited,
    /// Stopped after `max_seconds`.
    Timeout,
    /// Stopped because the owner used the keyboard or mouse.
    Input,
    /// The job file was invalid; details in `<id>.err.log`.
    InvalidJob,
    /// `kairo-devrun` was not running in the owner's interactive session (e.g. it ran from SSH).
    NotInteractive,
    /// The program could not be started; details in `<id>.err.log`.
    SpawnFailed,
    /// `kairo-devrun` failed while supervising the job; details in `<id>.err.log`.
    InternalError,
}

impl EndReason {
    /// True when `kairo-devrun` stopped the job's processes.
    pub fn is_abort(self) -> bool {
        matches!(self, Self::Timeout | Self::Input)
    }
}

/// A job result: `C:\kairo\logs\<id>.result.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobResult {
    /// The job's id.
    pub id: String,
    /// The first program's exit code as an unsigned 32-bit Windows code (3221225477 is an
    /// access violation); null if it never started. Stopped jobs exit with [`ABORT_EXIT_CODE`].
    pub exit_code: Option<u32>,
    /// True when `kairo-devrun` stopped the job (time limit or owner input).
    pub aborted: bool,
    /// Why the job ended.
    pub reason: EndReason,
    /// When `kairo-devrun` started the job, RFC 3339 UTC.
    pub started_utc: String,
    /// When the job ended, RFC 3339 UTC.
    pub ended_utc: String,
}

impl JobResult {
    /// A result; `aborted` follows from `reason`.
    pub fn new(
        id: &str,
        exit_code: Option<u32>,
        reason: EndReason,
        started_utc: String,
        ended_utc: String,
    ) -> Self {
        Self {
            id: id.to_owned(),
            exit_code,
            aborted: reason.is_abort(),
            reason,
            started_utc,
            ended_utc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"id":"m0-hello_1","cmd":["C:\\kairo\\bin\\current\\kairo-hello.exe","--x"],"cwd":null,"max_seconds":300,"abort_on_input":false,"window":"visible"}"#;

    #[test]
    fn parses_a_full_job() {
        let job = parse_job(FULL.as_bytes()).unwrap();
        assert_eq!(job.id, "m0-hello_1");
        assert_eq!(job.cmd, [r"C:\kairo\bin\current\kairo-hello.exe", "--x"]);
        assert_eq!(job.cwd, None);
        assert_eq!(job.max_seconds, 300);
        assert!(!job.abort_on_input);
        assert_eq!(job.window, WindowMode::Visible);
    }

    #[test]
    fn optional_fields_default_to_the_safe_choice() {
        let job = parse_job(br#"{"id":"a","cmd":["x.exe"],"max_seconds":5}"#).unwrap();
        assert_eq!(job.cwd, None);
        assert!(job.abort_on_input);
        assert_eq!(job.window, WindowMode::Hidden);
    }

    #[test]
    fn accepts_powershell_encodings() {
        let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
        utf8_bom.extend_from_slice(FULL.as_bytes());
        let mut utf16_le = vec![0xFF, 0xFE];
        utf16_le.extend(FULL.encode_utf16().flat_map(u16::to_le_bytes));
        let mut utf16_be = vec![0xFE, 0xFF];
        utf16_be.extend(FULL.encode_utf16().flat_map(u16::to_be_bytes));
        for bytes in [utf8_bom, utf16_le, utf16_be] {
            assert_eq!(parse_job(&bytes).unwrap().id, "m0-hello_1");
        }
        assert_eq!(parse_job(&[0xFF, 0xFE, 0x41]), Err(JobError::Encoding));
        assert_eq!(parse_job(&[0xC3, 0x28]), Err(JobError::Encoding));
    }

    #[test]
    fn rejects_bad_ids() {
        let too_long = "a".repeat(65);
        let ids = [
            "",
            "has space",
            "dot.dot",
            r"back\slash",
            "slash/x",
            "é",
            too_long.as_str(),
        ];
        for id in ids {
            let text = format!(
                r#"{{"id":{},"cmd":["x"],"max_seconds":1}}"#,
                serde_json::json!(id)
            );
            assert_eq!(parse_job(text.as_bytes()), Err(JobError::BadId), "{id}");
        }
        assert!(is_valid_job_id(&"a".repeat(64)));
        assert!(is_valid_job_id("Az09_-"));
    }

    #[test]
    fn rejects_bad_fields() {
        let cases = [
            (
                r#"{"id":"a","cmd":[],"max_seconds":1}"#,
                JobError::NoProgram,
            ),
            (
                r#"{"id":"a","cmd":[""],"max_seconds":1}"#,
                JobError::NoProgram,
            ),
            (
                r#"{"id":"a","cmd":["x"],"max_seconds":0}"#,
                JobError::BadMaxSeconds(0),
            ),
            (
                r#"{"id":"a","cmd":["x"],"max_seconds":86401}"#,
                JobError::BadMaxSeconds(86_401),
            ),
            (
                r#"{"id":"a","cmd":["x"],"max_seconds":1,"cwd":""}"#,
                JobError::EmptyCwd,
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_job(text.as_bytes()), Err(expected), "{text}");
        }
        for text in [
            r#"{"id":"a","cmd":["x"],"max_seconds":1,"maxSeconds":1}"#,
            r#"{"id":"a","cmd":["x"]}"#,
            r#"{"id":"a","cmd":"x","max_seconds":1}"#,
            r#"{"id":"a","cmd":["x"],"max_seconds":-1}"#,
            r#"{"id":"a","cmd":["x"],"max_seconds":1,"window":"minimized"}"#,
            "[]",
        ] {
            assert!(
                matches!(parse_job(text.as_bytes()), Err(JobError::Json(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn peeks_the_id_of_an_invalid_job() {
        assert_eq!(
            peek_job_id(br#"{"id":"ok-1","cmd":[]}"#),
            Some("ok-1".to_owned())
        );
        assert_eq!(peek_job_id(br#"{"id":"bad id"}"#), None);
        assert_eq!(peek_job_id(b"not json"), None);
    }

    #[test]
    fn input_detection() {
        assert!(!input_since(Some(10), Some(10)));
        assert!(input_since(Some(10), Some(11)));
        assert!(input_since(Some(u32::MAX), Some(0)));
        assert!(input_since(None, Some(5)));
        assert!(!input_since(Some(5), None));
        assert!(!input_since(None, None));
    }

    #[test]
    fn result_has_exactly_the_contract_fields() {
        let result = JobResult::new(
            "m0-1",
            Some(1),
            EndReason::Timeout,
            "2026-10-21T20:00:00.000Z".to_owned(),
            "2026-10-21T20:05:00.000Z".to_owned(),
        );
        assert!(result.aborted);
        let value = serde_json::to_value(&result).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "aborted",
                "ended_utc",
                "exit_code",
                "id",
                "reason",
                "started_utc"
            ]
        );
        assert_eq!(value["reason"], "timeout");
        let never_started = JobResult::new(
            "m0-2",
            None,
            EndReason::SpawnFailed,
            String::new(),
            String::new(),
        );
        let value = serde_json::to_value(&never_started).unwrap();
        assert_eq!(value["exit_code"], serde_json::Value::Null);
        assert_eq!(value["reason"], "spawn_failed");
        assert_eq!(value["aborted"], false);
    }

    #[test]
    fn end_reasons_serialize_in_snake_case() {
        let names: Vec<String> = [
            EndReason::Exited,
            EndReason::Timeout,
            EndReason::Input,
            EndReason::InvalidJob,
            EndReason::NotInteractive,
            EndReason::SpawnFailed,
            EndReason::InternalError,
        ]
        .iter()
        .map(|reason| {
            serde_json::to_value(reason)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
        assert_eq!(
            names,
            [
                "exited",
                "timeout",
                "input",
                "invalid_job",
                "not_interactive",
                "spawn_failed",
                "internal_error"
            ]
        );
    }
}
