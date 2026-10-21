use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind, GraphicsCaptureSession};
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Foundation::Metadata::ApiInformation;
use windows::core::{h, Result};

/// RAII guard: every engine thread that touches WinRT/COM calls this first.
pub struct MtaGuard;

impl MtaGuard {
    pub fn enter() -> Result<Self> {
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Ok(MtaGuard), // S_OK or S_FALSE (already MTA)
            // The thread is already STA (e.g. a GUI thread). Never capture from such a thread:
            // spawn a dedicated thread instead.
            Err(e) if e.code() == RPC_E_CHANGED_MODE => Err(e),
            Err(e) => Err(e),
        }
    }
}

impl Drop for MtaGuard {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

/// Feature probes for the capability report (do these instead of build-number checks).
pub struct WgcCaps {
    pub supported: bool,
    pub cursor_toggle: bool,       // 19041+
    pub border_toggle: bool,       // 20348+ (Windows 11 on clients)
    pub min_update_interval: bool, // 26100+
    pub secondary_windows: bool,   // 26100+
    pub dirty_regions: bool,       // 26100+
}

pub fn probe_wgc() -> Result<WgcCaps> {
    let t = h!("Windows.Graphics.Capture.GraphicsCaptureSession");
    let has = |p: &windows::core::HSTRING| ApiInformation::IsPropertyPresent(t, p);
    Ok(WgcCaps {
        supported: GraphicsCaptureSession::IsSupported()?,
        cursor_toggle: has(h!("IsCursorCaptureEnabled"))?,
        border_toggle: has(h!("IsBorderRequired"))?,
        min_update_interval: has(h!("MinUpdateInterval"))?,
        secondary_windows: has(h!("IncludeSecondaryWindows"))?,
        dirty_regions: has(h!("DirtyRegionMode"))?,
    })
}

/// Ask once at startup (MTA thread, never on a UI/STA thread). For unpackaged apps no
/// prompt is documented; OBS calls this before IsBorderRequired(false). Log the result.
pub fn request_borderless() -> Result<AppCapabilityAccessStatus> {
    GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)?.join()
}
