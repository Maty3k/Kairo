// SPDX-License-Identifier: GPL-3.0-or-later
//! Loadgen's log, `--log FILE` (docs/research/17-loadgen-bench/README.md §3.6, §3.9). It follows
//! `kairo_core::testbed_log`'s rules with a format of its own: UTF-8 text, the version line
//! [`LOG_HEADER`], one `h` header line, then one record per line, each written in a single
//! `write_all` so a run stopped midway leaves whole lines (a reader drops a last line without its
//! line break). Times are absolute QPC in 100 ns. The setup's events wait until the header, which
//! needs frame 0's time; a run that fails before frame 0 opens the log with the failure's time,
//! and one that fails before its header is known leaves no log.
//!
//! - `h workload=alu|bw count=<n|-> texture_mib=<n|-> seed=<0x…|-> bands=<n> size=<w>x<h>
//!   refresh_mhz=<n> format=bgra8 buffers=<n> latency=<n> present=512 sync=0 detail=1|0
//!   qpc_hz=<n> shader=<entry>:<fnv64>,… preempt=<granularity|-> hags=on|off|unknown
//!   hwcomp=<flags|none|-> rotation=<rotation|-> hdr=0|1 start_hns=<t>` (one line). `count` is
//!   `-` for `calibrate`, `texture_mib` and `seed` for `alu`.
//! - `f <n> <start_hns> <present_hns> <gpu_us|->`: frame n at the run's count, from 0, when its
//!   CPU work started, when `Present` returned, and its GPU time in whole µs (`-`: disjoint, or
//!   not resolved in time). Written in frame order once the time resolves, into one buffer reused
//!   for the whole run, so a frame allocates nothing (README §2 item 4). `calibrate` writes none.
//! - `e <name> [key=value …]`: the [`Event`]s.
//!
//! A log names no device path, monitor, adapter or PC name, window title or wall-clock time
//! (CLAUDE.md). It is a raw result of the PC: `C:\kairo\logs` and `.dogfood/` only.

use std::fmt::Write as _;
use std::io::Write;

use crate::workload::{Fit, Settled, Stage, Texture, Workload};

/// The first line of every loadgen log.
pub const LOG_HEADER: &str = "# kairo-loadgen 1";

/// The swap chain's format, in the header and the ready file: `DXGI_FORMAT_B8G8R8A8_UNORM`.
pub const FORMAT: &str = "bgra8";
/// The flags of every `Present`: `DXGI_PRESENT_ALLOW_TEARING`.
pub const PRESENT_FLAGS: u32 = 512;
/// The sync interval of every `Present`: uncapped.
pub const SYNC_INTERVAL: u32 = 0;

/// Whether hardware-accelerated GPU scheduling is on, from the D3DKMT read-out (README §3.10):
/// logging only, since Microsoft marks the WDDM 2.7 caps "reserved for system use".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hags {
    /// On.
    On,
    /// Off, or not supported.
    Off,
    /// The read-out failed.
    Unknown,
}

impl Hags {
    /// From `D3DKMT_WDDM_2_7_CAPS.Value`: bit 0 is `HwSchSupported`, bit 1 `HwSchEnabled`
    /// (d3dkmthk.h).
    pub fn from_wddm_2_7_caps(value: u32) -> Hags {
        if value & 0b10 != 0 {
            Hags::On
        } else {
            Hags::Off
        }
    }

    /// Its name in the header, the ready file and the fingerprint.
    pub fn name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Unknown => "unknown",
        }
    }
}

/// The adapter's graphics preemption granularity (`D3DKMDT_GRAPHICS_PREEMPTION_GRANULARITY`, the
/// coarsest of its engines) as the header's `preempt=`: `none`, `dma`, `primitive`, `triangle`,
/// `pixel` or `shader`; `other` for a value the header doesn't know, `-` if the read-out failed.
pub fn preempt_name(raw: Option<i32>) -> &'static str {
    match raw {
        None => "-",
        Some(0) => "none",
        Some(100) => "dma",
        Some(200) => "primitive",
        Some(300) => "triangle",
        Some(400) => "pixel",
        Some(500) => "shader",
        Some(_) => "other",
    }
}

