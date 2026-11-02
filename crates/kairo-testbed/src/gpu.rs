// SPDX-License-Identifier: GPL-3.0-or-later
//! D3D11 and DXGI for the testbed (docs/research/16-verify-testbed/README.md §4.1–4.4, §4.8,
//! §4.9): the device on the adapter that drives the monitor, chosen by LUID; a flip-discard swap
//! chain paced one of three ways ([`Pacing`]); exclusive fullscreen on the monitor's own output,
//! and the `ResizeBuffers` that losing it needs; the HDR colour space and a
//! watch on the monitor's HDR state; frame statistics; the monitor's own vblank clock, for swap
//! chains without statistics; and the renderer, which carries out a [`draw::Plan`] with
//! `ClearRenderTargetView`, `ClearView`, `UpdateSubresource` and `CopySubresourceRegion` only.
//! No shader runs.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use kairo_display::Luid;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND, LUID, RECT, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
    ID3D11DeviceContext1, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_DEVICE_REMOVED,
    DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_FRAME_STATISTICS_DISJOINT, DXGI_FRAME_STATISTICS,
    DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice1, IDXGIFactory2,
    IDXGIFactory4, IDXGIOutput, IDXGIOutput6, IDXGISwapChain1, IDXGISwapChain2, IDXGISwapChain3,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::{BOOL, HRESULT, Interface};

use crate::draw::{self, Copy, Format, Plan};
use crate::pace::{Period, Stats, WakeGrid};

/// The monitor's output, as DXGI reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputHdr {
    /// Whether the output is in HDR (PQ).
    pub pq: bool,
    /// Its peak, in nits.
    pub max_nits: f32,
}

/// The device on the monitor's adapter.
pub struct Gpu {
    factory: IDXGIFactory2,
    output: Option<IDXGIOutput>,
    device: ID3D11Device,
    context: ID3D11DeviceContext1,
    /// The adapter's DXGI index, valid in this process only.
    pub adapter_index: u32,
    /// The monitor's output index on it, if DXGI lists it.
    pub output_index: Option<u32>,
}

fn luid_of(luid: Luid) -> LUID {
    LUID {
        LowPart: luid.low,
        HighPart: luid.high,
    }
}

