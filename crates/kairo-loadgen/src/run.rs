// SPDX-License-Identifier: GPL-3.0-or-later
//! One `run` or `calibrate` on Windows (docs/research/17-loadgen-bench/README.md §3.3–3.10): find
//! the monitor by its device path and refuse a mirrored or HDR one, open the window at its
//! rectangle and the swap chain on its adapter, take the foreground with `--activate`, draw the
//! start-up ramp ([`crate::workload::Ramp`]) or the calibration ([`crate::workload::Calibrator`])
//! through the TDR guards, write the ready file and the log, and print the summary or calibration
//! line ([`crate::summary`]). It has no `unsafe`: the calls that need it are in `gpu` and
//! `kairo-window`.
//!
//! **One frame** (README §3.4): wait for the swap chain's latency object (1 s at most; on a
//! time-out, look at the stop flags and the device, then wait again); take the frame's QPC time,
//! which the picture is drawn for; fill the constant buffer; open the frame's query set; draw the
//! bands; close the set; `Present`; read the sets that are ready, oldest first, without waiting.
//! A frame allocates nothing: its `f` line goes through the log's one buffer, its time into a
//! histogram made once.
//!
//! **Steps.** The ramp's and the calibration's steps are timed by their own frames' GPU times
//! ([`StepTimer`]); a step whose frames never get one is exit 4. Frames drawn for an earlier step
//! that resolve late don't count for the next.
//!
//! **Stops.** `--seconds` after frame 0's start (`run`), `WM_CLOSE`, standard input's end with
//! `--stop-on-stdin-eof`, a display change (exit 3), three frames in a row over 250 ms of GPU
//! time (exit 3), and a lost device or a failed `Present` (exit 4), never recreated. At the end,
//! the frames still in flight get their times (a flushing read, 2 s at most) and their `f` lines.
//! Messages on stderr name no device path or file path (CLAUDE.md), and neither does the log.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kairo_core::timeline::HNS_PER_SECOND;
use kairo_display::{ColorMode, DisplayTarget};
use kairo_window::gpu::{Gpu, HdrWatch, PresentError, Presented};
use kairo_window::platform::{self, Frame, KeepDisplayOn, LockWait, Window};

use crate::cli::{CalibrateConfig, Command, Common, RunConfig, exit};
use crate::gpu::{self, AdapterCaps, Chain, RING, Renderer, Ring, Shaders};
use crate::log::{EndReason, Event, FrameRecord, Header, Log};
use crate::summary::{
    CalibrationLine, Conditions, GpuTime, Ready, RunStats, ShaderHashes, umd_version_text,
};
use crate::workload::{
    Calibrated, CalibrationStep, Calibrator, FrameConstants, Geometry, LONG_FRAME_MS, LongFrames,
    RAMP_FRAMES, Ramp, Refusal, StepTimer, StepTiming, Texture,
};

/// The window's class name.
const WINDOW_CLASS: &str = "KairoLoadgen";
/// The window's title (M1's contract: "Its window is 'Kairo loadgen'").
const WINDOW_TITLE: &str = "Kairo loadgen";
/// How long one wait for the swap chain may take, in ms (README §3.4).
const WAIT_MS: u32 = 1_000;
/// How often the displays are checked for a change.
const CHECK_EVERY_HNS: i64 = HNS_PER_SECOND / 2;
/// How often a run logs its present hints after `e ready` (README §3.9).
const HINT_EVERY_HNS: i64 = 10 * HNS_PER_SECOND;
/// How long the swap chain may take no frame before frame 0 of a run, or during a calibration:
/// then it isn't working (exit 4).
const NO_FRAME_LIMIT_HNS: i64 = 10 * HNS_PER_SECOND;
/// The longest the end of a run waits for the frames still in flight (README §3.8).
const DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// Why a run stopped: an exit code and a message without paths.
struct Stop(u8, String);

fn stop(code: u8, message: impl Into<String>) -> Stop {
    Stop(code, message.into())
}

