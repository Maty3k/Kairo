// SPDX-License-Identifier: GPL-3.0-or-later
//! One testbed run (docs/research/16-verify-testbed/README.md §4): find the monitor and its
//! adapter, open the window and the swap chain, take the foreground and exclusive fullscreen if
//! asked, then draw pattern v1 frame by frame until `--seconds`, `WM_CLOSE` or the end of
//! standard input, writing the QPC log as it goes.
//!
//! An HDR toggle during a run ends it with `e hdr_changed` and `e end reason=hdr_changed`:
//! recreating the swap chain mid-run (README §4.8) waits for M5's G2 tests, the first that
//! toggle HDR. Messages on stderr name no device path or file path (CLAUDE.md), and neither does
//! the log.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kairo_core::testbed_log::{
    Event, Miss, Mode, Pace, TESTBED_LOG_HEADER, TestbedHeader, TestbedRecord,
};
use kairo_core::testsig::{Options, PATTERN_VERSION, Patches, Scene, Size};
use kairo_core::timeline::HNS_PER_SECOND;
use kairo_display::DisplayTarget;

use crate::cli::{self, Config, Hdr, exit};
use crate::draw::{self, Format, Plan};
use crate::gpu::{self, Chain, Gpu, PresentError};
use crate::pace::{self, Period, Resolved, Shown, Step, TimerPacer, VsyncPacer};
use crate::platform::{self, Frame, KeepDisplayOn, Timer, Window};
use crate::ready::Ready;

/// How long `--activate` keeps asking for the foreground (README §4.1).
const ACTIVATE_FOR: Duration = Duration::from_secs(10);
/// How long the vsync loop warms up before it makes do without vblank timing.
const WARMUP_LIMIT_HNS: i64 = 3 * HNS_PER_SECOND;
/// How often the display's state is checked.
const CHECK_EVERY_HNS: i64 = HNS_PER_SECOND / 2;

/// Why a run stopped: an exit code and a message without paths.
struct Stop(u8, String);

/// Runs the testbed; returns the exit code.
pub fn run(config: &Config) -> u8 {
    let mut log = match Log::open(config) {
        Ok(log) => log,
        Err(Stop(code, message)) => {
            eprintln!("testbed: {message}");
            return code;
        }
    };
    let code = match go(config, &mut log) {
        Ok(()) => exit::OK,
        Err(Stop(code, message)) => {
            eprintln!("testbed: {message}");
            if log.end_reason == "seconds" {
                log.end_reason = "error";
            }
            code
        }
    };
    log.finish();
    code
}

/// The QPC log. Lines before frame 0 (setup events) wait for the header, which needs frame 0's
/// time.
struct Log {
    file: Option<File>,
    header: Option<TestbedHeader>,
    started: bool,
    waiting: Vec<String>,
    end_reason: &'static str,
}