/// `IDXGIOutput6::CheckHardwareCompositionSupport`'s flags as the header's `hwcomp=`:
/// `fullscreen`, `windowed` and `cursor_stretched`, comma-separated; `none` without any, `-` if
/// the call isn't there or failed.
pub fn hwcomp_names(flags: Option<u32>) -> String {
    let Some(flags) = flags else {
        return "-".to_owned();
    };
    let names: Vec<&str> = [(1, "fullscreen"), (2, "windowed"), (4, "cursor_stretched")]
        .into_iter()
        .filter(|&(bit, _)| flags & bit != 0)
        .map(|(_, name)| name)
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(",")
    }
}

/// The output's `DXGI_MODE_ROTATION` as the header's `rotation=`: `identity`, `90`, `180`, `270`
/// or `unspecified`; `-` if unknown.
pub fn rotation_name(raw: Option<i32>) -> &'static str {
    match raw {
        Some(0) => "unspecified",
        Some(1) => "identity",
        Some(2) => "90",
        Some(3) => "180",
        Some(4) => "270",
        _ => "-",
    }
}

/// `IDXGISwapChainMedia::GetFrameStatisticsMedia`'s `CompositionMode` as a present hint's
/// `media=`: `composed`, `overlay`, `none` or `failure`; `unavailable` if the call failed, as it
/// may on an RGB swap chain \[Unverified\]. A hint, never a verdict: PresentMon's CSV says which
/// present mode a run had (README §3.3).
pub fn media_name(raw: Option<i32>) -> &'static str {
    match raw {
        Some(0) => "composed",
        Some(1) => "overlay",
        Some(2) => "none",
        Some(3) => "failure",
        _ => "unavailable",
    }
}

/// The log's `h` line. Its `start_hns` is frame 0's, filled in when the log starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The workload.
    pub workload: Workload,
    /// The run's count; `None` for `calibrate`, whose count changes from step to step.
    pub count: Option<u32>,
    /// The bandwidth workload's texture.
    pub texture: Option<Texture>,
    /// Bands per frame.
    pub bands: u32,
    /// The swap chain's width and height.
    pub size: (u32, u32),
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// Swap-chain buffers.
    pub buffers: u32,
    /// The maximum frame latency.
    pub latency: u32,
    /// The grain.
    pub detail: bool,
    /// QPC's frequency.
    pub qpc_hz: i64,
    /// The compiled shaders' hashes (`summary::ShaderHashes::text`).
    pub shader: String,
    /// The preemption granularity's raw value, if read.
    pub preempt: Option<i32>,
    /// Hardware-accelerated GPU scheduling.
    pub hags: Hags,
    /// The output's hardware composition flags, if read.
    pub hwcomp: Option<u32>,
    /// The output's rotation, if read.
    pub rotation: Option<i32>,
    /// Whether the output is in HDR (loadgen refuses it, so a written header says 0).
    pub hdr: bool,
    /// Frame 0's start, QPC in 100 ns.
    pub start_hns: i64,
}

impl Header {
    /// The `h` line with its line break.
    pub fn line(&self) -> String {
        let dash = || "-".to_owned();
        let (width, height) = self.size;
        format!(
            "h workload={} count={} texture_mib={} seed={} bands={} size={width}x{height} \
             refresh_mhz={} format={FORMAT} buffers={} latency={} present={PRESENT_FLAGS} \
             sync={SYNC_INTERVAL} detail={} qpc_hz={} shader={} preempt={} hags={} hwcomp={} \
             rotation={} hdr={} start_hns={}\n",
            self.workload.name(),
            self.count.map_or_else(dash, |c| c.to_string()),
            self.texture.map_or_else(dash, |t| t.mib().to_string()),
            self.texture
                .map_or_else(dash, |t| format!("{:#010x}", t.seed())),
            self.bands,
            self.refresh_mhz,
            self.buffers,
            self.latency,
            u8::from(self.detail),
            self.qpc_hz,
            self.shader,
            preempt_name(self.preempt),
            self.hags.name(),
            hwcomp_names(self.hwcomp),
            rotation_name(self.rotation),
            u8::from(self.hdr),
            self.start_hns,
        )
    }
}

/// The longest `f` line, line break included, with room to spare: four 20-character numbers.
pub const FRAME_LINE_CAPACITY: usize = 96;

