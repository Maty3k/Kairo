// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-devrun run`: one job in the owner's session (docs/PLAN.md §3.1).
//!
//! The job's processes live in a kill-on-close job object. They are stopped when the time
//! limit passes or, if the job asks, at the owner's first keyboard or mouse input; otherwise
//! the job ends when its last process exits. The result file is written atomically, so whoever
//! polls for `<id>.result.json` never reads half a file.

use std::fs::{self, File};
use std::io;
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use kairo_core::job::{self, ABORT_EXIT_CODE, EndReason, JobFile, JobResult, WindowMode};
use kairo_core::layout::Layout;
use kairo_core::time;
use windows::Win32::System::Threading::{CREATE_NEW_CONSOLE, CREATE_NO_WINDOW};

use crate::app::{EXIT_BUSY, EXIT_FAILED};
use crate::spawn::{self, StartError, Started};
use crate::{devlog, platform};

/// One job at a time on the PC, across sessions.
const RUN_MUTEX: &str = r"Global\Kairo.Devrun.Run";
/// How often the job is checked: well inside the 2-second budget for stopping after input.
const POLL: Duration = Duration::from_millis(100);

/// Runs the job in `job_file` and writes its result. Exit code 0 when a result was written.
pub fn main(layout: &Layout, job_file: &str) -> ExitCode {
    let started_utc = time::now_utc();
    let bytes = match fs::read(job_file) {
        Ok(bytes) => bytes,
        Err(error) => {
            devlog::event(layout, "run", &format!("cannot read {job_file}: {error}"));
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let job = match job::parse_job(&bytes) {
        Ok(job) => job,
        Err(error) => {
            let message = format!("invalid job file {job_file}: {error}");
            return match job::peek_job_id(&bytes) {
                Some(id) => refuse(layout, &id, EndReason::InvalidJob, &message, started_utc),
                None => {
                    devlog::event(layout, "run", &message);
                    ExitCode::from(EXIT_FAILED)
                }
            };
        }
    };
    match platform::session_id() {
        Ok(0) => {
            let message = "not in the owner's interactive session (session 0, e.g. SSH); \
                           start jobs through the \\Kairo\\Run task";
            return refuse(
                layout,
                &job.id,
                EndReason::NotInteractive,
                message,
                started_utc,
            );
        }
        Ok(_) => {}
        Err(error) => {
            let message = format!("cannot read the session id: {error}");
            return refuse(
                layout,
                &job.id,
                EndReason::InternalError,
                &message,
                started_utc,
            );
        }
    }
    // Held until this function returns. A second job is refused without touching any file:
    // it may have the same id as the running one.
    let _lock = match platform::NamedMutex::create(RUN_MUTEX) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            let message = format!("job {} not run: another job is still running", job.id);
            devlog::event(layout, "run", &message);
            return ExitCode::from(EXIT_BUSY);
        }
        Err(error) => {
            let message = format!("cannot take the run lock: {error}");
            return refuse(
                layout,
                &job.id,
                EndReason::InternalError,
                &message,
                started_utc,
            );
        }
    };
    let message = format!(
        "job {} starts: {:?} (max {} s, abort on input: {})",
        job.id, job.cmd, job.max_seconds, job.abort_on_input
    );
    devlog::event(layout, "run", &message);
    let result = execute(layout, &job);
    let message = format!(
        "job {} ended: {:?}, exit code {:?}",
        result.id, result.reason, result.exit_code
    );
    devlog::event(layout, "run", &message);
    finish(layout, &result)
}

/// Runs a valid job to its end. Never fails: problems become the result's `reason`, with the
/// details in `<id>.err.log`.
pub fn execute(layout: &Layout, job: &JobFile) -> JobResult {
    let started_utc = time::now_utc();
    let _ = fs::create_dir_all(layout.logs_dir());
    let _ = fs::remove_file(layout.job_result_file(&job.id));
    let (reason, exit_code) = match supervise(layout, job) {
        Ok(outcome) => outcome,
        Err(failure) => {
            write_note(layout, &job.id, &failure.message);
            (failure.reason, None)
        }
    };
    JobResult::new(&job.id, exit_code, reason, started_utc, time::now_utc())
}

