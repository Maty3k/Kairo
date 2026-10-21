// SPDX-License-Identifier: GPL-3.0-or-later
//! Wires the window, the key log and the pipe server together.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use kairo_core::hello::{self, HelloInfo, HelloLogLine, HelloState};
use kairo_core::layout::Layout;
use kairo_core::time;

use crate::platform::{self, pipe, window};

/// Runs the probe until its window closes.
pub fn main() -> ExitCode {
    let layout = Layout::default();
    let log_file = layout.hello_log_file();
    let _ = fs::create_dir_all(layout.logs_dir());
    let info = HelloInfo {
        name: "kairo-hello".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        session_id: platform::session_id().unwrap_or(hello::UNKNOWN_SESSION),
        elevated: platform::is_elevated().unwrap_or(false),
    };
    let (session_id, elevated) = (info.session_id, info.elevated);
    let state = Arc::new(Mutex::new(HelloState::new(info, platform::qpc_frequency())));
    let pipe_status = start_pipe_server(&state);
    append(
        &log_file,
        &HelloLogLine::Start {
            ts_utc: &time::now_utc(),
            pid: std::process::id(),
            session_id,
            elevated,
            pipe: &hello::pipe_path(hello::PIPE_NAME),
            pipe_status: &pipe_status,
        },
    );
    let on_key = {
        let state = Arc::clone(&state);
        let log_file = log_file.clone();
        move |vk: u16| record_key(&state, &log_file, vk)
    };
    let text = {
        let state = Arc::clone(&state);
        let log_file = log_file.clone();
        move || {
            let state = lock(&state);
            let last = state.last_key().map(|press| press.key.as_str());
            hello::window_text(state.total_keys(), last, &log_file, &pipe_status)
        }
    };
    let callbacks = window::Callbacks {
        on_key: Box::new(on_key),
        text: Box::new(text),
    };
    let outcome = window::run(hello::WINDOW_TITLE, callbacks);
    let keys = lock(&state).total_keys();
    append(
        &log_file,
        &HelloLogLine::Stop {
            ts_utc: &time::now_utc(),
            keys,
        },
    );
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Starts the pipe server on its own thread; returns what the window says about the pipe.
fn start_pipe_server(state: &Arc<Mutex<HelloState>>) -> String {
    match pipe::Listener::bind(hello::PIPE_NAME) {
        Ok(listener) => {
            let state = Arc::clone(state);
            let handler: Arc<pipe::Handler> = Arc::new(move |frame: &[u8]| {
                hello::handle_frame(&lock(&state), frame).map(|response| response.to_bytes())
            });
            let _ = thread::spawn(move || listener.serve(handler));
            "listening".to_owned()
        }
        Err(error) => format!("not available ({error})"),
    }
}

fn record_key(state: &Mutex<HelloState>, log_file: &str, vk: u16) {
    let qpc = platform::qpc_now();
    let ts_utc = time::now_utc();
    let (press, qpc_frequency) = {
        let mut state = lock(state);
        let qpc_frequency = state.qpc_frequency();
        (state.record_key(qpc, ts_utc, vk), qpc_frequency)
    };
    append(
        log_file,
        &HelloLogLine::Key {
            seq: press.seq,
            qpc: press.qpc,
            qpc_frequency,
            ts_utc: &press.ts_utc,
            vk: press.vk,
            key: &press.key,
        },
    );
}

/// The state, even if a panic poisoned the lock (the data stays consistent: each update is
/// a single push).
fn lock(state: &Mutex<HelloState>) -> MutexGuard<'_, HelloState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Appends one JSON line to `hello.log`. Logging never stops the probe.
fn append(log_file: &str, line: &HelloLogLine<'_>) {
    let Some(mut text) = line.to_json() else {
        return;
    };
    text.push('\n');
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_file) {
        let _ = file.write_all(text.as_bytes());
    }
}
