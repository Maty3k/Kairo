// SPDX-License-Identifier: GPL-3.0-or-later
//! The two workloads and the maths around them (docs/research/17-loadgen-bench/README.md §3.4–3.6,
//! §3.10): the shaders each one compiles, the bands a frame is drawn in, the start-up ramp
//! ([`Ramp`]) and its TDR guards, the calibration that finds the count for a target GPU time
//! ([`Calibrator`]), and the constants each frame's shaders read ([`FrameConstants`]). Everything
//! here is pure: the GPU's times come in as arguments, so the guards, the ramp and the
//! calibration are tested on the Mac and Linux with simulated GPUs.
//!
//! - **`alu`** (`--iters N`): every pixel runs a loop of N steps, each a transcendental and a few
//!   ALU operations, with a per-pixel loop variable and a uniform trip count (§3.5).
//! - **`bw`** (`--reads N`, a multiple of 4): every pixel makes N random loads from an `R32_UINT`
//!   texture of `--texture-mib` (256 MiB by default), four issued before any is used.
//!
//! Both draw the same picture ([`crate::visual`]) in [`bands`] of one scissored full-screen
//! triangle each, and both target about 8 ms of GPU per frame.

use kairo_core::timeline::HNS_PER_SECOND;

use crate::cli::exit;
use crate::summary::{GpuTime, GpuTimes};

/// Every shader, as HLSL (README §3.5, §3.7): compiled at run time with `D3DCompile` from the
/// inbox `d3dcompiler_47.dll`, never shipped as bytecode. Written for Kairo; no third-party shader
/// code (CLAUDE.md).
pub const HLSL: &str = include_str!("loadgen.hlsl");

/// The source name `D3DCompile` gets: a bare name, used only in its error messages, so they name
/// no path (README §3.7).
pub const HLSL_NAME: &str = "loadgen.hlsl";

/// The macro `--detail off` defines as `0` (README §3.2): the picture keeps its drift and loses
/// its grain. Without it the shaders compile with `KAIRO_DETAIL 1`.
pub const DETAIL_MACRO: &str = "KAIRO_DETAIL";

/// A shader entry point and the target it compiles for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The function's name in [`HLSL`].
    pub name: &'static str,
    /// `vs_5_0` or `ps_5_0`.
    pub target: &'static str,
}

/// One triangle over the target, from `SV_VertexID` alone (no vertex buffer).
pub const VS_FULLSCREEN: Entry = Entry {
    name: "vs_fullscreen",
    target: "vs_5_0",
};
/// The ALU workload's pixel shader.
pub const PS_ALU: Entry = Entry {
    name: "ps_alu",
    target: "ps_5_0",
};
/// The bandwidth workload's pixel shader.
pub const PS_BW: Entry = Entry {
    name: "ps_bw",
    target: "ps_5_0",
};
/// Fills the bandwidth workload's texture once, on the GPU, from the seed.
pub const PS_FILL: Entry = Entry {
    name: "ps_fill",
    target: "ps_5_0",
};
/// Every entry point in [`HLSL`]; Windows CI compiles each one (research 17 §6, W tier).
pub const ENTRIES: [Entry; 4] = [VS_FULLSCREEN, PS_ALU, PS_BW, PS_FILL];

/// What each frame costs the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    /// Shader maths: `--iters` steps per pixel.
    Alu,
    /// Memory reads: `--reads` random loads per pixel from a large texture.
    Bw,
}

impl Workload {
    /// Both, in the bench's order.
    pub const ALL: [Workload; 2] = [Workload::Alu, Workload::Bw];

    /// Its name on the command line and in the log, the ready file and the JSON lines.
    pub fn name(self) -> &'static str {
        match self {
            Self::Alu => "alu",
            Self::Bw => "bw",
        }
    }

    /// The workload called `name`.
    pub fn from_name(name: &str) -> Option<Workload> {
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// The flag that gives `run` its count.
    pub fn count_flag(self) -> &'static str {
        match self {
            Self::Alu => "--iters",
            Self::Bw => "--reads",
        }
    }

    /// The step between two counts: `bw` reads in fours, so each pass of its loop has four loads
    /// in flight before it uses any (README §3.5).
    pub fn granularity(self) -> u32 {
        match self {
            Self::Alu => 1,
            Self::Bw => 4,
        }
    }

    /// The smallest count.
    pub fn min_count(self) -> u32 {
        self.granularity()
    }

    /// The largest count, a TDR guard (README §3.2, §3.10): 65,536 iterations or 4,096 reads.
    pub fn max_count(self) -> u32 {
        match self {
            Self::Alu => 65_536,
            Self::Bw => 4_096,
        }
    }

    /// The first count of the start-up ramp and of the calibration: tiny on any GPU, so an
    /// absurd request is refused before a long frame exists (README §3.10).
    pub fn first_count(self) -> u32 {
        match self {
            Self::Alu => 16,
            Self::Bw => 4,
        }
    }

    /// Whether `count` is one this workload takes.
    pub fn valid_count(self, count: u32) -> bool {
        (self.min_count()..=self.max_count()).contains(&count)
            && count.is_multiple_of(self.granularity())
    }

    /// The valid count nearest `x`, or `None` if `x` rounds to one outside the caps: the
    /// calibration's target is then out of reach.
    pub fn nearest_count(self, x: f64) -> Option<u32> {
        if !x.is_finite() {
            return None;
        }
        let step = f64::from(self.granularity());
        let count = (x / step).round() * step;
        (f64::from(self.min_count())..=f64::from(self.max_count()))
            .contains(&count)
            .then_some(count as u32)
    }

    /// Its pixel shader.
    pub fn pixel_shader(self) -> Entry {
        match self {
            Self::Alu => PS_ALU,
            Self::Bw => PS_BW,
        }
    }

    /// The entry points a run of this workload compiles, in the order the log header lists their
    /// bytecode's hashes (`summary::ShaderHashes`).
    pub fn entries(self) -> &'static [Entry] {
        match self {
            Self::Alu => &[VS_FULLSCREEN, PS_ALU],
            Self::Bw => &[VS_FULLSCREEN, PS_BW, PS_FILL],
        }
    }

    /// Whether it reads the texture (and so takes `--texture-mib` and `--seed`).
    pub fn uses_texture(self) -> bool {
        self == Self::Bw
    }
}

/// The bandwidth workload's texture size when `--texture-mib` isn't given: larger than any RDNA 4
/// Infinity Cache, so most loads go to DRAM (README §3.5).
pub const DEFAULT_TEXTURE_MIB: u32 = 256;
/// The largest texture, a cap (README §3.2): 16,384 × 16,384, D3D11's largest side.
pub const MAX_TEXTURE_MIB: u32 = 1_024;
/// The bandwidth workload's seed when `--seed` isn't given (research 17's loadgen note, §3.3).
pub const DEFAULT_SEED: u32 = 0x4B4C_4731;
/// The bytes one random load is taken to move, for `gbps`: one 64-byte line (README §3.5, an
/// estimate).
pub const BYTES_PER_READ: u64 = 64;

/// The `R32_UINT` texture's width and height for `mib` MiB: a power of two from 1 to
/// [`MAX_TEXTURE_MIB`], with power-of-two sides, the width the larger when they differ (256 MiB is
/// 8,192 × 8,192). `None` for any other size.
pub fn texture_size(mib: u32) -> Option<(u32, u32)> {
    if !mib.is_power_of_two() || mib > MAX_TEXTURE_MIB {
        return None;
    }
    // mib × 2^20 bytes of 4-byte texels.
    let texels_log2 = 18 + mib.trailing_zeros();
    Some((1 << texels_log2.div_ceil(2), 1 << (texels_log2 / 2)))
}

/// The masks the bandwidth shader turns a hash into texel coordinates with: width − 1 and
/// height − 1. x takes the hash's low bits and y the bits from 16 up, so they never share a bit.
pub fn texture_mask(mib: u32) -> Option<[u32; 2]> {
    texture_size(mib).map(|(w, h)| [w - 1, h - 1])
}

/// The bandwidth workload's texture: its size (`--texture-mib`) and the seed it is filled from
/// (`--seed`). Only [`Texture::new`] makes one, so its size is always valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Texture {
    mib: u32,
    seed: u32,
    width: u32,
    height: u32,
}

impl Texture {
    /// A texture of `mib` MiB filled from `seed`, or `None` if `mib` isn't a power of two from 1
    /// to [`MAX_TEXTURE_MIB`].
    pub fn new(mib: u32, seed: u32) -> Option<Texture> {
        texture_size(mib).map(|(width, height)| Texture {
            mib,
            seed,
            width,
            height,
        })
    }

    /// Its size in MiB.
    pub fn mib(&self) -> u32 {
        self.mib
    }

    /// The seed `ps_fill` fills it from.
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Its width and height in texels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// [`texture_mask`]: width − 1 and height − 1.
    pub fn mask(&self) -> [u32; 2] {
        [self.width - 1, self.height - 1]
    }
}

