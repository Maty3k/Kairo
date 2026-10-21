use std::ffi::{c_char, c_int, c_void, CStr};
use ffmpeg_next::ffi;
unsafe extern "C" fn ff_log(avcl: *mut c_void, level: c_int, fmt: *const c_char, vl: ffi::va_list) {
    if level > ffi::AV_LOG_INFO { return; }
    let (mut buf, mut prefix) = ([0 as c_char; 1024], 1);
    unsafe { ffi::av_log_format_line2(avcl, level, fmt, vl, buf.as_mut_ptr(), buf.len() as c_int, &mut prefix) };
    let line = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
    match level {
        l if l <= ffi::AV_LOG_ERROR => tracing::error!(target: "ffmpeg", "{}", line.trim_end()),
        l if l <= ffi::AV_LOG_WARNING => tracing::warn!(target: "ffmpeg", "{}", line.trim_end()),
        _ => tracing::info!(target: "ffmpeg", "{}", line.trim_end()),
    }
}
pub fn install() { unsafe { ffi::av_log_set_callback(Some(ff_log)) } }
