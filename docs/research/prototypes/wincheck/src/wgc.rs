use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Condvar, Mutex};
use windows::core::{h, IInspectable, Interface, Result};
use windows::Foundation::{Metadata::ApiInformation, TypedEventHandler};
use windows::Graphics::Capture::*;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::Foundation::{E_INVALIDARG, HWND};
use windows::Win32::Graphics::{Direct3D11::*, Gdi::HMONITOR};
use windows::Win32::System::WinRT::{Direct3D11::IDirect3DDxgiInterfaceAccess, Graphics::Capture::IGraphicsCaptureItemInterop};

#[derive(Default)]
pub struct Mailbox { pub slot: Mutex<Option<Direct3D11CaptureFrame>>, pub cv: Condvar, pub closed: AtomicBool }
pub struct WgcCaps { pub supported: bool, pub cursor: bool, pub border: bool, pub min_interval: bool, pub secondary: bool }

pub fn caps() -> Result<WgcCaps> {
    let t = h!("Windows.Graphics.Capture.GraphicsCaptureSession");
    Ok(WgcCaps { supported: GraphicsCaptureSession::IsSupported()?,
        cursor: ApiInformation::IsPropertyPresent(t, h!("IsCursorCaptureEnabled"))?,       // 19041
        border: ApiInformation::IsPropertyPresent(t, h!("IsBorderRequired"))?,             // 20348
        min_interval: ApiInformation::IsPropertyPresent(t, h!("MinUpdateInterval"))?,      // 26100
        secondary: ApiInformation::IsPropertyPresent(t, h!("IncludeSecondaryWindows"))? }) // 26100
}
/// Once at startup, on an MTA worker (never .join() on an STA thread). Log the status; then always
/// SetIsBorderRequired(false) - ignored (not an error) if consent was denied.
pub fn request_borderless() -> Result<AppCapabilityAccessStatus> {
    GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)?.join()
}

pub enum Target { Monitor(HMONITOR), Window(HWND) }
pub fn create_item(t: &Target) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    match t {
        Target::Monitor(m) => unsafe { interop.CreateForMonitor(*m) },
        Target::Window(w) => {
            let mut last = None;
            for _ in 0..10 { // transient E_INVALIDARG right after a window appears (Qt retries 10 x 100 ms)
                match unsafe { interop.CreateForWindow::<GraphicsCaptureItem>(*w) } {
                    Ok(i) => return Ok(i),
                    Err(e) if e.code() == E_INVALIDARG => { last = Some(e); std::thread::sleep(std::time::Duration::from_millis(100)); }
                    Err(e) => return Err(e),
                }
            }
            Err(last.unwrap())
        }
    }
}

pub struct WgcSource { pub item: GraphicsCaptureItem, pub pool: Direct3D11CaptureFramePool, pub session: GraphicsCaptureSession,
                       frame_tok: i64, closed_tok: i64, pub size: SizeInt32, pub fp16: bool }

impl WgcSource {
    pub fn start(dev: &crate::device::GpuDevice, t: &Target, fp16: bool, caps: &WgcCaps, refresh_hz: f64, clip_fps: f64,
                 cursor: bool, mb: Arc<Mailbox>) -> Result<Self> {
        let item = create_item(t)?;
        let size = item.Size()?;
        let fmt = if fp16 { DirectXPixelFormat::R16G16B16A16Float } else { DirectXPixelFormat::B8G8R8A8UIntNormalized };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&dev.winrt, fmt, 2, size)?; // 2 buffers, no DispatcherQueue
        let session = pool.CreateCaptureSession(&item)?;
        if caps.cursor { session.SetIsCursorCaptureEnabled(cursor)?; }
        if caps.border { let _ = session.SetIsBorderRequired(false); }
        if caps.min_interval { let _ = crate::wgc_rate::apply(&session, refresh_hz, clip_fps)?; }
        if caps.secondary { session.SetIncludeSecondaryWindows(false)?; }
        let m = mb.clone();
        let frame_tok = pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
            let Some(pool) = sender.as_ref() else { return Ok(()) };
            let frame = match pool.TryGetNextFrame() {
                Ok(f) => f,
                Err(e) if e.code().is_ok() => return Ok(()), // "no frame" = Err with HRESULT(0)
                Err(e) => return Err(e),
            };
            if let Some(old) = m.slot.lock().unwrap().replace(frame) { let _ = old.Close(); } // newest wins
            m.cv.notify_one();
            Ok(())
        }))?;
        let m = mb.clone();
        let closed_tok = item.Closed(&TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            m.closed.store(true, Ordering::Release); m.cv.notify_one(); Ok(()) // window closed/swapped, monitor gone
        }))?;
        session.StartCapture()?;
        Ok(Self { item, pool, session, frame_tok, closed_tok, size, fp16 })
    }

    /// GPU thread, under the GpuLock. Returns the frame's QPC time (100 ns) or None if dropped/resized.
    pub fn consume(&mut self, dev: &crate::device::GpuDevice, frame: Direct3D11CaptureFrame, dst: &ID3D11Texture2D) -> Result<Option<i64>> {
        let content = frame.ContentSize()?;
        if content.Width <= 0 || content.Height <= 0 { let _ = frame.Close(); return Ok(None); }
        if content != self.size {
            let _ = frame.Close();
            let fmt = if self.fp16 { DirectXPixelFormat::R16G16B16A16Float } else { DirectXPixelFormat::B8G8R8A8UIntNormalized };
            self.pool.Recreate(&dev.winrt, fmt, 2, content)?; // caller re-creates its ring textures
            self.size = content;
            return Ok(None);
        }
        let ts = frame.SystemRelativeTime()?.Duration;
        let src: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
        let (mut s, mut d) = (D3D11_TEXTURE2D_DESC::default(), D3D11_TEXTURE2D_DESC::default());
        unsafe { src.GetDesc(&mut s); dst.GetDesc(&mut d); }
        if (s.Width, s.Height, s.Format) != (d.Width, d.Height, d.Format) { let _ = frame.Close(); return Ok(None); }
        unsafe { dev.context.CopyResource(dst, &src) };
        drop(src);
        frame.Close()?;
        Ok(Some(ts))
    }
}
impl Drop for WgcSource {
    fn drop(&mut self) { // OBS order: revoke FrameArrived, close pool, close session
        let _ = self.pool.RemoveFrameArrived(self.frame_tok);
        let _ = self.item.RemoveClosed(self.closed_tok);
        let _ = self.pool.Close();
        let _ = self.session.Close();
    }
}