/// Rows `top..bottom` of band `index` of `count` horizontal bands over `height` rows. Band
/// boundaries fall at `index × height / count`, rounded down, so every row is in exactly one
/// band, the bands differ by at most one row, the tallest has ⌈height ÷ count⌉ rows, and none is
/// empty when `height ≥ count`. (README §3.5 sketches bands of ⌈height ÷ count⌉ rows with the
/// last taking the rest, which leaves empty bands at some heights: 17 rows in 16 bands.)
pub fn band(height: u32, count: u32, index: u32) -> (u32, u32) {
    let at = |i: u32| (u64::from(height) * u64::from(i) / u64::from(count.max(1))) as u32;
    (at(index), at(index + 1))
}

/// Every band, top to bottom: each is one scissor rectangle and one `Draw(3, 0)` (README §3.4).
pub fn bands(height: u32, count: u32) -> impl Iterator<Item = (u32, u32)> {
    (0..count).map(move |i| band(height, count, i))
}

/// The tallest band's rows: ⌈height ÷ count⌉.
pub fn tallest_band(height: u32, count: u32) -> u32 {
    height.div_ceil(count.max(1))
}

/// The swap chain's size and the bands a frame is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The number of bands (`--bands`).
    pub bands: u32,
}

impl Geometry {
    /// The pixels a frame draws.
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// The share of a frame's GPU time the tallest band takes: every pixel costs the same.
    pub fn tallest_share(&self) -> f64 {
        if self.height == 0 {
            return 1.0;
        }
        f64::from(tallest_band(self.height, self.bands)) / f64::from(self.height)
    }
}

/// The longest predicted band, in ms: 80 times below Windows' 2 s TDR limit, whatever the GPU's
/// preemption granularity, since a draw that can't be preempted lasts as long as its band. At the
/// 8 ms target a band of 16 takes about 0.5 ms, 4,000 times below the limit (README §3.10).
pub const BAND_LIMIT_MS: f64 = 25.0;
/// The longest predicted frame, in ms.
pub const FRAME_LIMIT_MS: f64 = 100.0;
/// The run-time guard: [`LONG_FRAMES`] frames in a row over this much GPU time end a run.
pub const LONG_FRAME_MS: f64 = 250.0;
/// How many long frames in a row end a run.
pub const LONG_FRAMES: u32 = 3;
/// Each ramp step multiplies the count by this much.
pub const RAMP_FACTOR: u32 = 4;
/// The frames each ramp step is timed over, at least.
pub const RAMP_FRAMES: u32 = 10;

/// A step the guards refuse (exit 3, `e too_heavy`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TooHeavy {
    /// The count that would have run.
    pub count: u32,
    /// Its predicted (or measured) GPU time per frame, in ms.
    pub predicted_ms: f64,
}

/// The GPU time per frame `to_count` is predicted to take, from `from_count`'s: scaled by the
/// count. Every frame also has a fixed part that doesn't grow with the count, so this
/// over-predicts, the safe side.
pub fn predict_ms(from_count: u32, from_ms: f64, to_count: u32) -> f64 {
    from_ms * f64::from(to_count) / f64::from(from_count.max(1))
}

/// Refuses a frame of `count` predicted to take `ms` if it would pass [`FRAME_LIMIT_MS`], or its
/// tallest band [`BAND_LIMIT_MS`]. A time that isn't a number is refused too.
pub fn check(geometry: &Geometry, count: u32, ms: f64) -> Result<(), TooHeavy> {
    let fits =
        ms.is_finite() && ms <= FRAME_LIMIT_MS && ms * geometry.tallest_share() <= BAND_LIMIT_MS;
    if fits {
        Ok(())
    } else {
        Err(TooHeavy {
            count,
            predicted_ms: ms,
        })
    }
}

/// The guard before a step from `from` (a count and its median GPU ms) to `to_count`.
pub fn guard_step(geometry: &Geometry, from: (u32, f64), to_count: u32) -> Result<(), TooHeavy> {
    check(geometry, to_count, predict_ms(from.0, from.1, to_count))
}

/// The start-up ramp of a run at `count` (README §3.10): steps of ×[`RAMP_FACTOR`] from the
/// workload's [`first_count`](Workload::first_count), every one below `count`. Each runs at
/// least [`RAMP_FRAMES`] frames and is timed; the next step, and at the end `count` itself, runs
/// only if [`guard_step`] allows it. Empty when `count` is no larger than the first step: it is
/// tiny itself. The ramp never changes the count: it reaches it or refuses.
pub fn ramp_steps(workload: Workload, count: u32) -> Vec<u32> {
    let mut steps = Vec::new();
    let mut step = workload.first_count();
    while step < count {
        steps.push(step);
        step = step.saturating_mul(RAMP_FACTOR);
    }
    steps
}

/// The start-up ramp of a run (README §3.10), as counts its caller draws: each of
/// [`ramp_steps`] for at least [`RAMP_FRAMES`] timed frames, then the run's count. Every move up
/// needs [`guard_step`]'s leave, predicted from the median of the step just drawn, so a count
/// that would take too long is refused (exit 3, `e too_heavy`) before one of its frames exists.
/// `ready.json` comes only once the run's count is drawn, so PresentMon never records the ramp.
#[derive(Debug, Clone)]
pub struct Ramp {
    geometry: Geometry,
    count: u32,
    steps: Vec<u32>,
    at: usize,
}

impl Ramp {
    /// The ramp of a run of `workload` at `count` on `geometry`.
    pub fn new(workload: Workload, count: u32, geometry: Geometry) -> Ramp {
        Ramp {
            geometry,
            count,
            steps: ramp_steps(workload, count),
            at: 0,
        }
    }

    /// The count to draw now: a ramp step, or the run's count once the ramp is over.
    pub fn current(&self) -> u32 {
        self.steps.get(self.at).copied().unwrap_or(self.count)
    }

    /// The current ramp step's number, from 0 (`e ramp step=`), or `None` once the ramp is over.
    pub fn step(&self) -> Option<u32> {
        (self.at < self.steps.len()).then_some(self.at as u32)
    }

    /// Whether the ramp is over: from now on every frame is drawn at the run's count.
    pub fn done(&self) -> bool {
        self.at >= self.steps.len()
    }

    /// Reports the median GPU ms of the step just drawn and returns the next count to draw, if
    /// the guards allow it.
    pub fn measured(&mut self, gpu_ms: f64) -> Result<u32, Refusal> {
        if self.done() {
            return Err(Refusal::OutOfOrder);
        }
        if !(gpu_ms.is_finite() && gpu_ms > 0.0) {
            return Err(Refusal::NoTiming);
        }
        let from = self.current();
        let to = self.steps.get(self.at + 1).copied().unwrap_or(self.count);
        guard_step(&self.geometry, (from, gpu_ms), to).map_err(Refusal::TooHeavy)?;
        self.at += 1;
        Ok(to)
    }
}

/// How long past its own length a ramp or calibration step may go on without the timed frames it
/// needs, in 100 ns: then the GPU's timestamps count as broken (exit 4, `e refused
/// reason=no_timing`). 10 s is 100 frames at the 100 ms frame guard, so a step the guards allow
/// always gets its frames in time.
pub const STEP_LIMIT_HNS: i64 = 10 * HNS_PER_SECOND;

/// Where a step's timing stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepTiming {
    /// Still drawing: too few frames timed yet, or too short.
    Running,
    /// Enough: the median GPU time of the step's timed frames (nearest rank), and how many there
    /// were.
    Done {
        /// The median, in µs.
        median_us: u64,
        /// The frames timed.
        frames: u32,
    },
    /// [`STEP_LIMIT_HNS`] past the step's length, still without enough timed frames.
    NoTiming,
}

/// Times one ramp step or calibration step (README §3.6, §3.10): a step is done once at least
/// `frames` of its frames have a GPU time and it has lasted at least `seconds`, and its time is
/// their median. Frames carry the id of the step they were drawn for ([`StepTimer::start`]), so a
/// frame of an earlier step that resolves late never counts for the next one, whose count
/// differs. The times go into a histogram allocated once ([`GpuTimes`]), so a frame allocates
/// nothing.
#[derive(Debug, Clone)]
pub struct StepTimer {
    id: u32,
    frames: u32,
    until_hns: i64,
    limit_hns: i64,
    times: GpuTimes,
}

impl Default for StepTimer {
    fn default() -> StepTimer {
        StepTimer::new()
    }
}

impl StepTimer {
    /// A timer with no step yet: id 0 is never a step's, so frames drawn outside a step can
    /// carry it.
    pub fn new() -> StepTimer {
        StepTimer {
            id: 0,
            frames: 1,
            until_hns: i64::MAX,
            limit_hns: i64::MAX,
            times: GpuTimes::new(),
        }
    }

