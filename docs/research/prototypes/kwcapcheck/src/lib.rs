#![cfg(windows)]
//! What using windows-capture 2.0.1 looks like for Kairo (compile-checked).
use std::time::Duration;

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D};
use windows::core::Interface;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

pub struct KairoHandler {
    device: ID3D11Device,        // created by the crate: default adapter, BGRA_SUPPORT only
    context: ID3D11DeviceContext,
    latest: Option<ID3D11Texture2D>,
}

impl GraphicsCaptureApiHandler for KairoHandler {
    type Flags = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        // The only way to "share" the device: adopt the crate's device for the encoder.
        let mt: ID3D11Multithread = ctx.device_context.cast()?;
        unsafe { let _ = mt.SetMultithreadProtected(true); }
        Ok(Self { device: ctx.device, context: ctx.device_context, latest: None })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _c: InternalCaptureControl) -> Result<(), Self::Error> {
        let _ts_100ns = frame.timestamp()?.Duration;        // SystemRelativeTime
        let src: &ID3D11Texture2D = frame.as_raw_texture();  // zero-copy, valid only inside this callback
        if let Some(dst) = &self.latest {
            unsafe { self.context.CopyResource(dst, src) };
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

pub fn start(monitor: Monitor) -> Result<CaptureControl<KairoHandler, Box<dyn std::error::Error + Send + Sync>>, Box<dyn std::error::Error>> {
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithCursor,
        DrawBorderSettings::WithoutBorder,            // errors out on Windows 10 (BorderConfigUnsupported)
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Custom(Duration::from_micros(8_333)), // errors out before 26100
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,                            // default is Rgba8; Rgba16F for HDR
        (),
    );
    Ok(KairoHandler::start_free_threaded(settings)?)
}