/// What the command line asked for.
#[derive(Clone, Copy)]
enum Job<'a> {
    Run(&'a RunConfig),
    Calibrate(&'a CalibrateConfig),
}

/// Runs `command`; returns the exit code.
pub fn run(command: &Command) -> u8 {
    let (common, job) = match command {
        Command::Run(config) => (&config.common, Job::Run(config)),
        Command::Calibrate(config) => (&config.common, Job::Calibrate(config)),
        // The binary answers these itself.
        Command::Version | Command::Help => return exit::USAGE,
    };
    let file = match &common.log {
        Some(path) => match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => Some(file),
            Err(e) => {
                eprintln!("kairo-loadgen: can't make the log: {}", e.kind());
                return exit::USAGE;
            }
        },
        None => None,
    };
    let mut log = Log::new(file);
    let (code, reason) = match go(common, job, &mut log) {
        Ok(reason) => (exit::OK, reason),
        Err(Stop(code, message)) => {
            eprintln!("kairo-loadgen: {message}");
            (code, EndReason::Error)
        }
    };
    // A run that failed before its header was known has nothing to log: its empty file goes.
    if !log.end(reason, platform::now_hns())
        && let Some(path) = &common.log
    {
        drop(log.into_inner());
        let _ = fs::remove_file(path);
    }
    code
}

/// Writes `text` to standard output, which the bench keeps; a closed output isn't an error.
fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// What loadgen learnt about the monitor, its adapter and its shaders before the first frame.
struct Facts {
    refresh_mhz: u64,
    adapter_luid: String,
    umd_version: Option<String>,
    caps: AdapterCaps,
    hashes: ShaderHashes,
    fingerprint: u64,
    qpc_hz: i64,
}

