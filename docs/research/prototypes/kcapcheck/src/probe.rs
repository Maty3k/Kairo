//! `kairo-capture-probe` core: run WGC on a monitor for N seconds on a dedicated GPU thread,
//! pace at 60 Hz on QPC, and report delivery statistics (what the PC spike needs).
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::core::Result;

use crate::clock::now_hns;
use crate::device::{adapter_for_monitor, create_device, factory};
use crate::wgc::{create_ring_texture, Consumed, Target, WgcConfig, WgcSource};
use crate::winrt_init::{probe_wgc, MtaGuard};

#[derive(Debug, Default)]
pub struct ProbeStats {
    pub delivered: u64,
    pub resized: u64,
    pub ticks: u64,
    pub ticks_with_new_frame: u64,
    pub delta_hist_ms: [u64; 32], // 0..31 ms buckets of SystemRelativeTime deltas
    pub max_staleness_hns: i64,   // tick time - timestamp of the frame chosen for that tick
    pub closed: bool,
}

pub fn run_wgc_probe(monitor: HMONITOR, hdr: bool, seconds: u64, fps: u32) -> Result<ProbeStats> {
    let _mta = MtaGuard::enter()?; // this thread is the GPU thread: owns the immediate context
    let caps = probe_wgc()?;
    let f = factory()?;
    let adapter = adapter_for_monitor(&f, monitor)?.map(|(a, _)| a);
    let dev = create_device(adapter.as_ref())?;
    let format = if hdr { DXGI_FORMAT_R16G16B16A16_FLOAT } else { DXGI_FORMAT_B8G8R8A8_UNORM };
    let cfg = WgcConfig { format, target_fps: fps, cursor: true };
    let mut src = WgcSource::start(&dev, &caps, Target::Monitor(monitor), &cfg)?;

    let mut ring = create_ring_texture(&dev, src.pool_size.Width as u32, src.pool_size.Height as u32, format)?;
    let mut stats = ProbeStats::default();
    let mut last_ts: Option<i64> = None;
    let mut newest_ts: Option<i64> = None;
    let tick = Duration::from_nanos(1_000_000_000 / fps as u64);
    let start = Instant::now();
    let mut next_tick = start + tick;

    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(frame) = src.mailbox.wait_frame(next_tick) {
            match src.consume(&dev, frame, &ring)? {
                Consumed::Frame { ts_hns: ts, .. } => {
                    stats.delivered += 1;
                    if let Some(prev) = last_ts {
                        let ms = ((ts - prev) / 10_000).clamp(0, 31) as usize;
                        stats.delta_hist_ms[ms] += 1;
                    }
                    last_ts = Some(ts);
                    newest_ts = Some(ts);
                }
                Consumed::Resized(size) => {
                    // CopyResource needs equal sizes: recreate the ring to match the new pool size.
                    stats.resized += 1;
                    ring = create_ring_texture(&dev, size.Width as u32, size.Height as u32, format)?;
                }
                Consumed::Dropped => {}
            }
        }
        if src.mailbox.closed.load(std::sync::atomic::Ordering::Acquire) {
            stats.closed = true;
            break;
        }
        if Instant::now() >= next_tick {
            // Pacer tick: a real engine runs the convert shader on the newest ring slot here
            // (re-encoding the previous one if nothing new arrived).
            stats.ticks += 1;
            if let Some(ts) = newest_ts.take() {
                stats.ticks_with_new_frame += 1;
                stats.max_staleness_hns = stats.max_staleness_hns.max(now_hns() - ts);
            }
            next_tick += tick;
        }
    }
    Ok(stats)
}