impl Gpu {
    /// A D3D11 device on the adapter with `luid`, and the output that shows `hmonitor`.
    pub fn on_adapter(luid: Luid, hmonitor: isize) -> Result<Gpu, String> {
        let factory = new_factory().ok_or("no DXGI factory")?;
        // SAFETY: looks the adapter up by the LUID kairo-display found for the monitor.
        let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByLuid(luid_of(luid)) }
            .map_err(|e| format!("no adapter with LUID {luid}: {e}"))?;
        let mut adapter_index = 0;
        for i in 0.. {
            // SAFETY: enumeration ends with an error.
            let Ok(a) = (unsafe { factory.EnumAdapters1(i) }) else {
                break;
            };
            // SAFETY: reads the adapter's description.
            if unsafe { a.GetDesc1() }.is_ok_and(|d| d.AdapterLuid == luid_of(luid)) {
                adapter_index = i;
                break;
            }
        }
        let (output, output_index) = match find_output(&adapter, hmonitor) {
            Some((output, index)) => (Some(output), Some(index)),
            None => (None, None),
        };
        let plain: IDXGIAdapter = adapter.cast().map_err(|e| format!("adapter: {e}"))?;
        let (device, context) = create_device(Some(&plain), D3D_DRIVER_TYPE_UNKNOWN)?;
        Ok(Gpu {
            factory: factory.cast().map_err(|e| format!("DXGI factory: {e}"))?,
            output,
            device,
            context,
            adapter_index,
            output_index,
        })
    }

    /// Whether the monitor's output is in HDR, and its peak (README §4.8).
    pub fn output_hdr(&self) -> Option<OutputHdr> {
        read_hdr(self.output.as_ref()?)
    }

    /// A swap chain for window `hwnd`'s client area of `width` × `height` in `format`, paced as
    /// `pacing` says.
    pub fn swap_chain(
        &self,
        hwnd: isize,
        width: u32,
        height: u32,
        format: Format,
        pacing: Pacing,
    ) -> Result<Chain, String> {
        let dxgi_format = dxgi_format(format);
        let flags = if pacing == Pacing::Waitable {
            DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32
        } else {
            0
        };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: dxgi_format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: if pacing == Pacing::Timer { 3 } else { 2 },
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: flags,
            ..Default::default()
        };
        let window = HWND(hwnd as *mut _);
        // SAFETY: our device, our window and a complete description.
        let swap = unsafe {
            self.factory
                .CreateSwapChainForHwnd(&self.device, window, &desc, None, None)
        }
        .map_err(|e| format!("swap chain: {e}"))?;
        // SAFETY: this factory made the swap chain for this live window. NO_ALT_ENTER stops only
        // Alt+Enter; DXGI still leaves fullscreen when the window loses focus (leave_if_lost).
        let _ = unsafe {
            self.factory
                .MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER)
        };
        let mut waitable_handle = None;
        match pacing {
            Pacing::Waitable => {
                let swap2: IDXGISwapChain2 =
                    swap.cast().map_err(|e| format!("swap chain 2: {e}"))?;
                // SAFETY: the swap chain was made with the waitable flag.
                unsafe { swap2.SetMaximumFrameLatency(1) }.map_err(|e| format!("latency: {e}"))?;
                // SAFETY: as above; the handle is ours to close (Drop).
                waitable_handle = Some(unsafe { swap2.GetFrameLatencyWaitableObject() });
            }
            Pacing::Blocking => {
                let device: IDXGIDevice1 =
                    self.device.cast().map_err(|e| format!("device: {e}"))?;
                // SAFETY: our own device.
                unsafe { device.SetMaximumFrameLatency(1) }.map_err(|e| format!("latency: {e}"))?;
            }
            Pacing::Timer => {}
        }
        if let Format::Fp16 { .. } = format {
            let swap3: IDXGISwapChain3 = swap.cast().map_err(|e| format!("swap chain 3: {e}"))?;
            // SAFETY: asks whether this colour space can be presented.
            let support =
                unsafe { swap3.CheckColorSpaceSupport(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709) }
                    .unwrap_or(0);
            if support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT.0 as u32 == 0 {
                return Err("the output can't present scRGB".to_owned());
            }
            // SAFETY: a supported colour space.
            unsafe { swap3.SetColorSpace1(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709) }
                .map_err(|e| format!("colour space: {e}"))?;
        }
        let (texture, rtv) = back_buffer(&self.device, &swap)?;
        let renderer = Renderer::new(&self.device, width, height, format)?;
        Ok(Chain {
            swap,
            waitable: waitable_handle,
            flags,
            format,
            width,
            height,
            texture: Some(texture),
            rtv: Some(rtv),
            renderer,
            context: self.context.clone(),
            device: self.device.clone(),
            output: self.output.clone(),
            fullscreen: false,
        })
    }
}

/// The output of `adapter` that shows `hmonitor`, and its index.
fn find_output(adapter: &IDXGIAdapter1, hmonitor: isize) -> Option<(IDXGIOutput, u32)> {
    for i in 0.. {
        // SAFETY: enumeration ends with an error.
        let Ok(output) = (unsafe { adapter.EnumOutputs(i) }) else {
            return None;
        };
        // SAFETY: reads the output's description.
        if unsafe { output.GetDesc() }.is_ok_and(|d| d.Monitor == HMONITOR(hmonitor as *mut _)) {
            return Some((output, i));
        }
    }
    None
}

fn read_hdr(output: &IDXGIOutput) -> Option<OutputHdr> {
    let output: IDXGIOutput6 = output.cast().ok()?;
    // SAFETY: reads the output's description.
    let desc = unsafe { output.GetDesc1() }.ok()?;
    Some(OutputHdr {
        pq: desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
        max_nits: desc.MaxLuminance,
    })
}

fn new_factory() -> Option<IDXGIFactory4> {
    // SAFETY: plain factory creation.
    unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.ok()
}

/// The output of the adapter with `luid` that shows `hmonitor`, through `factory`.
fn output_of(factory: &IDXGIFactory4, luid: Luid, hmonitor: isize) -> Option<IDXGIOutput> {
    // SAFETY: looks the adapter up by its LUID.
    let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByLuid(luid_of(luid)) }.ok()?;
    Some(find_output(&adapter, hmonitor)?.0)
}

/// Watches the monitor's HDR state (README §4.8) through a DXGI factory of its own. A factory
/// goes stale at the first display change and stays stale, so after each change the watch reads
/// the state through a new one and keeps it: `IsCurrent` stays a cheap check until the next
/// change.
pub struct HdrWatch {
    factory: IDXGIFactory4,
    luid: Luid,
    hmonitor: isize,
}