/// Sets the run up, runs it, and reads the frames still in flight; the reason it ended.
fn go(common: &Common, job: Job<'_>, log: &mut Log<File>) -> Result<EndReason, Stop> {
    let exists = |path: &Option<std::path::PathBuf>| path.as_ref().is_some_and(|p| p.exists());
    match job {
        Job::Run(config) if exists(&config.ready) => {
            return Err(stop(exit::USAGE, "the ready file exists already"));
        }
        Job::Calibrate(config) if exists(&config.out) => {
            return Err(stop(exit::USAGE, "the --out file exists already"));
        }
        _ => {}
    }
    kairo_display::make_dpi_aware();
    platform::opt_out_of_power_throttling();
    let targets =
        kairo_display::targets().map_err(|_| stop(exit::REFUSED, "can't list the displays"))?;
    let target = kairo_display::find_by_path(&targets, &common.monitor)
        .ok_or_else(|| stop(exit::REFUSED, "the monitor given isn't an active display"))?;
    if target.cloned {
        return Err(stop(exit::REFUSED, "the monitor mirrors another display"));
    }
    let refresh_mhz = target
        .refresh
        .millihertz()
        .ok_or_else(|| stop(exit::REFUSED, "the monitor's refresh rate is unknown"))?;
    // The watch's factory is made before the device's, so a change from here on makes it stale.
    let hdr_watch = HdrWatch::new(target.adapter_luid, target.hmonitor);
    let gpu =
        Gpu::on_adapter(target.adapter_luid, target.hmonitor).map_err(|e| stop(exit::DEVICE, e))?;
    if hdr_on(target, &gpu) {
        return Err(stop(
            exit::REFUSED,
            "the monitor is in HDR: loadgen draws SDR only in M1",
        ));
    }
    if !gpu::tearing_supported(gpu.factory()) {
        return Err(stop(
            exit::REFUSED,
            "DXGI can't present with tearing here, and loadgen never presents without it",
        ));
    }
    let shaders =
        Shaders::compile(common.workload, common.detail).map_err(|e| stop(exit::DEVICE, e))?;
    let hashes = shaders
        .hashes()
        .ok_or_else(|| stop(exit::DEVICE, "a shader is missing"))?;
    let caps = gpu::adapter_caps(target.adapter_luid);
    let umd_version = gpu::umd_version(gpu.device()).map(umd_version_text);
    let rect = target.rect;
    let size = (rect.width(), rect.height());
    let (Ok(width), Ok(height)) = (i32::try_from(size.0), i32::try_from(size.1)) else {
        return Err(stop(exit::REFUSED, "the monitor's rectangle is too large"));
    };
    let fingerprint = Conditions {
        workload: common.workload,
        shader: &hashes,
        size,
        refresh_mhz,
        umd_version: umd_version.as_deref(),
        hags: caps.hags,
        texture_mib: common.texture.map(|t| t.mib()),
    }
    .fingerprint();
    let window = Window::create(
        WINDOW_CLASS,
        WINDOW_TITLE,
        Frame::Popup,
        rect.left,
        rect.top,
        width,
        height,
    )
    .map_err(|e| stop(exit::DEVICE, e))?;
    let covers = |window: &Window| window.client_rect() == (rect.left, rect.top, width, height);
    if !covers(&window) {
        return Err(not_covered());
    }
    let facts = Facts {
        refresh_mhz,
        adapter_luid: target.adapter_luid.to_string(),
        umd_version,
        caps,
        hashes,
        fingerprint,
        qpc_hz: platform::qpc_frequency(),
    };
    // The header is known from here, so the setup's events reach the log even if frame 0 never
    // comes.
    log.set_header(Header {
        workload: common.workload,
        count: match job {
            Job::Run(config) => Some(config.count),
            Job::Calibrate(_) => None,
        },
        texture: common.texture,
        bands: common.bands,
        size,
        refresh_mhz,
        buffers: common.buffers,
        latency: common.latency,
        detail: common.detail,
        qpc_hz: facts.qpc_hz,
        shader: facts.hashes.text(),
        preempt: facts.caps.preempt,
        hags: facts.caps.hags,
        hwcomp: gpu::hardware_composition(gpu.output()),
        rotation: gpu::rotation(gpu.output()),
        hdr: false,
        start_hns: 0,
    });
    let _display = KeepDisplayOn::new();
    let stdin_eof = Arc::new(AtomicBool::new(false));
    if common.stop_on_stdin_eof {
        kairo_window::stdin::watch_eof(Arc::clone(&stdin_eof));
    }
    if common.activate {
        match platform::wait_for_foreground_lock(&window, &stdin_eof) {
            LockWait::Ready => {}
            LockWait::Closed => return Ok(EndReason::Close),
            LockWait::StdinEof => return Ok(EndReason::Stdin),
        }
        let ok = platform::take_foreground(&window);
        log.event(&Event::Activate { ok });
        if !ok {
            return Err(stop(exit::FOCUS, "the window didn't get the foreground"));
        }
    } else {
        window.show(false);
    }
    // DirectFlip needs the client area to cover the screen (README §3.3).
    if !covers(&window) {
        return Err(not_covered());
    }
    let chain = Chain::new(
        gpu.factory(),
        gpu.device(),
        window.hwnd(),
        size,
        common.buffers,
        common.latency,
        true,
    )
    .map_err(|e| stop(exit::DEVICE, e))?;
    let renderer = Renderer::new(
        gpu.device(),
        gpu.context(),
        &shaders,
        common.texture,
        size,
        common.bands,
    )
    .map_err(|e| stop(exit::DEVICE, e))?;
    let ring = Ring::new(gpu.device(), gpu.context()).map_err(|e| stop(exit::DEVICE, e))?;
    let now = platform::now_hns();
    let mut load = Load {
        common,
        log,
        ring,
        renderer,
        chain,
        gpu,
        window,
        facts,
        hdr_watch,
        stdin_eof,
        size,
        geometry: Geometry {
            width: size.0,
            height: size.1,
            bands: common.bands,
        },
        texture: common.texture,
        anchor_hns: None,
        records: [Submitted::default(); RING],
        resolved: [(0, GpuTime::Unknown); RING],
        timer: StepTimer::new(),
        long: LongFrames::new(),
        stats: RunStats::new(),
        run_frames: 0,
        frame0_hns: None,
        next_check_hns: now,
        last_frame_hns: now,
    };
    let result = match job {
        Job::Run(config) => load.run(config),
        Job::Calibrate(config) => load.calibrate(config),
    };
    load.finish(job, &result);
    result
}

/// Whether the monitor's output is in HDR, as the display configuration or DXGI says.
fn hdr_on(target: &DisplayTarget, gpu: &Gpu) -> bool {
    target.hdr.active
        || target.hdr.mode == Some(ColorMode::Hdr)
        || gpu.output_hdr().is_some_and(|output| output.pq)
}

fn not_covered() -> Stop {
    stop(
        exit::REFUSED,
        "the window's client area isn't exactly the monitor's",
    )
}

/// A frame the GPU was given, until its time comes back.
#[derive(Debug, Clone, Copy, Default)]
struct Submitted {
    /// The step it was drawn for ([`StepTimer`]'s id; 0 outside a step).
    step: u32,
    /// Its count.
    count: u32,
    /// Its number in the run, from frame 0 at the run's count; `None` for the ramp and a
    /// calibration.
    run_frame: Option<u64>,
    /// When its CPU work started, QPC in 100 ns.
    start_hns: i64,
    /// When `Present` returned.
    present_hns: i64,
}