/// One frame at the run's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRecord {
    /// Its number, from 0 at the first frame at the full count.
    pub frame: u64,
    /// When its CPU work started, QPC in 100 ns: the time its picture is drawn for.
    pub start_hns: i64,
    /// When `Present` returned.
    pub present_hns: i64,
    /// Its GPU time in whole µs; `None` if disjoint or not resolved in time.
    pub gpu_us: Option<u64>,
}

impl FrameRecord {
    /// Writes its `f` line, line break included, into `buf` in place of what it held. Once
    /// `buf` holds [`FRAME_LINE_CAPACITY`] bytes, this never allocates.
    pub fn write_line(&self, buf: &mut String) {
        buf.clear();
        // Writing into a String can't fail.
        let _ = match self.gpu_us {
            Some(us) => writeln!(
                buf,
                "f {} {} {} {us}",
                self.frame, self.start_hns, self.present_hns
            ),
            None => writeln!(
                buf,
                "f {} {} {} -",
                self.frame, self.start_hns, self.present_hns
            ),
        };
    }
}

/// Why a run or a calibration ended (`e end reason=`, and the summary's `end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// `run` reached `--seconds`.
    Seconds,
    /// Standard input ended (`--stop-on-stdin-eof`).
    Stdin,
    /// The window was asked to close (`WM_CLOSE`).
    Close,
    /// A refusal or a failure: the exit code says which.
    Error,
    /// `calibrate` found its count.
    Done,
}

impl EndReason {
    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Stdin => "stdin",
            Self::Close => "close",
            Self::Error => "error",
            Self::Done => "done",
        }
    }
}

/// An `e` line. Every value is a number or one of a few fixed words, so no line can hold a
/// space, a path or a name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// `ramp step=<k> count=<c> gpu_us=<p50|->`: ramp step k, drawn and timed (README §3.10).
    Ramp {
        /// The step's number, from 0.
        step: u32,
        /// Its count.
        count: u32,
        /// Its median GPU time in µs.
        gpu_us: Option<u64>,
    },
    /// `activate result=ok|failed`: `--activate`'s attempt on the foreground.
    Activate {
        /// Whether the window got it.
        ok: bool,
    },
    /// `ready`: the run's count is drawn and the ready file written.
    Ready,
    /// `present_hint media=<mode> occluded=<n>`: at ready, then every 10 s.
    PresentHint {
        /// `CompositionMode`'s raw value, if the call answered.
        media: Option<i32>,
        /// Presents so far that returned `DXGI_STATUS_OCCLUDED`.
        occluded: u64,
    },
    /// `too_heavy count=<c> predicted_ms=<x>`: a guard refused a step before it was drawn.
    TooHeavy {
        /// The count refused.
        count: u32,
        /// Its predicted GPU time per frame.
        predicted_ms: f64,
    },
    /// `too_heavy count=<c> gpu_ms=<x>`: a step drawn over the guards, measured. In a run, the
    /// run-time guard, three frames in a row over 250 ms (`gpu_ms` is the third's); in
    /// `calibrate`, a step whose median passed the TDR guards.
    LongFrames {
        /// The run's count.
        count: u32,
        /// The last long frame's GPU time.
        gpu_ms: f64,
    },
    /// `display_changed`: the monitor's mode or HDR state changed, which ends the run.
    DisplayChanged,
    /// `device_lost reason=0x…`: `GetDeviceRemovedReason`'s HRESULT.
    DeviceLost {
        /// The HRESULT, as its bits.
        reason: u32,
    },
    /// `present_failed hr=0x…`: `Present` failed without losing the device.
    PresentFailed {
        /// The HRESULT, as its bits.
        hr: u32,
    },
    /// `step stage=double|warm|verify k=<k> count=<c> frames=<n> gpu_us=<p50>`: one calibration
    /// step measured.
    Step {
        /// Which measurement.
        stage: Stage,
        /// Its number within the stage.
        step: u32,
        /// Its count.
        count: u32,
        /// The frames it timed.
        frames: u32,
        /// Their median GPU time in µs.
        gpu_us: u64,
    },
    /// `hold count=<c> seconds=<s>`: the calibration's warm-up begins.
    Hold {
        /// The count held.
        count: u32,
        /// For how long.
        seconds: u32,
    },
    /// `fit a_ms=<a> b_ms=<b> r2=<r2>`: the least-squares line through the warm steps.
    Fit(Fit),
    /// `calibrated count=<c> gpu_us=<p50> settled=target|granularity`.
    Calibrated {
        /// The count found.
        count: u32,
        /// Its verified median GPU time in µs.
        gpu_us: u64,
        /// How it was accepted.
        settled: Settled,
    },
    /// `refused reason=<reason>`: a calibration that can't reach its target, or timestamps that
    /// never resolved (`workload::Refusal::reason`).
    Refused {
        /// Why.
        reason: &'static str,
    },
    /// `end reason=seconds|stdin|close|error|done`: the last line.
    End(EndReason),
}