impl HdrWatch {
    /// A watch on the monitor `hmonitor` of the adapter with `luid`.
    pub fn new(luid: Luid, hmonitor: isize) -> Option<HdrWatch> {
        Some(HdrWatch {
            factory: new_factory()?,
            luid,
            hmonitor,
        })
    }

    /// `None` while the displays haven't changed; after a change, whether the monitor's output
    /// is in HDR (PQ) now (no, if it can't be read).
    pub fn changed(&mut self) -> Option<bool> {
        // SAFETY: reads our factory's state.
        if unsafe { self.factory.IsCurrent() }.as_bool() {
            return None;
        }
        let factory = new_factory()?;
        let pq = output_of(&factory, self.luid, self.hmonitor)
            .and_then(|output| read_hdr(&output))
            .is_some_and(|o| o.pq);
        self.factory = factory;
        Some(pq)
    }
}

/// `latest` before the first vblank.
const NO_VBLANK: i64 = i64::MIN;

/// The monitor's own vblank clock (README §4.9), for swap chains without frame statistics: a
/// thread that waits for each vblank of the monitor's output (`IDXGIOutput::WaitForVBlank`,
/// through a factory of its own) and keeps the latest vblank's time, as [`WakeGrid`] places it.
pub struct VblankClock {
    latest: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl VblankClock {
    /// Starts the clock on the output of the adapter with `luid` that shows `hmonitor`, whose
    /// refresh period is `period`.
    pub fn start(luid: Luid, hmonitor: isize, period: Period) -> VblankClock {
        let latest = Arc::new(AtomicI64::new(NO_VBLANK));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, done) = mpsc::channel();
        let thread = {
            let latest = Arc::clone(&latest);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("vblank clock".to_owned())
                .spawn(move || {
                    tick(luid, hmonitor, period, &latest, &stop);
                    let _ = sender.send(());
                })
                .ok()
        };
        VblankClock {
            latest,
            stop,
            done,
            thread,
        }
    }

    /// The latest vblank's QPC time in 100 ns, once there has been one.
    pub fn latest(&self) -> Option<i64> {
        let t = self.latest.load(Ordering::Relaxed);
        (t != NO_VBLANK).then_some(t)
    }
}

/// The clock's thread: vblank after vblank until `stop`, or until the output can't be found or
/// fails 100 waits in a row.
fn tick(luid: Luid, hmonitor: isize, period: Period, latest: &AtomicI64, stop: &AtomicBool) {
    let Some(output) = new_factory().and_then(|f| output_of(&f, luid, hmonitor)) else {
        return;
    };
    let mut grid = WakeGrid::new(period);
    let mut failures = 0;
    while !stop.load(Ordering::Relaxed) {
        // SAFETY: waits on this thread's own output object.
        if unsafe { output.WaitForVBlank() }.is_err() {
            failures += 1;
            if failures == 100 {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        failures = 0;
        latest.store(grid.on_wake(crate::platform::now_hns()), Ordering::Relaxed);
    }
}

impl Drop for VblankClock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The thread ends at its next vblank; one that a stopped output holds is left behind
        // rather than waited for.
        if self.done.recv_timeout(Duration::from_millis(200)).is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

fn dxgi_format(format: Format) -> DXGI_FORMAT {
    match format {
        Format::Bgra8 => DXGI_FORMAT_B8G8R8A8_UNORM,
        Format::Fp16 { .. } => DXGI_FORMAT_R16G16B16A16_FLOAT,
    }
}

/// A device on `adapter`, or of `driver` when there is none (WARP in the tests).
fn create_device(
    adapter: Option<&IDXGIAdapter>,
    driver: D3D_DRIVER_TYPE,
) -> Result<(ID3D11Device, ID3D11DeviceContext1), String> {
    let mut device = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: out-pointers to locals; feature levels 11.1 then 11.0.
    unsafe {
        D3D11CreateDevice(
            adapter,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| format!("D3D11 device: {e}"))?;
    let device = device.ok_or("D3D11 device: none")?;
    let context = context
        .ok_or("D3D11 context: none")?
        .cast::<ID3D11DeviceContext1>()
        .map_err(|e| format!("D3D11.1 context: {e}"))?;
    Ok((device, context))
}

fn back_buffer(
    device: &ID3D11Device,
    swap: &IDXGISwapChain1,
) -> Result<(ID3D11Texture2D, ID3D11RenderTargetView), String> {
    // SAFETY: buffer 0 is the current back buffer of a flip-model swap chain in D3D11.
    let texture: ID3D11Texture2D =
        unsafe { swap.GetBuffer(0) }.map_err(|e| format!("back buffer: {e}"))?;
    let mut rtv = None;
    // SAFETY: a view of our own texture; out-pointer to a local.
    unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut rtv)) }
        .map_err(|e| format!("render target: {e}"))?;
    Ok((texture, rtv.ok_or("render target: none")?))
}