/// What a pass of the loop did.
enum Pass {
    /// A frame went out, started at this time.
    Drawn(i64),
    /// The swap chain took no frame within the wait.
    NoFrame,
    /// The run was asked to stop.
    Stopped(EndReason),
}

/// A run once it is set up. Fields drop in order: the queries, the renderer and the swap chain
/// before the device, and all of them before the window.
struct Load<'a> {
    common: &'a Common,
    log: &'a mut Log<File>,
    ring: Ring,
    renderer: Renderer,
    chain: Chain,
    gpu: Gpu,
    window: Window,
    facts: Facts,
    hdr_watch: Option<HdrWatch>,
    stdin_eof: Arc<AtomicBool>,
    size: (u32, u32),
    geometry: Geometry,
    texture: Option<Texture>,
    /// The picture's time origin: the first frame's start, so the ramp, the run and the end draw
    /// one picture with no step in time (README §3.11).
    anchor_hns: Option<i64>,
    /// The frames in flight, by query set.
    records: [Submitted; RING],
    /// The times a poll of the query sets gave back.
    resolved: [(u64, GpuTime); RING],
    timer: StepTimer,
    long: LongFrames,
    stats: RunStats,
    /// Frames presented at the run's count.
    run_frames: u64,
    /// Frame 0's start, once the run is ready (frame 0 out, the ready file written): a run
    /// prints its summary only then.
    frame0_hns: Option<i64>,
    next_check_hns: i64,
    /// When the swap chain last took a frame.
    last_frame_hns: i64,
}