impl Log {
    fn open(config: &Config) -> Result<Log, Stop> {
        let file = match &config.log {
            Some(path) => Some(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|e| Stop(exit::USAGE, format!("can't make the log: {}", e.kind())))?,
            ),
            None => None,
        };
        Ok(Log {
            file,
            header: None,
            started: false,
            waiting: Vec::new(),
            end_reason: "seconds",
        })
    }

    fn write(&mut self, line: Option<String>) {
        let Some(line) = line else { return };
        if !self.started {
            self.waiting.push(line);
        } else if let Some(file) = &mut self.file {
            // One write per line, so a run stopped midway leaves whole lines.
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// Writes the header with frame 0's time, then what waited for it.
    fn start(&mut self, start_hns: i64) {
        if self.started {
            return;
        }
        let Some(mut header) = self.header else {
            return;
        };
        header.start_hns = start_hns.max(0);
        self.started = true;
        if let Some(file) = &mut self.file {
            let _ = file.write_all(format!("{TESTBED_LOG_HEADER}\n").as_bytes());
            if let Some(line) = header.line() {
                let _ = file.write_all(line.as_bytes());
            }
            for line in self.waiting.drain(..) {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn event(&mut self, name: &str, fields: &[(&str, &str)]) {
        let line = Event::new(name, fields).and_then(|e| TestbedRecord::Event(e).line());
        self.write(line);
    }

    fn frame(&mut self, resolved: Resolved, miss: Miss) {
        self.write(TestbedRecord::Frame(resolved.record).line());
        if resolved.missed {
            self.missed(resolved.record.frame, miss);
        }
    }

    fn missed(&mut self, frame: u64, miss: Miss) {
        self.write(TestbedRecord::Miss(frame, miss).line());
    }

    /// The `end` event; if frame 0 never came, the header still opens the log (with the time
    /// now), so the setup events parse.
    fn finish(&mut self) {
        let reason = self.end_reason;
        self.event("end", &[("reason", reason)]);
        if !self.started {
            self.start(platform::now_hns());
        }
    }
}

/// What a run holds once it is set up.
struct Ctx<'a> {
    config: &'a Config,
    log: &'a mut Log,
    window: Window,
    gpu: Gpu,
    chain: Chain,
    target: &'a DisplayTarget,
    size: Size,
    options: Options,
    fps: u32,
    period: Period,
    refresh_mhz: u64,
    hdr: Option<(f64, f64)>,
    stdin_eof: Arc<AtomicBool>,
    next_check_hns: i64,
    fullscreen_lost: bool,
}

/// Sets the run up and runs it.
fn go(config: &Config, log: &mut Log) -> Result<(), Stop> {
    kairo_display::make_dpi_aware();
    platform::opt_out_of_power_throttling();
    if config.ready.as_ref().is_some_and(|p| p.exists()) {
        return Err(Stop(
            exit::USAGE,
            "the ready file exists already".to_owned(),
        ));
    }
    let targets = kairo_display::targets()
        .map_err(|_| Stop(exit::MONITOR, "can't list the displays".to_owned()))?;
    let target = kairo_display::find_by_path(&targets, &config.monitor).ok_or_else(|| {
        Stop(
            exit::MONITOR,
            "the monitor given isn't an active display".to_owned(),
        )
    })?;
    if target.cloned {
        return Err(Stop(
            exit::MONITOR,
            "the monitor mirrors another display".to_owned(),
        ));
    }
    let refresh = target.refresh;
    let (Some(refresh_mhz), Some(period)) = (
        refresh.millihertz(),
        Period::from_refresh(refresh.numerator, refresh.denominator),
    ) else {
        return Err(Stop(
            exit::MONITOR,
            "the monitor's refresh rate is unknown".to_owned(),
        ));
    };
    let fps = config.fps.unwrap_or_else(|| cli::default_fps(refresh_mhz));
    let k = refresh.fits_vsync(fps);
    if let Some(why) = cli::refuse_rate(config.pace, fps, k) {
        return Err(Stop(exit::MONITOR, why));
    }
    let gpu =
        Gpu::on_adapter(target.adapter_luid, target.hmonitor).map_err(|e| Stop(exit::DEVICE, e))?;
    let hdr = match config.hdr {
        Hdr::Off => None,
        Hdr::Auto => gpu.output_hdr().filter(|o| o.pq).map(|o| {
            let white = target.hdr.sdr_white_nits;
            let white = if white.is_finite() && white > 0.0 {
                white
            } else {
                80.0
            };
            (white, f64::from(o.max_nits).clamp(400.0, 4_000.0))
        }),
    };
    let format = match hdr {
        Some((white, _)) => Format::Fp16 {
            sdr_white_nits: white,
        },
        None => Format::Bgra8,
    };
    let rect = target.rect;
    let (frame, left, top, width, height) = match config.mode {
        Mode::Windowed => {
            let (w, h) = config.size;
            let left = rect.left + (rect.width() as i32 - w as i32) / 2;
            let top = rect.top + (rect.height() as i32 - h as i32) / 2;
            (Frame::Framed, left, top, w as i32, h as i32)
        }
        Mode::Borderless | Mode::Exclusive => (
            Frame::Popup,
            rect.left,
            rect.top,
            rect.width() as i32,
            rect.height() as i32,
        ),
    };
    let window =
        Window::create(frame, left, top, width, height).map_err(|e| Stop(exit::DEVICE, e))?;
    let _display = KeepDisplayOn::new();
    let stdin_eof = Arc::new(AtomicBool::new(false));
    if config.stop_on_stdin_eof {
        watch_stdin(Arc::clone(&stdin_eof));
    }
    if config.activate {
        if !wait_for_foreground_lock(&window, &stdin_eof, log) {
            return Ok(());
        }
        let ok = take_foreground(&window);
        log.event("activate", &[("result", if ok { "ok" } else { "failed" })]);
        if !ok {
            return Err(Stop(
                exit::FOCUS,
                "the window didn't get the foreground".to_owned(),
            ));
        }
    } else {
        window.show(false);
    }
    let (_, _, cw, ch) = window.client_rect();
    let size = Size::new(cw.max(0) as u32, ch.max(0) as u32)
        .map_err(|e| Stop(exit::DEVICE, format!("the client area: {e}")))?;
    let mut chain = gpu
        .swap_chain(
            window.hwnd(),
            size.width(),
            size.height(),
            format,
            config.pace == Pace::Vsync,
        )
        .map_err(|e| Stop(exit::DEVICE, e))?;
    if config.mode == Mode::Exclusive {
        let entered = chain.enter_fullscreen();
        let result = if entered.is_ok() { "ok" } else { "failed" };
        log.event("fse", &[("result", result)]);
        entered.map_err(|e| Stop(exit::FOCUS, format!("exclusive fullscreen: {e}")))?;
    }
    let run_tag = config.run_tag.unwrap_or_else(cli::random_run_tag);
    let options = Options {
        pattern: config.pattern,
        nominal_fps: fps,
        run_tag,
        load: config.load,
        patches: match hdr {
            Some((white, _)) => Patches::Hdr {
                sdr_white_nits: white,
                bright: config.hdr_patches,
            },
            None => Patches::Sdr,
        },
    };
    log.header = Some(TestbedHeader {
        pattern: PATTERN_VERSION,
        run_tag,
        mode: config.mode,
        pace: config.pace,
        fps,
        nominal_fps: fps,
        width: size.width(),
        height: size.height(),
        refresh_mhz,
        hdr: hdr.is_some(),
        sdr_white_nits: hdr.map(|h| h.0),
        max_nits: hdr.map(|h| h.1),
        start_hns: 0,
    });
    let mut ctx = Ctx {
        config,
        log,
        window,
        gpu,
        chain,
        target,
        size,
        options,
        fps,
        period,
        refresh_mhz,
        hdr,
        stdin_eof,
        next_check_hns: 0,
        fullscreen_lost: false,
    };
    match (config.pace, k) {
        (Pace::Vsync, Some(k)) => vsync_loop(&mut ctx, k),
        _ => timer_loop(&mut ctx),
    }
}

/// Reads standard input to its end, then sets `eof` (`--stop-on-stdin-eof`).
fn watch_stdin(eof: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0_u8; 256];
        while let Ok(n) = stdin.read(&mut buffer) {
            if n == 0 {
                break;
            }
        }
        eof.store(true, Ordering::Relaxed);
    });
}

