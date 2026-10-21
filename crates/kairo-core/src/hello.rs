// SPDX-License-Identifier: GPL-3.0-or-later
//! The M0 hello probe (`kairo-hello.exe`): its window title, pipe, JSON-RPC methods and key log.
//!
//! The probe proves the bridge works: Claude starts it in the owner's session from the Mac,
//! the owner presses a key in its window, and `kairo-cli` reads the key back over the pipe.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rpc::{self, Incoming, Response, RpcError};

/// Window title.
pub const WINDOW_TITLE: &str = "Hello from Kairo";
/// Pipe name; the full path is `\\.\pipe\kairo-hello`.
pub const PIPE_NAME: &str = "kairo-hello";
/// Who the probe is: `{name, version, session_id, elevated}`.
pub const METHOD_ENGINE_HELLO: &str = "engine.hello";
/// Recent key presses: `{qpc_frequency, total, keys: [...]}`; optional param `{"limit": n}`.
pub const METHOD_HELLO_KEYS: &str = "hello.keys";
/// Key presses kept in memory.
pub const KEYS_KEPT: usize = 256;
/// Key presses `hello.keys` returns without a `limit`.
pub const KEYS_RETURNED_BY_DEFAULT: usize = 50;
/// `session_id` when the probe could not read its own session.
pub const UNKNOWN_SESSION: u32 = u32::MAX;

/// The `engine.hello` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloInfo {
    /// `kairo-hello`.
    pub name: String,
    /// The crate version.
    pub version: String,
    /// The Windows session the probe runs in (0 would mean the services session).
    pub session_id: u32,
    /// True if the probe runs elevated; it should not.
    pub elevated: bool,
}

/// One key press in the probe's window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPress {
    /// 1 for the first key press since the window opened.
    pub seq: u64,
    /// `QueryPerformanceCounter` when the window received the key.
    pub qpc: i64,
    /// Wall-clock time, RFC 3339 UTC.
    pub ts_utc: String,
    /// Windows virtual-key code.
    pub vk: u16,
    /// Readable key name, e.g. `F8`.
    pub key: String,
}

/// The `hello.keys` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysResult {
    /// `QueryPerformanceFrequency`: QPC ticks per second.
    pub qpc_frequency: i64,
    /// Key presses since the window opened.
    pub total: u64,
    /// The most recent presses, oldest first.
    pub keys: Vec<KeyPress>,
}

/// The probe's state: who it is and its most recent key presses.
#[derive(Debug, Clone)]
pub struct HelloState {
    info: HelloInfo,
    qpc_frequency: i64,
    total: u64,
    recent: VecDeque<KeyPress>,
}

impl HelloState {
    /// A probe that has seen no key yet.
    pub fn new(info: HelloInfo, qpc_frequency: i64) -> Self {
        Self {
            info,
            qpc_frequency,
            total: 0,
            recent: VecDeque::with_capacity(KEYS_KEPT),
        }
    }

    /// Who the probe is.
    pub fn info(&self) -> &HelloInfo {
        &self.info
    }

    /// QPC ticks per second.
    pub fn qpc_frequency(&self) -> i64 {
        self.qpc_frequency
    }

    /// Records a key press and returns it, numbered, for the log.
    pub fn record_key(&mut self, qpc: i64, ts_utc: String, vk: u16) -> KeyPress {
        self.total += 1;
        let press = KeyPress {
            seq: self.total,
            qpc,
            ts_utc,
            vk,
            key: key_name(vk),
        };
        if self.recent.len() == KEYS_KEPT {
            self.recent.pop_front();
        }
        self.recent.push_back(press.clone());
        press
    }

    /// Key presses since the window opened.
    pub fn total_keys(&self) -> u64 {
        self.total
    }

    /// The latest key press.
    pub fn last_key(&self) -> Option<&KeyPress> {
        self.recent.back()
    }

