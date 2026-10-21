use std::ffi::{CStr, CString};
use ffmpeg_next::{self as ff, ffi};
use crate::hw::{check, HwRef};

pub struct VideoCfg { pub width: u32, pub height: u32, pub fps: i32, pub bitrate: usize, pub maxrate: usize }

/// h264_amf, minimal game impact (research 13). Every key is checked; a typo is an error, not a no-op.
pub const H264_AMF: &[(&str, &str)] = &[
    ("usage", "transcoding"), ("quality", "balanced"), ("rc", "vbr_peak"), ("vbaq", "1"),
    ("preanalysis", "0"), ("high_motion_quality_boost_enable", "0"), ("preencode", "0"),
    ("bf", "0"), ("frame_skipping", "0"), ("filler_data", "0"), ("profile", "high"), ("coder", "cabac"),
    ("header_spacing", "60"), ("async_depth", "4"),
];

pub unsafe fn open_video(name: &str, cfg: &VideoCfg, frames: &HwRef, opts: &[(&str, &str)]) -> anyhow::Result<ff::encoder::video::Encoder> { unsafe {
    let codec = ff::encoder::find_by_name(name).ok_or_else(|| anyhow::anyhow!("{name} not in this FFmpeg build"))?;
    let mut v = ff::codec::Context::new_with_codec(codec).encoder().video()?; // codec defaults (+loop, g=-1) applied
    v.set_width(cfg.width); v.set_height(cfg.height);
    v.set_format(ff::format::Pixel::D3D11);
    v.set_time_base((1, cfg.fps)); v.set_frame_rate(Some((cfg.fps, 1)));
    v.set_gop(cfg.fps as u32);     // 1 s. AMF sets IDR_PERIOD only if gop_size != -1 (its own default: 240 frames)
    v.set_max_b_frames(0);         // NVENC/QSV default -1 = driver decides; AMF also gets its private "bf"
    v.set_bit_rate(cfg.bitrate); v.set_max_bit_rate(cfg.maxrate);
    v.set_color_range(ff::color::Range::MPEG); v.set_colorspace(ff::color::Space::BT709);
    v.set_color_primaries(ff::color::Primaries::BT709);
    v.set_color_transfer_characteristic(ff::color::TransferCharacteristic::BT709); // all 3 set => MP4 `colr nclx`
    let c = v.as_mut_ptr();
    (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32; // OR it in: set_flags() would drop "+loop"/"+cgop"
    (*c).rc_buffer_size = cfg.maxrate as i32;
    (*c).hw_frames_ctx = ffi::av_buffer_ref(frames.0);     // frames MUST come from this ctx (amfenc av_assert0 aborts)
    for (k, val) in opts {
        let (ck, cv) = (CString::new(*k)?, CString::new(*val)?);
        let r = ffi::av_opt_set((*c).priv_data, ck.as_ptr(), cv.as_ptr(), 0);
        anyhow::ensure!(r >= 0, "{name}: option {k}={val} rejected: {}", ff::Error::from(r));
    }
    Ok(v.open_as(codec)?)
}}

/// Read back what the encoder will use (capability report / tracing).
pub unsafe fn read_opts(enc: &mut ff::encoder::video::Encoder, keys: &[&str]) -> Vec<(String, String)> { unsafe {
    let c = enc.as_mut_ptr();
    keys.iter().filter_map(|k| {
        let ck = CString::new(*k).ok()?;
        let mut out: *mut u8 = std::ptr::null_mut();
        (ffi::av_opt_get((*c).priv_data, ck.as_ptr(), 0, &mut out) >= 0).then(|| {
            let s = CStr::from_ptr(out as *const _).to_string_lossy().into_owned();
            ffi::av_free(out as *mut _);
            (k.to_string(), s)
        })
    }).collect()
}}

/// Drain until EAGAIN/EOF. AMF/NVENC/QSV submit inside receive_packet: always drain after send_frame.
pub fn drain(enc: &mut ff::encoder::video::Encoder, out: &mut impl FnMut(ff::Packet)) -> anyhow::Result<()> {
    loop {
        let mut pkt = ff::Packet::empty();
        match enc.receive_packet(&mut pkt) {
            Ok(()) => out(pkt),
            Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return Ok(()),
            Err(ff::Error::Eof) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Encoder thread: fed by a 2-4 slot channel from the pacer (amfenc may Sleep(1) when its queue is full).
pub fn encode_thread(mut enc: ff::encoder::video::Encoder, rx: std::sync::mpsc::Receiver<ff::frame::Video>,
                     mut out: impl FnMut(ff::Packet)) -> anyhow::Result<()> {
    for f in rx {
        enc.send_frame(&f)?;      // FFmpeg takes its own ref
        drain(&mut enc, &mut out)?;
    }
    enc.send_eof()?;
    drain(&mut enc, &mut out)
}

/// Native LGPL AAC: FLTP in, 1024-sample frames, initial_padding 1024 (first packet pts = -1024).
pub fn open_aac(layout: ff::ChannelLayout, bitrate: usize) -> anyhow::Result<ff::encoder::audio::Encoder> {
    let codec = ff::encoder::find_by_name("aac").ok_or_else(|| anyhow::anyhow!("aac missing"))?;
    let mut a = ff::codec::Context::new_with_codec(codec).encoder().audio()?;
    a.set_rate(48_000);
    a.set_channel_layout(layout);
    a.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
    a.set_bit_rate(bitrate);            // codec default is 0: always set it
    a.set_time_base((1, 48_000));
    unsafe { (*a.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32; }
    Ok(a.open_as(codec)?)
}

/// Lazy, sequential probe (Sunshine #5385): open, send ~5 frames, require a KEY packet + extradata.
pub fn probe_ok(enc: &mut ff::encoder::video::Encoder, frames: &HwRef, n: i64) -> anyhow::Result<bool> {
    let mut key = false;
    for i in 0..n {
        let (f, _tex, _slice) = unsafe { crate::hw::get_frame(frames, i)? }; // (render something first in real code)
        enc.send_frame(&f)?;
        drain(enc, &mut |p: ff::Packet| key |= p.is_key())?;
    }
    let extradata = unsafe { (*enc.as_ptr()).extradata_size } > 0;
    let _ = check(0)?;
    Ok(key && extradata)
}
