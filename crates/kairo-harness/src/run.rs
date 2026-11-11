// SPDX-License-Identifier: GPL-3.0-or-later
//! `run`, gate 2's capture (docs/research/16-verify-testbed/README.md §8.3): setup, the testbed,
//! its warm-up, FFmpeg's capture, the testbed's stop, kairo-verify's judgement and the report,
//! each phase within its limit. Everything happens in a new folder in the logs folder, and every
//! way out of a phase that made the folder leaves `harness.json` in it. Children run with no
//! console window, their output in files in the folder; each is killed and reaped before the
//! harness exits, since `kairo-devrun` waits for the whole job.

use std::fs::{self, File};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use kairo_core::layout::{DEFAULT_ROOT, Layout};
use kairo_core::testbed_log::TestbedLog;
use serde_json::Value;

use crate::capture::{self, CLIP, Settings, Source};
use crate::cli::{Capture, Request};
use crate::expect::Expect;
use crate::ffmpeg_log::{self, LogCheck};
use crate::outcome::{self, Outcome};
use crate::phases::{Phase, exit, testbed_seconds};
use crate::platform::{self, CREATE_NO_WINDOW, KeepDisplayOn};
use crate::readback;
use crate::ready::{Expected, Ready};
use crate::report::{
    AdapterFacts, Asked, ChildFacts, DisplayFacts, PhaseOutcome, Report, VerifyFacts, WarmupFacts,
    last_line,
};
use crate::warmup::{self, Judgement};
use crate::{prune, report};

/// The least free space in the logs folder: a 30 s capture is about 100 MB, debug logs are tens
/// of MB, and the PC must not fill up.
const FREE_MIN: u64 = 2 << 30;
/// The most of FFmpeg's log the harness reads.
const LOG_MAX: u64 = 256 << 20;
/// The most of kairo-verify's report the harness reads.
const REPORT_MAX: u64 = 64 << 20;
/// How often a waiting phase looks again.
const POLL: Duration = Duration::from_millis(50);
/// How long a killed child may take to go.
const REAP: Duration = Duration::from_secs(2);

/// A child that is killed and reaped when dropped.
struct Guarded {
    child: Child,
    killed: bool,
}

impl Guarded {
    fn new(child: Child) -> Guarded {
        Guarded {
            child,
            killed: false,
        }
    }

