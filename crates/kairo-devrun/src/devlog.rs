// SPDX-License-Identifier: GPL-3.0-or-later
//! `C:\kairo\logs\devrun.log`: one JSON line per event. Logging never fails the caller.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use kairo_core::layout::Layout;
use kairo_core::time;
use serde_json::{Map, Value};

/// Past this size the log moves to `devrun.log.1`, replacing the previous one.
const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;

/// Whether this process has looked for lines in the old format.
static CHECKED: AtomicBool = AtomicBool::new(false);

/// Appends `{"ts_utc", "pid", "cmd", "msg"}` to the log.
pub fn event(layout: &Layout, command: &str, message: &str) {
    let path = layout.devrun_log_file();
    let _ = fs::create_dir_all(layout.logs_dir());
    let rotate = fs::metadata(&path).is_ok_and(|meta| meta.len() > ROTATE_AT_BYTES);
    if rotate || (!CHECKED.swap(true, Ordering::Relaxed) && old_format(&path)) {
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

/// Whether the log holds a job's start line from before this version, which named the job's
/// whole command line: it can name the owner's account or a monitor, and `pc logs devrun` or a
/// failed `pc run` would print it on the Mac. Such a log moves to `devrun.log.1` once, which no
/// tool shows (their ids can't hold a dot).
fn old_format(path: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text.contains(" starts: ["))
}
