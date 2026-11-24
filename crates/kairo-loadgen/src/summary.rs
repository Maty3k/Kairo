// SPDX-License-Identifier: GPL-3.0-or-later
//! What loadgen reports besides its log (docs/research/17-loadgen-bench/README.md §3.6, §3.8,
//! §3.9): the ready file (`{"kairo_loadgen_ready":1,…}`), the summary line on stdout at the end of
//! a run (`{"kairo_loadgen":1,…}`), the calibration line (`{"kairo_loadgen_calibration":1,…}`),
//! the hashes behind them, and the GPU times a run counts for its summary.
//!
//! - **Shader hashes.** FNV-1a 64, the hash M1's corpus manifests use, of each entry point's
//!   compiled bytecode ([`ShaderHashes`]): a Windows update that changed `d3dcompiler_47.dll`'s
//!   output shows in the log header, and `--detail off` (which compiles with `KAIRO_DETAIL=0`)
//!   hashes differently.
//! - **The fingerprint** ([`Conditions`]): what a calibrated count holds for. The bench refuses a
//!   session whose first ready file's fingerprint isn't the calibration's ("calibrate again").
//!   The adapter's LUID isn't in it, since Windows makes new LUIDs at every restart.
//! - **GPU times** ([`GpuTimes`], [`RunStats`]): counted as frames resolve, into a histogram
//!   allocated once, so counting a frame allocates nothing (README §2 item 4). Percentiles are
//!   nearest-rank, in integers.
//!
//! The ready file and the calibration line name this run's window and adapter, or the PC's driver
//! and screen: raw results of the PC, kept in `C:\kairo\logs` and `.dogfood/` (CLAUDE.md). None of
//! them names a device path, a monitor or a file. Loadgen's own `fps` is informational:
//! PresentMon's CSV is the measurement.

use std::io;
use std::path::Path;

use serde_json::{Value, json};

use crate::log::{EndReason, FORMAT, Hags, PRESENT_FLAGS, SYNC_INTERVAL};
use crate::workload::{BYTES_PER_READ, Calibrated, Workload};

/// The swap effect, in the ready file.
pub const SWAP_EFFECT: &str = "flip_discard";

/// FNV-1a 64 of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A hash as the JSON lines write it: `fnv64:` and 16 lower-case hex digits.
pub fn fnv_text(hash: u64) -> String {
    format!("fnv64:{hash:016x}")
}

/// The FNV-1a 64 hashes of a run's compiled shaders, one per entry point of its workload, in
/// `Workload::entries`' order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderHashes {
    hashes: Vec<(&'static str, u64)>,
}

impl ShaderHashes {
    /// The hashes of `bytecode`, one blob per entry of `workload.entries()`, in that order;
    /// `None` if the numbers differ.
    pub fn new(workload: Workload, bytecode: &[&[u8]]) -> Option<ShaderHashes> {
        let entries = workload.entries();
        (entries.len() == bytecode.len()).then(|| ShaderHashes {
            hashes: entries
                .iter()
                .zip(bytecode)
                .map(|(entry, code)| (entry.name, fnv1a64(code)))
                .collect(),
        })
    }

    /// The log header's `shader=`: `<entry>:<16 hex digits>` per entry, comma-separated.
    pub fn text(&self) -> String {
        let parts: Vec<String> = self
            .hashes
            .iter()
            .map(|(name, hash)| format!("{name}:{hash:016x}"))
            .collect();
        parts.join(",")
    }

    /// One hash for them all, the calibration line's `shader`: FNV-1a 64 of
    /// [`ShaderHashes::text`], so it can be checked against a log's header.
    pub fn digest(&self) -> u64 {
        fnv1a64(self.text().as_bytes())
    }
}

/// What a calibrated count holds for (README §3.6): calibrate again when the shaders, the size,
/// the refresh rate, the driver, HAGS or the texture change. Its [`Conditions::fingerprint`] goes
/// into the calibration line and every ready file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conditions<'a> {
    /// The workload.
    pub workload: Workload,
    /// Its compiled shaders.
    pub shader: &'a ShaderHashes,
    /// The swap chain's width and height.
    pub size: (u32, u32),
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// The adapter's user-mode driver version ([`umd_version_text`]), if read.
    pub umd_version: Option<&'a str>,
    /// Hardware-accelerated GPU scheduling.
    pub hags: Hags,
    /// The bandwidth workload's texture size in MiB.
    pub texture_mib: Option<u32>,
}