    /// Up to `limit` of the most recent key presses, oldest first.
    pub fn keys_result(&self, limit: usize) -> KeysResult {
        let skip = self.recent.len().saturating_sub(limit);
        KeysResult {
            qpc_frequency: self.qpc_frequency,
            total: self.total,
            keys: self.recent.iter().skip(skip).cloned().collect(),
        }
    }
}

/// Answers one JSON-RPC frame. `None` for notifications, which get no response.
pub fn handle_frame(state: &HelloState, payload: &[u8]) -> Option<Response> {
    match rpc::parse_incoming(payload) {
        Err(rejection) => Some(rejection.into_response()),
        Ok(Incoming::Notification { .. }) => None,
        Ok(Incoming::Call { id, method, params }) => Some(match call(state, &method, &params) {
            Ok(result) => Response::success(id, result),
            Err(error) => Response::failure(id, error),
        }),
    }
}

fn call(state: &HelloState, method: &str, params: &Value) -> Result<Value, RpcError> {
    let result = match method {
        METHOD_ENGINE_HELLO => serde_json::to_value(&state.info),
        METHOD_HELLO_KEYS => serde_json::to_value(state.keys_result(keys_limit(params)?)),
        _ => return Err(RpcError::new(rpc::METHOD_NOT_FOUND, "Method not found")),
    };
    result.map_err(|error| RpcError::new(rpc::INTERNAL_ERROR, error.to_string()))
}

/// One line of `C:\kairo\logs\hello.log` (JSON Lines, field `event` first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HelloLogLine<'a> {
    /// The probe started.
    Start {
        /// RFC 3339 UTC.
        ts_utc: &'a str,
        /// Process id.
        pid: u32,
        /// Windows session.
        session_id: u32,
        /// Whether the probe runs elevated.
        elevated: bool,
        /// The pipe's full path.
        pipe: &'a str,
        /// `listening` or why the pipe is not available.
        pipe_status: &'a str,
    },
    /// The owner pressed a key in the window.
    Key {
        /// 1 for the first key press.
        seq: u64,
        /// `QueryPerformanceCounter` when the window received the key.
        qpc: i64,
        /// QPC ticks per second.
        qpc_frequency: i64,
        /// RFC 3339 UTC.
        ts_utc: &'a str,
        /// Windows virtual-key code.
        vk: u16,
        /// Readable key name.
        key: &'a str,
    },
    /// The window closed.
    Stop {
        /// RFC 3339 UTC.
        ts_utc: &'a str,
        /// Key presses while the window was open.
        keys: u64,
    },
}

impl HelloLogLine<'_> {
    /// The line as JSON, without the newline; `None` only if serialization failed.
    pub fn to_json(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}

fn keys_limit(params: &Value) -> Result<usize, RpcError> {
    let invalid = || {
        RpcError::new(
            rpc::INVALID_PARAMS,
            format!("params must be {{}} or {{\"limit\": 1..={KEYS_KEPT}}}"),
        )
    };
    let limit = match params {
        Value::Null => return Ok(KEYS_RETURNED_BY_DEFAULT),
        Value::Object(object) => match object.get("limit") {
            None | Some(Value::Null) => return Ok(KEYS_RETURNED_BY_DEFAULT),
            Some(limit) => limit.as_u64().ok_or_else(invalid)?,
        },
        _ => return Err(invalid()),
    };
    usize::try_from(limit)
        .ok()
        .filter(|limit| (1..=KEYS_KEPT).contains(limit))
        .ok_or_else(invalid)
}

/// The text the probe's window shows.
pub fn window_text(total_keys: u64, last_key: Option<&str>, log_file: &str, pipe: &str) -> String {
    let last = last_key.unwrap_or("none yet");
    format!(
        "{WINDOW_TITLE}\n\n\
         This is Kairo's setup test window. Claude opened it from the Mac to check that \
         programs can start on your screen.\n\n\
         Press any key while this window is selected. Each key press is saved with a precise \
         timestamp in {log_file}, so Claude can see that it arrived.\n\n\
         You can close this window when Claude says the test is done.\n\n\
         Key presses so far: {total_keys} (last: {last})\n\
         Pipe {path}: {pipe}",
        path = pipe_path(PIPE_NAME),
    )
}

