//! WGC source: free-threaded frame pool on our shared device; FrameArrived only hands the frame
//! to the GPU thread (mailbox), which copies it out and closes it.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{E_INVALIDARG, HWND};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface, Result};

use crate::device::GpuDevice;
use crate::winrt_init::WgcCaps;

#[derive(Clone, Copy)]
pub enum Target {
    Window(HWND),
    Monitor(HMONITOR),
}

pub struct WgcConfig {
    pub format: DXGI_FORMAT, // DXGI_FORMAT_B8G8R8A8_UNORM or DXGI_FORMAT_R16G16B16A16_FLOAT (HDR)
    pub target_fps: u32,     // output CFR, e.g. 60
    pub cursor: bool,
}

/// Latest-frame mailbox shared between the WGC worker thread and the GPU thread.
#[derive(Default)]
pub struct Mailbox {
    slot: Mutex<Option<Direct3D11CaptureFrame>>,
    cv: Condvar,
    pub closed: AtomicBool,
}

impl Mailbox {
    /// GPU thread: wait until a frame arrives or `deadline` passes (the next pacer tick).
    pub fn wait_frame(&self, deadline: Instant) -> Option<Direct3D11CaptureFrame> {
        let mut slot = self.slot.lock().unwrap();
        while slot.is_none() && !self.closed.load(Ordering::Acquire) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            slot = self.cv.wait_timeout(slot, deadline - now).unwrap().0;
        }
        slot.take()
    }
}