/// A texture of `width` × `height` in `format` that only copies use.
fn copy_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: Format,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: dxgi_format(format),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: 0,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: a complete description and an out-pointer to a local.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(|e| format!("texture: {e}"))?;
    texture.ok_or_else(|| "texture: none".to_owned())
}

/// How a swap chain is paced (README §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacing {
    /// Vsync in a window: two buffers and a frame-latency waitable object at latency 1.
    Waitable,
    /// Vsync in exclusive fullscreen, where D3D11 has no waitable object: two buffers, the
    /// device's frame latency at 1, and a `Present` that blocks until there is room.
    Blocking,
    /// Timer pacing: three buffers and the default latency, so `Present(0, 0)` never waits for a
    /// vblank, and the display shows the newest frame at each.
    Timer,
}

/// What `GetFrameStatistics` said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statistics {
    /// A reading.
    Read(Stats),
    /// `DXGI_ERROR_FRAME_STATISTICS_DISJOINT`: the first call, or a change such as fullscreen to
    /// windowed, after which counts don't compare with the ones before.
    Disjoint,
    /// Nothing to say, yet or at all.
    Nothing,
}

/// `D3DDDIERR_DEVICEREMOVED`, which `Present` returns for a card removed or a driver upgraded.
const D3DDDIERR_DEVICEREMOVED: HRESULT = HRESULT(0x8876_0870_u32 as i32);

/// Why presenting failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentError {
    /// The device was removed or reset (`e device_lost`).
    DeviceLost(String),
    /// Anything else.
    Other(String),
}

/// A swap chain and what draws into it.
pub struct Chain {
    swap: IDXGISwapChain1,
    waitable: Option<HANDLE>,
    flags: u32,
    format: Format,
    width: u32,
    height: u32,
    texture: Option<ID3D11Texture2D>,
    rtv: Option<ID3D11RenderTargetView>,
    renderer: Renderer,
    context: ID3D11DeviceContext1,
    device: ID3D11Device,
    output: Option<IDXGIOutput>,
    fullscreen: bool,
}

impl Chain {
    /// The swap chain's format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Waits up to `ms` for the swap chain to take another frame; `true` if it will.
    pub fn wait(&self, ms: u32) -> bool {
        match self.waitable {
            // SAFETY: the swap chain's own waitable object, open until Drop.
            Some(handle) => (unsafe { WaitForSingleObject(handle, ms) }) == WAIT_OBJECT_0,
            None => true,
        }
    }

    /// Draws `plan` into the back buffer.
    pub fn draw(&self, plan: &Plan) {
        if let (Some(texture), Some(rtv)) = (&self.texture, &self.rtv) {
            self.renderer.draw(&self.context, texture, rtv, plan);
        }
    }

    /// `Present(interval, 0)`. A failure is a lost device when the device says it was removed
    /// or reset, whatever code `Present` gave.
    pub fn present(&self, interval: u32) -> Result<(), PresentError> {
        // SAFETY: presents our own swap chain; no flags.
        let result = unsafe { self.swap.Present(interval, DXGI_PRESENT(0)) };
        // DXGI_STATUS_OCCLUDED is a success: the frame still counts.
        if result.is_ok() {
            return Ok(());
        }
        // SAFETY: reads whether, and why, the device went away.
        let removed = unsafe { self.device.GetDeviceRemovedReason() };
        let lost = [
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_DEVICE_RESET,
            D3DDDIERR_DEVICEREMOVED,
        ];
        if removed.is_err() || lost.contains(&result) {
            return Err(PresentError::DeviceLost(format!(
                "{result:?} ({removed:?})"
            )));
        }
        result
            .ok()
            .map_err(|e| PresentError::Other(format!("present: {e}")))
    }