impl Event {
    /// Its `e` line with its line break.
    pub fn line(&self) -> String {
        let us = |us: Option<u64>| us.map_or_else(|| "-".to_owned(), |us| us.to_string());
        let body = match *self {
            Self::Ramp {
                step,
                count,
                gpu_us,
            } => format!("ramp step={step} count={count} gpu_us={}", us(gpu_us)),
            Self::Activate { ok } => {
                format!("activate result={}", if ok { "ok" } else { "failed" })
            }
            Self::Ready => "ready".to_owned(),
            Self::PresentHint { media, occluded } => {
                format!(
                    "present_hint media={} occluded={occluded}",
                    media_name(media)
                )
            }
            Self::TooHeavy {
                count,
                predicted_ms,
            } => format!("too_heavy count={count} predicted_ms={predicted_ms:.1}"),
            Self::LongFrames { count, gpu_ms } => {
                format!("too_heavy count={count} gpu_ms={gpu_ms:.1}")
            }
            Self::DisplayChanged => "display_changed".to_owned(),
            Self::DeviceLost { reason } => format!("device_lost reason={reason:#010x}"),
            Self::PresentFailed { hr } => format!("present_failed hr={hr:#010x}"),
            Self::Step {
                stage,
                step,
                count,
                frames,
                gpu_us,
            } => format!(
                "step stage={} k={step} count={count} frames={frames} gpu_us={gpu_us}",
                stage.name()
            ),
            Self::Hold { count, seconds } => format!("hold count={count} seconds={seconds}"),
            Self::Fit(fit) => format!(
                "fit a_ms={:.4} b_ms={:.8} r2={:.6}",
                fit.a_ms, fit.b_ms, fit.r2
            ),
            Self::Calibrated {
                count,
                gpu_us,
                settled,
            } => format!(
                "calibrated count={count} gpu_us={gpu_us} settled={}",
                settled.name()
            ),
            Self::Refused { reason } => format!("refused reason={reason}"),
            Self::End(reason) => format!("end reason={}", reason.name()),
        };
        format!("e {body}\n")
    }
}

/// The log as it is written. `W` is the file, or any writer in tests; `None` without `--log`.
/// Lines before frame 0 wait for the header; a write that fails is dropped, never fatal.
#[derive(Debug)]
pub struct Log<W: Write> {
    out: Option<W>,
    header: Option<Header>,
    started: bool,
    waiting: Vec<String>,
    line: String,
}

impl<W: Write> Log<W> {
    /// A log that writes to `out`.
    pub fn new(out: Option<W>) -> Log<W> {
        Log {
            out,
            header: None,
            started: false,
            waiting: Vec::new(),
            line: String::with_capacity(FRAME_LINE_CAPACITY),
        }
    }

    /// Sets the header once the window, the swap chain and the read-outs are known, so the
    /// setup's events reach the log even if frame 0 never comes.
    pub fn set_header(&mut self, header: Header) {
        self.header = Some(header);
    }

    /// Whether the header is known.
    pub fn has_header(&self) -> bool {
        self.header.is_some()
    }

    /// Whether frame 0 has started the log.
    pub fn started(&self) -> bool {
        self.started
    }

    /// Writes the version line and the header with frame 0's time `start_hns`, then what waited
    /// for it. Does nothing before the header is known, or a second time.
    pub fn start(&mut self, start_hns: i64) {
        if self.started {
            return;
        }
        let Some(header) = self.header.as_mut() else {
            return;
        };
        header.start_hns = start_hns.max(0);
        let header_line = header.line();
        self.started = true;
        let waiting = std::mem::take(&mut self.waiting);
        if let Some(out) = &mut self.out {
            let _ = out.write_all(format!("{LOG_HEADER}\n").as_bytes());
            let _ = out.write_all(header_line.as_bytes());
            for line in waiting {
                let _ = out.write_all(line.as_bytes());
            }
        }
    }

