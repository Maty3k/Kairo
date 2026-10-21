//! Wrap Kairo's ID3D11Device as an FFmpeg D3D11VA device + NV12 frames context. Raw pointers only:
//! this module never names a `windows` type, so it builds the same on Windows and in Mac checks.
use core::ffi::c_void;
use ffmpeg_next::{self as ff, ffi};

/// Mirror of libavutil/hwcontext_d3d11va.h (FFmpeg 9.0.2: BindFlags/MiscFlags follow lock_ctx).
/// Use it instead of ffi::AVD3D11VADeviceContext so builds never depend on libclang seeing <d3d11.h>.
#[repr(C)] #[allow(non_snake_case)]
pub struct AVD3D11VADeviceContext {
    pub device: *mut c_void,         // ID3D11Device*, Released by FFmpeg on uninit => pass an AddRef'd pointer
    pub device_context: *mut c_void, // filled by init
    pub video_device: *mut c_void,   // filled by init (QI ID3D11VideoDevice: needs D3D11_CREATE_DEVICE_VIDEO_SUPPORT)
    pub video_context: *mut c_void,
    pub lock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub lock_ctx: *mut c_void,       // NOT freed by FFmpeg
    pub BindFlags: u32,
    pub MiscFlags: u32,
}
#[repr(C)] pub struct AVD3D11FrameDescriptor { pub texture: *mut c_void, pub index: isize }
#[repr(C)] #[allow(non_snake_case)]
pub struct AVD3D11VAFramesContext { pub texture: *mut c_void, pub BindFlags: u32, pub MiscFlags: u32, pub texture_infos: *mut AVD3D11FrameDescriptor }

pub struct HwRef(pub *mut ffi::AVBufferRef);
impl Drop for HwRef { fn drop(&mut self) { unsafe { ffi::av_buffer_unref(&mut self.0) } } }
unsafe impl Send for HwRef {}

pub fn check(r: i32) -> anyhow::Result<i32> { if r < 0 { anyhow::bail!("{}", ff::Error::from(r)) } Ok(r) }

/// `device_addrefd` = `gpu.device.clone().into_raw()`; `lock_ctx` = `GpuLock::ffmpeg_ctx()`.
pub unsafe fn d3d11va_device(device_addrefd: *mut c_void, lock_ctx: *mut c_void,
    lock: unsafe extern "C" fn(*mut c_void), unlock: unsafe extern "C" fn(*mut c_void)) -> anyhow::Result<HwRef> { unsafe {
    let mut r = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
    anyhow::ensure!(!r.is_null(), "av_hwdevice_ctx_alloc");
    let hw = &mut *((*r).data as *mut ffi::AVHWDeviceContext);
    let d = &mut *(hw.hwctx as *mut AVD3D11VADeviceContext);
    d.device = device_addrefd;
    (d.lock, d.unlock, d.lock_ctx) = (Some(lock), Some(unlock), lock_ctx); // else FFmpeg makes a private CreateMutex lock
    if let Err(e) = check(ffi::av_hwdevice_ctx_init(r)) { ffi::av_buffer_unref(&mut r); return Err(e); }
    Ok(HwRef(r))
}}

const D3D11_BIND_RENDER_TARGET: u32 = 0x20;
pub unsafe fn nv12_frames(dev: &HwRef, w: i32, h: i32) -> anyhow::Result<HwRef> { unsafe {
    let mut r = ffi::av_hwframe_ctx_alloc(dev.0);
    anyhow::ensure!(!r.is_null(), "av_hwframe_ctx_alloc");
    let fc = &mut *((*r).data as *mut ffi::AVHWFramesContext);
    fc.format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
    fc.sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;  // P010 later for HDR10
    (fc.width, fc.height) = (w, h);                      // both even
    fc.initial_pool_size = 0;                            // dynamic pool: one texture per buffer (QSV-safe)
    (*(fc.hwctx as *mut AVD3D11VAFramesContext)).BindFlags = D3D11_BIND_RENDER_TARGET; // our shader writes the planes
    if let Err(e) = check(ffi::av_hwframe_ctx_init(r)) { ffi::av_buffer_unref(&mut r); return Err(e); }
    Ok(HwRef(r))
}}

/// Per tick: a pool frame; data[0] = ID3D11Texture2D* (borrowed), data[1] = array slice (0 here).
pub unsafe fn get_frame(frames: &HwRef, pts: i64) -> anyhow::Result<(ff::frame::Video, *mut c_void, u32)> { unsafe {
    let mut f = ff::frame::Video::empty();
    check(ffi::av_hwframe_get_buffer(frames.0, f.as_mut_ptr(), 0))?;
    (*f.as_mut_ptr()).pts = pts;
    let tex = (*f.as_ptr()).data[0] as *mut c_void;
    let slice = (*f.as_ptr()).data[1] as usize as u32;
    Ok((f, tex, slice))
}}

// Doc-form variants (cookbook §4.2 / §4.6)
pub unsafe fn doc_form(r: *mut ffi::AVBufferRef) -> *mut AVD3D11VADeviceContext { unsafe {
    let d = &mut *((*((*r).data as *mut ffi::AVHWDeviceContext)).hwctx as *mut AVD3D11VADeviceContext);
    d as *mut _
}}
pub fn layouts() -> (ff::ChannelLayout, ff::ChannelLayout) { (ff::ChannelLayout::STEREO, ff::ChannelLayout::MONO) }