    /// The number of `Present` calls so far (`GetLastPresentCount`).
    pub fn present_count(&self) -> Option<u32> {
        // SAFETY: reads our own swap chain's counter.
        unsafe { self.swap.GetLastPresentCount() }.ok()
    }

    /// The frame statistics.
    pub fn stats(&self) -> Statistics {
        let mut s = DXGI_FRAME_STATISTICS::default();
        // SAFETY: `s` is ours.
        match unsafe { self.swap.GetFrameStatistics(&mut s) } {
            Ok(()) => Statistics::Read(Stats {
                present_count: s.PresentCount,
                present_refresh: s.PresentRefreshCount,
                sync_refresh: s.SyncRefreshCount,
                sync_hns: crate::platform::qpc_to_hns(s.SyncQPCTime),
            }),
            Err(e) if e.code() == DXGI_ERROR_FRAME_STATISTICS_DISJOINT => Statistics::Disjoint,
            Err(_) => Statistics::Nothing,
        }
    }

    /// Exclusive fullscreen on the monitor's output (README §4.1): `SetFullscreenState`, then
    /// `ResizeBuffers` as flip-model swap chains need, then `GetFullscreenState` must agree.
    /// The window must have the foreground.
    pub fn enter_fullscreen(&mut self) -> Result<(), String> {
        let output = self
            .output
            .clone()
            .ok_or("DXGI lists no output for the monitor")?;
        // SAFETY: our swap chain, on the output that shows our window's monitor.
        unsafe { self.swap.SetFullscreenState(true, &output) }
            .map_err(|e| format!("SetFullscreenState: {e}"))?;
        self.fullscreen = true;
        self.resize_buffers()?;
        if self.is_fullscreen() {
            Ok(())
        } else {
            Err("GetFullscreenState says windowed".to_owned())
        }
    }

    /// Whether DXGI has taken the swap chain out of exclusive fullscreen (the window lost
    /// focus); if so, the `ResizeBuffers` a flip-model swap chain needs before its next
    /// `Present` (MS), and the run goes on in the window.
    pub fn leave_if_lost(&mut self) -> Result<bool, String> {
        if !self.fullscreen || self.is_fullscreen() {
            return Ok(false);
        }
        self.fullscreen = false;
        self.resize_buffers()?;
        Ok(true)
    }

    /// `ResizeBuffers` at the same count, size and format, as a change between fullscreen and
    /// windowed needs; then the new back buffer.
    fn resize_buffers(&mut self) -> Result<(), String> {
        // Every reference to the back buffer must go first.
        self.rtv = None;
        self.texture = None;
        // SAFETY: unbinds everything from our immediate context, then submits what it queued,
        // with the views just released, so nothing still holds the old buffers.
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
        // SAFETY: our swap chain, with no references to its buffers left; the flags it was made
        // with.
        unsafe {
            self.swap.ResizeBuffers(
                0,
                self.width,
                self.height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(self.flags as i32),
            )
        }
        .map_err(|e| format!("ResizeBuffers: {e}"))?;
        let (texture, rtv) = back_buffer(&self.device, &self.swap)?;
        self.texture = Some(texture);
        self.rtv = Some(rtv);
        Ok(())
    }