    /// Starts a step at `now_hns` that needs `frames` timed frames (at least one) and `seconds`;
    /// returns its id, which the frames drawn for it carry. Forgets the step before.
    pub fn start(&mut self, now_hns: i64, frames: u32, seconds: u32) -> u32 {
        self.id = match self.id.wrapping_add(1) {
            0 => 1,
            id => id,
        };
        self.frames = frames.max(1);
        self.until_hns = now_hns.saturating_add(i64::from(seconds) * HNS_PER_SECOND);
        self.limit_hns = self.until_hns.saturating_add(STEP_LIMIT_HNS);
        self.times.clear();
        self.id
    }

    /// The current step's id (0 before the first).
    pub fn id(&self) -> u32 {
        self.id
    }

    /// One resolved frame drawn for step `id`: counted if that is the current step and its time
    /// is known.
    pub fn add(&mut self, id: u32, time: GpuTime) {
        if let (true, GpuTime::Us(us)) = (id == self.id && id != 0, time) {
            self.times.add(us);
        }
    }

    /// Where the step stands at `now_hns`.
    pub fn timing(&self, now_hns: i64) -> StepTiming {
        let timed = self.times.count();
        if timed >= u64::from(self.frames)
            && now_hns >= self.until_hns
            && let Some(median_us) = self.times.median_us()
        {
            return StepTiming::Done {
                median_us,
                frames: u32::try_from(timed).unwrap_or(u32::MAX),
            };
        }
        if now_hns >= self.limit_hns {
            StepTiming::NoTiming
        } else {
            StepTiming::Running
        }
    }
}

/// The run-time guard (README §3.10): [`LONG_FRAMES`] frames in a row over [`LONG_FRAME_MS`] of
/// GPU time end the run, which catches the device landing somewhere slow mid-run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LongFrames {
    streak: u32,
}

impl LongFrames {
    /// No long frame yet.
    pub fn new() -> LongFrames {
        LongFrames::default()
    }

    /// Counts one frame's GPU time (`None`: unknown or disjoint) and says whether the run must
    /// end. A frame of unknown time neither extends nor breaks the streak, so an unknown frame
    /// between long ones can't hide them.
    pub fn frame(&mut self, gpu_ms: Option<f64>) -> bool {
        match gpu_ms {
            Some(ms) if ms > LONG_FRAME_MS => self.streak += 1,
            Some(_) => self.streak = 0,
            None => {}
        }
        self.streak >= LONG_FRAMES
    }
}

/// The frames each calibration step is timed over, at least (README §3.6).
pub const CALIBRATE_FRAMES: u32 = 30;
/// How long the calibration verifies a count, in seconds.
pub const VERIFY_SECONDS: u32 = 5;
/// How close to the target a verified count must come: ±3%.
pub const ACCEPT_FRACTION: f64 = 0.03;
/// The fit's R² must reach this, or the calibration refuses: the GPU time isn't linear in the
/// count.
pub const MIN_R2: f64 = 0.99;
/// The fit is made over this many doubling steps, re-measured warm.
pub const FIT_POINTS: usize = 3;
/// The calibration's default target, in ms of GPU per frame (`--target-ms`).
pub const DEFAULT_TARGET_MS: f64 = 8.0;
/// The calibration's default warm-up, in seconds (`--warmup`).
pub const DEFAULT_WARMUP_SECONDS: u32 = 120;

/// A least-squares line through (count, GPU ms) points: ms = a + b · count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// The fixed part of a frame, in ms.
    pub a_ms: f64,
    /// The cost of one more iteration or read, in ms.
    pub b_ms: f64,
    /// R², the share of the variance the line explains.
    pub r2: f64,
}

impl Fit {
    /// Whether the calibration may use it: R² at least [`MIN_R2`] and a cost that grows with the
    /// count.
    pub fn accepted(&self) -> bool {
        self.r2 >= MIN_R2 && self.b_ms > 0.0 && self.a_ms.is_finite()
    }

    /// The GPU ms it predicts for `count`.
    pub fn predict(&self, count: u32) -> f64 {
        self.a_ms + self.b_ms * f64::from(count)
    }

    /// The count it predicts takes `target_ms`, not yet rounded to a valid one.
    pub fn count_for(&self, target_ms: f64) -> f64 {
        (target_ms - self.a_ms) / self.b_ms
    }
}

/// The least-squares line through `points`, or `None` with fewer than two different counts.
pub fn fit(points: &[(u32, f64)]) -> Option<Fit> {
    if points.len() < 2 {
        return None;
    }
    let n = points.len() as f64;
    let mean_x = points.iter().map(|&(x, _)| f64::from(x)).sum::<f64>() / n;
    let mean_y = points.iter().map(|&(_, y)| y).sum::<f64>() / n;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for &(x, y) in points {
        let (dx, dy) = (f64::from(x) - mean_x, y - mean_y);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    if sxx <= 0.0 {
        return None;
    }
    let b = sxy / sxx;
    // With an intercept, R² = 1 − SSres ÷ SStot = Sxy² ÷ (Sxx · Syy).
    let r2 = if syy > 0.0 {
        sxy * sxy / (sxx * syy)
    } else {
        0.0
    };
    Some(Fit {
        a_ms: mean_y - b * mean_x,
        b_ms: b,
        r2,
    })
}

/// Which measurement a calibration step is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// A doubling step, from the workload's first count up (README §3.6 step 2).
    Double,
    /// One of the last three doubling steps, re-measured after the warm-up (step 4).
    Warm,
    /// The fitted count, or its one correction, timed for [`VERIFY_SECONDS`] (step 5).
    Verify,
}

impl Stage {
    /// Its name in the log.
    pub fn name(self) -> &'static str {
        match self {
            Self::Double => "double",
            Self::Warm => "warm",
            Self::Verify => "verify",
        }
    }
}

/// What the calibration asks its caller to do next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CalibrationStep {
    /// Draw at `count` for at least `frames` frames and at least `seconds` seconds, then report
    /// the median GPU ms with [`Calibrator::measured`].
    Measure {
        /// Which measurement this is.
        stage: Stage,
        /// Its number within the stage, from 0.
        step: u32,
        /// The count to draw at.
        count: u32,
        /// At least this many timed frames.
        frames: u32,
        /// For at least this long (0 for none).
        seconds: u32,
    },
    /// Hold `count` for `seconds`, so clocks and temperature settle as they do before the bench's
    /// runs (README §3.6 step 3), then call [`Calibrator::held`].
    Hold {
        /// The count to hold.
        count: u32,
        /// For how long.
        seconds: u32,
    },
    /// The calibration is done.
    Done(Calibrated),
}

/// How a calibrated count was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Within ±3% of the target.
    Target,
    /// The nearest the workload's granularity allows: no valid count comes within ±3%, which a
    /// bandwidth count in fours can't always do (a step of 4 reads may be a large share of the
    /// target).
    Granularity,
}

impl Settled {
    /// Its name in the log.
    pub fn name(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Granularity => "granularity",
        }
    }
}

/// A finished calibration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibrated {
    /// The count for the target.
    pub count: u32,
    /// Its median GPU ms, as verified.
    pub gpu_ms: f64,
    /// The fit it came from.
    pub fit: Fit,
    /// How it was accepted.
    pub settled: Settled,
}

/// Why a calibration (or a ramp) can't go on: exit 3, except [`Refusal::NoTiming`] and
/// [`Refusal::OutOfOrder`], which are failures of the GPU's timing or of the caller (exit 4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Refusal {
    /// A TDR guard refused the next step.
    TooHeavy(TooHeavy),
    /// The count reached the workload's cap while still below half the target.
    CapReached {
        /// The last count measured.
        count: u32,
        /// Its median GPU ms.
        gpu_ms: f64,
    },
    /// The GPU time isn't linear in the count (R² below [`MIN_R2`], or a cost that doesn't grow).
    NotLinear(Fit),
    /// The count for the target is outside the workload's caps.
    OutOfReach {
        /// The count the fit asked for.
        count: f64,
    },
    /// Neither the fitted count nor its correction came within ±3% of the target, and the
    /// granularity doesn't explain it.
    Unsettled {
        /// The last count verified.
        count: u32,
        /// Its median GPU ms.
        gpu_ms: f64,
    },
    /// A step reported no GPU time: the timestamps never resolved (exit 4).
    NoTiming,
    /// The caller reported a step the calibration didn't ask for.
    OutOfOrder,
}