impl Load<'_> {
    /// `run` (README §3.6, §3.10): the start-up ramp, then the run's count from frame 0 until
    /// `--seconds` after it.
    fn run(&mut self, config: &RunConfig) -> Result<EndReason, Stop> {
        let mut ramp = Ramp::new(self.common.workload, config.count, self.geometry);
        let mut step = if ramp.done() {
            0
        } else {
            self.timer.start(platform::now_hns(), RAMP_FRAMES, 0)
        };
        let mut end_hns = None;
        let mut next_hint_hns = i64::MAX;
        loop {
            let now = platform::now_hns();
            if end_hns.is_some_and(|end| now >= end) {
                return Ok(EndReason::Seconds);
            }
            if end_hns.is_none() {
                self.check_progress(now)?;
            }
            let count = ramp.current();
            let run_frame = ramp.done().then_some(self.run_frames);
            let start_hns = match self.pass(count, step, run_frame)? {
                Pass::Drawn(start_hns) => start_hns,
                Pass::NoFrame => continue,
                Pass::Stopped(reason) => return Ok(reason),
            };
            if run_frame == Some(0) {
                self.started(config, start_hns)?;
                end_hns =
                    Some(start_hns.saturating_add(i64::from(config.seconds) * HNS_PER_SECOND));
                next_hint_hns = start_hns.saturating_add(HINT_EVERY_HNS);
            }
            let now = platform::now_hns();
            if now >= next_hint_hns {
                self.hint();
                next_hint_hns = next_hint_hns.saturating_add(HINT_EVERY_HNS);
            }
            if ramp.done() {
                continue;
            }
            let ramp_step = ramp.step().unwrap_or(0);
            match self.timer.timing(now) {
                StepTiming::Running => {}
                StepTiming::NoTiming => {
                    self.log.event(&Event::Ramp {
                        step: ramp_step,
                        count,
                        gpu_us: None,
                    });
                    return Err(self.refused(Refusal::NoTiming));
                }
                StepTiming::Done { median_us, .. } => {
                    self.log.event(&Event::Ramp {
                        step: ramp_step,
                        count,
                        gpu_us: Some(median_us),
                    });
                    if let Err(refusal) = ramp.measured(median_us as f64 / 1_000.0) {
                        return Err(self.refused(refusal));
                    }
                    step = if ramp.done() {
                        0
                    } else {
                        self.timer.start(now, RAMP_FRAMES, 0)
                    };
                }
            }
        }
    }

    /// Frame 0 went out at the run's count: the log's header, the ready file, `e ready` and the
    /// first present hint.
    fn started(&mut self, config: &RunConfig, start_hns: i64) -> Result<(), Stop> {
        self.log.start(start_hns);
        if let Some(path) = &config.ready {
            let ready = Ready {
                pid: std::process::id(),
                hwnd: self.window.hwnd(),
                client: self.window.client_rect(),
                refresh_mhz: self.facts.refresh_mhz,
                adapter_luid: self.facts.adapter_luid.clone(),
                adapter_index: self.gpu.adapter_index,
                output_index: self.gpu.output_index,
                umd_version: self.facts.umd_version.clone(),
                hdr: false,
                buffers: self.common.buffers,
                latency: self.common.latency,
                activated: self.common.activate,
                workload: self.common.workload,
                count: config.count,
                bands: self.common.bands,
                detail: self.common.detail,
                qpc_hz: self.facts.qpc_hz,
                hags: self.facts.caps.hags,
                fingerprint: self.facts.fingerprint,
                start_hns,
                seconds: config.seconds,
            };
            ready.write(path).map_err(|e| {
                stop(
                    exit::USAGE,
                    format!("can't write the ready file: {}", e.kind()),
                )
            })?;
        }
        self.frame0_hns = Some(start_hns);
        self.log.event(&Event::Ready);
        self.hint();
        Ok(())
    }

    /// `e present_hint`: the composition mode the swap chain reports, if any, and the presents
    /// that found the window occluded so far.
    fn hint(&mut self) {
        let media = self.chain.media_hint();
        self.log.event(&Event::PresentHint {
            media,
            occluded: self.stats.occluded(),
        });
    }

    /// `calibrate` (README §3.6): the calibration's steps, then its line.
    fn calibrate(&mut self, config: &CalibrateConfig) -> Result<EndReason, Stop> {
        let mut calibrator = Calibrator::new(
            self.common.workload,
            config.target_ms,
            config.warmup_seconds,
            self.geometry,
        );
        let mut step = calibrator.first();
        let mut hold_until_hns = i64::MAX;
        let mut fit_logged = false;
        let now = platform::now_hns();
        // The log starts with the first frame; a calibration has no frame 0.
        self.log.start(now);
        self.begin(&step, now, &mut hold_until_hns);
        loop {
            self.check_progress(platform::now_hns())?;
            let (count, id) = match step {
                CalibrationStep::Measure { count, .. } => (count, self.timer.id()),
                CalibrationStep::Hold { count, .. } => (count, 0),
                CalibrationStep::Done(calibrated) => return self.calibrated(config, calibrated),
            };
            match self.pass(count, id, None)? {
                Pass::Drawn(_) => {}
                Pass::NoFrame => continue,
                Pass::Stopped(reason) => return Ok(reason),
            }
            let now = platform::now_hns();
            let next = match step {
                CalibrationStep::Measure {
                    stage,
                    step: k,
                    count,
                    ..
                } => match self.timer.timing(now) {
                    StepTiming::Running => continue,
                    StepTiming::NoTiming => return Err(self.refused(Refusal::NoTiming)),
                    StepTiming::Done { median_us, frames } => {
                        self.log.event(&Event::Step {
                            stage,
                            step: k,
                            count,
                            frames,
                            gpu_us: median_us,
                        });
                        calibrator.measured(median_us as f64 / 1_000.0)
                    }
                },
                CalibrationStep::Hold { .. } if now < hold_until_hns => continue,
                CalibrationStep::Hold { .. } => calibrator.held(),
                CalibrationStep::Done(_) => continue,
            };
            if !fit_logged {
                let fit = match &next {
                    Err(Refusal::NotLinear(fit)) => Some(*fit),
                    _ => calibrator.fit(),
                };
                if let Some(fit) = fit {
                    self.log.event(&Event::Fit(fit));
                    fit_logged = true;
                }
            }
            step = next.map_err(|refusal| self.refused(refusal))?;
            self.begin(&step, now, &mut hold_until_hns);
        }
    }

    /// Starts a calibration step: its timer, or the hold's end.
    fn begin(&mut self, step: &CalibrationStep, now: i64, hold_until_hns: &mut i64) {
        match *step {
            CalibrationStep::Measure {
                frames, seconds, ..
            } => {
                self.timer.start(now, frames, seconds);
            }
            CalibrationStep::Hold { count, seconds } => {
                self.log.event(&Event::Hold { count, seconds });
                *hold_until_hns = now.saturating_add(i64::from(seconds) * HNS_PER_SECOND);
            }
            CalibrationStep::Done(_) => {}
        }
    }

    /// The calibration found its count: `e calibrated`, the line on stdout and in `--out`.
    fn calibrated(
        &mut self,
        config: &CalibrateConfig,
        calibrated: Calibrated,
    ) -> Result<EndReason, Stop> {
        self.log.event(&Event::Calibrated {
            count: calibrated.count,
            gpu_us: (calibrated.gpu_ms * 1_000.0).round() as u64,
            settled: calibrated.settled,
        });
        let line = CalibrationLine {
            workload: self.common.workload,
            target_ms: config.target_ms,
            calibrated,
            bands: self.common.bands,
            size: self.size,
            texture_mib: self.texture.map(|t| t.mib()),
            shader_digest: self.facts.hashes.digest(),
            fingerprint: self.facts.fingerprint,
        };
        say(&line.json());
        if let Some(path) = &config.out {
            line.write(path, std::process::id()).map_err(|e| {
                stop(
                    exit::USAGE,
                    format!("can't write the --out file: {}", e.kind()),
                )
            })?;
        }
        Ok(EndReason::Done)
    }

    /// Logs a refusal (`e too_heavy` for a TDR guard, `e refused reason=` otherwise) and turns it
    /// into the stop it is.
    fn refused(&mut self, refusal: Refusal) -> Stop {
        match refusal {
            Refusal::TooHeavy(heavy) => self.log.event(&Event::TooHeavy {
                count: heavy.count,
                predicted_ms: heavy.predicted_ms,
            }),
            _ => self.log.event(&Event::Refused {
                reason: refusal.reason(),
            }),
        }
        Stop(refusal.exit_code(), refusal.message())
    }

    /// Refuses a swap chain that has taken no frame for [`NO_FRAME_LIMIT_HNS`].
    fn check_progress(&self, now: i64) -> Result<(), Stop> {
        if now.saturating_sub(self.last_frame_hns) > NO_FRAME_LIMIT_HNS {
            return Err(stop(
                exit::DEVICE,
                "the swap chain has taken no frame for 10 s",
            ));
        }
        Ok(())
    }

    /// Handles the window's messages; the reason to stop, if there is one.
    fn stop_requested(&self) -> Option<EndReason> {
        self.window.pump();
        if platform::close_requested() {
            Some(EndReason::Close)
        } else if self.stdin_eof.load(Ordering::Relaxed) {
            Some(EndReason::Stdin)
        } else {
            None
        }
    }

    /// Every half second: have the displays changed? That ends the run (`e display_changed`,
    /// exit 3): it would no longer compare with the others.
    fn check_displays(&mut self, now: i64) -> Result<(), Stop> {
        if now < self.next_check_hns {
            return Ok(());
        }
        self.next_check_hns = now.saturating_add(CHECK_EVERY_HNS);
        if self
            .hdr_watch
            .as_mut()
            .and_then(HdrWatch::changed)
            .is_some()
        {
            self.log.event(&Event::DisplayChanged);
            return Err(stop(exit::REFUSED, "the displays changed during the run"));
        }
        Ok(())
    }

    /// A lost device: `e device_lost reason=`, exit 4. Loadgen never recreates it (README §3.10).
    fn lost(&mut self, reason: u32) -> Stop {
        self.log.event(&Event::DeviceLost { reason });
        stop(
            exit::DEVICE,
            format!("the device was lost (reason {reason:#010x})"),
        )
    }

    /// One pass of the loop (README §3.4): the stop flags, the displays, the wait, then one frame
    /// at `count` for step `step` (0 outside a step), numbered `run_frame` in the run.
    fn pass(&mut self, count: u32, step: u32, run_frame: Option<u64>) -> Result<Pass, Stop> {
        if let Some(reason) = self.stop_requested() {
            return Ok(Pass::Stopped(reason));
        }
        self.check_displays(platform::now_hns())?;
        if !self.chain.wait(WAIT_MS) {
            if let Some(reason) = gpu::removed_reason(self.gpu.device()) {
                return Err(self.lost(reason));
            }
            return Ok(Pass::NoFrame);
        }
        let start_hns = platform::now_hns();
        let anchor = *self.anchor_hns.get_or_insert(start_hns);
        let constants = FrameConstants::new(start_hns - anchor, count, self.texture, self.size);
        if let Err(why) = self.renderer.set_constants(&constants) {
            return Err(match gpu::removed_reason(self.gpu.device()) {
                Some(reason) => self.lost(reason),
                None => stop(exit::DEVICE, why),
            });
        }
        let (frame, evicted) = self.ring.begin();
        if let Some(old) = evicted {
            self.resolve(old, GpuTime::Unknown, true)?;
        }
        let slot = (frame % RING as u64) as usize;
        self.records[slot] = Submitted {
            step,
            count,
            run_frame,
            start_hns,
            present_hns: start_hns,
        };
        self.renderer.draw(self.chain.target());
        self.ring.end();
        let presented = self.chain.present();
        let present_hns = platform::now_hns();
        let occluded = match presented {
            Ok(Presented::Queued) => false,
            Ok(Presented::Occluded) => true,
            Err(failed) => {
                // The frame never went out: its time counts for nothing.
                self.records[slot] = Submitted::default();
                return Err(match failed {
                    PresentError::DeviceLost { reason, .. } => self.lost(reason.0 as u32),
                    PresentError::Other(hr) => {
                        self.log.event(&Event::PresentFailed { hr: hr.0 as u32 });
                        stop(exit::DEVICE, failed.to_string())
                    }
                });
            }
        };
        self.records[slot].present_hns = present_hns;
        self.last_frame_hns = present_hns;
        if run_frame.is_some() {
            self.run_frames += 1;
            self.stats.presented(occluded);
        }
        let read = self.ring.poll(false, &mut self.resolved);
        let resolved = self.resolved;
        // Every frame read gets its line, even after one trips the guard.
        let mut guarded = Ok(());
        for &(frame, time) in &resolved[..read] {
            let checked = self.resolve(frame, time, guarded.is_ok());
            if guarded.is_ok() {
                guarded = checked;
            }
        }
        guarded.map(|()| Pass::Drawn(start_hns))
    }

    /// Frame `frame`'s GPU time came back (or never will, `Unknown`): it counts for its step,
    /// and a run frame gets its `f` line and its place in the summary. With `guard`, three frames
    /// in a row over 250 ms end the run (README §3.10).
    fn resolve(&mut self, frame: u64, time: GpuTime, guard: bool) -> Result<(), Stop> {
        let record = self.records[(frame % RING as u64) as usize];
        self.timer.add(record.step, time);
        if let Some(n) = record.run_frame {
            self.stats.gpu(time);
            self.log.frame(&FrameRecord {
                frame: n,
                start_hns: record.start_hns,
                present_hns: record.present_hns,
                gpu_us: time.us(),
            });
        }
        let gpu_ms = time.us().map(|us| us as f64 / 1_000.0);
        if guard && self.long.frame(gpu_ms) {
            self.log.event(&Event::LongFrames {
                count: record.count,
                gpu_ms: gpu_ms.unwrap_or(LONG_FRAME_MS),
            });
            return Err(stop(
                exit::REFUSED,
                format!("three frames in a row took over {LONG_FRAME_MS} ms of GPU time"),
            ));
        }
        Ok(())
    }

    /// The end: the frames still in flight get their times (a flushing read, bounded, unless the
    /// device is gone) and their `f` lines, and a run that reached `e ready` prints its summary.
    fn finish(&mut self, job: Job<'_>, result: &Result<EndReason, Stop>) {
        let stopped_hns = platform::now_hns();
        let mut out = [(0, GpuTime::Unknown); RING];
        let read = if gpu::removed_reason(self.gpu.device()).is_none() {
            self.ring.drain(DRAIN_LIMIT, &mut out)
        } else {
            self.ring.abandon(&mut out)
        };
        for &(frame, time) in &out[..read] {
            // The run has ended: no guard any more.
            let _ = self.resolve(frame, time, false);
        }
        if let (Job::Run(config), Some(frame0_hns)) = (job, self.frame0_hns) {
            let seconds = (stopped_hns - frame0_hns).max(0) as f64 / HNS_PER_SECOND as f64;
            let end = match result {
                Ok(reason) => *reason,
                Err(_) => EndReason::Error,
            };
            let summary = self
                .stats
                .summary(self.common.workload, config.count, seconds, end);
            say(&summary.json());
        }
    }
}