impl Conditions<'_> {
    /// The text the fingerprint hashes, one `key=value` line each after a version line.
    pub fn text(&self) -> String {
        let (width, height) = self.size;
        format!(
            "kairo-loadgen fingerprint 1\nworkload={}\nshader={}\nsize={width}x{height}\n\
             refresh_mhz={}\numd_version={}\nhags={}\ntexture_mib={}\n",
            self.workload.name(),
            self.shader.text(),
            self.refresh_mhz,
            self.umd_version.unwrap_or("-"),
            self.hags.name(),
            self.texture_mib
                .map_or_else(|| "-".to_owned(), |mib| mib.to_string()),
        )
    }

    /// FNV-1a 64 of [`Conditions::text`].
    pub fn fingerprint(&self) -> u64 {
        fnv1a64(self.text().as_bytes())
    }
}

/// The user-mode driver version `IDXGIAdapter::CheckInterfaceSupport` gives as one 64-bit
/// number, as `a.b.c.d` (16 bits each, high to low), the way Windows shows driver versions.
pub fn umd_version_text(raw: i64) -> String {
    let part = |shift: u32| (raw >> shift) & 0xFFFF;
    format!("{}.{}.{}.{}", part(48), part(32), part(16), part(0))
}

/// One frame's GPU time, from its query set (README §3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuTime {
    /// Whole µs between the frame's two timestamps.
    Us(u64),
    /// The disjoint query said the timestamps can't be trusted (`gpu_disjoint`).
    Disjoint,
    /// The set was still pending when its slot came round again (`gpu_unknown`).
    Unknown,
}

impl GpuTime {
    /// From a resolved query set: `(end − begin) × 10⁶ ÷ frequency`, rounded to whole µs, only
    /// when `disjoint` is false. Timestamps that run backwards, or a frequency of 0, can't be
    /// trusted either.
    pub fn from_ticks(begin: u64, end: u64, frequency: u64, disjoint: bool) -> GpuTime {
        if disjoint || frequency == 0 || end < begin {
            return GpuTime::Disjoint;
        }
        let ticks = u128::from(end - begin);
        let frequency = u128::from(frequency);
        let us = (ticks * 1_000_000 + frequency / 2) / frequency;
        GpuTime::Us(u64::try_from(us).unwrap_or(u64::MAX))
    }

    /// The `f` line's value: the µs, or `None` for `-`.
    pub fn us(self) -> Option<u64> {
        match self {
            Self::Us(us) => Some(us),
            Self::Disjoint | Self::Unknown => None,
        }
    }
}

/// GPU times in whole µs, counted into [`GpuTimes::BINS`] bins of 1 µs (512 KiB, allocated once),
/// so counting one never allocates. Percentiles are nearest-rank and exact below
/// [`GpuTimes::BINS`] µs, past the 100 ms frame guard; one that falls above reads as the longest
/// time counted, an upper bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTimes {
    bins: Vec<u32>,
    count: u64,
    max_us: u64,
}

impl Default for GpuTimes {
    fn default() -> GpuTimes {
        GpuTimes::new()
    }
}

impl GpuTimes {
    /// The bins: 0 to 131,071 µs.
    pub const BINS: usize = 1 << 17;

    /// No times yet.
    pub fn new() -> GpuTimes {
        GpuTimes {
            bins: vec![0; GpuTimes::BINS],
            count: 0,
            max_us: 0,
        }
    }

    /// Forgets every time, keeping the bins: a ramp or calibration step starts afresh.
    pub fn clear(&mut self) {
        self.bins.fill(0);
        self.count = 0;
        self.max_us = 0;
    }

    /// Counts one time.
    pub fn add(&mut self, us: u64) {
        if let Some(bin) = usize::try_from(us).ok().and_then(|i| self.bins.get_mut(i)) {
            *bin = bin.saturating_add(1);
        }
        self.count += 1;
        self.max_us = self.max_us.max(us);
    }

    /// How many times were counted.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The longest.
    pub fn max_us(&self) -> Option<u64> {
        (self.count > 0).then_some(self.max_us)
    }