/// The SDDL for the pipe: SYSTEM and the given user only, no inherited entries
/// (docs/PLAN.md §2.1; the default DACL would let Everyone read).
pub fn pipe_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;{user_sid})")
}

/// The full path of a local pipe: `kairo-hello` → `\\.\pipe\kairo-hello`.
pub fn pipe_path(name: &str) -> String {
    format!(r"\\.\pipe\{name}")
}

/// Accepts `NAME` or `\\.\pipe\NAME` and returns `NAME` if it is a plain local pipe name.
pub fn parse_pipe_arg(arg: &str) -> Option<String> {
    let name = arg.strip_prefix(r"\\.\pipe\").unwrap_or(arg);
    is_valid_pipe_name(name).then(|| name.to_owned())
}

/// Letters, digits, `-`, `_` and `.`, up to 200 characters.
pub fn is_valid_pipe_name(name: &str) -> bool {
    (1..=200).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A readable name for a Windows virtual-key code, e.g. `0x77` → `F8`.
pub fn key_name(vk: u16) -> String {
    let named = match vk {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x10 => "Shift",
        0x11 => "Ctrl",
        0x12 => "Alt",
        0x13 => "Pause",
        0x14 => "CapsLock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "PrintScreen",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5B => "LeftWin",
        0x5C => "RightWin",
        0x5D => "Menu",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0xA0 => "LeftShift",
        0xA1 => "RightShift",
        0xA2 => "LeftCtrl",
        0xA3 => "RightCtrl",
        0xA4 => "LeftAlt",
        0xA5 => "RightAlt",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => u8::try_from(vk).map_or_else(
            |_| format!("VK_0x{vk:02X}"),
            |ascii| char::from(ascii).to_string(),
        ),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("VK_0x{vk:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn state() -> HelloState {
        HelloState::new(
            HelloInfo {
                name: "kairo-hello".to_owned(),
                version: "0.1.0".to_owned(),
                session_id: 1,
                elevated: false,
            },
            10_000_000,
        )
    }

    fn call_frame(state: &HelloState, request: &str) -> Value {
        let response = handle_frame(state, request.as_bytes()).unwrap();
        serde_json::from_slice(&response.to_bytes()).unwrap()
    }

    #[test]
    fn engine_hello_returns_exactly_the_contract_fields() {
        let reply = call_frame(
            &state(),
            r#"{"jsonrpc":"2.0","id":1,"method":"engine.hello"}"#,
        );
        assert_eq!(
            reply,
            json!({"jsonrpc": "2.0", "id": 1, "result": {
                "name": "kairo-hello", "version": "0.1.0", "session_id": 1, "elevated": false
            }})
        );
    }

    #[test]
    fn hello_keys_returns_the_latest_presses_oldest_first() {
        let mut state = state();
        for (i, vk) in [0x41_u16, 0x77, 0x20].into_iter().enumerate() {
            let press = state.record_key(100 + i64::try_from(i).unwrap(), "t".to_owned(), vk);
            assert_eq!(press.seq, u64::try_from(i).unwrap() + 1);
        }
        let reply = call_frame(
            &state,
            r#"{"jsonrpc":"2.0","id":"k","method":"hello.keys","params":{"limit":2}}"#,
        );
        let result = &reply["result"];
        assert_eq!(result["total"], 3);
        assert_eq!(result["qpc_frequency"], 10_000_000);
        let keys: Vec<&str> = result["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["F8", "Space"]);
        assert_eq!(result["keys"][1]["qpc"], 102);
        let all = call_frame(&state, r#"{"jsonrpc":"2.0","id":2,"method":"hello.keys"}"#);
        assert_eq!(all["result"]["keys"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn only_the_newest_presses_are_kept() {
        let mut state = state();
        for i in 0..(KEYS_KEPT + 10) {
            state.record_key(i64::try_from(i).unwrap(), String::new(), 0x41);
        }
        assert_eq!(state.total_keys(), u64::try_from(KEYS_KEPT + 10).unwrap());
        let result = state.keys_result(KEYS_KEPT);
        assert_eq!(result.keys.len(), KEYS_KEPT);
        assert_eq!(result.keys[0].seq, 11);
        assert_eq!(state.last_key().map(|key| key.seq), Some(266));
    }

    #[test]
    fn bad_calls_get_standard_errors() {
        let state = state();
        let unknown = call_frame(&state, r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#);
        assert_eq!(unknown["error"]["code"], rpc::METHOD_NOT_FOUND);
        for params in [
            r#"{"limit":0}"#,
            r#"{"limit":257}"#,
            r#"{"limit":"5"}"#,
            "[5]",
        ] {
            let request =
                format!(r#"{{"jsonrpc":"2.0","id":1,"method":"hello.keys","params":{params}}}"#);
            let reply = call_frame(&state, &request);
            assert_eq!(reply["error"]["code"], rpc::INVALID_PARAMS, "{params}");
        }
        let notification = r#"{"jsonrpc":"2.0","method":"engine.hello"}"#;
        assert!(handle_frame(&state, notification.as_bytes()).is_none());
        let garbage = call_frame(&state, "not json");
        assert_eq!(garbage["error"]["code"], rpc::PARSE_ERROR);
    }

    #[test]
    fn key_names() {
        let cases = [
            (0x41, "A"),
            (0x5A, "Z"),
            (0x30, "0"),
            (0x39, "9"),
            (0x70, "F1"),
            (0x77, "F8"),
            (0x87, "F24"),
            (0x60, "Numpad0"),
            (0x12, "Alt"),
            (0x1B, "Esc"),
            (0xFF, "VK_0xFF"),
        ];
        for (vk, name) in cases {
            assert_eq!(key_name(vk), name);
        }
    }

    #[test]
    fn pipe_names() {
        assert_eq!(pipe_path(PIPE_NAME), r"\\.\pipe\kairo-hello");
        assert_eq!(
            parse_pipe_arg("kairo-hello"),
            Some("kairo-hello".to_owned())
        );
        assert_eq!(
            parse_pipe_arg(r"\\.\pipe\kairo-x.1_2"),
            Some("kairo-x.1_2".to_owned())
        );
        assert_eq!(parse_pipe_arg(r"\\server\pipe\kairo-hello"), None);
        assert_eq!(parse_pipe_arg(""), None);
        assert_eq!(parse_pipe_arg("a b"), None);
        assert_eq!(parse_pipe_arg(&"p".repeat(201)), None);
    }

    #[test]
    fn sddl_allows_system_and_the_user_only() {
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;SY)(A;;GA;;;S-1-5-21-1-2-3-1001)"
        );
    }

    #[test]
    fn log_lines_are_json_with_the_event_first() {
        let key = HelloLogLine::Key {
            seq: 1,
            qpc: 42,
            qpc_frequency: 10_000_000,
            ts_utc: "2026-10-21T20:15:03.123Z",
            vk: 0x77,
            key: "F8",
        };
        assert_eq!(
            key.to_json().unwrap(),
            r#"{"event":"key","seq":1,"qpc":42,"qpc_frequency":10000000,"ts_utc":"2026-10-21T20:15:03.123Z","vk":119,"key":"F8"}"#
        );
        let stop = HelloLogLine::Stop {
            ts_utc: "t",
            keys: 3,
        };
        assert_eq!(
            stop.to_json().unwrap(),
            r#"{"event":"stop","ts_utc":"t","keys":3}"#
        );
    }

    #[test]
    fn window_text_mentions_the_count_and_the_pipe() {
        let text = window_text(3, Some("F8"), r"C:\kairo\logs\hello.log", "listening");
        assert!(text.starts_with("Hello from Kairo\n"));
        assert!(text.contains(r"C:\kairo\logs\hello.log"));
        assert!(text.contains("Key presses so far: 3 (last: F8)"));
        assert!(text.contains(r"Pipe \\.\pipe\kairo-hello: listening"));
        assert!(window_text(0, None, "x", "y").contains("(last: none yet)"));
    }
}
