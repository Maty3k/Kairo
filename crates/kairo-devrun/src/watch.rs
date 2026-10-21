// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-devrun watch`: the watch agent (docs/PLAN.md §3.1).
//!
//! It must run in the owner's session, because `GetLastInputInfo` only sees input in its own
//! session, and it rewrites `C:\kairo\state\watch.json` every second. One agent runs per PC
//! (a global mutex). When `bin\current` moves to another build the agent exits, and `ensure`
//! starts the new build's agent within a minute.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use kairo_core::layout::Layout;
use kairo_core::time;
use kairo_core::watch::{self, WatchStatus};

use crate::app::{self, EXIT_FAILED};
use crate::{devlog, platform};

/// Held while a watch agent runs; `ensure` looks for it.
pub const WATCH_MUTEX: &str = r"Global\Kairo.Devrun.Watch";
/// Check for a newer build every this many samples.
const BUILD_CHECK_EVERY: u64 = 10;
/// Failures to write `watch.json` are logged at most this often.
const ERROR_LOG_INTERVAL: Duration = Duration::from_secs(60);

/// Runs the watch agent until a newer build is current (or the session ends).
pub fn main(layout: &Layout) -> ExitCode {
    if let Err(code) = app::require_interactive(layout, "watch") {
        return code;
    }
    let _instance = match platform::NamedMutex::create(WATCH_MUTEX) {
        Ok(Some(instance)) => instance,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            let message = format!("cannot create the watch mutex: {error}");
            devlog::event(layout, "watch", &message);
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let _ = fs::create_dir_all(layout.state_dir());
    let build = BuildCheck::new(layout);
    devlog::event(layout, "watch", "started");
    let interval = Duration::from_millis(watch::WATCH_INTERVAL_MS);
    let mut names = ExeNames::default();
    let mut last_error_logged: Option<Instant> = None;
    let mut samples: u64 = 0;
    loop {
        let began = Instant::now();
        samples += 1;
        if let Err(error) = write_status(layout, &sample(&mut names))
            && last_error_logged.is_none_or(|at| at.elapsed() >= ERROR_LOG_INTERVAL)
        {
            devlog::event(
                layout,
                "watch",
                &format!("cannot write watch.json: {error}"),
            );
            last_error_logged = Some(Instant::now());
        }
        if samples.is_multiple_of(BUILD_CHECK_EVERY) && build.replaced() {
            let message = "another build is current now; exiting so ensure starts its agent";
            devlog::event(layout, "watch", message);
            return ExitCode::SUCCESS;
        }
        thread::sleep(interval.saturating_sub(began.elapsed()));
    }
}

fn sample(names: &mut ExeNames) -> WatchStatus {
    // Input time first, then the clock, so the difference can never go below zero.
    let last_input = platform::last_input_tick();
    let now = platform::tick_count();
    let last_input_ms_ago = last_input.map(|tick| watch::ticks_between(tick, now));
    // Only a fullscreen program is named (a game, as far as M0 can tell). Which other program is
    // in front, such as a browser or a chat app, is not reported (house rules: no non-game
    // window titles), and its process is never opened.
    let (fullscreen, exe) = match platform::foreground_window() {
        Some(window) if window.fullscreen => (true, names.lookup(window.window_id, window.pid)),
        _ => (false, None),
    };
    WatchStatus::new(time::now_utc(), last_input_ms_ago, fullscreen, exe)
}

/// Remembers the foreground program's name, so a process is opened once per foreground change
/// rather than every second.
#[derive(Default)]
struct ExeNames {
    key: Option<(usize, u32)>,
    name: Option<String>,
}

impl ExeNames {
    fn lookup(&mut self, window_id: usize, pid: u32) -> Option<String> {
        if self.key != Some((window_id, pid)) {
            self.key = Some((window_id, pid));
            self.name = platform::process_exe_name(pid);
        }
        self.name.clone()
    }
}

/// Writes `watch.json` through a temporary file and a rename, so readers never see half a
/// file. A reader holding the file open can make the rename fail; the next second retries.
fn write_status(layout: &Layout, status: &WatchStatus) -> io::Result<()> {
    let path = layout.watch_file();
    let temporary = format!("{path}.tmp");
    let mut json = serde_json::to_vec(status).map_err(io::Error::other)?;
    json.push(b'\n');
    fs::write(&temporary, &json)?;
    fs::rename(&temporary, &path)
}

/// Notices when `bin\current` resolves to a different build than the one this agent runs from.
struct BuildCheck {
    running: Option<PathBuf>,
    bin_dir: Option<PathBuf>,
    current: PathBuf,
}

impl BuildCheck {
    fn new(layout: &Layout) -> Self {
        Self {
            running: std::env::current_exe().and_then(fs::canonicalize).ok(),
            bin_dir: fs::canonicalize(layout.bin_dir()).ok(),
            current: Path::new(&layout.current_bin_dir()).join("kairo-devrun.exe"),
        }
    }

    /// True if this agent runs from a build under `bin` and `bin\current` now points elsewhere.
    fn replaced(&self) -> bool {
        let (Some(running), Some(bin_dir)) = (&self.running, &self.bin_dir) else {
            return false;
        };
        running.starts_with(bin_dir)
            && fs::canonicalize(&self.current).is_ok_and(|current| current != *running)
    }
}