    /// The nearest-rank `percent`th percentile: the smallest time at least `percent`% of the
    /// times are no longer than (rank ⌈percent · n ÷ 100⌉, at least 1).
    pub fn percentile_us(&self, percent: u32) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = (u64::from(percent.min(100)) * self.count)
            .div_ceil(100)
            .max(1);
        let mut seen = 0_u64;
        for (us, &n) in self.bins.iter().enumerate() {
            seen += u64::from(n);
            if seen >= rank {
                return Some(us as u64);
            }
        }
        Some(self.max_us)
    }

    /// The median (nearest-rank, the lower middle of an even count): a ramp or calibration
    /// step's time.
    pub fn median_us(&self) -> Option<u64> {
        self.percentile_us(50)
    }
}

/// A run's frames, counted as they come, for its summary line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunStats {
    times: GpuTimes,
    frames: u64,
    unknown: u64,
    disjoint: u64,
    occluded: u64,
}

impl RunStats {
    /// No frames yet.
    pub fn new() -> RunStats {
        RunStats::default()
    }

    /// One frame presented at the run's count; `occluded` if `Present` returned
    /// `DXGI_STATUS_OCCLUDED`, which loadgen counts and draws on through.
    pub fn presented(&mut self, occluded: bool) {
        self.frames += 1;
        self.occluded += u64::from(occluded);
    }

    /// A frame's GPU time, once its query set resolved or its slot came round.
    pub fn gpu(&mut self, time: GpuTime) {
        match time {
            GpuTime::Us(us) => self.times.add(us),
            GpuTime::Disjoint => self.disjoint += 1,
            GpuTime::Unknown => self.unknown += 1,
        }
    }

    /// Frames presented.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Presents that returned `DXGI_STATUS_OCCLUDED` (`e present_hint occluded=`).
    pub fn occluded(&self) -> u64 {
        self.occluded
    }

    /// The GPU times counted.
    pub fn times(&self) -> &GpuTimes {
        &self.times
    }

    /// The summary of a run of `workload` at `count` that lasted `seconds` from frame 0 and
    /// ended for `end`.
    pub fn summary(&self, workload: Workload, count: u32, seconds: f64, end: EndReason) -> Summary {
        Summary {
            workload,
            count,
            frames: self.frames,
            seconds,
            gpu_p50_us: self.times.percentile_us(50),
            gpu_p99_us: self.times.percentile_us(99),
            gpu_max_us: self.times.max_us(),
            gpu_unknown: self.unknown,
            gpu_disjoint: self.disjoint,
            occluded: self.occluded,
            end,
        }
    }
}

/// `x` rounded to `decimals` places, for JSON that reads well.
fn round(x: f64, decimals: i32) -> f64 {
    let scale = 10_f64.powi(decimals);
    (x * scale).round() / scale
}

/// Whole µs as ms, or JSON's null.
fn ms(us: Option<u64>) -> Value {
    us.map_or(Value::Null, |us| json!(us as f64 / 1_000.0))
}

/// The line `run` prints on stdout at its end, once it was ready.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The workload.
    pub workload: Workload,
    /// Its count.
    pub count: u32,
    /// Frames presented at the count.
    pub frames: u64,
    /// From frame 0's start to the end, in seconds.
    pub seconds: f64,
    /// The median GPU time, µs.
    pub gpu_p50_us: Option<u64>,
    /// The 99th percentile.
    pub gpu_p99_us: Option<u64>,
    /// The longest.
    pub gpu_max_us: Option<u64>,
    /// Frames whose query set was still pending when its slot came round.
    pub gpu_unknown: u64,
    /// Frames whose timestamps were disjoint.
    pub gpu_disjoint: u64,
    /// Presents that returned `DXGI_STATUS_OCCLUDED`.
    pub occluded: u64,
    /// Why the run ended.
    pub end: EndReason,
}

impl Summary {
    /// Frames per second, informational; `None` for a run of no length.
    pub fn fps(&self) -> Option<f64> {
        (self.seconds > 0.0).then(|| self.frames as f64 / self.seconds)
    }

