// SPDX-License-Identifier: GPL-3.0-or-later
//! D3D11 and DXGI pieces M1's instruments share (docs/research/17-loadgen-bench/README.md §3.1;
//! written for the testbed, research 16 §4.1, §4.8): the device on the adapter that drives the
//! monitor, chosen by LUID, with the adapter's DXGI index and the output that shows the monitor;
//! the output's HDR state, and a watch on it; a swap chain's back buffer 0 and its render target
//! view; `Present` with an occluded window and a lost device told apart; and a swap chain's
//! frame-latency wait. Each program makes its own swap chain and draws its own way.

use std::fmt;

use kairo_display::Luid;
use windows::Win32::Foundation::{
    CloseHandle, DXGI_STATUS_OCCLUDED, HANDLE, HMODULE, LUID, S_OK, WAIT_OBJECT_0,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11DeviceContext1, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_DEVICE_REMOVED,
    DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory2,
    IDXGIFactory4, IDXGIOutput, IDXGIOutput6, IDXGISwapChain1, IDXGISwapChain2,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::{HRESULT, Interface};

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

    /// Whether the monitor's output is in HDR, and its peak (research 16 §4.8).
    pub fn output_hdr(&self) -> Option<OutputHdr> {
        read_hdr(self.output.as_ref()?)
    }

    /// The DXGI factory the device's adapter came from, for the program's swap chain.
    pub fn factory(&self) -> &IDXGIFactory2 {
        &self.factory
    }

    /// The device.
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// Its immediate context.
    pub fn context(&self) -> &ID3D11DeviceContext1 {
        &self.context
    }

    /// The output that shows the monitor, if DXGI lists it (for exclusive fullscreen).
    pub fn output(&self) -> Option<&IDXGIOutput> {
        self.output.as_ref()
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

/// A new DXGI factory.
pub fn new_factory() -> Option<IDXGIFactory4> {
    // SAFETY: plain factory creation.
    unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.ok()
}

/// The output of the adapter with `luid` that shows `hmonitor`, through `factory`.
pub fn output_of(factory: &IDXGIFactory4, luid: Luid, hmonitor: isize) -> Option<IDXGIOutput> {
    // SAFETY: looks the adapter up by its LUID.
    let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByLuid(luid_of(luid)) }.ok()?;
    Some(find_output(&adapter, hmonitor)?.0)
}

/// Watches the monitor's HDR state (research 16 §4.8) through a DXGI factory of its own. A
/// factory goes stale at the first display change and stays stale, so after each change the
/// watch reads the state through a new one and keeps it: `IsCurrent` stays a cheap check until
/// the next change.
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

/// A device on `adapter`, or of `driver` when there is none (WARP in the tests).
pub fn create_device(
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

/// Back buffer 0 of a flip-model swap chain made on `device`, and a render target view of it.
pub fn back_buffer(
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

/// `D3DDDIERR_DEVICEREMOVED`, which `Present` returns for a card removed or a driver upgraded.
const D3DDDIERR_DEVICEREMOVED: HRESULT = HRESULT(0x8876_0870_u32 as i32);

/// What a `Present` that succeeded returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presented {
    /// The frame was queued (`S_OK`).
    Queued,
    /// `DXGI_STATUS_OCCLUDED`: none of the window can be seen. A success: the frame still
    /// counts, and loadgen counts these (`occluded=`).
    Occluded,
}

/// Why presenting failed, with the HRESULTs a log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentError {
    /// The device was removed or reset (the testbed's and loadgen's `e device_lost`).
    DeviceLost {
        /// What `Present` returned.
        hr: HRESULT,
        /// Why the device went: `GetDeviceRemovedReason`'s HRESULT (`S_OK` if it says the device
        /// is still there).
        reason: HRESULT,
    },
    /// Anything else, with what `Present` returned: `DXGI_ERROR_INVALID_CALL` when a condition
    /// of the present flags broke, for one (loadgen's `e present_failed`).
    Other(HRESULT),
}

impl fmt::Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost { hr, reason } => write!(
                f,
                "Present returned {:#010x}, and the device's removed reason is {:#010x}",
                hr.0 as u32, reason.0 as u32
            ),
            Self::Other(hr) => write!(f, "Present returned {:#010x}", hr.0 as u32),
        }
    }
}

/// `Present(interval, flags)` on `swap`, a swap chain made on `device`. A failure is a lost
/// device when the device says it was removed or reset, whatever code `Present` gave.
pub fn present(
    device: &ID3D11Device,
    swap: &IDXGISwapChain1,
    interval: u32,
    flags: DXGI_PRESENT,
) -> Result<Presented, PresentError> {
    // SAFETY: presents the program's own swap chain, with the flags it asked for.
    let result = unsafe { swap.Present(interval, flags) };
    if result.is_ok() {
        return Ok(if result == DXGI_STATUS_OCCLUDED {
            Presented::Occluded
        } else {
            Presented::Queued
        });
    }
    // SAFETY: reads whether, and why, the device went away.
    let removed = unsafe { device.GetDeviceRemovedReason() };
    let lost = [
        DXGI_ERROR_DEVICE_REMOVED,
        DXGI_ERROR_DEVICE_RESET,
        D3DDDIERR_DEVICEREMOVED,
    ];
    if removed.is_err() || lost.contains(&result) {
        return Err(PresentError::DeviceLost {
            hr: result,
            reason: removed.map_or_else(|e| e.code(), |()| S_OK),
        });
    }
    Err(PresentError::Other(result))
}

/// A swap chain's frame-latency waitable object: the swap chain takes another frame when it is
/// signalled. The swap chain must have been made with
/// `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT`; the handle is closed in Drop.
pub struct LatencyWait(HANDLE);

impl LatencyWait {
    /// Sets `swap`'s maximum frame latency to `latency` frames and takes its waitable object.
    pub fn new(swap: &IDXGISwapChain1, latency: u32) -> Result<LatencyWait, String> {
        let swap2: IDXGISwapChain2 = swap.cast().map_err(|e| format!("swap chain 2: {e}"))?;
        // SAFETY: the swap chain was made with the waitable flag (DXGI refuses the call if not).
        unsafe { swap2.SetMaximumFrameLatency(latency) }.map_err(|e| format!("latency: {e}"))?;
        // SAFETY: as above; the handle is ours to close (Drop).
        let handle = unsafe { swap2.GetFrameLatencyWaitableObject() };
        if handle.is_invalid() {
            return Err("latency: no waitable object".to_owned());
        }
        Ok(LatencyWait(handle))
    }

    /// Waits up to `ms` for the swap chain to take another frame; `true` if it will.
    pub fn wait(&self, ms: u32) -> bool {
        // SAFETY: the swap chain's own waitable object, open until Drop.
        (unsafe { WaitForSingleObject(self.0, ms) }) == WAIT_OBJECT_0
    }
}

impl Drop for LatencyWait {
    fn drop(&mut self) {
        // SAFETY: the waitable object's handle, which the swap chain gave us to close, closed
        // once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;

    /// WARP makes a device the way a program's adapter does: feature level 11.0 or 11.1, with
    /// the D3D11.1 context the renderers use.
    #[test]
    fn warp_makes_a_level_11_device() {
        let (device, _context) = create_device(None, D3D_DRIVER_TYPE_WARP).unwrap();
        // SAFETY: reads our own device's feature level.
        let level = unsafe { device.GetFeatureLevel() };
        assert!(level == D3D_FEATURE_LEVEL_11_1 || level == D3D_FEATURE_LEVEL_11_0);
    }
}
