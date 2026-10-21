// SPDX-License-Identifier: GPL-3.0-or-later
//! `C:\kairo\logs\devrun.log`: one JSON line per event. Logging never fails the caller.

use std::fs::{self, OpenOptions};
use std::io::Write;

use kairo_core::layout::Layout;
use kairo_core::time;
use serde_json::{Map, Value};

/// Past this size the log moves to `devrun.log.1`, replacing the previous one.
const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;

/// Appends `{"ts_utc", "pid", "cmd", "msg"}` to the log.
pub fn event(layout: &Layout, command: &str, message: &str) {
    let path = layout.devrun_log_file();
    let _ = fs::create_dir_all(layout.logs_dir());
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > ROTATE_AT_BYTES) {
        let _ = fs::rename(&path, format!("{path}.1"));
    }
    let mut line = Map::new();
    line.insert("ts_utc".to_owned(), Value::from(time::now_utc()));
    line.insert("pid".to_owned(), Value::from(std::process::id()));
    line.insert("cmd".to_owned(), Value::from(command));
    line.insert("msg".to_owned(), Value::from(message));
    let mut text = Value::Object(line).to_string();
    text.push('\n');
    // One write per line, so lines from the watch agent and a job runner never interleave.
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(text.as_bytes());
    }
}
