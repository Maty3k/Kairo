//! DXGI Desktop Duplication source (Windows 10 default, Windows 11 fallback).
//! Polled from the GPU thread once per pacer tick: AcquireNextFrame(0) returns the newest image.
use windows::Win32::Foundation::{E_ACCESSDENIED, POINT};
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_INVALID_CALL, DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
    DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_DESC, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR, DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME, IDXGIOutput, IDXGIOutput5, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
use windows::core::{Interface, Result};

use crate::device::GpuDevice;

#[derive(Default)]
pub struct PointerState {
    pub visible: bool,
    /// Top-left of the shape in output coordinates (NOT the hotspot).
    pub pos: POINT,
    pub shape: Vec<u8>,
    pub info: DXGI_OUTDUPL_POINTER_SHAPE_INFO,
    pub kind: PointerKind,
    pub shape_dirty: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointerKind {
    #[default]
    None,
    Color,       // 32 bpp ARGB, straight alpha blend
    MaskedColor, // 32 bpp; alpha 0 = replace, 0xFF = XOR with the screen
    Monochrome,  // 1 bpp AND mask stacked over 1 bpp XOR mask: real height = info.Height / 2
}

pub enum Poll {
    NewImage { qpc: i64 },
    NoChange,
    AccessLost,
}

pub enum DupError {
    /// Secure desktop (UAC, lock screen, Ctrl+Alt+Del): wait for EVENT_SYSTEM_DESKTOPSWITCH, retry.
    AccessDenied,
    /// Unsupported mode, or the DDA-on-dGPU hybrid laptop case (KB3019314): wait for WM_DISPLAYCHANGE.
    Unsupported,
    /// Four processes already duplicate this session.
    TooManyDuplicators,
    SessionDisconnected,
    Other(windows::core::Error),
}

pub struct DdaSource {
    output: IDXGIOutput5,
    dupl: Option<IDXGIOutputDuplication>,
    pub desc: DXGI_OUTDUPL_DESC,
    formats: Vec<DXGI_FORMAT>,
    holding: bool,
    pub pointer: PointerState,
    pub access_lost_count: u64,
}

impl DdaSource {
    /// `output` must come from the adapter `dev` was created on (DuplicateOutput1 docs).
    pub fn new(output: &IDXGIOutput, hdr: bool) -> Result<Self> {
        let output: IDXGIOutput5 = output.cast()?;
        // Always include BGRA (MS docs). FP16 first only when the monitor is in HDR mode.
        let formats = if hdr {
            vec![DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM]
        } else {
            vec![DXGI_FORMAT_B8G8R8A8_UNORM]
        };
        Ok(Self {
            output,
            dupl: None,
            desc: DXGI_OUTDUPL_DESC::default(),
            formats,
            holding: false,
            pointer: PointerState::default(),
            access_lost_count: 0,
        })
    }

    pub fn duplicate(&mut self, dev: &GpuDevice) -> std::result::Result<(), DupError> {
        self.dupl = None; // release the old duplication before asking for a new one
        self.holding = false;
        // FFmpeg ddagrab sets per-monitor-v2 on the calling thread around DuplicateOutput1; without
        // PMv2 it is reported to fail with DXGI_ERROR_UNSUPPORTED. Also declare PMv2 in the manifest.
        let prev = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let r = unsafe { self.output.DuplicateOutput1(&dev.device, 0, &self.formats) };
        if !prev.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(prev) };
        }
        match r {
            Ok(d) => {
                self.desc = unsafe { d.GetDesc() };
                self.dupl = Some(d);
                Ok(())
            }
            Err(e) if e.code() == E_ACCESSDENIED => Err(DupError::AccessDenied),
            Err(e) if e.code() == DXGI_ERROR_UNSUPPORTED => Err(DupError::Unsupported),
            Err(e) if e.code() == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE => Err(DupError::TooManyDuplicators),
            Err(e) if e.code() == DXGI_ERROR_SESSION_DISCONNECTED => Err(DupError::SessionDisconnected),
            Err(e) => Err(DupError::Other(e)),
        }
    }

    fn lost(&mut self) -> Poll {
        self.dupl = None;
        self.holding = false;
        self.access_lost_count += 1; // feed the freeze watchdog (OBS #14021: ~15 ACCESS_LOST/s loops)
        Poll::AccessLost
    }

    /// `dst` must have the duplication's size/format (self.desc.ModeDesc).
    pub fn poll(&mut self, dev: &GpuDevice, dst: &ID3D11Texture2D, timeout_ms: u32) -> Result<Poll> {
        let Some(dupl) = self.dupl.clone() else { return Ok(Poll::AccessLost) };
        if self.holding {
            // MS: release just before the next AcquireNextFrame (lets DXGI track only dirty regions).
            match unsafe { dupl.ReleaseFrame() } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => return Ok(self.lost()),
                Err(e) if e.code() == DXGI_ERROR_INVALID_CALL => {}
                Err(e) => return Err(e),
            }
            self.holding = false;
        }
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        match unsafe { dupl.AcquireNextFrame(timeout_ms, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(Poll::NoChange),
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => return Ok(self.lost()),
            Err(e) => return Err(e),
        }
        self.holding = true;

        if info.LastMouseUpdateTime != 0 {
            self.update_pointer(&dupl, &info)?;
        }
        // Pointer-only update: LastPresentTime/AccumulatedFrames are 0 and (FFmpeg) the surface
        // format can even flicker - keep using our last copy.
        if info.LastPresentTime == 0 || info.AccumulatedFrames == 0 {
            return Ok(Poll::NoChange);
        }
        let Some(res) = res else { return Ok(Poll::NoChange) };
        let tex: ID3D11Texture2D = res.cast()?;
        let mut d = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut d) };
        if d.Format != self.desc.ModeDesc.Format && !self.formats.contains(&d.Format) {
            return Ok(Poll::NoChange); // unexpected format: skip, never panic
        }
        unsafe { dev.context.CopyResource(dst, &tex) };
        Ok(Poll::NewImage { qpc: info.LastPresentTime }) // raw QPC ticks -> clock::qpc_to_hns
    }

    fn update_pointer(&mut self, dupl: &IDXGIOutputDuplication, info: &DXGI_OUTDUPL_FRAME_INFO) -> Result<()> {
        self.pointer.visible = info.PointerPosition.Visible.as_bool();
        self.pointer.pos = info.PointerPosition.Position;
        if info.PointerShapeBufferSize > 0 {
            self.pointer.shape.resize(info.PointerShapeBufferSize as usize, 0);
            let mut required = 0u32;
            unsafe {
                dupl.GetFramePointerShape(
                    info.PointerShapeBufferSize,
                    self.pointer.shape.as_mut_ptr().cast(),
                    &mut required,
                    &mut self.pointer.info,
                )?
            };
            // Type is a u32 in windows 0.62.2 while the constants are DXGI_OUTDUPL_POINTER_SHAPE_TYPE(i32).
            let t = self.pointer.info.Type as i32;
            self.pointer.kind = if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 {
                PointerKind::Color
            } else if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 {
                PointerKind::MaskedColor
            } else if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 {
                PointerKind::Monochrome
            } else {
                PointerKind::None
            };
            self.pointer.shape_dirty = true;
        }
        Ok(())
    }
}