    /// The line: one JSON object and a line break.
    pub fn json(&self) -> String {
        json!({
            "kairo_loadgen": 1,
            "workload": self.workload.name(),
            "count": self.count,
            "frames": self.frames,
            "seconds": round(self.seconds, 3),
            "fps": self.fps().map(|fps| round(fps, 2)),
            "gpu_ms_p50": ms(self.gpu_p50_us),
            "gpu_ms_p99": ms(self.gpu_p99_us),
            "gpu_ms_max": ms(self.gpu_max_us),
            "gpu_unknown": self.gpu_unknown,
            "gpu_disjoint": self.gpu_disjoint,
            "occluded": self.occluded,
            "end": self.end.name(),
        })
        .to_string()
            + "\n"
    }
}

/// The line `calibrate` prints on stdout, and writes create-new with `--out`.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationLine {
    /// The workload.
    pub workload: Workload,
    /// `--target-ms`.
    pub target_ms: f64,
    /// What the calibration found.
    pub calibrated: Calibrated,
    /// Bands per frame.
    pub bands: u32,
    /// The swap chain's width and height.
    pub size: (u32, u32),
    /// The bandwidth workload's texture size in MiB.
    pub texture_mib: Option<u32>,
    /// [`ShaderHashes::digest`].
    pub shader_digest: u64,
    /// [`Conditions::fingerprint`].
    pub fingerprint: u64,
}

impl CalibrationLine {
    /// The bandwidth workload's achieved rate in GB/s: W × H × `count` × 64 B ÷ the GPU time,
    /// taking one 64-byte line per random load \[Estimate\]; `None` for `alu`.
    pub fn gbps(&self) -> Option<f64> {
        if self.workload != Workload::Bw || self.calibrated.gpu_ms <= 0.0 {
            return None;
        }
        let (width, height) = self.size;
        let bytes = f64::from(width)
            * f64::from(height)
            * f64::from(self.calibrated.count)
            * BYTES_PER_READ as f64;
        Some(bytes / (self.calibrated.gpu_ms / 1_000.0) / 1e9)
    }

    /// The line: one JSON object and a line break.
    pub fn json(&self) -> String {
        let (width, height) = self.size;
        let fit = self.calibrated.fit;
        json!({
            "kairo_loadgen_calibration": 1,
            "workload": self.workload.name(),
            "target_ms": self.target_ms,
            "count": self.calibrated.count,
            "gpu_ms_p50": round(self.calibrated.gpu_ms, 3),
            "fit": {
                "a_ms": round(fit.a_ms, 4),
                "b_ms": round(fit.b_ms, 8),
                "r2": round(fit.r2, 6),
            },
            "settled": self.calibrated.settled.name(),
            "bands": self.bands,
            "size": format!("{width}x{height}"),
            "texture_mib": self.texture_mib,
            "shader": fnv_text(self.shader_digest),
            "fingerprint": fnv_text(self.fingerprint),
            "gbps": self.gbps().map(|gbps| round(gbps, 1)),
        })
        .to_string()
            + "\n"
    }

    /// Writes the line to `path` whole and only if nothing is there yet, as the ready file is
    /// written (`kairo_window::ready::write_new`); `pid` is this process's id.
    pub fn write(&self, path: &Path, pid: u32) -> io::Result<()> {
        kairo_window::ready::write_new(path, pid, &self.json())
    }
}

/// `ready.json` (README §3.9): written once, create-new, when the ramp has reached the run's
/// count and that frame was presented. PresentMon starts after it, so it never records the ramp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    /// Loadgen's process id: PresentMon's `--process_id`.
    pub pid: u32,
    /// The window handle, as a number.
    pub hwnd: isize,
    /// The client area in desktop pixels: left, top, width, height.
    pub client: (i32, i32, i32, i32),
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// The adapter's LUID, as `0x` and 16 hex digits.
    pub adapter_luid: String,
    /// The adapter's DXGI index, valid in this process only.
    pub adapter_index: u32,
    /// The monitor's DXGI output index on it, valid in this process only.
    pub output_index: Option<u32>,
    /// The user-mode driver version ([`umd_version_text`]), if read.
    pub umd_version: Option<String>,
    /// Whether the output is in HDR (loadgen refuses HDR, so a written file says false).
    pub hdr: bool,
    /// Swap-chain buffers.
    pub buffers: u32,
    /// The maximum frame latency.
    pub latency: u32,
    /// Whether `--activate` took the foreground.
    pub activated: bool,
    /// The workload.
    pub workload: Workload,
    /// Its count.
    pub count: u32,
    /// Bands per frame.
    pub bands: u32,
    /// The grain.
    pub detail: bool,
    /// QPC's frequency.
    pub qpc_hz: i64,
    /// Hardware-accelerated GPU scheduling.
    pub hags: Hags,
    /// [`Conditions::fingerprint`].
    pub fingerprint: u64,
    /// Frame 0's start, QPC in 100 ns.
    pub start_hns: i64,
    /// `--seconds`: the run ends that long after `start_hns`, unless stopped first.
    pub seconds: u32,
}