pub fn create_item(target: Target) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    match target {
        Target::Monitor(hmon) => unsafe { interop.CreateForMonitor(hmon) },
        Target::Window(hwnd) => {
            // QTBUG-116025: CreateForWindow can fail transiently with E_INVALIDARG; Qt retries 10 x 100 ms.
            let mut last = None;
            for _ in 0..10 {
                match unsafe { interop.CreateForWindow::<GraphicsCaptureItem>(hwnd) } {
                    Ok(item) => return Ok(item),
                    Err(e) if e.code() == E_INVALIDARG => {
                        last = Some(e);
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(last.unwrap())
        }
    }
}

pub struct WgcSource {
    pub item: GraphicsCaptureItem,
    pub pool: Direct3D11CaptureFramePool,
    pub session: GraphicsCaptureSession,
    pub mailbox: Arc<Mailbox>,
    pub pool_size: SizeInt32,
    pub format: DXGI_FORMAT,
    frame_token: i64,
    closed_token: i64,
}

pub enum Consumed {
    /// Copied; `ts_hns` is SystemRelativeTime (100 ns, QPC timeline).
    Frame { ts_hns: i64, content: SizeInt32 },
    /// Pool recreated at a new size: recreate ring textures, re-check the encoder canvas.
    Resized(SizeInt32),
    /// Stale frame from before a Recreate (or format switch): skipped.
    Dropped,
}

pub fn pixel_format(f: DXGI_FORMAT) -> DirectXPixelFormat {
    DirectXPixelFormat(f.0) // numeric values are shared: 87 = B8G8R8A8UIntNormalized, 10 = R16G16B16A16Float
}

impl WgcSource {
    pub fn start(dev: &GpuDevice, caps: &WgcCaps, target: Target, cfg: &WgcConfig) -> Result<Self> {
        let item = create_item(target)?;
        let size = item.Size()?;
        // 2 buffers (OBS, FFmpeg, Sunshine); free-threaded => no DispatcherQueue / message loop needed.
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &dev.winrt,
            pixel_format(cfg.format),
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;

        if caps.cursor_toggle {
            session.SetIsCursorCaptureEnabled(cfg.cursor)?;
        }
        if caps.border_toggle {
            // FFmpeg: "can return failure but still work". Ignored (border shown) if consent was denied.
            let _ = session.SetIsBorderRequired(false);
        }
        if caps.min_update_interval {
            // OBS PR #12759 and Apollo PR #785 use half the output frame time (8.33 ms at 60 fps).
            let ivl = Duration::from_nanos(1_000_000_000 / (2 * cfg.target_fps as u64));
            session.SetMinUpdateInterval(TimeSpan::from(ivl))?;
        }
        if caps.secondary_windows {
            session.SetIncludeSecondaryWindows(false)?; // default is false; keep pop-ups out of clips
        }

        let mailbox = Arc::new(Mailbox::default());

        let mb = mailbox.clone();
        let frame_token = pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |sender, _| {
                let Some(pool) = sender.as_ref() else { return Ok(()) };
                let frame = match pool.TryGetNextFrame() {
                    Ok(f) => f,
                    // A null frame comes back as an "empty" error whose code() is S_OK.
                    Err(e) if e.code().is_ok() => return Ok(()),
                    Err(e) => return Err(e),
                };
                let mut slot = mb.slot.lock().unwrap();
                if let Some(older) = slot.replace(frame) {
                    let _ = older.Close(); // GPU thread is behind: newest wins, buffer goes back to the pool
                }
                drop(slot);
                mb.cv.notify_one();
                Ok(())
            },
        ))?;

        let mb = mailbox.clone();
        let closed_token = item.Closed(&TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(
            move |_, _| {
                // Window closed, monitor unplugged, or the app swapped its window (MS remark).
                mb.closed.store(true, Ordering::Release);
                mb.cv.notify_one();
                Ok(())
            },
        ))?;

        session.StartCapture()?;
        Ok(Self { item, pool, session, mailbox, pool_size: size, format: cfg.format, frame_token, closed_token })
    }

    /// GPU thread: copy the frame into our own texture (ring slot), then return the buffer at once.
    pub fn consume(
        &mut self,
        dev: &GpuDevice,
        frame: Direct3D11CaptureFrame,
        dst: &ID3D11Texture2D,
    ) -> Result<Consumed> {
        let content = frame.ContentSize()?;
        if content.Width <= 0 || content.Height <= 0 {
            let _ = frame.Close(); // defensive: never Recreate with an empty size
            return Ok(Consumed::Dropped);
        }
        if content.Width != self.pool_size.Width || content.Height != self.pool_size.Height {
            // Surface still has the old size: drop it and resize the pool (all queued frames are discarded).
            let _ = frame.Close();
            self.pool.Recreate(&dev.winrt, pixel_format(self.format), 2, content)?;
            self.pool_size = content;
            return Ok(Consumed::Resized(content));
        }
        let ts = frame.SystemRelativeTime()?.Duration;
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let src: ID3D11Texture2D = unsafe { access.GetInterface()? };
        // OBS: "need GetDesc because ContentSize is not reliable" - a frame from before Recreate
        // can still have the old surface size; CopyResource needs identical sizes and formats.
        let mut s = D3D11_TEXTURE2D_DESC::default();
        let mut d = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            src.GetDesc(&mut s);
            dst.GetDesc(&mut d);
        }
        if (s.Width, s.Height, s.Format) != (d.Width, d.Height, d.Format) {
            let _ = frame.Close();
            return Ok(Consumed::Dropped);
        }
        unsafe { dev.context.CopyResource(dst, &src) };
        drop(src);
        frame.Close()?;
        Ok(Consumed::Frame { ts_hns: ts, content })
    }

    /// HDR toggled on the captured monitor (detected via IDXGIFactory1::IsCurrent polling).
    pub fn switch_format(&mut self, dev: &GpuDevice, format: DXGI_FORMAT) -> Result<()> {
        self.format = format;
        self.pool.Recreate(&dev.winrt, pixel_format(format), 2, self.pool_size)
    }
}

impl Drop for WgcSource {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.pool.Close();
        let _ = self.session.Close();
        if let Some(f) = self.mailbox.slot.lock().unwrap().take() {
            let _ = f.Close();
        }
    }
}

/// Ring texture the frame is copied into (shader-readable for the convert pass).
pub fn create_ring_texture(dev: &GpuDevice, w: u32, h: u32, format: DXGI_FORMAT) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { dev.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
    Ok(tex.unwrap())
}
