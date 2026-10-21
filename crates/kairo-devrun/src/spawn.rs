// SPDX-License-Identifier: GPL-3.0-or-later
//! Starting programs: jobs in a job object (`run`, `engine`) and the detached watch agent
//! (`ensure`).

use std::fmt;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};

use kairo_core::job::ABORT_EXIT_CODE;
use kairo_core::layout::Layout;
use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED,
};

use crate::app::{self, EXIT_FAILED};
use crate::platform::{self, Job};
use crate::{devlog, watch};

/// A program running inside its own kill-on-close job object.
pub struct Started {
    /// The program itself.
    pub child: Child,
    /// Its job: the program and everything it starts.
    pub job: Job,
}

/// Why [`start_in_job`] failed.
pub enum StartError {
    /// Windows could not start the program (missing file, bad folder, ...).
    Spawn(io::Error),
    /// The program started but could not be put in a job or resumed; it was stopped.
    Job(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "cannot start the program: {error}"),
            Self::Job(message) => f.write_str(message),
        }
    }
}

/// Starts `command` suspended, puts it in a new kill-on-close job and lets it run, so even its
/// first instructions run inside the job and every process it starts can be stopped.
// TODO(verify) on the PC (M0): this nests correctly when `kairo-devrun` itself runs inside the
// scheduled task's job (nested jobs exist since Windows 8; CI covers only the plain case).
pub fn start_in_job(command: &mut Command, creation_flags: u32) -> Result<Started, StartError> {
    command.creation_flags(creation_flags | CREATE_SUSPENDED.0);
    let mut child = command.spawn().map_err(StartError::Spawn)?;
    let job = match Job::kill_on_close().and_then(|job| job.assign(&child).map(|()| job)) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(StartError::Job(format!(
                "cannot put the program in a job object: {error}"
            )));
        }
    };
    if let Err(error) = platform::resume_process(child.id()) {
        let _ = job.terminate(ABORT_EXIT_CODE);
        let _ = child.wait();
        return Err(StartError::Job(format!(
            "cannot resume the program: {error}"
        )));
    }
    Ok(Started { child, job })
}

/// `ensure`: starts the watch agent unless it already runs, then exits.
pub fn ensure(layout: &Layout) -> ExitCode {
    if let Err(code) = app::require_interactive(layout, "ensure") {
        return code;
    }
    if platform::mutex_exists(watch::WATCH_MUTEX) {
        return ExitCode::SUCCESS;
    }
    let started = std::env::current_exe().and_then(|exe| spawn_detached(layout, &exe, &["watch"]));
    match started {
        Ok(child) => {
            let message = format!("started the watch agent (pid {})", child.id());
            devlog::event(layout, "ensure", &message);
            ExitCode::SUCCESS
        }
        Err(error) => {
            let message = format!("cannot start the watch agent: {error}");
            devlog::event(layout, "ensure", &message);
            ExitCode::from(EXIT_FAILED)
        }
    }
}

/// `engine`: runs `kairo-hello.exe` from this build until it exits. It runs in a kill-on-close
/// job, so stopping `\Kairo\Engine` (which ends only this process) also closes the window.
// TODO(verify) on the PC (M0): Stop-ScheduledTask \Kairo\Engine closes the hello window.
pub fn engine(layout: &Layout) -> ExitCode {
    if let Err(code) = app::require_interactive(layout, "engine") {
        return code;
    }
    let hello = match std::env::current_exe() {
        Ok(exe) => exe.with_file_name("kairo-hello.exe"),
        Err(error) => {
            devlog::event(
                layout,
                "engine",
                &format!("cannot find this program: {error}"),
            );
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let mut command = Command::new(&hello);
    command
        .current_dir(working_dir(layout))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut started = match start_in_job(&mut command, 0) {
        Ok(started) => started,
        Err(error) => {
            let message = format!("cannot run {}: {error}", hello.display());
            devlog::event(layout, "engine", &message);
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let message = format!("started kairo-hello (pid {})", started.child.id());
    devlog::event(layout, "engine", &message);
    let status = started.child.wait();
    devlog::event(layout, "engine", &format!("kairo-hello ended: {status:?}"));
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Starts a program that outlives this process: without standard handles or a console, and
/// outside the scheduled task's job if Windows allows it.
// TODO(verify) on the PC (M0): the watch agent outlives the \Kairo\Ensure task instance in both
// branches below (cookbook §12, item 16); devrun.log should show one "started" per logon.
fn spawn_detached(layout: &Layout, exe: &Path, args: &[&str]) -> io::Result<Child> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .current_dir(working_dir(layout))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let detached = (CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP).0;
    command.creation_flags(detached | CREATE_BREAKAWAY_FROM_JOB.0);
    match command.spawn() {
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => {
            // The task's job does not allow breakaway: start inside it instead.
            command.creation_flags(detached);
            command.spawn()
        }
        other => other,
    }
}

/// The layout root if it exists, else this program's folder.
fn working_dir(layout: &Layout) -> PathBuf {
    let root = PathBuf::from(layout.root());
    if root.is_dir() {
        return root;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or(root)
}