impl Ready {
    /// The file's text: one JSON object and a line break.
    pub fn json(&self) -> String {
        let (left, top, width, height) = self.client;
        json!({
            "kairo_loadgen_ready": 1,
            "pid": self.pid,
            "hwnd": self.hwnd,
            "client": {"left": left, "top": top, "width": width, "height": height},
            "refresh_mhz": self.refresh_mhz,
            "adapter_luid": self.adapter_luid,
            "dxgi_adapter_index": self.adapter_index,
            "dxgi_output_index": self.output_index,
            "umd_version": self.umd_version,
            "format": FORMAT,
            "hdr": self.hdr,
            "swap_effect": SWAP_EFFECT,
            "buffers": self.buffers,
            "latency": self.latency,
            "tearing": true,
            "present_flags": PRESENT_FLAGS,
            "sync_interval": SYNC_INTERVAL,
            "activated": self.activated,
            "workload": self.workload.name(),
            "count": self.count,
            "bands": self.bands,
            "detail": self.detail,
            "qpc_hz": self.qpc_hz,
            "hags": self.hags.name(),
            "fingerprint": fnv_text(self.fingerprint),
            "start_hns": self.start_hns,
            "seconds": self.seconds,
        })
        .to_string()
            + "\n"
    }

    /// Writes the file whole, and only if it doesn't exist (`kairo_window::ready::write_new`).
    pub fn write(&self, path: &Path) -> io::Result<()> {
        kairo_window::ready::write_new(path, self.pid, &self.json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::{Fit, Settled};
    use std::fs;

    #[test]
    fn fnv1a64_is_the_corpus_manifests_hash() {
        // FNV-1a 64's own test vectors, as kairo-verify's corpus tests have them.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv_text(0xaf63_dc4c_8601_ec8c), "fnv64:af63dc4c8601ec8c");
        assert_eq!(fnv_text(1), "fnv64:0000000000000001");
    }

    #[test]
    fn shader_hashes_follow_the_workloads_entries() {
        let alu = ShaderHashes::new(Workload::Alu, &[b"a", b"foobar"]).unwrap();
        assert_eq!(
            alu.text(),
            "vs_fullscreen:af63dc4c8601ec8c,ps_alu:85944171f73967e8"
        );
        assert_eq!(alu.digest(), fnv1a64(alu.text().as_bytes()));
        let bw = ShaderHashes::new(Workload::Bw, &[b"a", b"foobar", b""]).unwrap();
        assert_eq!(
            bw.text(),
            "vs_fullscreen:af63dc4c8601ec8c,ps_bw:85944171f73967e8,ps_fill:cbf29ce484222325"
        );
        assert_ne!(alu.digest(), bw.digest());
        // One blob per entry, no more, no fewer.
        assert_eq!(ShaderHashes::new(Workload::Alu, &[b"a"]), None);
        assert_eq!(ShaderHashes::new(Workload::Bw, &[b"a", b"b"]), None);
    }

    #[test]
    fn the_fingerprint_changes_with_what_the_count_depends_on() {
        let shader = ShaderHashes::new(Workload::Bw, &[b"a", b"foobar", b""]).unwrap();
        let base = Conditions {
            workload: Workload::Bw,
            shader: &shader,
            size: (2_560, 1_440),
            refresh_mhz: 179_960,
            umd_version: Some("32.0.21025.1006"),
            hags: Hags::On,
            texture_mib: Some(256),
        };
        assert_eq!(
            base.text(),
            "kairo-loadgen fingerprint 1\nworkload=bw\nshader=vs_fullscreen:af63dc4c8601ec8c,\
             ps_bw:85944171f73967e8,ps_fill:cbf29ce484222325\nsize=2560x1440\nrefresh_mhz=179960\n\
             umd_version=32.0.21025.1006\nhags=on\ntexture_mib=256\n"
        );
        assert_eq!(base.fingerprint(), fnv1a64(base.text().as_bytes()));
        let other_shader = ShaderHashes::new(Workload::Bw, &[b"b", b"foobar", b""]).unwrap();
        let changed = [
            Conditions {
                shader: &other_shader,
                ..base
            },
            Conditions {
                size: (1_920, 1_080),
                ..base
            },
            Conditions {
                refresh_mhz: 143_999,
                ..base
            },
            Conditions {
                umd_version: Some("32.0.21025.1007"),
                ..base
            },
            Conditions {
                umd_version: None,
                ..base
            },
            Conditions {
                hags: Hags::Off,
                ..base
            },
            Conditions {
                texture_mib: Some(512),
                ..base
            },
        ];
        for conditions in changed {
            assert_ne!(
                conditions.fingerprint(),
                base.fingerprint(),
                "{conditions:?}"
            );
        }
        let alu_shader = ShaderHashes::new(Workload::Alu, &[b"a", b"foobar"]).unwrap();
        let alu = Conditions {
            workload: Workload::Alu,
            shader: &alu_shader,
            texture_mib: None,
            umd_version: None,
            hags: Hags::Unknown,
            ..base
        };
        assert!(
            alu.text()
                .ends_with("umd_version=-\nhags=unknown\ntexture_mib=-\n")
        );
        // The same conditions give the same fingerprint, run after run.
        assert_eq!(alu.fingerprint(), Conditions { ..alu }.fingerprint());
    }

    #[test]
    fn umd_versions_read_like_windows_shows_them() {
        assert_eq!(umd_version_text(0x0020_0000_5221_03EE), "32.0.21025.1006");
        assert_eq!(umd_version_text(0), "0.0.0.0");
        assert_eq!(umd_version_text(-1), "65535.65535.65535.65535");
    }

    #[test]
    fn gpu_times_come_from_resolved_timestamps() {
        // A 1 GHz timestamp: 8 ms is 8,000,000 ticks.
        assert_eq!(
            GpuTime::from_ticks(1_000, 8_001_000, 1_000_000_000, false),
            GpuTime::Us(8_000)
        );
        // Rounded to the nearest µs.
        assert_eq!(
            GpuTime::from_ticks(0, 1_499, 1_000_000_000, false),
            GpuTime::Us(1)
        );
        assert_eq!(
            GpuTime::from_ticks(0, 1_500, 1_000_000_000, false),
            GpuTime::Us(2)
        );
        assert_eq!(GpuTime::from_ticks(0, 3, 3_000_000, false), GpuTime::Us(1));
        // Never trusted: disjoint, backwards, or no frequency.
        assert_eq!(GpuTime::from_ticks(0, 10, 1_000, true), GpuTime::Disjoint);
        assert_eq!(GpuTime::from_ticks(10, 0, 1_000, false), GpuTime::Disjoint);
        assert_eq!(GpuTime::from_ticks(0, 10, 0, false), GpuTime::Disjoint);
        // Far apart, still no overflow.
        assert_eq!(
            GpuTime::from_ticks(0, u64::MAX, 1, false),
            GpuTime::Us(u64::MAX)
        );
        assert_eq!(GpuTime::Us(7).us(), Some(7));
        assert_eq!(GpuTime::Disjoint.us(), None);
        assert_eq!(GpuTime::Unknown.us(), None);
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let mut times = GpuTimes::new();
        assert_eq!(times.percentile_us(50), None);
        assert_eq!(times.max_us(), None);
        for us in (1..=100).rev() {
            times.add(us);
        }
        assert_eq!(times.count(), 100);
        assert_eq!(times.percentile_us(50), Some(50));
        assert_eq!(times.percentile_us(99), Some(99));
        assert_eq!(times.percentile_us(100), Some(100));
        assert_eq!(times.percentile_us(0), Some(1), "rank 1 at least");
        assert_eq!(times.max_us(), Some(100));
        // Ten frames: the median is the 5th, the 99th percentile the 10th.
        times.clear();
        assert_eq!(times.count(), 0);
        for us in [
            8_010, 7_990, 8_000, 8_005, 7_995, 8_020, 7_980, 8_000, 8_001, 9_000,
        ] {
            times.add(us);
        }
        assert_eq!(times.median_us(), Some(8_000));
        assert_eq!(times.percentile_us(99), Some(9_000));
        // One frame.
        times.clear();
        times.add(42);
        assert_eq!(
            (times.median_us(), times.percentile_us(99)),
            (Some(42), Some(42))
        );
        // Past the bins, a percentile reads as the longest time: an upper bound.
        times.clear();
        times.add(8_000);
        times.add(500_000);
        times.add(400_000);
        assert_eq!(times.percentile_us(33), Some(8_000));
        assert_eq!(times.percentile_us(50), Some(500_000));
        assert_eq!(times.max_us(), Some(500_000));
        assert!(GpuTimes::BINS as u64 > 100_000, "past the frame guard");
    }

    #[test]
    fn the_summary_is_one_json_line() {
        let mut stats = RunStats::new();
        for i in 0..7_500_u64 {
            stats.presented(i % 1_000 == 999);
            stats.gpu(match i {
                10 => GpuTime::Disjoint,
                20 | 30 => GpuTime::Unknown,
                5_000 => GpuTime::Us(12_000),
                _ => GpuTime::Us(7_950 + i % 100),
            });
        }
        assert_eq!((stats.frames(), stats.occluded()), (7_500, 7));
        assert_eq!(stats.times().count(), 7_497);
        let summary = stats.summary(Workload::Alu, 3_072, 60.0, EndReason::Stdin);
        let text = summary.json();
        assert!(text.ends_with("}\n") && !text[..text.len() - 1].contains('\n'));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kairo_loadgen"], 1);
        assert_eq!(value["workload"], "alu");
        assert_eq!(value["count"], 3_072);
        assert_eq!(value["frames"], 7_500);
        assert_eq!(value["seconds"], 60.0);
        assert_eq!(value["fps"], 125.0);
        // 7,497 times from 7.950 to 8.049 ms and one of 12 ms: ranks 3,749 and 7,423.
        assert_eq!(value["gpu_ms_p50"], 8.0);
        assert_eq!(value["gpu_ms_p99"], 8.049);
        assert_eq!(value["gpu_ms_max"], 12.0);
        assert_eq!(value["gpu_unknown"], 2);
        assert_eq!(value["gpu_disjoint"], 1);
        assert_eq!(value["occluded"], 7);
        assert_eq!(value["end"], "stdin");
        assert_eq!(value.as_object().unwrap().len(), 13);
        // A run that resolved no time, and one of no length.
        let empty = RunStats::new().summary(Workload::Bw, 24, 0.0, EndReason::Error);
        let value: Value = serde_json::from_str(&empty.json()).unwrap();
        assert!(value["fps"].is_null() && value["gpu_ms_p50"].is_null());
        assert!(value["gpu_ms_max"].is_null());
        assert_eq!(value["end"], "error");
    }

    fn calibrated(count: u32, gpu_ms: f64) -> Calibrated {
        Calibrated {
            count,
            gpu_ms,
            fit: Fit {
                a_ms: 0.299_999_4,
                b_ms: 0.002_500_000_3,
                r2: 0.999_999_1,
            },
            settled: Settled::Target,
        }
    }

    #[test]
    fn the_calibration_line_holds_the_count_and_its_conditions() {
        let line = CalibrationLine {
            workload: Workload::Alu,
            target_ms: 8.0,
            calibrated: calibrated(3_080, 8.0012),
            bands: 16,
            size: (2_560, 1_440),
            texture_mib: None,
            shader_digest: 0xaf63_dc4c_8601_ec8c,
            fingerprint: 1,
        };
        let text = line.json();
        assert!(text.ends_with("}\n"));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kairo_loadgen_calibration"], 1);
        assert_eq!(value["workload"], "alu");
        assert_eq!(value["target_ms"], 8.0);
        assert_eq!(value["count"], 3_080);
        assert_eq!(value["gpu_ms_p50"], 8.001);
        assert_eq!(value["fit"]["a_ms"], 0.3);
        assert_eq!(value["fit"]["b_ms"], 0.0025);
        assert_eq!(value["fit"]["r2"], 0.999_999);
        assert_eq!(value["settled"], "target");
        assert_eq!(value["bands"], 16);
        assert_eq!(value["size"], "2560x1440");
        assert!(value["texture_mib"].is_null());
        assert_eq!(value["shader"], "fnv64:af63dc4c8601ec8c");
        assert_eq!(value["fingerprint"], "fnv64:0000000000000001");
        assert!(value["gbps"].is_null(), "alu has no rate");
        assert_eq!(value.as_object().unwrap().len(), 13);
        // Bandwidth: 2560 × 1440 pixels × 8 reads × 64 B in 8 ms is 235.9 GB/s.
        let bw = CalibrationLine {
            workload: Workload::Bw,
            calibrated: Calibrated {
                settled: Settled::Granularity,
                ..calibrated(8, 8.0)
            },
            texture_mib: Some(256),
            ..line
        };
        let value: Value = serde_json::from_str(&bw.json()).unwrap();
        assert_eq!(value["gbps"], 235.9);
        assert_eq!(value["texture_mib"], 256);
        assert_eq!(value["settled"], "granularity");
        // Written create-new, like the ready file.
        let dir = std::env::temp_dir().join(format!("kairo-loadgen-cal-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calibration.json");
        bw.write(&path, 4_242).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), bw.json());
        let err = line.write(&path, 4_242).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), bw.json());
        fs::remove_dir_all(&dir).unwrap();
    }

    fn ready() -> Ready {
        Ready {
            pid: 4_242,
            hwnd: 197_794,
            client: (-2_560, 0, 2_560, 1_440),
            refresh_mhz: 179_960,
            adapter_luid: "0x00000000000123ab".to_owned(),
            adapter_index: 1,
            output_index: Some(0),
            umd_version: Some("32.0.21025.1006".to_owned()),
            hdr: false,
            buffers: 3,
            latency: 2,
            activated: true,
            workload: Workload::Alu,
            count: 3_072,
            bands: 16,
            detail: true,
            qpc_hz: 10_000_000,
            hags: Hags::On,
            fingerprint: 0x0123_4567_89ab_cdef,
            start_hns: 123_450_000_000,
            seconds: 90,
        }
    }

    #[test]
    fn the_ready_file_is_one_json_object_written_once() {
        let ready = ready();
        let text = ready.json();
        assert!(text.ends_with("}\n"));
        let value: Value = serde_json::from_str(&text).unwrap();
        let expected = json!({
            "kairo_loadgen_ready": 1, "pid": 4242, "hwnd": 197794,
            "client": {"left": -2560, "top": 0, "width": 2560, "height": 1440},
            "refresh_mhz": 179960, "adapter_luid": "0x00000000000123ab",
            "dxgi_adapter_index": 1, "dxgi_output_index": 0, "umd_version": "32.0.21025.1006",
            "format": "bgra8", "hdr": false, "swap_effect": "flip_discard", "buffers": 3,
            "latency": 2, "tearing": true, "present_flags": 512, "sync_interval": 0,
            "activated": true, "workload": "alu", "count": 3072, "bands": 16, "detail": true,
            "qpc_hz": 10000000, "hags": "on", "fingerprint": "fnv64:0123456789abcdef",
            "start_hns": 123450000000_i64, "seconds": 90,
        });
        assert_eq!(value, expected);
        // Unknown read-outs are null, never made up.
        let unknown = Ready {
            output_index: None,
            umd_version: None,
            hags: Hags::Unknown,
            ..ready.clone()
        };
        let value: Value = serde_json::from_str(&unknown.json()).unwrap();
        assert!(value["dxgi_output_index"].is_null() && value["umd_version"].is_null());
        assert_eq!(value["hags"], "unknown");
        // Written whole, and never over another run's.
        let dir = std::env::temp_dir().join(format!("kairo-loadgen-ready-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ready.json");
        ready.write(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(!dir.join("ready.json.4242.partial").exists());
        let other = Ready { pid: 77, ..ready };
        assert_eq!(
            other.write(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        fs::remove_dir_all(&dir).unwrap();
    }
}