impl Refusal {
    /// The exit code it ends the program with.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::NoTiming | Self::OutOfOrder => exit::DEVICE,
            _ => exit::REFUSED,
        }
    }

    /// Its name in the log's `e refused reason=` line (a guard's refusal is `e too_heavy`).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::TooHeavy(_) => "too_heavy",
            Self::CapReached { .. } => "cap_reached",
            Self::NotLinear(_) => "not_linear",
            Self::OutOfReach { .. } => "out_of_reach",
            Self::Unsettled { .. } => "unsettled",
            Self::NoTiming => "no_timing",
            Self::OutOfOrder => "out_of_order",
        }
    }

    /// What stderr says: no path, no device name.
    pub fn message(&self) -> String {
        match self {
            Self::TooHeavy(heavy) => format!(
                "a frame at count {} is predicted to take {:.1} ms of GPU time, over the TDR \
                 guards ({FRAME_LIMIT_MS} ms a frame, {BAND_LIMIT_MS} ms a band)",
                heavy.count, heavy.predicted_ms
            ),
            Self::CapReached { count, gpu_ms } => format!(
                "the count reached its cap at {count} with {gpu_ms:.3} ms of GPU time, below half \
                 the target"
            ),
            Self::NotLinear(fit) => format!(
                "the GPU time isn't linear in the count (r2 {:.4}, below {MIN_R2}, or a slope of \
                 {:.6} ms)",
                fit.r2, fit.b_ms
            ),
            Self::OutOfReach { count } => {
                format!("the target needs a count of {count:.1}, outside the workload's caps")
            }
            Self::Unsettled { count, gpu_ms } => format!(
                "count {count} took {gpu_ms:.3} ms of GPU time, and the target wasn't reached \
                 within 3% after one correction"
            ),
            Self::NoTiming => "the GPU's timestamps gave no time".to_owned(),
            Self::OutOfOrder => "the calibration's steps came out of order".to_owned(),
        }
    }
}

/// Where a calibration is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Doubling,
    Holding,
    Warm,
    Verify { attempt: u32 },
    Done,
}

/// The calibration (README §3.6), as steps its caller runs on the GPU:
///
/// 1. Doubling steps from the workload's first count, [`CALIBRATE_FRAMES`] frames each, until a
///    step's median passes half the target and at least [`FIT_POINTS`] steps exist; the TDR
///    guard runs before each one.
/// 2. A hold at the last count for the warm-up.
/// 3. The last [`FIT_POINTS`] doubling counts re-measured warm, and a least-squares fit through
///    them; R² below [`MIN_R2`] refuses.
/// 4. The fitted count verified for [`VERIFY_SECONDS`]: within ±3% of the target it is accepted;
///    otherwise one correction step along the fit's slope, accepted within ±3%. When the
///    workload's granularity can't come closer (the correction rounds back to the same count, or
///    the target lies between two neighbouring counts), the nearer one is accepted as
///    [`Settled::Granularity`]. Anything else refuses.
#[derive(Debug, Clone)]
pub struct Calibrator {
    workload: Workload,
    target_ms: f64,
    warmup_seconds: u32,
    geometry: Geometry,
    phase: Phase,
    count: u32,
    doubling: Vec<(u32, f64)>,
    warm: Vec<(u32, f64)>,
    fit: Option<Fit>,
    first_verify: Option<(u32, f64)>,
}

impl Calibrator {
    /// A calibration of `workload` for `target_ms` per frame on `geometry`, holding
    /// `warmup_seconds` before the fit.
    pub fn new(
        workload: Workload,
        target_ms: f64,
        warmup_seconds: u32,
        geometry: Geometry,
    ) -> Calibrator {
        Calibrator {
            workload,
            target_ms,
            warmup_seconds,
            geometry,
            phase: Phase::Doubling,
            count: workload.first_count(),
            doubling: Vec::new(),
            warm: Vec::new(),
            fit: None,
            first_verify: None,
        }
    }

    /// The first step: the workload's first count, which needs no guard.
    pub fn first(&self) -> CalibrationStep {
        self.measure(Stage::Double, 0, self.count)
    }

    /// The fit, once made.
    pub fn fit(&self) -> Option<Fit> {
        self.fit
    }

    fn measure(&self, stage: Stage, step: u32, count: u32) -> CalibrationStep {
        CalibrationStep::Measure {
            stage,
            step,
            count,
            frames: CALIBRATE_FRAMES,
            seconds: if stage == Stage::Verify {
                VERIFY_SECONDS
            } else {
                0
            },
        }
    }

    /// Reports the median GPU ms of the step just measured, and returns the next step.
    pub fn measured(&mut self, gpu_ms: f64) -> Result<CalibrationStep, Refusal> {
        if !(gpu_ms.is_finite() && gpu_ms > 0.0) {
            return Err(Refusal::NoTiming);
        }
        // What was just drawn must itself have stayed within the guards.
        check(&self.geometry, self.count, gpu_ms).map_err(Refusal::TooHeavy)?;
        match self.phase {
            Phase::Doubling => self.doubled(gpu_ms),
            Phase::Warm => self.warmed(gpu_ms),
            Phase::Verify { attempt } => self.verified(attempt, gpu_ms),
            Phase::Holding | Phase::Done => Err(Refusal::OutOfOrder),
        }
    }

    /// Reports that the hold is over, and returns the next step.
    pub fn held(&mut self) -> Result<CalibrationStep, Refusal> {
        if self.phase != Phase::Holding {
            return Err(Refusal::OutOfOrder);
        }
        let first = self
            .warm_counts()
            .first()
            .copied()
            .ok_or(Refusal::OutOfOrder)?;
        self.phase = Phase::Warm;
        self.count = first;
        Ok(self.measure(Stage::Warm, 0, first))
    }

    fn doubled(&mut self, gpu_ms: f64) -> Result<CalibrationStep, Refusal> {
        let count = self.count;
        self.doubling.push((count, gpu_ms));
        if gpu_ms > self.target_ms / 2.0 && self.doubling.len() >= FIT_POINTS {
            self.phase = Phase::Holding;
            return Ok(CalibrationStep::Hold {
                count,
                seconds: self.warmup_seconds,
            });
        }
        let next = count
            .checked_mul(2)
            .filter(|&next| next <= self.workload.max_count())
            .ok_or(Refusal::CapReached { count, gpu_ms })?;
        guard_step(&self.geometry, (count, gpu_ms), next).map_err(Refusal::TooHeavy)?;
        self.count = next;
        Ok(self.measure(Stage::Double, self.doubling.len() as u32, next))
    }

    /// The last [`FIT_POINTS`] doubling counts, smallest first.
    fn warm_counts(&self) -> Vec<u32> {
        let from = self.doubling.len().saturating_sub(FIT_POINTS);
        self.doubling[from..].iter().map(|&(c, _)| c).collect()
    }

    fn warmed(&mut self, gpu_ms: f64) -> Result<CalibrationStep, Refusal> {
        self.warm.push((self.count, gpu_ms));
        let counts = self.warm_counts();
        if let Some(&next) = counts.get(self.warm.len()) {
            self.count = next;
            return Ok(self.measure(Stage::Warm, self.warm.len() as u32, next));
        }
        let fit = fit(&self.warm).ok_or(Refusal::OutOfOrder)?;
        if !fit.accepted() {
            return Err(Refusal::NotLinear(fit));
        }
        self.fit = Some(fit);
        let wanted = fit.count_for(self.target_ms);
        let count = self
            .workload
            .nearest_count(wanted)
            .ok_or(Refusal::OutOfReach { count: wanted })?;
        check(&self.geometry, count, fit.predict(count)).map_err(Refusal::TooHeavy)?;
        self.phase = Phase::Verify { attempt: 0 };
        self.count = count;
        Ok(self.measure(Stage::Verify, 0, count))
    }

    fn verified(&mut self, attempt: u32, gpu_ms: f64) -> Result<CalibrationStep, Refusal> {
        let fit = self.fit.ok_or(Refusal::OutOfOrder)?;
        let count = self.count;
        let target = self.target_ms;
        let done = |count, gpu_ms, settled| {
            CalibrationStep::Done(Calibrated {
                count,
                gpu_ms,
                fit,
                settled,
            })
        };
        if (gpu_ms - target).abs() <= ACCEPT_FRACTION * target {
            self.phase = Phase::Done;
            return Ok(done(count, gpu_ms, Settled::Target));
        }
        if attempt == 0 {
            let wanted = f64::from(count) + (target - gpu_ms) / fit.b_ms;
            let corrected = self
                .workload
                .nearest_count(wanted)
                .ok_or(Refusal::OutOfReach { count: wanted })?;
            if corrected == count {
                // Half a granularity step or less from the target: nothing valid is nearer.
                self.phase = Phase::Done;
                return Ok(done(count, gpu_ms, Settled::Granularity));
            }
            let predicted = gpu_ms + fit.b_ms * (f64::from(corrected) - f64::from(count));
            check(&self.geometry, corrected, predicted).map_err(Refusal::TooHeavy)?;
            self.first_verify = Some((count, gpu_ms));
            self.phase = Phase::Verify { attempt: 1 };
            self.count = corrected;
            return Ok(self.measure(Stage::Verify, 1, corrected));
        }
        let (first_count, first_ms) = self.first_verify.ok_or(Refusal::OutOfOrder)?;
        let between = (first_ms - target).signum() != (gpu_ms - target).signum();
        let neighbours = first_count.abs_diff(count) == self.workload.granularity();
        if between && neighbours {
            self.phase = Phase::Done;
            return Ok(if (first_ms - target).abs() < (gpu_ms - target).abs() {
                done(first_count, first_ms, Settled::Granularity)
            } else {
                done(count, gpu_ms, Settled::Granularity)
            });
        }
        Err(Refusal::Unsettled { count, gpu_ms })
    }
}