    /// Its exit status, if it has ended.
    fn ended(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Waits until it ends or `deadline` passes.
    fn wait_until(&mut self, deadline: Instant) -> Option<ExitStatus> {
        loop {
            if let Some(status) = self.ended() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Kills it and waits up to [`REAP`] for it to go.
    fn kill(&mut self) -> Option<ExitStatus> {
        if let Some(status) = self.ended() {
            return Some(status);
        }
        self.killed = true;
        let _ = self.child.kill();
        self.wait_until(Instant::now() + REAP)
    }
}

impl Drop for Guarded {
    fn drop(&mut self) {
        self.kill();
    }
}

/// An exit status as a number: Windows' 32-bit codes, as `i32`.
fn code(status: Option<ExitStatus>) -> Option<i32> {
    status.and_then(|s| s.code())
}

/// Where the programs and folders are.
struct Paths {
    logs: PathBuf,
    folder: PathBuf,
    ffmpeg: PathBuf,
    pin: PathBuf,
    testbed: PathBuf,
    verify: PathBuf,
    tone: PathBuf,
}

impl Paths {
    fn new(run_id: &str) -> Result<Paths, Outcome> {
        let layout = Layout::new(DEFAULT_ROOT);
        let exe = std::env::current_exe()
            .map_err(|_| Outcome::new(exit::INTERNAL, "the harness can't find itself"))?;
        let bin = exe.parent().ok_or(Outcome::new(
            exit::INTERNAL,
            "the harness can't find its folder",
        ))?;
        let logs = PathBuf::from(layout.logs_dir());
        let tools = PathBuf::from(layout.tools_dir()).join("ffmpeg");
        Ok(Paths {
            folder: logs.join(run_id),
            logs,
            ffmpeg: tools.join("bin").join("ffmpeg.exe"),
            pin: tools.join("KAIRO-PIN.json"),
            testbed: bin.join("testbed.exe"),
            verify: bin.join("kairo-verify.exe"),
            tone: bin.join("kairo-tone.exe"),
        })
    }
}

/// What the run found and holds.
struct Ctx<'a> {
    request: &'a Request,
    paths: Paths,
    report: Report,
    /// The testbed's rate.
    fps: u32,
    k: u32,
    refresh_mhz: u64,
    run_tag: u8,
    luid: String,
    adapter_index: u32,
    device: String,
    source: Option<Source>,
    output_index: Option<u32>,
    monitor_rect: (i32, i32, i32, i32),
    testbed: Option<Guarded>,
    testbed_stdin: Option<ChildStdin>,
    tone: Option<Guarded>,
    tone_stdin: Option<ChildStdin>,
    log_check: LogCheck,
    /// Whether FFmpeg's log was read: the read-back table needs it.
    log_read: bool,
    verify_report: Option<Value>,
}

/// Runs `request`; returns the exit code.
pub fn run(request: &Request) -> u8 {
    use std::io::Write;
    if request.oled {
        // The OLED shows moving content only, never a capture's pattern (M1:43).
        let _ = writeln!(
            std::io::stderr(),
            "kairo-harness: the OLED is refused: it shows moving content only"
        );
        return exit::REFUSED;
    }
    let paths = match Paths::new(&request.run_id) {
        Ok(paths) => paths,
        Err(o) => {
            let _ = writeln!(std::io::stderr(), "kairo-harness: {}", o.reason);
            return o.code;
        }
    };
    // The folder first, so every later way out leaves a report in it.
    if let Err(e) = fs::create_dir(&paths.folder) {
        let reason = if e.kind() == std::io::ErrorKind::AlreadyExists {
            "the run folder exists already: another run has this id"
        } else {
            "the run folder couldn't be made"
        };
        let _ = writeln!(std::io::stderr(), "kairo-harness: {reason}");
        return exit::USAGE;
    }
    let _display = KeepDisplayOn::new();
    let mut ctx = Ctx {
        request,
        paths,
        report: Report::new(&request.run_id, kairo_core::time::now_utc(), asked(request)),
        fps: 0,
        k: 1,
        refresh_mhz: 0,
        run_tag: 0,
        luid: String::new(),
        adapter_index: 0,
        device: String::new(),
        source: None,
        output_index: None,
        monitor_rect: (0, 0, 0, 0),
        testbed: None,
        testbed_stdin: None,
        tone: None,
        tone_stdin: None,
        log_check: LogCheck::default(),
        log_read: false,
        verify_report: None,
    };
    let outcome = ctx.phases();
    ctx.finish(outcome)
}

fn asked(r: &Request) -> Asked {
    let (w, h) = r.output.size();
    Asked {
        profile: r.profile.name(),
        seconds: r.seconds,
        output: format!("{w}x{h}@{}", r.output.fps()),
        testbed_fps: r.testbed_fps.unwrap_or(0),
        mode: "borderless",
        capture: match r.capture {
            Capture::Gfx => "gfxcapture",
            Capture::Dda => "ddagrab",
        },
        max_framerate: r.max_framerate,
        loglevel: r.loglevel.name(),
        finishing: r.finishing,
        voice: r.voice,
    }
}

impl Ctx<'_> {
    /// Runs the phases in order; the first that stops the run says why.
    fn phases(&mut self) -> Outcome {
        let done = self
            .step(Phase::Setup, Self::setup)
            .and_then(|()| self.step(Phase::Testbed, Self::start_testbed))
            .and_then(|()| self.step(Phase::Warmup, Self::warm_up))
            .and_then(|()| self.step(Phase::Capture, Self::capture))
            .and_then(|()| self.step(Phase::Stop, Self::stop))
            .and_then(|()| self.step(Phase::Verify, Self::verify));
        match done {
            Ok(()) => outcome::verify(
                self.report.verify.as_ref().and_then(|v| v.exit_code),
                self.verify_report.as_ref(),
            ),
            Err(o) => o,
        }
    }

    /// Runs one phase and records how long it took and how it went.
    fn step(
        &mut self,
        phase: Phase,
        run: impl FnOnce(&mut Self) -> Result<(), Outcome>,
    ) -> Result<(), Outcome> {
        let start = Instant::now();
        let result = run(self);
        let seconds = start.elapsed().as_secs_f64();
        let how = match &result {
            Ok(()) => PhaseOutcome::Ok,
            Err(o) if o.code == exit::REFUSED => PhaseOutcome::Refused,
            Err(o) if o.reason.contains("limit") => PhaseOutcome::Timeout,
            Err(_) => PhaseOutcome::Failed,
        };
        self.report.phase(phase, seconds, how);
        // A capture that went wrong still stops the testbed cleanly.
        if result.is_err() && phase == Phase::Capture {
            let _ = self.stop();
        }
        result
    }

    /// Phase 0.
    fn setup(&mut self) -> Result<(), Outcome> {
        platform::no_error_dialogs();
        kairo_display::make_dpi_aware();
        self.report.prune = prune::prune(&self.paths.logs, prune::KEEP);
        let refuse = |reason| Err(Outcome::new(exit::REFUSED, reason));
        match platform::free_bytes(&self.paths.logs) {
            Ok(free) if free >= FREE_MIN => {}
            Ok(_) => return refuse("less than 2 GB free for the logs"),
            Err(_) => return refuse("the free space for the logs can't be read"),
        }
        match platform::count_processes_named("ffmpeg.exe") {
            Ok(0) => {}
            Ok(_) => return refuse("another FFmpeg is running: one encoder session at a time"),
            Err(_) => return refuse("the running programs can't be listed"),
        }
        for (path, reason) in [
            (
                &self.paths.ffmpeg,
                "FFmpeg isn't installed in the tools folder",
            ),
            (&self.paths.pin, "FFmpeg's pin file is missing"),
            (
                &self.paths.testbed,
                "testbed.exe isn't deployed beside the harness",
            ),
            (
                &self.paths.verify,
                "kairo-verify.exe isn't deployed beside the harness",
            ),
        ] {
            if !path.is_file() {
                return refuse(reason);
            }
        }
        if self.request.voice && !self.paths.tone.is_file() {
            return refuse("kairo-tone.exe isn't deployed beside the harness");
        }
        self.read_pin();
        let targets = kairo_display::targets()
            .map_err(|_| Outcome::new(exit::FAILED, "the displays can't be listed"))?;
        let Some(target) = kairo_display::find_by_path(&targets, &self.request.monitor) else {
            return refuse("the monitor isn't connected or active");
        };
        if target.cloned {
            return refuse("the monitor mirrors another");
        }
        let Some(refresh_mhz) = target.refresh.millihertz() else {
            return refuse("the monitor's refresh rate is unknown");
        };
        if target.hdr.active || target.hdr.mode == Some(kairo_display::ColorMode::Hdr) {
            return refuse("the monitor is in HDR");
        }
        let fps = self
            .request
            .testbed_fps
            .unwrap_or(((refresh_mhz + 500) / 1_000) as u32);
        let Some(k) = target.refresh.fits_vsync(fps) else {
            return refuse("the monitor can't show the testbed's rate with vsync");
        };
        let adapter = kairo_display::adapter_for_luid(target.adapter_luid)
            .map_err(|_| Outcome::new(exit::REFUSED, "the monitor's adapter can't be found"))?;
        (self.fps, self.k, self.refresh_mhz) = (fps, k, refresh_mhz);
        self.luid = target.adapter_luid.to_string();
        self.adapter_index = adapter.dxgi_index;
        self.device = format!("{:04x}:{:04x}", adapter.vendor_id, adapter.device_id);
        let r = target.rect;
        self.monitor_rect = (r.left, r.top, r.right, r.bottom);
        if self.request.capture == Capture::Dda {
            self.output_index = Some(
                kairo_display::output_index(target.adapter_luid, target.hmonitor).map_err(
                    |_| Outcome::new(exit::REFUSED, "the monitor has no DXGI output to duplicate"),
                )?,
            );
        }
        self.report.request.testbed_fps = fps;
        self.report.display = Some(DisplayFacts {
            refresh_mhz,
            source_hz: (refresh_mhz + 500) / 1_000,
            vsync_k: k,
            color_mode: target
                .hdr
                .mode
                .map(|m| format!("{m:?}").to_ascii_lowercase()),
        });
        self.report.adapter = Some(AdapterFacts {
            luid: self.luid.clone(),
            dxgi_index: self.adapter_index,
            device: self.device.clone(),
            log_device: None,
            matches: None,
        });
        if self.request.finishing && !self.has_h264_metadata() {
            return refuse("this FFmpeg has no h264_metadata filter for --finishing");
        }
        self.run_tag = run_tag();
        self.report.run_tag = Some(self.run_tag);
        let expect = Expect::new(
            self.request.output.size(),
            self.request.output.fps(),
            refresh_mhz,
            self.request.seconds,
            self.run_tag,
        );
        write_new(&self.paths.folder.join("expect.json"), &expect.json())
            .map_err(|_| Outcome::new(exit::FAILED, "expect.json couldn't be written"))
    }

    /// `KAIRO-PIN.json`: which FFmpeg build this is.
    fn read_pin(&mut self) {
        let Some(pin) = read_capped(&self.paths.pin, 1 << 20)
            .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok())
        else {
            self.report.ffmpeg.licence = "unknown".to_owned();
            return;
        };
        let text = |key: &str| pin.get(key).and_then(Value::as_str).map(str::to_owned);
        let asset = text("asset");
        self.report.ffmpeg.licence = if asset.as_deref().is_some_and(|a| a.contains("-lgpl-")) {
            "LGPL".to_owned()
        } else {
            "unknown".to_owned()
        };
        self.report.ffmpeg.asset = asset;
        self.report.ffmpeg.sha256 = text("sha256");
        self.report.ffmpeg.installed_utc = text("installed_utc");
    }