/// Waits until the session's last input is older than the foreground lock time-out plus 5 s
/// (README §4.1). `false` if the run was closed or stopped meanwhile.
fn wait_for_foreground_lock(window: &Window, stdin_eof: &AtomicBool, log: &mut Log) -> bool {
    loop {
        window.pump();
        if platform::close_requested() {
            log.end_reason = "close";
            return false;
        }
        if stdin_eof.load(Ordering::Relaxed) {
            log.end_reason = "stdin";
            return false;
        }
        let need = u64::from(platform::foreground_lock_timeout_ms()) + 5_000;
        if u64::from(platform::idle_ms()) >= need {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Asks for the foreground for up to [`ACTIVATE_FOR`]; never with input.
fn take_foreground(window: &Window) -> bool {
    let until = std::time::Instant::now() + ACTIVATE_FOR;
    loop {
        if window.take_foreground() {
            return true;
        }
        window.pump();
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl Ctx<'_> {
    /// Handles messages; the reason to stop, if there is one.
    fn stop_requested(&mut self) -> Option<&'static str> {
        self.window.pump();
        if platform::close_requested() {
            Some("close")
        } else if self.stdin_eof.load(Ordering::Relaxed) {
            Some("stdin")
        } else {
            None
        }
    }

    /// Every half second: has exclusive fullscreen been lost, or HDR toggled? The latter ends
    /// the run.
    fn check_display(&mut self, now: i64) -> Option<&'static str> {
        if now < self.next_check_hns {
            return None;
        }
        self.next_check_hns = now + CHECK_EVERY_HNS;
        if self.config.mode == Mode::Exclusive
            && !self.fullscreen_lost
            && !self.chain.is_fullscreen()
        {
            self.fullscreen_lost = true;
            self.log.event("fullscreen_lost", &[]);
        }
        if self.config.hdr == Hdr::Auto && !self.gpu.is_current() {
            let pq = gpu::output_hdr_now(self.target.adapter_luid, self.target.hmonitor)
                .is_some_and(|o| o.pq);
            if pq != self.hdr.is_some() {
                self.log
                    .event("hdr_changed", &[("hdr", if pq { "1" } else { "0" })]);
                return Some("hdr_changed");
            }
        }
        None
    }

    /// Frame `frame`'s plan.
    fn plan(&self, frame: u64) -> Result<Plan, Stop> {
        let scene = Scene::new(self.size, frame, &self.options)
            .map_err(|e| Stop(exit::DEVICE, format!("frame {frame}: {e}")))?;
        draw::plan(&scene, self.chain.format())
            .map_err(|e| Stop(exit::DEVICE, format!("frame {frame}: {e}")))
    }

    /// Presents; a lost device ends the run (`e device_lost`, exit 4).
    fn present(&mut self, interval: u32) -> Result<(), Stop> {
        match self.chain.present(interval) {
            Ok(()) => Ok(()),
            Err(PresentError::DeviceLost(why)) => {
                self.log.event("device_lost", &[]);
                Err(Stop(exit::DEVICE, format!("the device was lost: {why}")))
            }
            Err(PresentError::Other(why)) => Err(Stop(exit::DEVICE, why)),
        }
    }

    /// Frame 0 is on its way: the log's header, the ready file and `e ready`.
    fn started(&mut self, start_hns: i64) -> Result<(), Stop> {
        self.log.start(start_hns);
        if let Some(path) = &self.config.ready {
            let ready = Ready {
                pid: std::process::id(),
                hwnd: self.window.hwnd(),
                client: self.window.client_rect(),
                refresh_mhz: self.refresh_mhz,
                adapter_luid: self.target.adapter_luid.to_string(),
                adapter_index: self.gpu.adapter_index,
                output_index: self.gpu.output_index,
                format: match self.chain.format() {
                    Format::Bgra8 => "bgra8",
                    Format::Fp16 { .. } => "fp16",
                },
                mode: match self.config.mode {
                    Mode::Borderless => "borderless",
                    Mode::Windowed => "windowed",
                    Mode::Exclusive => "exclusive",
                },
                fps: self.fps,
                start_hns,
            };
            ready.write(path).map_err(|e| {
                Stop(
                    exit::USAGE,
                    format!("can't write the ready file: {}", e.kind()),
                )
            })?;
        }
        self.log.event("ready", &[]);
        Ok(())
    }

    fn end_hns(&self, start_hns: i64) -> i64 {
        start_hns.saturating_add(i64::from(self.config.seconds).saturating_mul(HNS_PER_SECOND))
    }
}

/// Vsync pacing (README §4.3): wait for the swap chain, read its statistics, draw, present.
fn vsync_loop(ctx: &mut Ctx<'_>, k: u32) -> Result<(), Stop> {
    let loop_start = platform::now_hns();
    let mut pacer = VsyncPacer::new(k, ctx.period, pace::next_whole_second(loop_start));
    let black = Plan::black();
    let mut dxgi_stats = false;
    let mut timing_missing = false;
    let mut end = None;
    loop {
        if let Some(reason) = ctx.stop_requested() {
            ctx.log.end_reason = reason;
            break;
        }
        let now = platform::now_hns();
        if end.is_some_and(|end| now >= end) {
            break;
        }
        if let Some(reason) = ctx.check_display(now) {
            ctx.log.end_reason = reason;
            break;
        }
        if !ctx.chain.wait(1_000) {
            continue;
        }
        match ctx.chain.stats() {
            Some(stats) => {
                dxgi_stats = true;
                for r in pacer.on_stats(stats) {
                    ctx.log.frame(r, Miss::Vblank);
                }
            }
            None if !dxgi_stats => match platform::dwm_vblank() {
                Some((count, t)) => pacer.on_dwm_vblank(count, t),
                None if !timing_missing && now - loop_start > WARMUP_LIMIT_HNS => {
                    // Neither DXGI nor DWM says when vblanks happen: a grid from now.
                    timing_missing = true;
                    ctx.log.event("no_vblank_timing", &[]);
                    pacer.on_dwm_vblank(0, now);
                }
                None => {}
            },
            None => {}
        }
        match pacer.next(platform::now_hns()) {
            Step::Warmup => {
                ctx.chain.draw(&black);
                ctx.present(1)?;
            }
            Step::Frame {
                frame,
                target_hns,
                skipped,
            } => {
                for s in skipped {
                    ctx.log.missed(s, Miss::Vblank);
                }
                let plan = ctx.plan(frame)?;
                ctx.chain.draw(&plan);
                ctx.present(if frame == 0 { 1 } else { k })?;
                let present_hns = platform::now_hns();
                let id = ctx.chain.present_count().unwrap_or(0);
                pacer.presented(frame, target_hns, id, present_hns);
                if frame == 0 {
                    ctx.started(target_hns)?;
                    end = Some(ctx.end_hns(target_hns));
                }
            }
        }
    }
    // What the statistics can still say about the last frames.
    ctx.chain.wait(200);
    if let Some(stats) = ctx.chain.stats() {
        for r in pacer.on_stats(stats) {
            ctx.log.frame(r, Miss::Vblank);
        }
    }
    for r in pacer.finish() {
        ctx.log.frame(r, Miss::Vblank);
    }
    Ok(())
}

/// Timer pacing (README §4.3): draw, wait for the slot, `Present(0, 0)`.
fn timer_loop(ctx: &mut Ctx<'_>) -> Result<(), Stop> {
    let timer = Timer::new();
    let start = pace::next_whole_second(platform::now_hns());
    let end = ctx.end_hns(start);
    let mut pacer = TimerPacer::new(start, ctx.fps);
    let mut shown = Shown::new(None, ctx.period);
    let mut started = false;
    loop {
        if let Some(reason) = ctx.stop_requested() {
            ctx.log.end_reason = reason;
            break;
        }
        let now = platform::now_hns();
        if let Some(reason) = ctx.check_display(now) {
            ctx.log.end_reason = reason;
            break;
        }
        if let Some(stats) = ctx.chain.stats() {
            for r in shown.on_stats(stats) {
                ctx.log.frame(r, Miss::Slot);
            }
        }
        let Step::Frame {
            frame,
            target_hns,
            skipped,
        } = pacer.next(now)
        else {
            continue;
        };
        if target_hns >= end {
            break;
        }
        for s in skipped {
            ctx.log.missed(s, Miss::Slot);
        }
        let plan = ctx.plan(frame)?;
        ctx.chain.draw(&plan);
        match &timer {
            Some(timer) => timer.wait_until(target_hns),
            None => {
                let left = target_hns - platform::now_hns();
                if left > 0 {
                    std::thread::sleep(Duration::from_nanos(left as u64 * 100));
                }
            }
        }
        ctx.present(0)?;
        let present_hns = platform::now_hns();
        let id = ctx.chain.present_count().unwrap_or(0);
        let missed = pacer.missed(frame, present_hns);
        shown.presented(frame, target_hns, id, present_hns, missed);
        if !started {
            started = true;
            ctx.started(target_hns)?;
        }
    }
    std::thread::sleep(Duration::from_millis(100));
    if let Some(stats) = ctx.chain.stats() {
        for r in shown.on_stats(stats) {
            ctx.log.frame(r, Miss::Slot);
        }
    }
    for r in shown.finish() {
        ctx.log.frame(r, Miss::Slot);
    }
    Ok(())
}