struct Failure {
    reason: EndReason,
    message: String,
}

fn internal(message: String) -> Failure {
    Failure {
        reason: EndReason::InternalError,
        message,
    }
}

fn supervise(layout: &Layout, job: &JobFile) -> Result<(EndReason, Option<u32>), Failure> {
    let stdout_file = layout.job_stdout_file(&job.id);
    let stderr_file = layout.job_stderr_file(&job.id);
    let stdout = File::create(&stdout_file)
        .map_err(|error| internal(format!("cannot create {stdout_file}: {error}")))?;
    let stderr = File::create(&stderr_file)
        .map_err(|error| internal(format!("cannot create {stderr_file}: {error}")))?;
    let Some((program, args)) = job.cmd.split_first() else {
        return Err(Failure {
            reason: EndReason::InvalidJob,
            message: "`cmd` is empty".to_owned(),
        });
    };
    let cwd = job.cwd.clone().unwrap_or_else(|| layout.root().to_owned());
    let console = match job.window {
        WindowMode::Hidden => CREATE_NO_WINDOW,
        WindowMode::Visible => CREATE_NEW_CONSOLE,
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let Started {
        mut child,
        job: processes,
    } = spawn::start_in_job(&mut command, console.0).map_err(|error| match error {
        StartError::Spawn(error) => Failure {
            reason: EndReason::SpawnFailed,
            message: format!("cannot start {program} in {cwd}: {error}"),
        },
        StartError::Job(message) => internal(message),
    })?;
    let deadline = Instant::now() + Duration::from_secs(job.max_seconds);
    let input_at_start = platform::last_input_tick();
    let mut first_exit: Option<u32> = None;
    loop {
        if first_exit.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => first_exit = Some(exit_code(status)),
                Ok(None) => {}
                Err(error) => {
                    let _ = processes.terminate(ABORT_EXIT_CODE);
                    return Err(internal(format!("cannot check the program: {error}")));
                }
            }
        }
        if first_exit.is_some() && processes.active_processes().unwrap_or(0) == 0 {
            return Ok((EndReason::Exited, first_exit));
        }
        let stop = if Instant::now() >= deadline {
            Some(EndReason::Timeout)
        } else if job.abort_on_input
            && job::input_since(input_at_start, platform::last_input_tick())
        {
            Some(EndReason::Input)
        } else {
            None
        };
        if let Some(reason) = stop {
            let _ = processes.terminate(ABORT_EXIT_CODE);
            let code = first_exit.or_else(|| child.wait().ok().map(exit_code));
            return Ok((reason, code));
        }
        thread::sleep(POLL);
    }
}

/// The exit code as Windows reports it: an unsigned 32-bit value.
fn exit_code(status: ExitStatus) -> u32 {
    status
        .code()
        .map_or(ABORT_EXIT_CODE, |code| code.cast_unsigned())
}

/// Reports a job that never started, in the files the caller is waiting for.
fn refuse(
    layout: &Layout,
    id: &str,
    reason: EndReason,
    message: &str,
    started_utc: String,
) -> ExitCode {
    devlog::event(layout, "run", &format!("job {id} not run: {message}"));
    write_note(layout, id, message);
    let result = JobResult::new(id, None, reason, started_utc, time::now_utc());
    finish(layout, &result)
}

/// Explains in `<id>.err.log`, where the program's errors would go, why it could not run.
fn write_note(layout: &Layout, id: &str, message: &str) {
    let _ = fs::create_dir_all(layout.logs_dir());
    let _ = fs::write(
        layout.job_stderr_file(id),
        format!("kairo-devrun: {message}\n"),
    );
}