    fn has_h264_metadata(&self) -> bool {
        Command::new(&self.paths.ffmpeg)
            .args(["-hide_banner", "-h", "bsf=h264_metadata"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).contains("h264_metadata")
            })
    }

    /// Phase 1: the testbed starts, with `kairo-tone` beside it when asked, and writes its
    /// ready file.
    fn start_testbed(&mut self) -> Result<(), Outcome> {
        let seconds = testbed_seconds(self.request.seconds).to_string();
        let tag = self.run_tag.to_string();
        let fps = self.fps.to_string();
        let beep = if self.request.voice { "on" } else { "off" };
        let err = output_file(&self.paths.folder, "testbed.err")?;
        let mut testbed = Command::new(&self.paths.testbed)
            .args([
                "--monitor",
                &self.request.monitor,
                "--mode",
                "borderless",
                "--fps",
                &fps,
                "--seconds",
                &seconds,
                "--stop-on-stdin-eof",
                "--run-tag",
                &tag,
                "--log",
                "testbed.log",
                "--ready",
                "ready.json",
                "--beep",
                beep,
            ])
            .current_dir(&self.paths.folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(err)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(Guarded::new)
            .map_err(|_| Outcome::new(exit::FAILED, "the testbed didn't start"))?;
        self.testbed_stdin = testbed.child.stdin.take();
        let pid = testbed.child.id();
        self.testbed = Some(testbed);
        if self.request.voice {
            let err = output_file(&self.paths.folder, "tone.err")?;
            let mut tone = Command::new(&self.paths.tone)
                .args([
                    "beeps",
                    "--seconds",
                    &seconds,
                    "--run-tag",
                    &tag,
                    "--log",
                    "tone.log",
                    "--stop-on-stdin-eof",
                ])
                .current_dir(&self.paths.folder)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(err)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map(Guarded::new)
                .map_err(|_| Outcome::new(exit::FAILED, "kairo-tone didn't start"))?;
            self.tone_stdin = tone.child.stdin.take();
            self.tone = Some(tone);
        }
        let ready_path = self.paths.folder.join("ready.json");
        let deadline = Instant::now() + Duration::from_secs(Phase::Testbed.limit_s(0).into());
        let ready = loop {
            if let Some(text) = read_capped(&ready_path, 1 << 16) {
                break Ready::parse(&text).map_err(|r| Outcome::new(r.code, r.reason))?;
            }
            if let Some(testbed) = self.testbed.as_mut()
                && let Some(status) = testbed.ended()
            {
                self.note_testbed(Some(status));
                return Err(outcome::testbed_before_ready(code(Some(status))));
            }
            if Instant::now() >= deadline {
                return Err(Outcome::new(
                    exit::FAILED,
                    "the testbed wasn't ready within its limit",
                ));
            }
            std::thread::sleep(POLL);
        };
        ready
            .check(&Expected {
                pid,
                luid: &self.luid,
                refresh_mhz: self.refresh_mhz,
                mode: "borderless",
                fps: self.fps,
            })
            .map_err(|r| Outcome::new(r.code, r.reason))?;
        self.source = Some(match self.request.capture {
            Capture::Gfx => Source::Window { hwnd: ready.hwnd },
            Capture::Dda => {
                let c = ready.client;
                let crop =
                    capture::desktop_crop(self.monitor_rect, (c.left, c.top, c.width, c.height))
                        .ok_or(Outcome::new(
                            exit::REFUSED,
                            "the testbed's window isn't wholly on the monitor",
                        ))?;
                Source::Desktop {
                    output: self.output_index.unwrap_or(0),
                    crop,
                }
            }
        });
        // The log's header is written before the ready file: it must be this run's.
        let log = self
            .testbed_log()
            .ok_or(Outcome::new(exit::FAILED, "the testbed's log doesn't read"))?;
        let h = &log.header;
        let size_ok = i64::from(h.width) == i64::from(ready.client.width)
            && i64::from(h.height) == i64::from(ready.client.height);
        if h.run_tag != self.run_tag || h.hdr || h.fps != self.fps || !size_ok {
            return Err(Outcome::new(
                exit::INTERNAL,
                "the testbed's log isn't the run the harness started",
            ));
        }
        Ok(())
    }

    fn testbed_log(&self) -> Option<TestbedLog> {
        read_capped(&self.paths.folder.join("testbed.log"), LOG_MAX)
            .and_then(|t| TestbedLog::parse(&t).ok())
    }

    /// Phase 2: 3 s for the testbed to settle, then the judgement.
    fn warm_up(&mut self) -> Result<(), Outcome> {
        let end = Instant::now() + Duration::from_secs(Phase::Warmup.limit_s(0).into());
        loop {
            if let Some(testbed) = self.testbed.as_mut()
                && let Some(status) = testbed.ended()
            {
                self.note_testbed(Some(status));
                return Err(Outcome::new(
                    exit::FAILED,
                    "the testbed ended during the warm-up",
                ));
            }
            let now = Instant::now();
            let log = self.testbed_log();
            let judgement = log.as_ref().map(|l| warmup::judge(l, self.k));
            match judgement {
                Some(Judgement::Ended(reason)) => return Err(outcome::testbed_ended(&reason)),
                Some(Judgement::Off(reason)) if reason == "display off: no vblank timing" => {
                    return Err(Outcome::new(exit::REFUSED, reason));
                }
                Some(Judgement::Steady(s)) if now >= end => {
                    self.report.warmup = Some(WarmupFacts {
                        basis: s.basis.name(),
                        frames: s.frames,
                        display_share: s.display_share,
                        median_step_hns: s.median_step_hns,
                        step_hns: s.step_hns,
                        misses: s.misses,
                    });
                    return Ok(());
                }
                Some(Judgement::Off(reason)) if now >= end => {
                    return Err(Outcome::new(exit::REFUSED, reason));
                }
                None if now >= end => {
                    return Err(Outcome::new(exit::FAILED, "the testbed's log doesn't read"));
                }
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// Phase 3: FFmpeg records, then its log and the clip are checked.
    fn capture(&mut self) -> Result<(), Outcome> {
        let source = self
            .source
            .ok_or(Outcome::new(exit::INTERNAL, "no capture source"))?;
        let settings = Settings {
            source,
            adapter_index: self.adapter_index,
            max_framerate: self.request.max_framerate,
            output: self.request.output,
            seconds: self.request.seconds,
            loglevel: self.request.loglevel,
            finishing: self.request.finishing,
        };
        let log_file = output_file(&self.paths.folder, "ffmpeg.log")?;
        let start = Instant::now();
        let mut ffmpeg = Command::new(&self.paths.ffmpeg)
            .args(capture::args(&settings))
            .current_dir(&self.paths.folder)
            .env_remove("FFREPORT")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log_file)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(Guarded::new)
            .map_err(|_| Outcome::new(exit::FAILED, "FFmpeg didn't start"))?;
        let limit = Duration::from_secs(Phase::Capture.limit_s(self.request.seconds).into());
        let status = ffmpeg.wait_until(start + limit);
        let status = status.or_else(|| ffmpeg.kill());
        let killed = ffmpeg.killed;
        drop(ffmpeg);
        let f = &mut self.report.ffmpeg;
        f.exit_code = code(status);
        f.killed = killed;
        f.seconds = Some((start.elapsed().as_secs_f64() * 1_000.0).round() / 1_000.0);
        f.bytes = fs::metadata(self.paths.folder.join(CLIP))
            .ok()
            .map(|m| m.len());
        let testbed_alive = self.testbed.as_mut().is_some_and(|t| t.ended().is_none());
        if let Some(testbed) = self.report.testbed.as_mut() {
            testbed.alive_through_capture = testbed_alive;
        } else {
            self.report.testbed = Some(ChildFacts {
                alive_through_capture: testbed_alive,
                ..ChildFacts::default()
            });
        }
        if killed {
            return Err(Outcome::new(
                exit::FAILED,
                "the capture ran past its limit (an AMF hang?)",
            ));
        }
        let Some(log) = read_capped(&self.paths.folder.join("ffmpeg.log"), LOG_MAX) else {
            return Err(Outcome::new(
                exit::READBACK,
                "FFmpeg's log is missing or too big",
            ));
        };
        let window = matches!(source, Source::Window { .. });
        self.log_check = ffmpeg_log::check(&log, self.adapter_index, &self.device, window);
        self.log_read = true;
        let f = &mut self.report.ffmpeg;
        f.problems = self.log_check.problems.iter().map(|p| p.id()).collect();
        f.bframe_warnings = self.log_check.bframe_warnings;
        f.border_warning = self.log_check.border_warning;
        if let Some(adapter) = self.report.adapter.as_mut() {
            adapter.log_device = self.log_check.log_device.clone();
            adapter.matches = self
                .log_check
                .log_device
                .as_ref()
                .map(|d| d.eq_ignore_ascii_case(&self.device));
        }
        if f.exit_code != Some(0) {
            return Err(Outcome::new(exit::FAILED, "FFmpeg failed"));
        }
        if !self.log_check.ok() {
            return Err(Outcome::new(
                exit::READBACK,
                "FFmpeg's log lacks a line the read-back needs: nothing is verified",
            ));
        }
        if f.bytes.unwrap_or(0) == 0 {
            return Err(Outcome::new(exit::FAILED, "FFmpeg wrote no clip"));
        }
        if !testbed_alive {
            let end = self.testbed_log().and_then(|l| end_reason(&l));
            return Err(outcome::testbed_ended(end.as_deref().unwrap_or("")));
        }
        Ok(())
    }

    /// Phase 4: standard input closes, so the testbed (and kairo-tone) end their logs; killed
    /// at the limit. A testbed alive through the capture is judged even if it was killed here:
    /// its log covers the clip.
    fn stop(&mut self) -> Result<(), Outcome> {
        drop(self.testbed_stdin.take());
        drop(self.tone_stdin.take());
        let deadline = Instant::now() + Duration::from_secs(Phase::Stop.limit_s(0).into());
        if let Some(mut testbed) = self.testbed.take() {
            let status = testbed.wait_until(deadline).or_else(|| testbed.kill());
            let killed = testbed.killed;
            drop(testbed);
            self.note_testbed(status);
            if let Some(t) = self.report.testbed.as_mut() {
                t.killed |= killed;
            }
        }
        if let Some(mut tone) = self.tone.take() {
            let status = tone.wait_until(deadline).or_else(|| tone.kill());
            let message = read_capped(&self.paths.folder.join("tone.err"), 1 << 20)
                .and_then(|e| last_line(&e));
            let end = read_capped(&self.paths.folder.join("tone.log"), LOG_MAX).and_then(|t| {
                t.lines()
                    .rev()
                    .find_map(|l| l.strip_prefix("e end reason=").map(str::to_owned))
            });
            self.report.tone = Some(ChildFacts {
                exit_code: code(status),
                end,
                killed: tone.killed,
                alive_through_capture: true,
                message,
            });
        }
        Ok(())
    }

    /// What the testbed's end was.
    fn note_testbed(&mut self, status: Option<ExitStatus>) {
        let message = read_capped(&self.paths.folder.join("testbed.err"), 1 << 20)
            .and_then(|e| last_line(&e));
        let end = self.testbed_log().and_then(|l| end_reason(&l));
        let facts = self.report.testbed.get_or_insert_with(ChildFacts::default);
        facts.exit_code = code(status);
        facts.end = end;
        facts.message = message;
    }

    /// Phase 5: kairo-verify judges the clip against the testbed's log.
    fn verify(&mut self) -> Result<(), Outcome> {
        let err = output_file(&self.paths.folder, "verify.err")?;
        let start = Instant::now();
        let mut verify = Command::new(&self.paths.verify)
            .args([
                "check",
                CLIP,
                "--profile",
                self.request.profile.name(),
                "--expect",
                "expect.json",
                "--testbed-log",
                "testbed.log",
                "--json",
                "verify.json",
                "--summary",
                "verify.txt",
            ])
            .current_dir(&self.paths.folder)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(err)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(Guarded::new)
            .map_err(|_| Outcome::new(exit::FAILED, "kairo-verify didn't start"))?;
        let limit = Duration::from_secs(Phase::Verify.limit_s(0).into());
        let status = verify.wait_until(start + limit);
        let timed_out = status.is_none();
        drop(verify);
        let report = read_capped(&self.paths.folder.join("verify.json"), REPORT_MAX)
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let check = |id: &str| {
            report
                .as_ref()?
                .get("checks")?
                .as_array()?
                .iter()
                .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
                .cloned()
        };
        let failing = report
            .as_ref()
            .and_then(|r| r.get("checks")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|c| {
                matches!(
                    c.get("status").and_then(Value::as_str),
                    Some("fail" | "error")
                )
            })
            .filter_map(|c| Some(c.get("id")?.as_str()?.to_owned()))
            .collect();
        self.report.verify = Some(VerifyFacts {
            exit_code: if timed_out { None } else { code(status) },
            report: report.is_some(),
            verdict: report
                .as_ref()
                .and_then(|r| r.get("verdict")?.as_str().map(str::to_owned)),
            seconds: Some((start.elapsed().as_secs_f64() * 1_000.0).round() / 1_000.0),
            barcode_share: check("content.barcode")
                .and_then(|c| c.get("value")?.get("share")?.as_f64()),
            failing,
        });
        self.verify_report = report;
        let o = outcome::verify(
            self.report.verify.as_ref().and_then(|v| v.exit_code),
            self.verify_report.as_ref(),
        );
        if o.code == exit::OK { Ok(()) } else { Err(o) }
    }

    /// Phase 6: leftovers killed, the read-back table, `harness.json` and the summary.
    fn finish(mut self, outcome: Outcome) -> u8 {
        use std::io::Write;
        if let Some(mut testbed) = self.testbed.take() {
            drop(self.testbed_stdin.take());
            let status = testbed.kill();
            drop(testbed);
            self.note_testbed(status);
        }
        if let Some(mut tone) = self.tone.take() {
            drop(self.tone_stdin.take());
            tone.kill();
        }
        if self.report.phases.len() < Phase::ALL.len() {
            self.report.skip_rest();
        }
        if let Some(source) = self.source.filter(|_| self.log_read) {
            self.report.readback = readback::table(
                self.verify_report.as_ref(),
                &self.log_check,
                source,
                self.request.seconds,
                self.request.finishing,
            );
            self.report.readback_counts = readback::counts(&self.report.readback);
        }
        self.report.exit_code = outcome.code;
        self.report.reason = outcome.reason.to_owned();
        self.report.ended_utc = kairo_core::time::now_utc();
        self.report.files = files(&self.paths.folder);
        if !self.report.files.iter().any(|f| f == report::REPORT) {
            self.report.files.push(report::REPORT.to_owned());
            self.report.files.sort();
        }
        let mut code = outcome.code;
        if self.report.write(&self.paths.folder).is_err() {
            self.report.exit_code = exit::INTERNAL;
            self.report.reason = "harness.json couldn't be written".to_owned();
            code = exit::INTERNAL;
        }
        let _ = write!(std::io::stdout(), "{}", self.report.summary());
        if code != exit::OK {
            let _ = writeln!(std::io::stderr(), "kairo-harness: {}", self.report.reason);
        }
        code
    }
}

/// The log's `e end` reason, if it has one.
fn end_reason(log: &TestbedLog) -> Option<String> {
    log.records.iter().rev().find_map(|r| match r {
        kairo_core::testbed_log::TestbedRecord::Event(e) if e.name() == "end" => e
            .fields()
            .iter()
            .find(|(k, _)| k == "reason")
            .map(|(_, v)| v.clone()),
        _ => None,
    })
}

/// A run tag, 0 to 127, that differs from run to run.
fn run_tag() -> u8 {
    use std::hash::BuildHasher;
    let hash = std::collections::hash_map::RandomState::new().hash_one(std::process::id());
    (hash & 0x7F) as u8
}

/// A new file for a child's output; it must not exist yet.
fn output_file(folder: &Path, name: &str) -> Result<File, Outcome> {
    File::options()
        .write(true)
        .create_new(true)
        .open(folder.join(name))
        .map_err(|_| {
            Outcome::new(
                exit::FAILED,
                "a log file in the run folder couldn't be made",
            )
        })
}

/// Writes a new file; it must not exist yet.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    File::options()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(text.as_bytes())
}

/// A file's text, if it is a regular file of at most `max` bytes.
fn read_capped(path: &Path, max: u64) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take(max)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// The names of the files in the run folder, sorted.
fn files(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| !n.ends_with(".tmp"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_is_killed_and_reaped_at_its_deadline() {
        let child = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        let mut guarded = Guarded::new(child);
        let start = Instant::now();
        assert!(guarded.wait_until(start + Duration::from_secs(1)).is_none());
        assert!(guarded.kill().is_some(), "reaped");
        assert!(guarded.killed);
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn files_are_read_only_when_small_and_real() {
        let dir = std::env::temp_dir().join(format!("kairo-harness-run-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.json");
        write_new(&path, "{}").unwrap();
        assert!(write_new(&path, "{}").is_err(), "never replaced");
        assert_eq!(read_capped(&path, 10).as_deref(), Some("{}"));
        assert_eq!(read_capped(&path, 1), None);
        assert_eq!(read_capped(&dir, 10), None);
        assert_eq!(files(&dir), ["a.json"]);
        fs::remove_dir_all(&dir).unwrap();
        assert!(run_tag() < 128);
    }
}