    /// Whether the swap chain is in exclusive fullscreen.
    pub fn is_fullscreen(&self) -> bool {
        let mut state = BOOL(0);
        // SAFETY: `state` is ours; the output isn't asked for.
        unsafe { self.swap.GetFullscreenState(Some(&mut state), None) }.is_ok() && state.as_bool()
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        if self.fullscreen {
            // A swap chain must leave fullscreen before it is released.
            // SAFETY: our swap chain; no output.
            let _ = unsafe { self.swap.SetFullscreenState(false, None) };
        }
        if let Some(handle) = self.waitable.take() {
            // SAFETY: the waitable object's handle, which the swap chain gave us to close.
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}

/// The row and scratch textures, and the plan's operations.
struct Renderer {
    rows: ID3D11Texture2D,
    scratch: ID3D11Texture2D,
    width: u32,
    bytes_per_pixel: u32,
}

impl Renderer {
    fn new(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<Renderer, String> {
        Ok(Renderer {
            rows: copy_texture(device, width, draw::ROW_TEXTURE_ROWS, format)?,
            scratch: copy_texture(device, width, draw::scratch_rows(height), format)?,
            width,
            bytes_per_pixel: format.bytes_per_pixel() as u32,
        })
    }

    /// Draws `plan` into `target` through `rtv`: black, then the band copies, then the clears.
    fn draw(
        &self,
        context: &ID3D11DeviceContext1,
        target: &ID3D11Texture2D,
        rtv: &ID3D11RenderTargetView,
        plan: &Plan,
    ) {
        let rows = |top: u32, n: u32| D3D11_BOX {
            left: 0,
            top,
            front: 0,
            right: self.width,
            bottom: top + n,
            back: 1,
        };
        // SAFETY: our own view and colour.
        unsafe { context.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 1.0]) };
        if plan.row_count > 0 {
            let area = rows(0, plan.row_count);
            // SAFETY: `plan.rows` holds row_count rows of width pixels in the texture's format,
            // as the row pitch says, and outlives the call.
            unsafe {
                context.UpdateSubresource(
                    &self.rows,
                    0,
                    Some(&area),
                    plan.rows.as_ptr().cast(),
                    self.width * self.bytes_per_pixel,
                    0,
                )
            };
        }
        for copy in &plan.copies {
            let (dst, y, src, area) = match *copy {
                Copy::RowToTarget { row, y } => (target, y, &self.rows, rows(row, 1)),
                Copy::TargetToScratch { y, rows: n } => (&self.scratch, 0, target, rows(y, n)),
                Copy::ScratchToTarget { y, rows: n } => (target, y, &self.scratch, rows(0, n)),
            };
            // SAFETY: different textures of one format and size class; the boxes lie inside
            // both (draw::plan sizes them from the same client size).
            unsafe { context.CopySubresourceRegion(dst, 0, 0, y, 0, src, 0, Some(&area)) };
        }
        for clear in &plan.clears {
            let rects: Vec<RECT> = clear
                .rects
                .iter()
                .map(|r| RECT {
                    left: r.left,
                    top: r.top,
                    right: r.right,
                    bottom: r.bottom,
                })
                .collect();
            // SAFETY: our own view; the rectangles lie inside it.
            unsafe { context.ClearView(rtv, &clear.colour, Some(&rects)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kairo_core::testsig::{Load, Options, Patches, Pattern, Scene, Size};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
        D3D11_USAGE_STAGING,
    };

    fn texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: Format,
        staging: bool,
    ) -> ID3D11Texture2D {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: dxgi_format(format),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: if staging {
                D3D11_USAGE_STAGING
            } else {
                D3D11_USAGE_DEFAULT
            },
            BindFlags: if staging {
                0
            } else {
                D3D11_BIND_RENDER_TARGET.0 as u32
            },
            CPUAccessFlags: if staging {
                D3D11_CPU_ACCESS_READ.0 as u32
            } else {
                0
            },
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: a complete description and an out-pointer to a local.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }.unwrap();
        texture.unwrap()
    }

    /// Draws `plan` on WARP into a `width` × `height` texture and reads it back.
    fn draw_on_warp(plan: &Plan, format: Format, width: u32, height: u32) -> Vec<u8> {
        let (device, context) = create_device(None, D3D_DRIVER_TYPE_WARP).unwrap();
        let target = texture(&device, width, height, format, false);
        let mut rtv = None;
        // SAFETY: a view of our own texture; an out-pointer to a local.
        unsafe { device.CreateRenderTargetView(&target, None, Some(&mut rtv)) }.unwrap();
        let renderer = Renderer::new(&device, width, height, format).unwrap();
        renderer.draw(&context, &target, rtv.as_ref().unwrap(), plan);
        let staging = texture(&device, width, height, format, true);
        // SAFETY: two textures of one size and format, neither mapped.
        unsafe { context.CopyResource(&staging, &target) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: maps our staging texture for reading; unmapped below.
        unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.unwrap();
        let row = width as usize * format.bytes_per_pixel();
        let mut out = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            // SAFETY: Map gave `height` rows of RowPitch bytes, each at least `row` long.
            let line = unsafe {
                std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    row,
                )
            };
            out.extend_from_slice(line);
        }
        // SAFETY: the texture mapped above, after its last read.
        unsafe { context.Unmap(&staging, 0) };
        out
    }

    fn first_difference(got: &[u8], want: &[u8], width: u32, bpp: usize) -> String {
        match got.iter().zip(want).position(|(a, b)| a != b) {
            Some(i) => {
                let pixel = i / bpp;
                let (x, y) = (pixel % width as usize, pixel / width as usize);
                let at = pixel * bpp;
                format!(
                    "first difference at ({x}, {y}): {:?} drawn, {:?} wanted",
                    &got[at..at + bpp],
                    &want[at..at + bpp]
                )
            }
            None => format!("lengths {} and {}", got.len(), want.len()),
        }
    }

    /// README §4.4's WARP test: the renderer's operations on a WARP device equal `testsig`'s CPU
    /// raster byte for byte.
    #[test]
    fn warp_draws_pattern_v1_byte_for_byte() {
        for (w, h) in [(1_920, 1_080), (2_560, 1_440)] {
            let size = Size::new(w, h).unwrap();
            for frame in [0, 1, 179, 180, 181, 5_000] {
                let options = Options {
                    pattern: Pattern::Full,
                    nominal_fps: 180,
                    run_tag: 93,
                    load: Load::Scroll,
                    patches: Patches::Sdr,
                };
                let scene = Scene::new(size, frame, &options).unwrap();
                let plan = draw::plan(&scene, Format::Bgra8).unwrap();
                let drawn = draw_on_warp(&plan, Format::Bgra8, w, h);
                let want: Vec<u8> = scene
                    .raster_srgb()
                    .unwrap()
                    .into_iter()
                    .flat_map(|[r, g, b]| [b, g, r, 255])
                    .collect();
                assert!(
                    drawn == want,
                    "{w}x{h} frame {frame}: {}",
                    first_difference(&drawn, &want, w, 4)
                );
            }
        }
    }

    /// The motion pattern, the other loads, 720p and an odd size on a device, where boxes reach
    /// the client's last row and bands split where they wrap.
    #[test]
    fn warp_draws_motion_loads_and_odd_sizes() {
        for (w, h) in [(1_280, 720), (641, 361)] {
            let size = Size::new(w, h).unwrap();
            let mut cases = vec![(Pattern::Full, Load::None, 33)];
            for frame in [150, 200, 700, 800, 1_079] {
                for load in [Load::Scroll, Load::Churn] {
                    cases.push((Pattern::Motion, load, frame));
                }
            }
            for (pattern, load, frame) in cases {
                let options = Options {
                    pattern,
                    nominal_fps: 60,
                    run_tag: 5,
                    load,
                    patches: Patches::Sdr,
                };
                let scene = Scene::new(size, frame, &options).unwrap();
                let plan = draw::plan(&scene, Format::Bgra8).unwrap();
                let drawn = draw_on_warp(&plan, Format::Bgra8, w, h);
                let want = draw::reference(&scene, Format::Bgra8).unwrap();
                assert!(
                    drawn == want,
                    "{w}x{h} {pattern:?} {load:?} frame {frame}: {}",
                    first_difference(&drawn, &want, w, 4)
                );
            }
        }
        // FP16, with the patches above SDR white.
        let format = Format::Fp16 {
            sdr_white_nits: 203.0,
        };
        let options = Options {
            pattern: Pattern::Motion,
            nominal_fps: 60,
            run_tag: 5,
            load: Load::Scroll,
            patches: Patches::Hdr {
                sdr_white_nits: 203.0,
                bright: true,
            },
        };
        let scene = Scene::new(Size::new(1_280, 720).unwrap(), 1_079, &options).unwrap();
        let plan = draw::plan(&scene, format).unwrap();
        let drawn = draw_on_warp(&plan, format, 1_280, 720);
        let want = draw::reference(&scene, format).unwrap();
        assert!(
            drawn == want,
            "{}",
            first_difference(&drawn, &want, 1_280, 8)
        );
    }

    /// The same operations in FP16 scRGB, against the CPU reference.
    #[test]
    fn warp_draws_hdr_in_scrgb() {
        let format = Format::Fp16 {
            sdr_white_nits: 240.0,
        };
        let options = Options {
            pattern: Pattern::Full,
            nominal_fps: 60,
            run_tag: 5,
            load: Load::Scroll,
            patches: Patches::Hdr {
                sdr_white_nits: 240.0,
                bright: false,
            },
        };
        let scene = Scene::new(Size::new(1_920, 1_080).unwrap(), 61, &options).unwrap();
        let plan = draw::plan(&scene, format).unwrap();
        let drawn = draw_on_warp(&plan, format, 1_920, 1_080);
        let want = draw::reference(&scene, format).unwrap();
        assert!(
            drawn == want,
            "{}",
            first_difference(&drawn, &want, 1_920, 8)
        );
    }
}