/// The picture's colour drift period: one cycle every 20 s, 0.05 Hz (README §3.11).
pub const DRIFT_PERIOD_HNS: i64 = 20 * HNS_PER_SECOND;
/// How fast the grain scrolls, in pixels per second.
pub const SCROLL_PX_PER_SECOND: i64 = 120;
/// Where the scroll wraps: the grain's lattice repeats every 256 cells of 32 px.
pub const SCROLL_WRAP_PX: i64 = 8_192;

/// The colour drift at `t_hns` (100 ns since the run's anchor): frac(t ÷ 20 s), from the exact
/// integer remainder, so it never loses precision however long the run, and wraps where the
/// picture is periodic.
pub fn drift_at(t_hns: i64) -> f64 {
    t_hns.rem_euclid(DRIFT_PERIOD_HNS) as f64 / DRIFT_PERIOD_HNS as f64
}

/// The grain's scroll at `t_hns`, in pixels: (120 px/s · t) mod 8,192, from the exact integer
/// remainder, for the same reasons.
pub fn scroll_at(t_hns: i64) -> f64 {
    let travelled = i128::from(t_hns) * i128::from(SCROLL_PX_PER_SECOND);
    let wrap = i128::from(SCROLL_WRAP_PX) * i128::from(HNS_PER_SECOND);
    travelled.rem_euclid(wrap) as f64 / HNS_PER_SECOND as f64
}

/// The size of [`FrameConstants`] and of the constant buffer: two 16-byte registers.
pub const FRAME_CONSTANTS_SIZE: usize = 32;

/// The 32-byte constant buffer the shaders read (README §3.5), filled each frame with
/// `Map(WRITE_DISCARD)`. Its fields are `cbuffer Frame` in [`HLSL`], in the same order.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameConstants {
    /// [`drift_at`] the frame's time.
    pub drift: f32,
    /// [`scroll_at`] the frame's time.
    pub scroll_px: f32,
    /// `--iters` or `--reads`.
    pub count: u32,
    /// The bandwidth workload's seed (0 for ALU).
    pub seed: u32,
    /// 1 ÷ the target's width and height.
    pub inv_size: [f32; 2],
    /// [`texture_mask`] (zeros for ALU).
    pub tex_mask: [u32; 2],
}

impl FrameConstants {
    /// The constants of a frame that starts `t_hns` after the run's anchor, drawing at `count`
    /// (with `texture` for the bandwidth workload) on a `size` target. The picture depends only on
    /// the pixel and this time, never on the frame count, so two runs show the same picture at
    /// the same `t` whatever their frame rates (README §3.6). Making them allocates nothing.
    pub fn new(
        t_hns: i64,
        count: u32,
        texture: Option<Texture>,
        size: (u32, u32),
    ) -> FrameConstants {
        let inverse = |side: u32| 1.0 / side.max(1) as f32;
        FrameConstants {
            drift: drift_at(t_hns) as f32,
            scroll_px: scroll_at(t_hns) as f32,
            count,
            seed: texture.map_or(0, |t| t.seed()),
            inv_size: [inverse(size.0), inverse(size.1)],
            tex_mask: texture.map_or([0, 0], |t| t.mask()),
        }
    }