/// Writes the result; exit code 0 if it was written.
fn finish(layout: &Layout, result: &JobResult) -> ExitCode {
    match write_result(layout, result) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = format!("cannot write the result of job {}: {error}", result.id);
            devlog::event(layout, "run", &message);
            ExitCode::from(EXIT_FAILED)
        }
    }
}

/// Writes `<id>.result.json` through a temporary file and a rename, so it appears complete.
fn write_result(layout: &Layout, result: &JobResult) -> io::Result<()> {
    let path = layout.job_result_file(&result.id);
    let temporary = format!("{path}.tmp");
    let mut json = serde_json::to_vec_pretty(result).map_err(io::Error::other)?;
    json.push(b'\n');
    fs::create_dir_all(layout.logs_dir())?;
    fs::write(&temporary, &json)?;
    fs::rename(&temporary, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(tag: &str) -> Layout {
        let root = std::env::temp_dir().join(format!("kairo-devrun-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("logs")).unwrap();
        Layout::new(root.to_string_lossy().into_owned())
    }

    fn job(id: &str, cmd: &[&str], max_seconds: u64) -> JobFile {
        JobFile {
            id: id.to_owned(),
            cmd: cmd.iter().map(|part| (*part).to_owned()).collect(),
            cwd: None,
            max_seconds,
            abort_on_input: false,
            window: WindowMode::Hidden,
        }
    }

    #[test]
    fn runs_a_job_and_captures_its_output() {
        let layout = layout("echo");
        let result = execute(&layout, &job("echo", &["cmd", "/c", "echo kairo-ok"], 60));
        assert_eq!(result.reason, EndReason::Exited);
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.aborted);
        let out = fs::read_to_string(layout.job_stdout_file("echo")).unwrap();
        assert!(out.contains("kairo-ok"), "{out}");
    }

    #[test]
    fn reports_the_exit_code() {
        let layout = layout("exit7");
        let result = execute(&layout, &job("exit7", &["cmd", "/c", "exit 7"], 60));
        assert_eq!(result.reason, EndReason::Exited);
        assert_eq!(result.exit_code, Some(7));
    }

    #[test]
    fn stops_the_whole_tree_at_the_time_limit() {
        let layout = layout("timeout");
        let begin = Instant::now();
        // cmd starts ping as a grandchild; the job object must stop both.
        let cmd = ["cmd", "/c", "ping", "-n", "30", "127.0.0.1"];
        let result = execute(&layout, &job("timeout", &cmd, 1));
        assert_eq!(result.reason, EndReason::Timeout);
        assert!(result.aborted);
        assert_eq!(result.exit_code, Some(ABORT_EXIT_CODE));
        assert!(
            begin.elapsed() < Duration::from_secs(15),
            "{:?}",
            begin.elapsed()
        );
    }

    #[test]
    fn reports_a_program_that_cannot_start() {
        let layout = layout("missing");
        let program = r"C:\kairo-test-does-not-exist\nothing.exe";
        let result = execute(&layout, &job("missing", &[program], 5));
        assert_eq!(result.reason, EndReason::SpawnFailed);
        assert_eq!(result.exit_code, None);
        assert!(!result.aborted);
        let note = fs::read_to_string(layout.job_stderr_file("missing")).unwrap();
        assert!(note.starts_with("kairo-devrun: cannot start"), "{note}");
    }

    #[test]
    fn writes_the_result_file_atomically() {
        let layout = layout("result");
        let result = JobResult::new(
            "result",
            Some(0),
            EndReason::Exited,
            time::now_utc(),
            time::now_utc(),
        );
        write_result(&layout, &result).unwrap();
        let text = fs::read_to_string(layout.job_result_file("result")).unwrap();
        let parsed: JobResult = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, result);
        assert!(
            !std::path::Path::new(&format!("{}.tmp", layout.job_result_file("result"))).exists()
        );
    }
}