    /// Writes `event`, or keeps it until the header.
    pub fn event(&mut self, event: &Event) {
        let line = event.line();
        if !self.started {
            self.waiting.push(line);
        } else if let Some(out) = &mut self.out {
            let _ = out.write_all(line.as_bytes());
        }
    }

    /// Writes a frame's `f` line through the buffer kept for the whole run.
    pub fn frame(&mut self, record: &FrameRecord) {
        record.write_line(&mut self.line);
        if !self.started {
            self.waiting.push(self.line.clone());
        } else if let Some(out) = &mut self.out {
            let _ = out.write_all(self.line.as_bytes());
        }
    }

    /// Ends the log with `e end reason=…`. If frame 0 never came, the header opens the log with
    /// `now_hns`, so the setup's events still parse. `false` if no header was ever known: then
    /// nothing was written, and the caller removes the empty file.
    pub fn end(&mut self, reason: EndReason, now_hns: i64) -> bool {
        if self.header.is_none() {
            return false;
        }
        self.event(&Event::End(reason));
        self.start(now_hns);
        if let Some(out) = &mut self.out {
            let _ = out.flush();
        }
        true
    }

    /// The writer back.
    pub fn into_inner(self) -> Option<W> {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::DEFAULT_SEED;

    fn header() -> Header {
        Header {
            workload: Workload::Alu,
            count: Some(3_072),
            texture: None,
            bands: 16,
            size: (2_560, 1_440),
            refresh_mhz: 179_960,
            buffers: 3,
            latency: 2,
            detail: true,
            qpc_hz: 10_000_000,
            shader: "vs_fullscreen:0123456789abcdef,ps_alu:fedcba9876543210".to_owned(),
            preempt: Some(400),
            hags: Hags::On,
            hwcomp: Some(3),
            rotation: Some(1),
            hdr: false,
            start_hns: 0,
        }
    }

    #[test]
    fn the_header_is_one_line_of_words() {
        let line = Header {
            start_hns: 123_450_000_000,
            ..header()
        }
        .line();
        assert_eq!(
            line,
            "h workload=alu count=3072 texture_mib=- seed=- bands=16 size=2560x1440 \
             refresh_mhz=179960 format=bgra8 buffers=3 latency=2 present=512 sync=0 detail=1 \
             qpc_hz=10000000 shader=vs_fullscreen:0123456789abcdef,ps_alu:fedcba9876543210 \
             preempt=pixel hags=on hwcomp=fullscreen,windowed rotation=identity hdr=0 \
             start_hns=123450000000\n"
        );
        // The bandwidth workload's texture, a calibration's count and read-outs that failed.
        let line = Header {
            workload: Workload::Bw,
            count: None,
            texture: Texture::new(256, DEFAULT_SEED),
            detail: false,
            preempt: None,
            hags: Hags::Unknown,
            hwcomp: None,
            rotation: None,
            ..header()
        }
        .line();
        assert!(
            line.starts_with("h workload=bw count=- texture_mib=256 seed=0x4b4c4731 bands=16 ")
        );
        assert!(line.contains(" detail=0 "), "{line}");
        assert!(line.contains(" preempt=- hags=unknown hwcomp=- rotation=- hdr=0 "));
        assert!(line.ends_with(" start_hns=0\n"));
        // Every field is key=value with no space inside, and no path can get in.
        for line in [header().line(), line] {
            let fields: Vec<&str> = line.trim_end().split(' ').collect();
            assert_eq!(fields[0], "h");
            assert_eq!(fields.len(), 22, "{line}");
            assert!(
                fields[1..].iter().all(|f| f.split('=').count() == 2),
                "{line}"
            );
            assert!(!line.contains('\\') && !line.contains('/'));
        }
    }

    #[test]
    fn read_outs_have_names() {
        assert_eq!(Hags::from_wddm_2_7_caps(0b11), Hags::On);
        assert_eq!(Hags::from_wddm_2_7_caps(0b01), Hags::Off, "supported, off");
        assert_eq!(Hags::from_wddm_2_7_caps(0), Hags::Off);
        assert_eq!(Hags::from_wddm_2_7_caps(0b110), Hags::On);
        let preempt: Vec<&str> = [None, Some(0), Some(100), Some(200), Some(300), Some(400)]
            .into_iter()
            .chain([Some(500), Some(7)])
            .map(preempt_name)
            .collect();
        assert_eq!(
            preempt,
            [
                "-",
                "none",
                "dma",
                "primitive",
                "triangle",
                "pixel",
                "shader",
                "other"
            ]
        );
        assert_eq!(hwcomp_names(None), "-");
        assert_eq!(hwcomp_names(Some(0)), "none");
        assert_eq!(hwcomp_names(Some(1)), "fullscreen");
        assert_eq!(
            hwcomp_names(Some(7)),
            "fullscreen,windowed,cursor_stretched"
        );
        assert_eq!(hwcomp_names(Some(6)), "windowed,cursor_stretched");
        let rotation: Vec<&str> = [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]
            .into_iter()
            .map(rotation_name)
            .collect();
        assert_eq!(
            rotation,
            ["-", "unspecified", "identity", "90", "180", "270", "-"]
        );
        let media: Vec<&str> = [None, Some(0), Some(1), Some(2), Some(3), Some(4)]
            .into_iter()
            .map(media_name)
            .collect();
        assert_eq!(
            media,
            [
                "unavailable",
                "composed",
                "overlay",
                "none",
                "failure",
                "unavailable"
            ]
        );
    }

    #[test]
    fn frame_lines_reuse_one_buffer() {
        let mut buf = String::with_capacity(FRAME_LINE_CAPACITY);
        let record = FrameRecord {
            frame: 0,
            start_hns: 123_450_000_000,
            present_hns: 123_450_012_345,
            gpu_us: Some(7_987),
        };
        record.write_line(&mut buf);
        assert_eq!(buf, "f 0 123450000000 123450012345 7987\n");
        FrameRecord {
            frame: 1,
            gpu_us: None,
            ..record
        }
        .write_line(&mut buf);
        assert_eq!(buf, "f 1 123450000000 123450012345 -\n");
        // The longest line fits, so a run's frames never grow the buffer.
        let capacity = buf.capacity();
        for frame in [u64::MAX, 7, 1 << 40] {
            FrameRecord {
                frame,
                start_hns: i64::MIN,
                present_hns: i64::MAX,
                gpu_us: Some(u64::MAX),
            }
            .write_line(&mut buf);
            assert!(buf.len() <= FRAME_LINE_CAPACITY, "{buf}");
            assert_eq!(buf.capacity(), capacity);
        }
    }

    #[test]
    fn events_are_one_line_each() {
        let fit = Fit {
            a_ms: 0.3,
            b_ms: 0.0025,
            r2: 0.999_95,
        };
        let cases = [
            (
                Event::Ramp {
                    step: 2,
                    count: 256,
                    gpu_us: Some(940),
                },
                "e ramp step=2 count=256 gpu_us=940\n",
            ),
            (
                Event::Ramp {
                    step: 0,
                    count: 16,
                    gpu_us: None,
                },
                "e ramp step=0 count=16 gpu_us=-\n",
            ),
            (Event::Activate { ok: true }, "e activate result=ok\n"),
            (Event::Activate { ok: false }, "e activate result=failed\n"),
            (Event::Ready, "e ready\n"),
            (
                Event::PresentHint {
                    media: None,
                    occluded: 0,
                },
                "e present_hint media=unavailable occluded=0\n",
            ),
            (
                Event::PresentHint {
                    media: Some(1),
                    occluded: 12,
                },
                "e present_hint media=overlay occluded=12\n",
            ),
            (
                Event::TooHeavy {
                    count: 4_096,
                    predicted_ms: 120.0,
                },
                "e too_heavy count=4096 predicted_ms=120.0\n",
            ),
            (
                Event::LongFrames {
                    count: 3_072,
                    gpu_ms: 312.46,
                },
                "e too_heavy count=3072 gpu_ms=312.5\n",
            ),
            (Event::DisplayChanged, "e display_changed\n"),
            (
                Event::DeviceLost {
                    reason: 0x887A_0006,
                },
                "e device_lost reason=0x887a0006\n",
            ),
            (
                Event::PresentFailed { hr: 0x887A_0001 },
                "e present_failed hr=0x887a0001\n",
            ),
            (
                Event::Step {
                    stage: Stage::Double,
                    step: 3,
                    count: 128,
                    frames: 30,
                    gpu_us: 620,
                },
                "e step stage=double k=3 count=128 frames=30 gpu_us=620\n",
            ),
            (
                Event::Hold {
                    count: 2_048,
                    seconds: 120,
                },
                "e hold count=2048 seconds=120\n",
            ),
            (
                Event::Fit(fit),
                "e fit a_ms=0.3000 b_ms=0.00250000 r2=0.999950\n",
            ),
            (
                Event::Calibrated {
                    count: 3_080,
                    gpu_us: 8_002,
                    settled: Settled::Target,
                },
                "e calibrated count=3080 gpu_us=8002 settled=target\n",
            ),
            (
                Event::Refused {
                    reason: "not_linear",
                },
                "e refused reason=not_linear\n",
            ),
            (Event::End(EndReason::Seconds), "e end reason=seconds\n"),
            (Event::End(EndReason::Done), "e end reason=done\n"),
        ];
        for (event, line) in cases {
            assert_eq!(event.line(), line);
        }
        let names: Vec<&str> = [
            EndReason::Seconds,
            EndReason::Stdin,
            EndReason::Close,
            EndReason::Error,
            EndReason::Done,
        ]
        .into_iter()
        .map(EndReason::name)
        .collect();
        assert_eq!(names, ["seconds", "stdin", "close", "error", "done"]);
    }

    fn text(log: Log<Vec<u8>>) -> String {
        String::from_utf8(log.into_inner().unwrap()).unwrap()
    }

    #[test]
    fn setup_events_wait_for_frame_0() {
        let mut log = Log::new(Some(Vec::new()));
        log.set_header(header());
        log.event(&Event::Activate { ok: true });
        log.event(&Event::Ramp {
            step: 0,
            count: 16,
            gpu_us: Some(40),
        });
        assert!(!log.started());
        log.start(5_000);
        assert!(log.started());
        log.frame(&FrameRecord {
            frame: 0,
            start_hns: 5_000,
            present_hns: 5_100,
            gpu_us: Some(8_000),
        });
        log.event(&Event::Ready);
        // A second start changes nothing.
        log.start(9_999);
        assert!(log.end(EndReason::Stdin, 6_000));
        let text = text(log);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 7, "{text}");
        assert_eq!(lines[0], LOG_HEADER);
        assert!(lines[1].starts_with("h workload=alu ") && lines[1].ends_with(" start_hns=5000"));
        assert_eq!(
            lines[2..],
            [
                "e activate result=ok",
                "e ramp step=0 count=16 gpu_us=40",
                "f 0 5000 5100 8000",
                "e ready",
                "e end reason=stdin",
            ]
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_run_that_fails_before_frame_0_still_logs_its_setup() {
        let mut log = Log::new(Some(Vec::new()));
        log.set_header(header());
        log.event(&Event::TooHeavy {
            count: 256,
            predicted_ms: 128.0,
        });
        assert!(log.end(EndReason::Error, 77));
        let written = text(log);
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines[0], LOG_HEADER);
        assert!(lines[1].ends_with(" start_hns=77"), "{written}");
        assert_eq!(
            lines[2..],
            [
                "e too_heavy count=256 predicted_ms=128.0",
                "e end reason=error"
            ]
        );
        // A negative time never reaches the header.
        let mut log = Log::new(Some(Vec::new()));
        log.set_header(header());
        log.start(-5);
        assert!(text(log).contains(" start_hns=0\n"));
    }

    #[test]
    fn a_run_without_a_header_leaves_nothing() {
        let mut log = Log::new(Some(Vec::new()));
        log.event(&Event::Activate { ok: false });
        log.start(10);
        assert!(!log.has_header() && !log.started());
        assert!(!log.end(EndReason::Error, 11));
        assert!(text(log).is_empty());
        // Without --log, nothing is written and nothing fails.
        let mut log: Log<Vec<u8>> = Log::new(None);
        log.set_header(header());
        log.start(1);
        log.frame(&FrameRecord {
            frame: 0,
            start_hns: 1,
            present_hns: 2,
            gpu_us: None,
        });
        assert!(log.end(EndReason::Seconds, 3));
        assert!(log.into_inner().is_none());
    }
}