    /// Its bytes as the constant buffer holds them (little-endian, no padding), to copy into the
    /// mapped buffer.
    pub fn bytes(&self) -> [u8; FRAME_CONSTANTS_SIZE] {
        let words = [
            self.drift.to_bits(),
            self.scroll_px.to_bits(),
            self.count,
            self.seed,
            self.inv_size[0].to_bits(),
            self.inv_size[1].to_bits(),
            self.tex_mask[0],
            self.tex_mask[1],
        ];
        let mut bytes = [0_u8; FRAME_CONSTANTS_SIZE];
        for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
            *chunk = word.to_le_bytes();
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Geometry = Geometry {
        width: 2_560,
        height: 1_440,
        bands: 16,
    };

    #[test]
    fn the_hlsl_holds_every_entry_point_and_the_constant_buffer_in_order() {
        assert!(HLSL.is_ascii(), "the HLSL is ASCII");
        assert!(!HLSL.contains("#include"), "no include handler is given");
        for entry in ENTRIES {
            let returns = if entry.name == PS_FILL.name {
                "uint"
            } else {
                "float4"
            };
            let signature = format!("{returns} {}(", entry.name);
            assert_eq!(HLSL.matches(&signature).count(), 1, "{signature}");
        }
        assert!(HLSL.contains("#ifndef KAIRO_DETAIL\n#define KAIRO_DETAIL 1\n#endif"));
        // cbuffer Frame matches FrameConstants, field by field.
        let start = HLSL.find("cbuffer Frame : register(b0)").unwrap();
        let open = start + HLSL[start..].find('{').unwrap();
        let close = open + HLSL[open..].find('}').unwrap();
        let fields: Vec<String> = HLSL[open + 1..close]
            .lines()
            .map(|line| line.split("//").next().unwrap().trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            fields,
            [
                "float drift;",
                "float scroll_px;",
                "uint count;",
                "uint seed;",
                "float2 inv_size;",
                "uint2 tex_mask;",
            ]
        );
        assert!(HLSL.contains("Texture2D<uint> big : register(t0);"));
        assert_eq!(size_of::<FrameConstants>(), FRAME_CONSTANTS_SIZE);
        assert_eq!(FRAME_CONSTANTS_SIZE % 16, 0, "a whole number of registers");
    }

    #[test]
    fn each_workload_compiles_its_own_entries() {
        assert_eq!(Workload::Alu.entries(), [VS_FULLSCREEN, PS_ALU]);
        assert_eq!(Workload::Bw.entries(), [VS_FULLSCREEN, PS_BW, PS_FILL]);
        for workload in Workload::ALL {
            assert_eq!(Workload::from_name(workload.name()), Some(workload));
            assert_eq!(workload.entries()[1], workload.pixel_shader());
        }
        assert_eq!(Workload::from_name("bandwidth"), None);
        assert!(
            ENTRIES
                .iter()
                .all(|e| e.target == "ps_5_0" || e == &VS_FULLSCREEN)
        );
        assert_eq!(VS_FULLSCREEN.target, "vs_5_0");
    }

    #[test]
    fn counts_and_their_caps() {
        let alu = Workload::Alu;
        let bw = Workload::Bw;
        assert!(alu.valid_count(1) && alu.valid_count(3_072) && alu.valid_count(65_536));
        assert!(!alu.valid_count(0) && !alu.valid_count(65_537));
        assert!(bw.valid_count(4) && bw.valid_count(24) && bw.valid_count(4_096));
        assert!(!bw.valid_count(0) && !bw.valid_count(6) && !bw.valid_count(4_100));
        assert_eq!(alu.nearest_count(3_079.6), Some(3_080));
        assert_eq!(
            alu.nearest_count(0.4),
            None,
            "even one step is over the target"
        );
        assert_eq!(alu.nearest_count(65_536.4), Some(65_536));
        assert_eq!(alu.nearest_count(65_537.0), None);
        assert_eq!(bw.nearest_count(9.75), Some(8));
        assert_eq!(bw.nearest_count(10.1), Some(12));
        assert_eq!(bw.nearest_count(1.9), None);
        assert_eq!(bw.nearest_count(f64::NAN), None);
        assert_eq!(bw.nearest_count(f64::INFINITY), None);
    }

    #[test]
    fn texture_sizes_are_powers_of_two_up_to_16384() {
        assert_eq!(texture_size(1), Some((512, 512)));
        assert_eq!(texture_size(2), Some((1_024, 512)));
        assert_eq!(texture_size(256), Some((8_192, 8_192)));
        assert_eq!(texture_size(512), Some((16_384, 8_192)));
        assert_eq!(texture_size(1_024), Some((16_384, 16_384)));
        for mib in [0, 3, 100, 2_048] {
            assert_eq!(texture_size(mib), None, "{mib}");
        }
        for k in 0..=10 {
            let mib = 1_u32 << k;
            let (w, h) = texture_size(mib).unwrap();
            assert_eq!(u64::from(w) * u64::from(h) * 4, u64::from(mib) << 20);
            assert!(w.is_power_of_two() && h.is_power_of_two() && w >= h && w <= 16_384);
            // x uses the hash's low 14 bits at most, y bits 16 and up: never the same bit.
            let [mx, my] = texture_mask(mib).unwrap();
            assert_eq!(mx & (my << 16), 0);
            assert!(mx < 1 << 16);
        }
    }

    #[test]
    fn bands_cover_every_row_once_at_odd_heights() {
        for height in [
            1, 2, 7, 15, 16, 17, 179, 180, 719, 720, 1_079, 1_080, 1_439, 1_440, 2_161,
        ] {
            for count in [1, 2, 3, 4, 15, 16, 17, 63, 64] {
                let all: Vec<(u32, u32)> = bands(height, count).collect();
                assert_eq!(all.len(), count as usize);
                assert_eq!(all[0].0, 0, "{height}/{count}");
                assert_eq!(all[all.len() - 1].1, height, "{height}/{count}");
                for pair in all.windows(2) {
                    assert_eq!(pair[0].1, pair[1].0, "{height}/{count}: contiguous");
                }
                let rows: Vec<u32> = all.iter().map(|&(top, bottom)| bottom - top).collect();
                let (low, high) = (rows.iter().min().unwrap(), rows.iter().max().unwrap());
                assert!(high - low <= 1, "{height}/{count}: even");
                assert_eq!(*high, tallest_band(height, count), "{height}/{count}");
                if height >= count {
                    assert!(*low >= 1, "{height}/{count}: none empty");
                }
                assert_eq!(rows.iter().sum::<u32>(), height);
            }
        }
        // 1440 rows in 16 bands: 90 each, as the design has it.
        assert!(bands(1_440, 16).all(|(top, bottom)| bottom - top == 90));
    }

    #[test]
    fn the_ramp_climbs_by_four_and_stops_below_the_count() {
        assert_eq!(ramp_steps(Workload::Alu, 3_072), [16, 64, 256, 1_024]);
        assert_eq!(ramp_steps(Workload::Alu, 1_024), [16, 64, 256]);
        assert_eq!(ramp_steps(Workload::Alu, 17), [16]);
        assert!(ramp_steps(Workload::Alu, 16).is_empty());
        assert!(ramp_steps(Workload::Alu, 3).is_empty());
        assert_eq!(
            ramp_steps(Workload::Alu, 65_536),
            [16, 64, 256, 1_024, 4_096, 16_384]
        );
        assert_eq!(ramp_steps(Workload::Bw, 24), [4, 16]);
        assert!(ramp_steps(Workload::Bw, 4).is_empty());
        assert_eq!(ramp_steps(Workload::Bw, 4_096), [4, 16, 64, 256, 1_024]);
        // Every step is a valid count of its workload.
        for workload in Workload::ALL {
            assert!(
                ramp_steps(workload, workload.max_count())
                    .iter()
                    .all(|&c| workload.valid_count(c))
            );
        }
    }

    #[test]
    fn the_guards_predict_frames_and_bands() {
        // ×4 steps scale the median by 4; the last step by its own ratio.
        assert_eq!(predict_ms(1_024, 2.0, 4_096), 8.0);
        assert_eq!(predict_ms(1_024, 2.0, 3_072), 6.0);
        assert!(guard_step(&SCREEN, (1_024, 2.0), 3_072).is_ok());
        // 100 ms a frame is allowed, more isn't.
        assert!(guard_step(&SCREEN, (1_024, 25.0), 4_096).is_ok());
        assert_eq!(
            guard_step(&SCREEN, (1_024, 30.0), 4_096),
            Err(TooHeavy {
                count: 4_096,
                predicted_ms: 120.0
            })
        );
        // One band is the whole frame: 25 ms is the limit.
        let one = Geometry { bands: 1, ..SCREEN };
        assert!(guard_step(&one, (256, 6.25), 1_024).is_ok());
        assert_eq!(
            guard_step(&one, (256, 7.0), 1_024),
            Err(TooHeavy {
                count: 1_024,
                predicted_ms: 28.0
            })
        );
        // Four bands of 360 rows: the band limit and the frame limit meet at 100 ms.
        let four = Geometry { bands: 4, ..SCREEN };
        assert!(check(&four, 1, 100.0).is_ok());
        assert!(check(&four, 1, 100.1).is_err());
        // Three bands over 1080 rows: 360 rows, a third of the frame, so 75 ms is the limit.
        let three = Geometry {
            width: 1_920,
            height: 1_080,
            bands: 3,
        };
        assert!(check(&three, 1, 74.9).is_ok());
        assert!(check(&three, 1, 75.1).is_err());
        assert!(check(&SCREEN, 1, f64::NAN).is_err());
        assert!(check(&SCREEN, 1, f64::INFINITY).is_err());
    }

    #[test]
    fn a_step_is_timed_by_its_own_frames() {
        let second = HNS_PER_SECOND;
        let mut timer = StepTimer::new();
        assert_eq!(timer.id(), 0);
        // Nothing started: frames drawn outside a step carry id 0 and never count.
        timer.add(0, GpuTime::Us(5));
        let ramp = timer.start(100 * second, RAMP_FRAMES, 0);
        assert_ne!(ramp, 0);
        assert_eq!(timer.timing(100 * second), StepTiming::Running);
        for us in [900, 1_000, 950, 1_100, 1_050, 980, 1_020, 990, 1_010] {
            timer.add(ramp, GpuTime::Us(us));
        }
        // Nine timed frames, a disjoint one, one that never resolved, and one of step 0.
        timer.add(ramp, GpuTime::Disjoint);
        timer.add(ramp, GpuTime::Unknown);
        timer.add(0, GpuTime::Us(1));
        assert_eq!(timer.timing(101 * second), StepTiming::Running);
        timer.add(ramp, GpuTime::Us(1_005));
        assert_eq!(
            timer.timing(101 * second),
            StepTiming::Done {
                median_us: 1_000,
                frames: 10
            }
        );
        // The next step forgets the last; a late frame of the last one doesn't count.
        let verify = timer.start(102 * second, 2, VERIFY_SECONDS);
        assert_ne!(verify, ramp);
        timer.add(ramp, GpuTime::Us(1_000));
        timer.add(verify, GpuTime::Us(8_000));
        timer.add(verify, GpuTime::Us(8_200));
        // Enough frames, but not yet five seconds.
        assert_eq!(timer.timing(106 * second), StepTiming::Running);
        assert_eq!(
            timer.timing(107 * second),
            StepTiming::Done {
                median_us: 8_000,
                frames: 2
            }
        );
    }

    #[test]
    fn a_step_without_its_timed_frames_gives_up() {
        let second = HNS_PER_SECOND;
        let mut timer = StepTimer::new();
        let id = timer.start(0, CALIBRATE_FRAMES, 0);
        for _ in 0..1_000 {
            timer.add(id, GpuTime::Disjoint);
        }
        timer.add(id, GpuTime::Us(8_000));
        assert_eq!(timer.timing(STEP_LIMIT_HNS - 1), StepTiming::Running);
        assert_eq!(timer.timing(STEP_LIMIT_HNS), StepTiming::NoTiming);
        // A step's own length comes first.
        timer.start(0, 1, VERIFY_SECONDS);
        assert_eq!(
            timer.timing(STEP_LIMIT_HNS + 4 * second),
            StepTiming::Running
        );
        assert_eq!(
            timer.timing(STEP_LIMIT_HNS + 5 * second),
            StepTiming::NoTiming
        );
        // One timed frame is the least a step can need.
        let id = timer.start(0, 0, 0);
        timer.add(id, GpuTime::Us(3));
        assert_eq!(
            timer.timing(0),
            StepTiming::Done {
                median_us: 3,
                frames: 1
            }
        );
    }

    #[test]
    fn three_long_frames_in_a_row_end_a_run() {
        let mut guard = LongFrames::new();
        assert!(!guard.frame(Some(300.0)));
        assert!(!guard.frame(Some(300.0)));
        assert!(!guard.frame(Some(8.0)), "a normal frame breaks the streak");
        assert!(!guard.frame(Some(251.0)));
        assert!(!guard.frame(None), "an unknown frame doesn't break it");
        assert!(!guard.frame(Some(400.0)));
        assert!(guard.frame(Some(260.0)));
        let mut guard = LongFrames::new();
        assert!(
            !guard.frame(Some(LONG_FRAME_MS)),
            "250 ms itself isn't long"
        );
        assert!(!guard.frame(Some(LONG_FRAME_MS)));
        assert!(!guard.frame(Some(LONG_FRAME_MS)));
    }

    /// Three doubling points from a GPU whose time is `a + b · count`, times `wobble(i)`.
    fn points(a: f64, b: f64, counts: [u32; 3], wobble: impl Fn(usize) -> f64) -> Vec<(u32, f64)> {
        counts
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, (a + b * f64::from(c)) * wobble(i)))
            .collect()
    }

    #[test]
    fn the_fit_of_a_linear_gpu_is_exact() {
        let fit = fit(&points(0.3, 0.0025, [512, 1_024, 2_048], |_| 1.0)).unwrap();
        assert!((fit.a_ms - 0.3).abs() < 1e-12, "{fit:?}");
        assert!((fit.b_ms - 0.0025).abs() < 1e-15, "{fit:?}");
        assert!((fit.r2 - 1.0).abs() < 1e-12, "{fit:?}");
        assert!(fit.accepted());
        assert!((fit.count_for(8.0) - 3_080.0).abs() < 1e-9);
        assert!((fit.predict(3_080) - 8.0).abs() < 1e-12);
    }

    #[test]
    fn the_fit_of_a_noisy_gpu_is_accepted() {
        // ±1% timing noise.
        let fit = fit(&points(0.3, 0.0025, [512, 1_024, 2_048], |i| {
            [1.01, 0.99, 1.01][i]
        }))
        .unwrap();
        assert!(fit.accepted(), "{fit:?}");
        assert!(fit.r2 > 0.999, "{fit:?}");
        assert!((fit.b_ms / 0.0025 - 1.0).abs() < 0.03, "{fit:?}");
    }

    #[test]
    fn the_fit_of_a_non_linear_gpu_is_refused() {
        // Time growing with the square of the count.
        let square: Vec<(u32, f64)> = [1_u32, 2, 4]
            .iter()
            .map(|&c| (c, f64::from(c * c)))
            .collect();
        let fit_square = fit(&square).unwrap();
        assert!(
            fit_square.r2 < MIN_R2 && !fit_square.accepted(),
            "{fit_square:?}"
        );
        // A latency-bound plateau: more reads cost almost nothing more.
        let plateau = [(4, 1.0), (8, 1.9), (16, 2.2)];
        let fit_plateau = fit(&plateau).unwrap();
        assert!(
            fit_plateau.r2 < MIN_R2 && !fit_plateau.accepted(),
            "{fit_plateau:?}"
        );
        // A flat line: no cost per count.
        let flat = fit(&[(4, 2.0), (8, 2.0), (16, 2.0)]).unwrap();
        assert!(!flat.accepted(), "{flat:?}");
        // Nothing to fit.
        assert!(fit(&[(4, 2.0)]).is_none());
        assert!(fit(&[(4, 2.0), (4, 3.0)]).is_none());
    }

    /// Runs a calibration against a simulated GPU and returns each step it asked for and its end.
    fn calibrate(
        workload: Workload,
        geometry: Geometry,
        gpu: impl Fn(u32, Stage, u32) -> f64,
    ) -> (Vec<CalibrationStep>, Result<Calibrated, Refusal>) {
        let mut calibrator = Calibrator::new(workload, 8.0, 120, geometry);
        let mut seen = Vec::new();
        let mut step = calibrator.first();
        loop {
            seen.push(step);
            let next = match step {
                CalibrationStep::Measure {
                    stage,
                    step: k,
                    count,
                    ..
                } => calibrator.measured(gpu(count, stage, k)),
                CalibrationStep::Hold { .. } => calibrator.held(),
                CalibrationStep::Done(done) => return (seen, Ok(done)),
            };
            match next {
                Ok(next) => step = next,
                Err(refusal) => return (seen, Err(refusal)),
            }
            assert!(seen.len() < 64, "the calibration ends");
        }
    }

    fn counts_of(steps: &[CalibrationStep], wanted: Stage) -> Vec<u32> {
        steps
            .iter()
            .filter_map(|step| match *step {
                CalibrationStep::Measure { stage, count, .. } if stage == wanted => Some(count),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_linear_alu_calibrates_to_the_target() {
        let (steps, done) = calibrate(Workload::Alu, SCREEN, |count, _, _| {
            0.3 + 0.0025 * f64::from(count)
        });
        let done = done.unwrap();
        // 1,024 iterations take 2.86 ms, 2,048 take 5.42 ms: past half the target.
        assert_eq!(
            counts_of(&steps, Stage::Double),
            [16, 32, 64, 128, 256, 512, 1_024, 2_048]
        );
        assert!(steps.contains(&CalibrationStep::Hold {
            count: 2_048,
            seconds: 120
        }));
        assert_eq!(counts_of(&steps, Stage::Warm), [512, 1_024, 2_048]);
        assert_eq!(counts_of(&steps, Stage::Verify), [3_080]);
        assert_eq!(done.count, 3_080);
        assert_eq!(done.settled, Settled::Target);
        assert!((done.gpu_ms - 8.0).abs() < 1e-9);
        assert!(done.fit.accepted());
        // Every doubling step's frames and the verify's five seconds.
        for step in &steps {
            if let CalibrationStep::Measure {
                stage,
                frames,
                seconds,
                ..
            } = *step
            {
                assert_eq!(frames, CALIBRATE_FRAMES);
                assert_eq!(seconds, if stage == Stage::Verify { 5 } else { 0 });
            }
        }
    }

    #[test]
    fn a_noisy_fit_takes_one_correction() {
        // The warm points run about 10% fast, with ±1% noise: the fitted count overshoots the
        // target by about 10%, and one step along the fit's slope brings it within 3%.
        let (steps, done) = calibrate(Workload::Alu, SCREEN, |count, stage, k| {
            let real = 0.3 + 0.0025 * f64::from(count);
            match stage {
                Stage::Warm => real * [0.91, 0.89, 0.905][k as usize],
                _ => real,
            }
        });
        let done = done.unwrap();
        let verified = counts_of(&steps, Stage::Verify);
        assert_eq!(verified.len(), 2, "{steps:?}");
        assert!(verified[0] > verified[1], "{verified:?}");
        assert_eq!(done.count, verified[1]);
        assert_eq!(done.settled, Settled::Target);
        assert!((done.gpu_ms / 8.0 - 1.0).abs() <= ACCEPT_FRACTION);
    }

    #[test]
    fn a_bandwidth_count_settles_at_its_granularity() {
        // At a 4K-like size each read costs 0.8 ms, so a step of four reads is 3.2 ms: 8 ms can't
        // come within 3%. 8 reads take 6.6 ms, past half the target after two steps, so the
        // doubling goes on to a third.
        let (steps, done) = calibrate(Workload::Bw, SCREEN, |count, _, _| {
            0.2 + 0.8 * f64::from(count)
        });
        assert_eq!(counts_of(&steps, Stage::Double), [4, 8, 16]);
        assert_eq!(counts_of(&steps, Stage::Warm), [4, 8, 16]);
        let done = done.unwrap();
        // The fit asks for 9.75 reads: 8 is the nearest valid count, and the correction rounds
        // back to it.
        assert_eq!(counts_of(&steps, Stage::Verify), [8]);
        assert_eq!((done.count, done.settled), (8, Settled::Granularity));
    }

    #[test]
    fn between_two_neighbouring_counts_the_nearer_wins() {
        // Each read really costs 0.7 ms, but the warm points misjudge the slope (5 + 0.25 ms per
        // read): the fit asks for 12 reads, which take 8.6 ms, and its correction for 8, which
        // take 5.8 ms. The target lies between two neighbouring counts.
        let (steps, done) = calibrate(Workload::Bw, SCREEN, |count, stage, _| {
            if stage == Stage::Warm {
                5.0 + 0.25 * f64::from(count)
            } else {
                0.2 + 0.7 * f64::from(count)
            }
        });
        assert_eq!(counts_of(&steps, Stage::Verify), [12, 8], "{steps:?}");
        let done = done.unwrap();
        assert_eq!((done.count, done.settled), (12, Settled::Granularity));
        assert!((done.gpu_ms - 8.6).abs() < 1e-9);
    }

    #[test]
    fn a_count_that_wont_settle_is_refused() {
        // The ALU verify swings: 5% over, then 5% under at the corrected count. The two counts
        // aren't neighbours, so granularity can't explain it.
        let (steps, done) = calibrate(Workload::Alu, SCREEN, |count, stage, k| {
            let real = 0.3 + 0.0025 * f64::from(count);
            match (stage, k) {
                (Stage::Verify, 0) => real * 1.05,
                (Stage::Verify, _) => real * 0.95,
                _ => real,
            }
        });
        assert_eq!(counts_of(&steps, Stage::Verify).len(), 2);
        assert!(matches!(done, Err(Refusal::Unsettled { .. })), "{done:?}");
    }

    #[test]
    fn a_non_linear_gpu_is_refused_after_the_warm_up() {
        let (steps, done) = calibrate(Workload::Bw, SCREEN, |count, _, _| {
            // Latency-bound: the time flattens as reads grow.
            6.0 * (1.0 - (-f64::from(count) / 6.0).exp())
        });
        assert!(
            steps
                .iter()
                .any(|s| matches!(s, CalibrationStep::Hold { .. }))
        );
        assert!(matches!(done, Err(Refusal::NotLinear(_))), "{done:?}");
    }

    #[test]
    fn a_target_beyond_the_caps_is_refused() {
        // So fast that 65,536 iterations stay under 4 ms.
        let (steps, done) = calibrate(Workload::Alu, SCREEN, |count, _, _| {
            0.1 + 0.00001 * f64::from(count)
        });
        assert_eq!(counts_of(&steps, Stage::Double).last(), Some(&65_536));
        assert!(
            matches!(done, Err(Refusal::CapReached { count: 65_536, .. })),
            "{done:?}"
        );
        // So slow that even the first counts are over the target.
        let (_, done) = calibrate(Workload::Bw, SCREEN, |count, _, _| 5.0 * f64::from(count));
        assert!(matches!(done, Err(Refusal::OutOfReach { .. })), "{done:?}");
    }

    #[test]
    fn the_calibration_never_runs_a_step_the_guards_refuse() {
        // 16 iterations take 60 ms: 32 would be predicted at 120 ms.
        let (steps, done) = calibrate(Workload::Alu, SCREEN, |count, _, _| 3.75 * f64::from(count));
        assert_eq!(counts_of(&steps, Stage::Double), [16]);
        assert_eq!(
            done,
            Err(Refusal::TooHeavy(TooHeavy {
                count: 32,
                predicted_ms: 120.0
            }))
        );
        // A step that measured over the limits is refused too, before anything else runs on it.
        let (_, done) = calibrate(Workload::Alu, SCREEN, |_, _, _| 150.0);
        assert!(matches!(done, Err(Refusal::TooHeavy(_))), "{done:?}");
    }

    #[test]
    fn the_calibration_refuses_steps_out_of_order_and_missing_times() {
        let mut calibrator = Calibrator::new(Workload::Alu, 8.0, 120, SCREEN);
        assert_eq!(calibrator.held(), Err(Refusal::OutOfOrder));
        assert_eq!(calibrator.measured(f64::NAN), Err(Refusal::NoTiming));
        assert_eq!(calibrator.measured(0.0), Err(Refusal::NoTiming));
        for refusal in [
            Refusal::NoTiming,
            Refusal::OutOfOrder,
            Refusal::OutOfReach { count: 70_000.0 },
            Refusal::CapReached {
                count: 65_536,
                gpu_ms: 1.0,
            },
        ] {
            assert!(!refusal.message().contains('\\'));
        }
    }

    #[test]
    fn drift_and_scroll_are_exact_and_wrap_where_the_picture_repeats() {
        assert_eq!(drift_at(0), 0.0);
        assert_eq!(drift_at(5 * HNS_PER_SECOND), 0.25);
        assert_eq!(drift_at(DRIFT_PERIOD_HNS), 0.0);
        assert!(drift_at(DRIFT_PERIOD_HNS - 1) > 0.999_999_99);
        assert_eq!(drift_at(-5 * HNS_PER_SECOND), 0.75, "before the anchor too");
        assert_eq!(scroll_at(HNS_PER_SECOND), 120.0);
        assert_eq!(scroll_at(HNS_PER_SECOND / 4), 30.0);
        // 8,192 px at 120 px/s: the wrap is at 68.266… s.
        assert_eq!(scroll_at(682_666_667), 0.000_004);
        assert!(scroll_at(682_666_666) > 8_191.999_99);
        // A week in, the values are as exact as at the start: no drift from accumulated error.
        let week = 7 * 24 * 3_600 * HNS_PER_SECOND;
        assert_eq!(drift_at(week + 5 * HNS_PER_SECOND), 0.25);
        let at = week + 123_456_789;
        let exact = (i128::from(at) * 120 % (8_192 * 10_000_000)) as f64 / 1e7;
        assert_eq!(scroll_at(at), exact);
        assert!((0.0..8_192.0).contains(&scroll_at(-1)));
    }

    #[test]
    fn frame_constants_are_the_constant_buffer_byte_for_byte() {
        let c = FrameConstants::new(
            5 * HNS_PER_SECOND,
            24,
            Texture::new(DEFAULT_TEXTURE_MIB, DEFAULT_SEED),
            (2_560, 1_440),
        );
        assert_eq!(c.drift, 0.25);
        assert_eq!(c.scroll_px, 600.0);
        assert_eq!(c.inv_size, [1.0 / 2_560.0, 1.0 / 1_440.0]);
        assert_eq!(c.tex_mask, [8_191, 8_191]);
        let bytes = c.bytes();
        let word = |i: usize| u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
        assert_eq!(f32::from_bits(word(0)), 0.25);
        assert_eq!(f32::from_bits(word(1)), 600.0);
        assert_eq!(word(2), 24);
        assert_eq!(word(3), 0x4B4C_4731);
        assert_eq!(f32::from_bits(word(4)), 1.0 / 2_560.0);
        assert_eq!(f32::from_bits(word(5)), 1.0 / 1_440.0);
        assert_eq!((word(6), word(7)), (8_191, 8_191));
        // The same layout the struct has in memory (repr(C), no padding).
        assert_eq!(std::mem::offset_of!(FrameConstants, count), 8);
        assert_eq!(std::mem::offset_of!(FrameConstants, inv_size), 16);
        assert_eq!(std::mem::offset_of!(FrameConstants, tex_mask), 24);
        // A size of zero can't divide by zero.
        let zero = FrameConstants::new(0, 1, None, (0, 0));
        assert!(zero.inv_size.iter().all(|v| v.is_finite()));
        // The ALU workload reads no texture: no seed, no mask.
        let alu = FrameConstants::new(0, 3_072, None, (2_560, 1_440));
        assert_eq!((alu.seed, alu.tex_mask, alu.count), (0, [0, 0], 3_072));
    }

    #[test]
    fn a_texture_is_only_ever_a_valid_size() {
        let texture = Texture::new(DEFAULT_TEXTURE_MIB, DEFAULT_SEED).unwrap();
        assert_eq!(texture.mib(), 256);
        assert_eq!(texture.seed(), 0x4B4C_4731);
        assert_eq!(texture.size(), (8_192, 8_192));
        assert_eq!(texture.mask(), [8_191, 8_191]);
        // Windows CI's WARP size.
        let small = Texture::new(1, 7).unwrap();
        assert_eq!((small.size(), small.mask()), ((512, 512), [511, 511]));
        assert_eq!(Texture::new(1_024, 0).unwrap().size(), (16_384, 16_384));
        for mib in [0, 3, 768, 2_048] {
            assert_eq!(Texture::new(mib, 0), None, "{mib}");
        }
    }

    #[test]
    fn the_ramp_reaches_the_count_through_the_guards() {
        let gpu = |count: u32| 0.3 + 0.0025 * f64::from(count);
        let mut ramp = Ramp::new(Workload::Alu, 3_072, SCREEN);
        let mut drawn = Vec::new();
        while !ramp.done() {
            drawn.push((ramp.step(), ramp.current()));
            let next = ramp.measured(gpu(ramp.current())).unwrap();
            assert_eq!(next, ramp.current());
        }
        assert_eq!(
            drawn,
            [
                (Some(0), 16),
                (Some(1), 64),
                (Some(2), 256),
                (Some(3), 1_024)
            ]
        );
        assert_eq!((ramp.step(), ramp.current()), (None, 3_072));
        // Once at the count there is nothing left to report.
        assert_eq!(ramp.measured(8.0), Err(Refusal::OutOfOrder));
        // A count no larger than the first step needs no ramp.
        let tiny = Ramp::new(Workload::Bw, 4, SCREEN);
        assert!(tiny.done());
        assert_eq!(tiny.current(), 4);
    }

    #[test]
    fn the_ramp_refuses_before_a_long_frame_exists() {
        // Half a millisecond per iteration: 64 iterations take 32 ms, and 256 would take 128.
        let mut ramp = Ramp::new(Workload::Alu, 3_072, SCREEN);
        assert_eq!(ramp.measured(8.0), Ok(64));
        assert_eq!(
            ramp.measured(32.0),
            Err(Refusal::TooHeavy(TooHeavy {
                count: 256,
                predicted_ms: 128.0
            }))
        );
        // The refused count is never drawn.
        assert_eq!(ramp.current(), 64);
        // The last step is predicted by its own ratio: 1,024 at 25 ms predicts 75 ms for 3,072.
        let mut ramp = Ramp::new(Workload::Alu, 3_072, SCREEN);
        for ms in [0.4, 1.6, 6.25] {
            ramp.measured(ms).unwrap();
        }
        assert_eq!(ramp.measured(25.0), Ok(3_072));
        let mut ramp = Ramp::new(Workload::Alu, 3_072, SCREEN);
        for ms in [0.4, 1.6, 6.25] {
            ramp.measured(ms).unwrap();
        }
        assert!(matches!(ramp.measured(34.0), Err(Refusal::TooHeavy(_))));
        // A step the timestamps never timed can't be judged.
        let mut ramp = Ramp::new(Workload::Bw, 24, SCREEN);
        assert_eq!(ramp.measured(f64::NAN), Err(Refusal::NoTiming));
        assert_eq!(ramp.measured(0.0), Err(Refusal::NoTiming));
        assert_eq!(Refusal::NoTiming.exit_code(), exit::DEVICE);
        assert_eq!(
            Refusal::TooHeavy(TooHeavy {
                count: 1,
                predicted_ms: 1.0
            })
            .exit_code(),
            exit::REFUSED
        );
    }
}
