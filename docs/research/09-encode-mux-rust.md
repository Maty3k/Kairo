# 09: GPU colour conversion, hardware encoding and MP4 muxing from Rust via FFmpeg (LGPL)

*Code-level pre-implementation research for the Kairo engine, written for the implementation cookbook. Prepared 2026-10-21. Nothing is built yet.*

**Tags**
- **[H]**: checked in a primary source today. That means the FFmpeg 9.0.2 release tarball (file and line numbers given), crate source downloaded from crates.io, Microsoft Learn, or OBS source on GitHub `master`.
- **[M]**: a strong secondary source, or a direct inference from primary sources.
- **[Unverified]**: not confirmed. It needs a run on the Windows PC or more sources.
- **[Estimate]**: arithmetic or judgement. Measure it.
- **[Corrected]**: changes a claim made in the brief or in reports 01 and 05.

Line numbers refer to FFmpeg **n9.0.2** (`ffmpeg-9.0.2.tar.xz` from ffmpeg.org, VERSION file "9.0.2"). The same paths are on GitHub as `https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/<path>#L<n>`.

---

## TL;DR

1. **Bindings.** Use **`ffmpeg-next` 9.0.0** together with a pinned **`ffmpeg-sys-next` =9.0.1** (2026-10-09, WTFPL). It is the only maintained binding family that is both on crates.io for FFmpeg 9 *and* binds the D3D11VA hwcontext structs. Since 2026-07-01 the sys crate's `hwcontext_wrapper.h` binds `AVD3D11VADeviceContext`, `AVD3D11VAFramesContext` and `AVD3D11FrameDescriptor`, but only when libclang can see `<d3d11.h>` **[H]**.
   - `ffmpeg-next` has **no** hardware-context API. PR #274 ("hardware frame context wrappers") is still open.
   - So hardware setup is raw `ffmpeg_next::ffi` calls. Budget about 300 lines of `unsafe` code in one module **[H]**.
   - The alternatives fall short:
     - `rsmpeg` 0.18 and `rusty_ffmpeg` 0.17 stop at FFmpeg 8.x; their FFmpeg 9 PRs are open.
     - `ffmpeg-the-third` 6.0.0+ffmpeg-9.0 supports FFmpeg 9 but does not bind the D3D11 structs **[H]**.
2. **Zero-copy recipe (one path for all vendors) [H]:**
   - Wrap *our* `ID3D11Device` in a D3D11VA `AVHWDeviceContext`: `av_hwdevice_ctx_alloc`, set `.device` to an **AddRef'd** pointer, then `av_hwdevice_ctx_init`.
   - Create one `AVHWFramesContext`: `format=AV_PIX_FMT_D3D11`, `sw_format=AV_PIX_FMT_NV12`, **`initial_pool_size=0`**, `BindFlags=D3D11_BIND_RENDER_TARGET`. Pool size 0 makes a dynamic pool of single textures.
   - Each tick: `av_hwframe_get_buffer`, then **render the NV12 planes straight into `frame->data[0]`** (an `ID3D11Texture2D*`; `data[1]` is the array index, 0 here) through R8 and R8G8 RTVs, then `avcodec_send_frame`.
   - Per encoder:
     - **`h264_amf`, `h264_nvenc` and `h264_mf` take these frames directly.**
     - **`h264_qsv` does not.** It needs `AV_PIX_FMT_QSV` frames, made with a derived QSV device and frames context plus `av_hwframe_map`.
3. **AMF has an `av_assert0` trap.** `amfenc.c:411` aborts the process if `frame->hw_frames_ctx->data != avctx->hw_frames_ctx->data`. Frames must come from the exact frames context set on the encoder **[H]**.
4. **QSV plus texture arrays is broken in FFmpeg's mapping code when `D3D11_BIND_RENDER_TARGET` is set.** The array index is replaced by `MFX_INFINITE`, so slices other than 0 fail to map (`hwcontext_qsv.c:2081-2085, 2164-2170`). Use the dynamic pool, which gives one texture per frame.
   - **[Corrected]** The brief's CLI line `scale_d3d11=format=nv12,hwmap=derive_device=qsv` feeds a 10-slice render-target array (`vf_scale_d3d11.c:376-383`). By this reading it should fail on 9 of every 10 frames **[M: source reading, not run]**.
5. **NV12 conversion: a two-pass HLSL shader, ported in part from OBS (GPL-2.0-or-later, so allowed in GPL-3.0 Kairo with notices).**
   - Pass A scales the capture into an RGBA intermediate at output size, tone-mapping first when the source is HDR.
   - Pass B writes Y (R8) and UV (R8G8, half size, "left" chroma siting) with the BT.709 limited-range matrix given below.
   - Compile at **runtime with `D3DCompile`**: `windows` 0.62.2 links `d3dcompiler_47.dll` via raw-dylib, so no import library and no fxc are needed under cargo-xwin. OBS does the same, with an on-disk cache **[H]**.
6. **[Corrected] HDR.** OBS's *display capture* tone-maps scRGB to SDR with **Reinhard** (`DrawMultiplyTonemap`, multiplier 80/SDR-white), or clips in "force SDR" mode. The BT.2390 maxRGB EETF in `color.effect` is used for PQ/HLG *media* sources (`duplicator-monitor-capture.c:676-699`) **[H]**.
   - Kairo can still choose the EETF: source peak `Lw` = monitor `MaxLuminance`, target peak `Lmax` = SDR white. It leaves midtones unchanged below about 114 nits, and SDR white lands at about 80% (worked numbers in §3.5).
7. **Per-vendor GOP and header behaviour differs.** Set everything explicitly:
   - AMF H.264 maps `gop_size` to its IDR period.
   - NVENC sets `idrPeriod = gopLength`. With `AV_CODEC_FLAG_GLOBAL_HEADER` it emits **no in-band SPS/PPS at all** (`nvenc.c:1188-1189`).
   - QSV defaults to `+cgop` and makes every I-frame an IDR (`idr_interval=0`).
   - Default `bf` is "-1 = driver or preset decides" for NVENC and QSV, and unset (AMF default) for AMF. **Always set `bf`.**
8. **Recommendation: `bf=0` in v1.** This diverges from the brief's "B-frames 2 where supported".
   - pts = dts, so no `ctts`, and `use_editlist=0` produces MP4s with no edit list.
   - AMD HEVC has no B-frames anyway, and AMD H.264 B-frames depend on a capability flag.
   - Revisit B-frames in S2 with VMAF. With B-frames, keep FFmpeg's default edit list, or `+negative_cts_offsets`.
9. **Mux from the RAM ring:** one `AVFormatContext` per save, on a worker thread.
   - `movflags=+faststart`, `use_editlist=0`.
   - Streams take codecpar copied from the encoder session.
   - **Rescale timestamps to `st->time_base` read *after* `write_header`.** movenc changes it, to 1/15360 for 60 fps (`movenc.c:8492-8494, 8630`).
   - `disposition=AV_DISPOSITION_DEFAULT` only on the mixed audio track.
   - `colr nclx` is written only if primaries, transfer *and* matrix are all set (`movenc.c:2944-2953`) **[H]**.
10. **Two ffmpeg-next traps:**
    - **`Packet::clone()` deep-copies.** It calls `av_packet_ref` and then `av_packet_make_writable`. Snapshot the ring with raw `av_packet_ref`.
    - **`open_with` and `open_as_with` silently drop unknown options.** Set private options one at a time with `av_opt_set` and check the return value **[H]**.
11. **[Corrected] FFmpeg 9 BtbN LGPL builds are LGPL-*v3*** (`--enable-version3`; LICENSE.txt is LGPLv3).
    - The shared zip is **149 MB of DLLs**: `avcodec-63.dll` alone is 87 MB.
    - It uses nv-codec-headers 13.1.15, which needs an **NVIDIA driver ≥ 610**.
    - MSVC import libraries (`lib/avcodec.lib` …) are included, so `FFMPEG_DIR` works on MSVC and with cargo-xwin.
    - Pin a dated build. Daily builds are kept 14 days and month-end builds 2 years.
    - Plan a minimal custom LGPL build for the public release.
12. **cargo-xwin on the Mac.** cargo-xwin 0.23.1 passes the xwin CRT and SDK include dirs to bindgen through `BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc` (`src/compiler/clang_cl.rs:111-115`). bindgen adds `--target` from `TARGET` itself. ffmpeg-sys-next's feature probe `check.c` is compiled for the **host**, using only headers.
    - So `FFMPEG_DIR=<BtbN include+lib> cargo xwin check` should work, D3D11 structs included. **[M: every step verified in source, but no end-to-end run, because Rust isn't installed on the Mac yet. Make it the first S0 check.]**

---

## 1. Rust FFmpeg bindings (state on 2026-10-21)

### 1.1 Comparison

| Crate (latest) | Date | FFmpeg | D3D11VA hwctx structs | Safe hw API | Build | Licence |
|---|---|---|---|---|---|---|
| **`ffmpeg-sys-next` 9.0.1** | 2026-10-09 | 3.4 → **9.0** (cfg `ffmpeg_9_0` = lavc ≥ 63.1) | **Yes, conditional.** `hwcontext_wrapper.h` binds CUDA, D3D11VA, D3D12VA, VideoToolbox, Vulkan, VAAPI, QSV and MediaCodec structs. D3D11 needs `__has_include(<d3d11.h>)` (commit 369976d9 "Expose hwcontext structs", 2026-07-01; also in 9.0.0) | n/a | bindgen 0.73 (runtime libclang), always run. `FFMPEG_DIR` → vcpkg → pkg-config. No "prebuilt bindings" switch | WTFPL |
| **`ffmpeg-next` 9.0.0** | 2026-08-05 | depends on `ffmpeg-sys-next = "9.0.0"`, so 9.0.1 resolves. README still says "3.4 til 8.0" (stale) | via `ffmpeg_next::ffi` | **None.** PR #274 "add hardware frame context wrappers" open since 2026-07-17 | – | WTFPL |
| ffmpeg-next 9.0.1 | in git only (2026-10-09: "Update version to 9.0.1", extradata setters, soundness fixes) | – | – | – | **not on crates.io** today | – |
| `ffmpeg-the-third` / `-sys-the-third` 6.0.0+ffmpeg-9.0 | 2026-08-09 | 5.1 → 9.0 | **No.** Binds only `hwcontext.h` and `hwcontext_drm.h` | None | bindgen | WTFPL, MSRV 1.80 |
| `rsmpeg` 0.18.0+ffmpeg.8.0 | 2025-08-24 | 8.0 only. PR #256 "Support FFmpeg 9.0" open (2026-09-23) | No | No | via rusty_ffmpeg | MIT |
| `rusty_ffmpeg` 0.17.0+ffmpeg.8.1 | 2026-04-10 | ≤ 8.1. PR #154 "Update to FFmpeg 9" open (2026-08-12) | No: `hwcontext_d3d11va.h` commented out in `build.rs` | – | bindgen, **or** `FFMPEG_BINDING_PATH` / `use_prebuilt_binding` (prebuilt) | MIT |
| `ez-ffmpeg` 0.19.3 | 2026-10-06 | depends on `ffmpeg-next ^8.1` | – | – | – | MIT/Apache/MPL |
| `video-rs` 0.12.0 | 2026-09-09 | `ffmpeg-next =9.0.0` | – | – | – | MIT/Apache |
| `ac-ffmpeg` 0.19.0 | 2025-05-16 | own C glue | – | – | cc | MIT |

All rows are **[H]**, from crates.io API JSON and the downloaded `.crate` sources.

### 1.2 Recommendation

- Use `ffmpeg-next` for the safe plumbing: `codec::Context`, the encoders, `Frame`, `Packet`, `format::output_as`, `Dictionary` and error mapping.
- Use `ffmpeg_next::ffi`, which re-exports the sys crate, for everything hardware-related and for the few gaps listed in §1.3.
- Keep all of it in one Windows-only module, e.g. `kairo-engine/src/media/ffi_hw.rs`.

```toml
# kairo-engine/Cargo.toml: Windows-only, so the macOS mock engine never builds FFmpeg
[target.'cfg(windows)'.dependencies]
ffmpeg-next     = { version = "=9.0.0", default-features = false, features = ["codec", "format", "software-resampling"] }
ffmpeg-sys-next = { version = "=9.0.1", default-features = false, features = ["avcodec", "avformat", "swresample"] }
windows = { version = "=0.62.2", features = [
  "Win32_Foundation", "Win32_Graphics_Direct3D", "Win32_Graphics_Direct3D_Fxc",
  "Win32_Graphics_Direct3D11", "Win32_Graphics_Dxgi", "Win32_Graphics_Dxgi_Common",
  "Win32_Devices_Display",   # DisplayConfigGetDeviceInfo / DISPLAYCONFIG_SDR_WHITE_LEVEL
] }
```

- Disabling default features drops `avdevice` and `avfilter` from linking. You then ship only `avcodec-63`, `avformat-63`, `avutil-61` and `swresample-7` **[H: `ffmpeg-sys-next` build.rs `LIBRARIES` are feature-gated, lines 34-71]**.
- **`links = "ffmpeg"`.** Only one FFmpeg sys crate version can exist in the dependency graph. Adding `ez-ffmpeg`, which pulls `ffmpeg-sys-next` 8.1, would fail with "multiple packages link to native library `ffmpeg`" **[H: manifest `links`]**.

### 1.3 What `ffmpeg-next` 9.0.0 does not give you (verified in source)

| Need | Where | Do this |
|---|---|---|
| Hardware device and frames contexts | no wrapper | `ffi::av_hwdevice_ctx_alloc/init`, `av_hwframe_ctx_alloc/init`, `av_hwframe_get_buffer`, `av_hwdevice_ctx_create_derived`, `av_hwframe_ctx_create_derived`, `av_hwframe_map` |
| `AVCodecContext.hw_frames_ctx` | no setter | `(*enc.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(frames)` |
| OR-ing codec flags | `set_flags()` *overwrites* `flags`. There is no getter | `(*ctx.as_mut_ptr()).flags \|= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32`. Overwriting would drop codec defaults such as AMF's `+loop`, which disables deblocking, and QSV's `+cgop` (`amfenc_h264.c:632-641`, `qsvenc_h264.c` `qsv_enc_defaults`) |
| Private options with error reporting | `open_with` / `open_as_with` call `Dictionary::own(opts)` and drop the leftovers (`codec/encoder/video.rs:44-77`) | `ffi::av_opt_set((*ctx).priv_data, k, v, 0)` for each option. Treat `< 0` as an error (`AVERROR_OPTION_NOT_FOUND` catches typos). Muxer leftovers *are* returned by `write_header_with` |
| Cheap packet references | `Packet::clone()` calls `av_packet_ref` **then `av_packet_make_writable`**, which copies the payload when the buffer is shared (`codec/packet/packet.rs:277-297`) | `ffi::av_packet_ref(dst.as_mut_ptr(), src.as_ptr())` |
| Stream disposition | getter only | `(*st.as_mut_ptr()).disposition = ffi::AV_DISPOSITION_DEFAULT as i32` |
| FFmpeg log → `tracing` | `util::log` has `set_level` and `set_flags` only | `ffi::av_log_set_callback(Some(cb))`. Format inside `cb` with `ffi::av_log_format_line2(avcl, level, fmt, vl, buf, len, &mut prefix)`. The `va_list` is passed through, so stable Rust is fine **[Unverified: exact generated `va_list` alias name on `x86_64-pc-windows-msvc`]** |
| `AV_HWFRAME_MAP_*` | an anonymous C enum, so bindgen names it `_bindgen_ty_N`, which is unstable | Declare your own: `READ=1, WRITE=2, OVERWRITE=4, DIRECT=8` (`hwcontext.h:515-531`) |

Also note that `Packet::write_interleaved(&self, …)` passes the packet to `av_interleaved_write_frame`, which **takes ownership of the reference**. The `Packet` is blank afterwards. Write a fresh ref, never the ring's own packet.

---

## 2. Zero-copy: our `ID3D11Device` → FFmpeg → AMF / NVENC / QSV / MF

### 2.1 Device rules

- **Create the device yourself:**
  - on the adapter that will encode;
  - with `D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT`;
  - with feature levels `[11_1, 11_0]`;
  - **never** with `SINGLETHREADED`.
- **`VIDEO_SUPPORT` is harmless on WDDM 1.2+ hardware.** "If feature level 10 and higher is available, the runtime will use that feature level regardless of video support" **[H: MS Learn D3D11_CREATE_DEVICE_FLAG]**.
  - FFmpeg's own `d3d11va_device_create` sets it (`hwcontext_d3d11va.c:626`).
  - OBS's `texture-amf.cpp` creates its AMF device with flags `0` and it works **[H]**. So it is not strictly required.
- **`d3d11va_device_init` fails if `QueryInterface(ID3D11VideoDevice)` fails** (`hwcontext_d3d11va.c:529-534`).
- **Turn on multithread protection.** Query `ID3D11Multithread` from the **immediate context**, not the device (MS Learn remarks), and call `SetMultithreadProtected(true)`.
  - FFmpeg does this only when *it* creates the device (`hwcontext_d3d11va.c:687-691`).
  - For a user-supplied device, `d3d11va_device_init` does **not** do it **[H]**.
- **windows-capture 2.0.1 caveat (affects the capture report).** Its `create_d3d_device()` uses the default adapter, `BGRA_SUPPORT` only, and does not enable multithread protection (`src/d3d11.rs:161-200`) **[H]**. If Kairo keeps windows-capture, take its device from `frame.device()` and call `SetMultithreadProtected` yourself, accepting the default adapter. Otherwise own the WGC code.

```rust
use windows::{core::Interface, Win32::Foundation::HMODULE, Win32::Graphics::{Direct3D::*, Direct3D11::*, Dxgi::*}};

unsafe fn create_device(adapter: &IDXGIAdapter1) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext, ID3D11Multithread)> {
    let (mut dev, mut ctx) = (None, None);
    D3D11CreateDevice(adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION,
        Some(&mut dev), None, Some(&mut ctx))?;
    let (dev, ctx) = (dev.unwrap(), ctx.unwrap());
    let mt: ID3D11Multithread = ctx.cast()?;          // ID3D11Multithread comes from the immediate context
    let _ = mt.SetMultithreadProtected(true);
    Ok((dev, ctx, mt))
}
```

### 2.2 Wrapping the device as an FFmpeg D3D11VA device

The public struct in FFmpeg 9.0.2 has two fields added after `lock_ctx` (`hwcontext_d3d11va.h:45-111`). A hand-written mirror must include them:

```rust
// Mirror of libavutil/hwcontext_d3d11va.h (FFmpeg 9.0.2). Use ffi::AVD3D11VADeviceContext
// when the sys crate generated it; this mirror avoids depending on libclang seeing <d3d11.h>.
#[repr(C)] #[allow(non_snake_case)]
pub struct AVD3D11VADeviceContext {
    pub device: *mut c_void,          // ID3D11Device*. FFmpeg Release()s it on uninit
    pub device_context: *mut c_void,  // filled by init (GetImmediateContext) if null
    pub video_device: *mut c_void,    // filled by init (QI ID3D11VideoDevice)
    pub video_context: *mut c_void,   // filled by init (QI ID3D11VideoContext)
    pub lock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub lock_ctx: *mut c_void,
    pub BindFlags: u32,               // applied to every frames ctx of this device (OR-ed in)
    pub MiscFlags: u32,
}
#[repr(C)] pub struct AVD3D11FrameDescriptor { pub texture: *mut c_void, pub index: isize }
#[repr(C)] #[allow(non_snake_case)]
pub struct AVD3D11VAFramesContext {
    pub texture: *mut c_void,          // array texture for fixed pools, null for dynamic pools
    pub BindFlags: u32,
    pub MiscFlags: u32,
    pub texture_infos: *mut AVD3D11FrameDescriptor,
}

pub struct HwRef(pub *mut ffi::AVBufferRef);
impl Drop for HwRef { fn drop(&mut self) { unsafe { ffi::av_buffer_unref(&mut self.0) } } }
unsafe impl Send for HwRef {}

unsafe extern "C" fn d3d_lock(c: *mut c_void)   { ID3D11Multithread::from_raw_borrowed(&c).unwrap().Enter() }
unsafe extern "C" fn d3d_unlock(c: *mut c_void) { ID3D11Multithread::from_raw_borrowed(&c).unwrap().Leave() }

pub unsafe fn d3d11va_device(dev: &ID3D11Device, mt: &ID3D11Multithread) -> anyhow::Result<HwRef> {
    let mut r = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
    anyhow::ensure!(!r.is_null(), "av_hwdevice_ctx_alloc");
    let hw = &mut *((*r).data as *mut ffi::AVHWDeviceContext);
    let d = &mut *(hw.hwctx as *mut AVD3D11VADeviceContext);
    d.device = dev.clone().into_raw();      // +1 ref, owned by FFmpeg ("will always release this interface")
    d.lock = Some(d3d_lock);                // optional. Must be recursive: a D3D critical section is
    d.unlock = Some(d3d_unlock);
    d.lock_ctx = mt.as_raw();               // NOT released by FFmpeg: keep `mt` alive longer than every FFmpeg object
    let ret = ffi::av_hwdevice_ctx_init(r);
    if ret < 0 { ffi::av_buffer_unref(&mut r); anyhow::bail!("av_hwdevice_ctx_init: {}", ffmpeg_next::Error::from(ret)); }
    Ok(HwRef(r))
}
```

- **Ownership.** FFmpeg releases `device`, `device_context`, `video_device` and `video_context` in `d3d11va_device_uninit` (`hwcontext_d3d11va.c:546-575`), so `device` must be an owned reference (`clone().into_raw()`).
- **Lock.** If `lock` is unset, FFmpeg makes its own internal mutex (`:513-521`), which does not coordinate with Kairo's own context use.
- **Which encoders use it [H].** `amfenc` takes the *AMF* device lock (`amfenc.c:547-560`), not this one. This lock guards FFmpeg's own D3D11 context use, such as `transfer_data`.

### 2.3 The frames context: dynamic pool vs texture arrays

```rust
pub unsafe fn nv12_frames(dev: &HwRef, w: i32, h: i32) -> anyhow::Result<HwRef> {
    let mut r = ffi::av_hwframe_ctx_alloc(dev.0);
    anyhow::ensure!(!r.is_null(), "av_hwframe_ctx_alloc");
    let fc = &mut *((*r).data as *mut ffi::AVHWFramesContext);
    fc.format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
    fc.sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;     // P010 later for HDR10
    fc.width = w; fc.height = h;                            // NV12: both must be even (DXGI_FORMAT docs)
    fc.initial_pool_size = 0;                               // dynamic pool: one ArraySize=1 texture per buffer
    let d = &mut *(fc.hwctx as *mut AVD3D11VAFramesContext);
    d.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;        // our pixel shader writes the planes
    let ret = ffi::av_hwframe_ctx_init(r);
    if ret < 0 { ffi::av_buffer_unref(&mut r); anyhow::bail!("av_hwframe_ctx_init: {}", ffmpeg_next::Error::from(ret)); }
    Ok(HwRef(r))
}
```

**What FFmpeg does [H, `hwcontext_d3d11va.c`]**
- **`initial_pool_size > 0`:** one `ID3D11Texture2D` with `ArraySize = initial_pool_size`, capped at 64 (`MAX_ARRAY_SIZE`, `:45, 297-331`). `av_hwframe_ctx_init` pre-allocates all of them. Running past the size gives "Static surface pool size exceeded" (`:263-266`).
- **`initial_pool_size == 0`:** `d3d11va_alloc_single` creates one texture per new pool buffer (`:223-249, 258-259`). The pool grows to the peak number of frames in flight and reuses textures afterwards.
- **Frame layout:** `frame->data[0] = (ID3D11Texture2D*)`, `frame->data[1] = (intptr_t)array index`, `format = AV_PIX_FMT_D3D11` (`:347-364`).
- **Supported `sw_format`s:** NV12, P010, BGRA, X2BGR10, RGBAF16, AYUV/VUYX, YUY2, Y210, Y410, P016/P012, Y216/Y212, Y416/XV36 (`:93-117`).

| Pool type | AMF | NVENC | MF | QSV |
|---|---|---|---|---|
| Array (`initial_pool_size>0`) | OK: sets `AMFTextureArrayIndexGUID` private data (`amfenc.c:408-413`) | OK: registers (texture, `subResourceIndex`) (`nvenc.c:2253-2256`) | OK: `MFCreateDXGISurfaceBuffer(tex, subIdx)` (`mfenc.c:378-392`) | **Broken with `BIND_RENDER_TARGET`:** the index becomes `MFX_INFINITE` (`hwcontext_qsv.c:2081-2085, 631-634`) |
| Dynamic (`=0`) | OK | OK. Cache of 64 registrations, then unregisters unused ones (`nvenc.c:2189-2221`) | OK | OK: `qsv_dynamic_frames_derive_to` / `qsv_dynamic_pool_map_to` (`:1994-2033, 2240-2319`) |

**→ Use `initial_pool_size = 0` everywhere.**
- Optionally add `D3D11_BIND_VIDEO_ENCODER`, as `vf_scale_d3d11.c:383` does (`RENDER_TARGET | VIDEO_ENCODER`). Test it with `h264_mf`.
- **VRAM:** peak in-flight frames × 3.1 MB at 1080p NV12 **[Estimate]**. Limit in-flight frames with `async_depth=4` (AMF; the default is **16**, `amfenc_h264.c:117`) and QSV's default 4.

### 2.4 Rendering into a pool frame

```rust
// per pacer tick, on the video thread
let mut f = ffmpeg_next::frame::Video::empty();                 // safe ctor; derefs to Frame
check(ffi::av_hwframe_get_buffer(frames.0, f.as_mut_ptr(), 0))?;
let raw = (*f.as_ptr()).data[0] as *mut c_void;
let tex = ID3D11Texture2D::from_raw_borrowed(&raw).unwrap();    // borrowed: FFmpeg's AVBufferRef owns it
let slice = (*f.as_ptr()).data[1] as u32;                       // 0 with the dynamic pool
let y  = plane_rtv(&dev, tex, DXGI_FORMAT_R8_UNORM,   slice)?;  // Y plane  (W x H)
let uv = plane_rtv(&dev, tex, DXGI_FORMAT_R8G8_UNORM, slice)?;  // UV plane (W/2 x H/2)
mt.Enter(); conv.draw_nv12(&ctx, &src_srv, &y, &uv, w, h); ctx.Flush(); mt.Leave();
(*f.as_mut_ptr()).pts = tick;                                   // encoder time_base = 1/fps
enc.send_frame(&f)?;                                            // FFmpeg takes its own ref
loop { match enc.receive_packet(&mut pkt) { Ok(()) => ring.push(&pkt), Err(_) => break } } // EAGAIN/EOF end the drain
```

**Plane views [H: MS Learn DXGI_FORMAT_NV12]**
- Luma views are `R8_UNORM`; chroma views are `R8G8_UNORM` at half width and half height. "Supported view types are SRV, RTV, and UAV". "Width and height must be even".
- Create RTVs with `D3D11_RTV_DIMENSION_TEXTURE2D` (`MipSlice 0`). For an array slice, use `TEXTURE2DARRAY {FirstArraySlice: slice, ArraySize: 1}`.
- Creating 2 RTVs per frame costs microseconds. Cache them per texture pointer only if profiling says so, and clear the cache together with the frames context.

**Flush.** `ctx.Flush()` after the two draws matches OBS: `texture-amf.cpp` flushes after its copy and before `CreateSurfaceFromDX11Native`. It kicks the GPU work before the encoder reads **[H, OBS]**.

**Atomicity**
- Either hold `ID3D11Multithread::Enter`/`Leave` around *our* state + draw sequence.
- Or record into a deferred context and run it with one `ExecuteCommandList` call. FFmpeg's `gfxcapture` does that, with `SetMultithreadProtected(TRUE)` (`vsrc_gfxcapture_winrt.cpp:334-350, 1383-1392`).
- **Never** hold the D3D lock across `avcodec_send_frame` or `avcodec_receive_packet`. Vendor runtimes run their own threads on the same immediate context, and holding the lock risks a deadlock **[M]**.
- Re-bind the full pipeline state on every draw. Don't rely on state persisting, because AMF and VPL also use the immediate context **[M]**.

### 2.5 What each encoder needs (FFmpeg 9.0.2 source)

| Encoder | Input | Required setup | Notes |
|---|---|---|---|
| **`h264_amf` / `hevc_amf` / `av1_amf`** | `AV_PIX_FMT_D3D11` frames directly. `ff_amf_pix_fmts` contains D3D11, NV12, P010, BGRA/RGBA/X2BGR10/RGBAF16, AMF_SURFACE (`amfenc.c:55-74`). HW configs: `FRAMES(D3D11, D3D11VA)`, `DEVICE(NONE, D3D11VA)` (`:714-725`) | `avctx->pix_fmt = AV_PIX_FMT_D3D11` and `avctx->hw_frames_ctx = av_buffer_ref(frames)`. Init derives an **AMF device** from the frames' D3D11VA device: `av_hwdevice_ctx_create_derived(..., AV_HWDEVICE_TYPE_AMF, ...)` → `InitDX11(device, AMF_DX11_1)` (`amfenc.c:302-326`, `hwcontext_amf.c:748-760`) | **`av_assert0(frame->hw_frames_ctx->data == avctx->hw_frames_ctx->data)`** (`:411-412`). Wraps with `CreateSurfaceFromDX11Native` (`:414`). Keeps an `av_frame_clone` until output (`:446`, `amf_store_attached_frame_ref`). `SetCrop(0,0,frame->width,frame->height)` (`:449`). Loads `amfrt64.dll` from the driver |
| **`h264_nvenc` / `hevc_nvenc` / `av1_nvenc`** | D3D11 frames directly. HW configs `FRAMES(D3D11, D3D11VA)`, `DEVICE(NONE, D3D11VA)` (`nvenc.c:79-91`) | `pix_fmt = AV_PIX_FMT_D3D11` **and** `hw_frames_ctx` whose `format == pix_fmt`; otherwise "hw_frames_ctx must be set…" (`:2101-2114`). Takes the `ID3D11Device` from the frames' device context (`:719-756`) | `nvEncRegisterResource(resourceType=DIRECTX, subResourceIndex=data[1])` cached by (ptr, index) (`:2223-2275`). `sw_format` NV12 → `NV_ENC_BUFFER_FORMAT_NV12`. Driver floor set by the build's nv-codec-headers (§6.1) |
| **`h264_qsv` / `hevc_qsv` / `av1_qsv`** | **Not D3D11.** HW configs are only `FRAMES(QSV, QSV)`, `DEVICE(NV12, QSV)` and `DEVICE(P010, QSV)` (`qsvenc.c:2763-2765`) | `av_hwdevice_ctx_create_derived(&qsv_dev, AV_HWDEVICE_TYPE_QSV, d3d11_dev, 0)`. This picks the VPL implementation by the device's **adapter LUID** (`qsv_d3d11_update_config`, `hwcontext_qsv.c:832-900`). Then `av_hwframe_ctx_create_derived(&qsv_frames, AV_PIX_FMT_QSV, qsv_dev, d3d11_frames, 0)`, then per frame `av_hwframe_map(qsv_frame, d3d11_frame, READ\|DIRECT)` with `qsv_frame->format = AV_PIX_FMT_QSV` and `hw_frames_ctx = ref(qsv_frames)`. Encoder: `pix_fmt = AV_PIX_FMT_QSV`, `hw_frames_ctx = qsv_frames` | **Needs the dynamic pool (§2.3).** Surface info is aligned to 16 (`FFALIGN(w/h,16)`, `hwcontext_qsv.c:694-697`). The encoder crop comes from `avctx->width/height` (`qsvenc.c:837-838, 870-875`). **[Unverified]** Whether an exact 1920×1080 texture is accepted when `Info.Height=1088`; fallback: allocate 16-aligned (H.264) or 32-aligned (HEVC/AV1) frames and keep `avctx` at the real size. Requires oneVPL (`--enable-libvpl`, vpl ≥ 2.6); dynamic pools return early in `qsv_frame_alloc` (`qsv.c:834-838`) |
| **`h264_mf` / `hevc_mf`** | D3D11 frames directly in 9.0: `CODEC_PIXFMTS(NV12, YUV420P, D3D11)` (`mfenc.c:1484`) | `pix_fmt = AV_PIX_FMT_D3D11` with `hw_frames_ctx`. On the first frame it creates an `IMFDXGIDeviceManager` with the frames' device and sends `MFT_MESSAGE_SET_D3D_MANAGER` (`:330-355`). Then `MFCreateDXGISurfaceBuffer(tex, data[1])` (`:357-416`) | Set **`hw_encoding=1`**; otherwise it may pick the software MFT (`:1262`). The MFT is the **first** hardware MFT enumerated, with **no adapter-LUID filter**, which can be wrong on multi-GPU PCs. **No colour attributes are set** (no `MF_MT_YUV_MATRIX`/`NOMINAL_RANGE`), so VUI colour depends on the MFT. Default `g=0` (`:1478-1481`). Extradata comes from `MF_MT_MPEG_SEQUENCE_HEADER`, waiting up to 70 ms on async MFTs (`:1295-1316`) |

**Alternatives to rendering into pool frames**
- *Wrap an external texture.* Build an `AVFrame` with `format=AV_PIX_FMT_D3D11`, `data[0]=tex`, `data[1]=0`, `hw_frames_ctx=ref(encoder's frames ctx)` (AMF requires that one), and a `buf[0]` made with `av_buffer_create`. Its free callback must return the texture to *your* pool only when the encoder lets go; AMF and NVENC hold refs until output. More code, no benefit.
- *Copy.* `CopySubresourceRegion(pool_tex, idx, …, own_nv12, 0)` copies both NV12 planes. It costs one 3 MB GPU copy per frame. Use it only if the RTV path fails.

### 2.6 Opening the encoder (AMF example; others differ only in options)

```rust
let codec = ffmpeg_next::encoder::find_by_name("h264_amf").ok_or(anyhow!("h264_amf missing"))?;
let mut v = ffmpeg_next::codec::Context::new_with_codec(codec).encoder().video()?; // applies codec defaults (+loop, g=-1)
v.set_width(1920); v.set_height(1080);
v.set_format(ffmpeg_next::format::Pixel::D3D11);
v.set_time_base((1, 60)); v.set_frame_rate(Some((60, 1)));
v.set_gop(60);                     // 1 s
v.set_max_b_frames(0);             // generic bf; AMF reads its private "bf" below
v.set_bit_rate(25_000_000); v.set_max_bit_rate(40_000_000);
v.set_color_range(ffmpeg_next::color::Range::MPEG);
v.set_colorspace(ffmpeg_next::color::Space::BT709);
v.set_color_primaries(ffmpeg_next::color::Primaries::BT709);
v.set_color_transfer_characteristic(ffmpeg_next::color::TransferCharacteristic::BT709);
unsafe {
    let c = v.as_mut_ptr();
    (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;  // OR. Never set_flags(): it drops "+loop"
    (*c).rc_buffer_size = 40_000_000;
    (*c).hw_frames_ctx = ffi::av_buffer_ref(frames.0);
    for (k, val) in [("usage","transcoding"), ("quality","balanced"), ("rc","vbr_peak"), ("vbaq","1"),
                     ("bf","0"), ("header_spacing","60"), ("async_depth","4")] {
        let (k, val) = (CString::new(k)?, CString::new(val)?);
        let r = ffi::av_opt_set((*c).priv_data, k.as_ptr(), val.as_ptr(), 0);
        anyhow::ensure!(r >= 0, "option {:?}: {}", k, ffmpeg_next::Error::from(r));
    }
}
let enc = v.open_as(codec)?;                                   // avcodec_open2
let params = ffmpeg_next::codec::Parameters::from(&*enc);      // codecpar incl. extradata + colour, for muxing
```

**Driving the encoder**
- AMF, NVENC and QSV implement `receive_packet`: the real submission happens inside `avcodec_receive_packet`. Always drain it after each `send_frame`.
- When the AMF queue is full, `ff_amf_receive_packet` loops with `av_usleep(1000)` (`amfenc.c:655-668`). It can block the pacer thread for up to a frame period. Size `async_depth` accordingly.
- At session end, `send_eof` then drain until EOF to collect the tail packets.

### 2.7 Startup probe

- Pick candidates by the adapter's `DXGI_ADAPTER_DESC1.VendorId`: 0x1002 → AMF, 0x10DE → NVENC, 0x8086 → QSV, then `*_mf` (`hw_encoding=1`).
- For each candidate:
  1. Build the real configuration at the real output size.
  2. `avcodec_open2`.
  3. Send about 5 frames, then drain.
  4. Require ≥1 packet with `AV_PKT_FLAG_KEY` and `extradata_size > 0`.
  5. `avcodec_open2` alone is not proof: AMF and VPL can fail at the first submit.
- Record in the capability report:
  - codec name;
  - `avcodec_configuration()` (the configure line);
  - `av_version_info()`;
  - the AMF runtime version (`AVAMFDeviceContext.version` via the derived device);
  - extradata length;
  - the options that were refused.

---

## 3. NV12 conversion on the GPU (HLSL)

### 3.1 Pipeline

```
capture tex (BGRA8 UNORM | RGBA16F scRGB) ──Pass A: scale (+tone-map if HDR) ──► RGBA8 UNORM intermediate @ output size (gamma-encoded)
                                                                                     │
                                       Pass B1: Y  (RTV R8_UNORM,  viewport W×H)  ◄───┤
                                       Pass B2: UV (RTV R8G8_UNORM, viewport W/2×H/2) ◄┘   → FFmpeg pool NV12 texture
```

- **Two passes, as OBS does.** OBS scales into the canvas first, then runs `NV12_Y` and `NV12_UV` from `format_conversion.effect` (`:1168-1184`). Separate passes keep scaling, tone-mapping and chroma filtering simple and correct. The extra 1080p RGBA write and read is negligible **[Estimate: ≪ 0.2 ms]**.
- **Skip Pass A** when the source and output sizes match and the source is SDR. Pass B can then read the capture texture directly.
- **Fullscreen triangle, no vertex buffer:** `IASetInputLayout(None)`, `TRIANGLELIST`, `Draw(3, 0)`.

### 3.2 BT.709 limited-range matrix (RGB in [0,1], gamma-encoded)

Computed with Kr = 0.2126 and Kb = 0.0722, Y scaled by 219/255 and C by 224/255. **[H: arithmetic, cross-checked against SMPTE 75% bars]**

```
Y  =  0.182585882 R + 0.614230588 G + 0.062007059 B + 0.062745098   (16/255)
Cb = -0.100643732 R - 0.338571954 G + 0.439215686 B + 0.501960784   (128/255)
Cr =  0.439215686 R - 0.398942163 G - 0.040273524 B + 0.501960784
```

Expected 8-bit codes, used by the pixel test in §10:

| Input | Y | Cb | Cr |
|---|---|---|---|
| black | 16 | 128 | 128 |
| white 100% | 235 | 128 | 128 |
| white 75% | 180 | 128 | 128 |
| yellow 75% | 168 | 44 | 136 |
| cyan 75% | 145 | 147 | 44 |
| green 75% | 133 | 63 | 52 |
| magenta 75% | 63 | 193 | 204 |
| red 75% | 51 | 109 | 212 |
| blue 75% | 28 | 212 | 120 |

- The SDR desktop is sRGB-encoded. Like OBS and Game Bar, Kairo applies the 709 matrix to those values and signals BT.709 (1/1/1). The difference between the sRGB and 709 transfer curves is ignored by every desktop recorder **[M]**.

### 3.3 HLSL sketch

```hlsl
// kairo_nv12.hlsl: SDR/HDR → NV12, BT.709 limited range.
// Portions derived from OBS Studio libobs/data/color.effect and format_conversion.effect,
// Copyright (C) Lain Bailey <lain@obsproject.com>, jpark37 and OBS contributors,
// GPL-2.0-or-later; distributed here under GPL-3.0-or-later. Modified for Kairo, 2026-10.

struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut VSMain(uint id : SV_VertexID) {                    // (0,0) (2,0) (0,2) fullscreen triangle
    VSOut o; float2 t = float2((id << 1) & 2, id & 2);
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1); o.uv = t; return o;
}

Texture2D<float4> src : register(t0);
SamplerState linear_clamp : register(s0);                // MIN_MAG_MIP_LINEAR, ADDRESS_CLAMP

cbuffer Tone : register(b0) { float scrgb_to_pq_unit; float lw_nits; float lmax_nits; float rel_scale; };
// scrgb_to_pq_unit = 80/10000 (scRGB 1.0 = 80 nits; PQ helpers use 1.0 = 10000 nits)
// rel_scale = 10000 / lmax_nits (so the target peak maps to 1.0)

// --- Pass A ---
float4 PS_ScaleSdr(VSOut i) : SV_Target { return float4(src.SampleLevel(linear_clamp, i.uv, 0).rgb, 1); }

float4 PS_ScaleHdrToSdr(VSOut i) : SV_Target {           // needs rec709_to_rec2020, maxRGB_eetf_linear_to_linear,
    float3 c = src.SampleLevel(linear_clamp, i.uv, 0).rgb * scrgb_to_pq_unit;   //       rec2020_to_rec709 and
    c = max(rec709_to_rec2020(c), 0.0);                  //       srgb_linear_to_nonlinear from OBS color.effect
    if (lw_nits > lmax_nits) c = maxRGB_eetf_linear_to_linear(c, lw_nits, lmax_nits);
    c = saturate(rec2020_to_rec709(c) * rel_scale);      // gamut clip to 709, 1.0 = target peak
    return float4(srgb_linear_to_nonlinear(c), 1);       // same transfer as the SDR desktop
}

// --- Pass B (src = Pass A output, same size as the luma plane) ---
cbuffer Yuv : register(b0) { float4 cy; float4 cu; float4 cv; float2 texel; float2 pad; }; // texel = (1/W, 1/H)

float PS_Y(VSOut i) : SV_Target {                         // 1:1 texel load
    float3 c = src.Load(int3(i.pos.xy, 0)).rgb;
    return dot(cy.xyz, c) + cy.w;
}

float2 PS_UV(VSOut i) : SV_Target {                       // rendered at W/2 × H/2
    // i.uv = ((2x+1)/W, (2y+1)/H): the shared edge of luma columns 2x|2x+1 and rows 2y|2y+1.
    // Two bilinear taps at u-1/W and u give [1 2 1]/4 horizontally around column 2x
    // ("left" siting, H.264 default) and [1 1]/2 vertically, as OBS's VSTexPos_Left/PS_UV_Wide.
    float3 a = src.SampleLevel(linear_clamp, float2(i.uv.x - texel.x, i.uv.y), 0).rgb;
    float3 b = src.SampleLevel(linear_clamp, i.uv, 0).rgb;
    float3 c = 0.5 * (a + b);
    return float2(dot(cu.xyz, c) + cu.w, dot(cv.xyz, c) + cv.w);
}
```

- **Scaling quality.** Bilinear in Pass A is fine at exactly 2× (4K → 1080p becomes a box filter). At 1440p → 1080p (the user's monitors, 1.333×) it softens and aliases slightly. Port OBS `bicubic_scale.effect` (also GPL-2+) if S2 screenshots show it **[Estimate]**.
- **Gamma-correct scaling (optional).** Copy the WGC frame into a `B8G8R8A8_TYPELESS` texture (`CopyResource` allows the same type group). Sample it through a `B8G8R8A8_UNORM_SRGB` SRV, which linearises, and write Pass A to an `R8G8B8A8_UNORM_SRGB` RTV, which re-encodes. You cannot create an `_SRGB` view on a fully typed `B8G8R8A8_UNORM` texture **[M: D3D11 view-casting rules]**.
- **Constants.** Pass B's `cy`/`cu`/`cv` are the §3.2 rows: xyz = coefficients, w = offset.

### 3.4 The EETF code being ported (OBS `color.effect`, verbatim logic)

```hlsl
float eetf_0_Lmax(float maxRGB1_pq, float Lw, float Lmax) {   // ITU-R BT.2390 EETF, PQ domain
    float Lw_pq = linear_to_st2084_channel(Lw / 10000.);
    float E1 = saturate(maxRGB1_pq / Lw_pq);
    float maxLum = linear_to_st2084_channel(Lmax / 10000.) / Lw_pq;
    float KS = (1.5 * maxLum) - 0.5;
    float E2 = E1;
    if (E1 > KS) {
        float T = (E1 - KS) / (1. - KS), T2 = T * T, T3 = T2 * T;
        E2 = (2.*T3 - 3.*T2 + 1.) * KS + (T3 - 2.*T2 + T) * (1. - KS) + (-2.*T3 + 3.*T2) * maxLum;
    }
    return E2 * Lw_pq;
}
// maxRGB_eetf_linear_to_linear(rgb, Lw, Lmax) scales rgb by EETF(maxRGB)/maxRGB, with maxRGB floored at 6.10352e-5
```

### 3.5 HDR → SDR: parameters and numbers

**Inputs**
- **SDR white.** `DisplayConfigGetDeviceInfo(DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL)`. `SDRWhiteLevel` is "a multiplier of 80 nits, multiplied by 1000", so nits = value / 1000 × 80 **[H, MS Learn]**.
- **Source peak `Lw`.** `IDXGIOutput6::GetDesc1().MaxLuminance`, clamped to about 400–4000 nits. The same call gives `ColorSpace`, which detects HDR (`DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`) **[H: windows-rs 0.62.2 exposes `GetDesc1 -> DXGI_OUTPUT_DESC1`]**.

**EETF results with Lmax = SDR white [H: arithmetic, using the OBS formulas]**

| Lw (nits) | Lmax = SDR white | Knee (identity below) | SDR white lands at | 1000-nit highlight lands at |
|---|---|---|---|---|
| 1000 | 240 | ≈114 nits | 193 nits (80%) | 240 (100%) |
| 1000 | 300 | ≈161 nits | 250 nits (83%) | 300 |
| 800 | 200 | ≈97 nits | 162 nits (81%) | 200 |

**Options to expose (decide in S1 with HDR on, on the 360 Hz OLED)**
- **"Natural" (EETF), recommended default.** Midtones unchanged; highlights roll off; UI at SDR white reads slightly dim.
- **"Match SDR apps" (clip).** OBS's force-SDR path: `rgb × 80 / SDR_white`, then clip. UI is exact; HDR highlights clip harshly.
- **Reinhard.** OBS's default for display capture: SDR white lands at about 75% sRGB code and everything compresses slightly.

**Gamut and recapture**
- scRGB can be negative for wide-gamut colours. Tone-map in Rec.2020 and clamp, as OBS does, then clip to 709.
- When the user toggles HDR, recreate the capture with the other pool format and switch the Pass A shader. The encoder is unaffected: it stays NV12 8-bit, BT.709.

### 3.6 Capability check and fallbacks (copy OBS)

**Checks** (OBS `d3d11-subsystem.cpp:575-687`)
- Enable NV12 output only if `CheckFormatSupport(DXGI_FORMAT_NV12)` has `TEXTURE2D | RENDER_TARGET`.
- OBS also requires `ExtendedResourceSharing`, because it shares textures. Kairo, on one device, does not need that.
- Skip WARP (0x1414:0x8c).

**Self-test**
- OBS `HasBadNV12Output()` renders 1.0 into the Y plane and (1,1) into the UV plane of a 128×128 NV12 texture.
- It then stages the texture and checks the UV bytes are non-zero. Some drivers silently drop plane writes **[H]**.
- Kairo should do the same at startup and log the result in the capability report.

**Fallback when NV12 RTV fails**
- Make the frames context `sw_format = AV_PIX_FMT_BGRA` and copy the BGRA image into it.
- `h264_amf` and `h264_nvenc` accept BGRA D3D11 frames and convert internally (`ff_amf_pix_fmts`; NVENC RGB modes).
- Kairo loses exact control of matrix and range. Verify the VUI with ffprobe.
- QSV cannot take this path, so QSV falls back to `h264_mf`.

### 3.7 Licence check of the OBS shaders [H]

- **`libobs/data/format_conversion.effect`** carries the header "Copyright (C) 2023 by Lain Bailey … GNU General Public License … either version 2 of the License, or (at your option) any later version."
- **`libobs/data/color.effect`** has **no** file header. It is covered by the repository licence: README.rst says "distributed under the GNU General Public License v2 (or any later version)", and COPYING is the GPLv2 text.
  - Its git history is mostly by **jpark37**, 2022-2024, plus Anastasiia Sidorovskaia and Norihiro Kamae.
- **GPL-3.0 Kairo can include both**, by taking the "or later" option.
- **GPLv3 §5(a) duties:**
  - keep the copyright and licence notices;
  - add a prominent "modified by Kairo, <date>" note;
  - list the files in `THIRD_PARTY_NOTICES`.

### 3.8 Shader compilation: runtime `D3DCompile` (recommended)

- **The API.** `windows` 0.62.2 provides `Win32::Graphics::Direct3D::Fxc::D3DCompile(psrcdata, len, name, defines, include, entry, target, flags1, flags2, &mut Option<ID3DBlob>, Option<&mut Option<ID3DBlob>>)`.
- **No import library.** It links via `windows_core::link!("d3dcompiler_47.dll" …)`, and **windows-link uses `kind = "raw-dylib"`** on Windows targets. So nothing is needed at link time, under MSVC or cargo-xwin **[H: crate source]**.
- **The DLL.** `d3dcompiler_47.dll` ships with Windows 8.1, 10 and 11 **[M: secondary sources + OBS relies on it]**.
- **Precedent.** OBS compiles every effect at startup with `D3DCompile(…, "main", "vs_4_0"/"ps_4_0", D3D10_SHADER_OPTIMIZATION_LEVEL3, …)`. It caches blobs under `obs-studio/shader-cache` keyed by an FNV-1a hash of the source (`d3d11-shader.cpp:236-330`) **[H]**.
- **Kairo:**
  - embed the HLSL with `include_str!`;
  - compile `vs_5_0`/`ps_5_0` (Kairo requires FL 11) with `D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS` at engine start, off the hot path;
  - optionally cache in `%LOCALAPPDATA%\Kairo\shader-cache\<hash>.cso`.
- **Why not fxc in `build.rs`:**
  - `fxc.exe` is a Windows binary in the Windows SDK; xwin brings headers and libs, not SDK tools.
  - DXC runs on macOS and Linux but only emits DXIL/SPIR-V (SM 6+), which D3D11 cannot use.
  - So `build.rs` shader compilation would break `cargo xwin check` on the Mac.
- **If precompiled blobs are wanted later**, generate them on the Windows PC with `fxc /T ps_5_0 /E PS_Y /O3 /Fo …`, check them in, and add a test that the source hash matches.
- **Shader errors only surface on Windows.** Add a `#[test]` that compiles every entry point, run with `cargo test` over SSH. No HLSL compiler for SM5 is readily available on the Mac.

```rust
use windows::{core::{s, PCSTR}, Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude, Fxc::*}};
unsafe fn compile(src: &str, entry: PCSTR, target: PCSTR) -> anyhow::Result<Vec<u8>> {
    let (mut code, mut errs): (Option<ID3DBlob>, Option<ID3DBlob>) = (None, None);
    let r = D3DCompile(src.as_ptr().cast(), src.len(), s!("kairo_nv12.hlsl"), None, None::<&ID3DInclude>,
                       entry, target, D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS, 0,
                       &mut code, Some(&mut errs as *mut _));   // windows 0.62: Option<*mut Option<ID3DBlob>>
    if let Err(e) = r {
        let msg = errs.map(|b| String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned());
        anyhow::bail!("D3DCompile {e}: {}", msg.unwrap_or_default());
    }
    let b = code.unwrap();
    Ok(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec())
}
// device.CreatePixelShader(&bytes, None, Some(&mut ps))?
```

---

## 4. Encoder settings per vendor (libavcodec AVOptions, FFmpeg 9.0.2)

### 4.1 Common `AVCodecContext` fields (all video encoders)

| Field | Value | Why |
|---|---|---|
| `width`/`height` | even output size | NV12 rule |
| `time_base` / `framerate` | `1/fps` / `fps/1` | The pacer gives CFR ticks. pts = tick index |
| `gop_size` | `fps` (60) | 1 s GOP. **AMF's default is `g=-1`** (codec default, so the driver decides) |
| `max_b_frames` | **0 in v1** (see §4.4) | NVENC and QSV default `bf=-1` (preset or runtime decides). AMF reads its *private* `bf` |
| `bit_rate`, `rc_max_rate`, `rc_buffer_size` | 25 M / 40 M / 40 M (1080p60) | VBR peak; VBV = 1 s at peak **[Estimate]** |
| `color_range/colorspace/color_primaries/color_trc` | MPEG / BT709 / BT709 / BT709 | Drives the VUI in AMF (`amfenc_h264.c:277-298`), NVENC (`nvenc.c:1145-1166`) and QSV (`qsvenc.c:1216-1236`), and the MP4 `colr`. **MF ignores them** |
| `flags` | `\|= AV_CODEC_FLAG_GLOBAL_HEADER` | The MP4 muxer has `AVFMT_GLOBALHEADER` (`movenc.c:9358`) |
| `pix_fmt` + `hw_frames_ctx` | D3D11 (AMF, NVENC, MF) / QSV | §2.5 |

### 4.2 Private options

| Encoder | Recommended v1 options | Notes [H unless marked] |
|---|---|---|
| **`h264_amf`** | `usage=transcoding quality=balanced rc=vbr_peak vbaq=1 bf=0 header_spacing=60 async_depth=4` (+ `profile=high`) | `rc`: cqp, cbr, vbr_peak, vbr_latency, qvbr, hqvbr, hqcbr. Auto picks peak VBR when maxrate ≠ bitrate (`:298-313`). `gop_size` → `AMF_VIDEO_ENCODER_IDR_PERIOD` (`:574-576`), so every keyframe is an IDR. `header_spacing` → `HEADER_INSERTION_SPACING` (`:578-580`); its AMF default is **[Unverified]**. `bf` = B pattern, used only if `AMF_VIDEO_ENCODER_CAP_BFRAMES`, otherwise it warns and disables (`:476-530`). VBAQ is ignored under CQP. `forced_idr=1` only matters for frames you force to `pict_type=I`. Also: `preanalysis` and `pa_*` (quality, extra cost), `enforce_hrd`, `filler_data`, `frame_skipping`, `max_au_size`, `aud`, `coder` |
| **`hevc_amf`** | `usage=transcoding quality=balanced rc=vbr_peak vbaq=1 gops_per_idr=1 header_insertion_mode=idr async_depth=4` | **No B-frame option exists.** `gops_per_idr` defaults to 1 (`:97`). `gop_size` → `HEVC_GOP_SIZE` (`:298-301`). `header_insertion_mode` none/gop/idr. `bitdepth`, `profile_tier` |
| **`av1_amf`** | `usage=transcoding quality=balanced rc=vbr_peak header_insertion_mode=gop async_depth=4` (+ `bf=0`) | `align` defaults to `none` (no restrictions). On Navi3x the encoder reports alignment factors 64×16. FFmpeg then adds `AV_PKT_DATA_FRAME_CROPPING` coded side data (`:666-712`) and movenc writes `clap` (`movenc.c:3026-3041`). Player support for `clap` is weak. **Test 1920×1080 AV1 on the user's GPU.** `bf` only if `AV1_CAP_BFRAMES`. Odd code: `if (avctx->bit_rate > 0) rate_control_mode = CBR` runs *after* the RC property is set (`:414-417`), so read back the actual mode in DEBUG logs **[Unverified effect]** |
| **`h264_nvenc`** | `preset=p5 tune=hq rc=vbr bf=0 spatial-aq=1 aq-strength=8` (+ `profile=high`) | Presets p1-p7 (default p4); `tune` hq/ll/ull/lossless; `rc` constqp/vbr/cbr only, since pre-11.1-era modes were removed in 9.0. `gopLength=gop_size`, `idrPeriod = gopLength` (`:1197, 1761-1772`), so every GOP is IDR-led. **With GLOBAL_HEADER: `disableSPSPPS=1, repeatSPSPPS=0`** (`:1188-1189`). `forced-idr` defaults to 0, so a forced `pict_type=I` becomes FORCEINTRA (`:3017-3021`). `b_ref_mode=middle` only if bf ≥ 2. `strict_gop`, `rc-lookahead`, `multipass` stay off |
| **`h264_qsv`** | `preset=medium bf=0 async_depth=4 look_ahead=0 adaptive_i=0 adaptive_b=0` (+ `profile=high`) | **Codec defaults include `flags=+cgop`**, so the GOP is closed (`MFX_GOP_CLOSED`, `qsvenc.c:818-820`). `idr_interval=0` means every I is an IDR. `low_power` is -1, so the runtime decides (`:806-811`). Bitrate + maxrate ≠ bitrate gives `MFX_RATECONTROL_VBR` (`select_rc_mode`, `:570-660`). Timestamps go through 90 kHz (`:2669`). `repeat_pps` |
| **`h264_mf`** | `hw_encoding=1 rate_control=pc_vbr scenario=camera_record bf=0` | Options: rate_control (default, cbr, pc_vbr, u_vbr, quality, ld_vbr, g_vbr, gld_vbr), scenario, quality, hw_encoding (`mfenc.c:1452-1476`). `camera_record` selects CFR so the MFT does not drop frames (`:866-871` comment). GOP → `CODECAPI_AVEncMPVGOPSize`. IDR and closed-GOP behaviour depend on the MFT **[Unverified]** |

### 4.3 GOP, IDR and parameter-set behaviour

| | GOP start is IDR | Closed GOP | In-band SPS/PPS with GLOBAL_HEADER | Extradata after `avcodec_open2` |
|---|---|---|---|---|
| AMF H.264 | yes (IDR period) | yes | only with `header_spacing` | **always** (`AMF_VIDEO_ENCODER_EXTRADATA`, Annex B, `:604-628`) |
| AMF HEVC | yes (`gops_per_idr=1`) | yes | `header_insertion_mode=idr` | always |
| NVENC | yes | yes | **no** | yes (sequence params) |
| QSV | yes (`idr_interval=0`) | yes (`+cgop` default) | yes at IDR **[Unverified]** | always (`qsvenc.c:1428-1540`) |
| MF | MFT-dependent **[Unverified]** | MFT-dependent | MFT-dependent | from the output media type, or after ≤ 70 ms |

- **MP4 needs only extradata.** movenc turns Annex B extradata into `avcC` and reformats Annex B packets to length-prefixed itself.
- **In-band headers will matter later** for the disk-backed TS ring (v1.x). Use the `dump_extra` bitstream filter (`freq=keyframe`, `libavcodec/bsf/dump_extradata.c`), because NVENC has none.

### 4.4 B-frames: the v1 recommendation

**Recommend `bf=0` in v1 [M]**
- pts = dts for every packet, so cutting at an IDR is trivial.
- No `ctts`, and **`use_editlist=0` gives a file with no `edts` at all**, which YouTube and Instagram ask for.
- It behaves the same on every vendor: AMD HEVC has no B-frames, and AMD H.264 B-frames are capability-gated.
- At 25 Mbps 1080p60 the quality cost should be small **[Estimate; measure with VMAF in S2]**.

**What B-frames would cost**
- With B-frames and no edit list, movenc shifts all streams so the first video dts is 0.
- It then forces each track's first sample to dts 0, extending that sample (`movenc.c:7282-7290`).
- Video then presents from about the B-frame delay (2 frames ≈ 33 ms at 60 fps), while audio starts at 0. The result is about 33 ms of A/V offset on top of the ≤ 21 ms AAC quantisation **[M: source reading]**.
- If you enable B-frames, keep the default `use_editlist` (auto → 1, `movenc.c:8203-8213`), or use `movflags=+negative_cts_offsets`. That gives `ctts` v1 and the `iso4` brand (`:6215-6216, 3226-3229`). Player support for it on Windows/Media Foundation is **[Unverified]**.

### 4.5 Timestamps (one clock for all streams)

- **One clock.** Keep an engine epoch, a QPC value taken at start.
  - Video pts = pacer tick k (time_base 1/fps).
  - Audio pts = sample index on the 48 kHz timeline (time_base 1/48000).
  - Both are exact functions of the same epoch, so no 100 ns arithmetic is needed when cutting.
- **Keep counting across encoder sessions.** Ticks continue when the encoder is recreated after a resize, device loss or HDR toggle.
- **Output packets** carry pts in the encoder time_base. AMF passes pts through and computes dts (`amfenc.c:254-276`). NVENC's dts equals pts when `frameIntervalP=1` (`nvenc.c:2470-2485`). QSV goes via 90 kHz.
- **Set `pkt.duration`** to 1 tick (video) and 1024 samples (AAC sets it), so the last sample isn't zero-length.
- **AAC priming.** The native AAC encoder sets `initial_padding = 1024` (`aacenc.c:1165`), so output pts = input pts − 1024. Mid-stream packets are therefore already on the decoded-output timeline. Use them as-is.

### 4.6 Keyframe flags

- `AV_PKT_FLAG_KEY` marks:
  - **AMF:** IDR only (H.264, HEVC), KEY (AV1) (`amfenc.c:205-240`).
  - **NVENC:** `NV_ENC_PIC_TYPE_IDR` (`nvenc.c:2624-2625`).
  - **QSV:** IDR/xIDR (`qsvenc.c:2672-2674`).
  - **MF:** `MFSampleExtension_CleanPoint` (`mfenc.c:289-291`).
- Index the ring by these flags.

---

## 5. Muxing MP4 from the RAM ring

### 5.1 Storage and snapshot

**Storage**
- Keep each encoder output as an `ffmpeg_next::Packet`, holding a refcounted `AVBufferRef` that encoders allocate per packet.
- Store with it: stream id, encoder-session id, pts/dts/duration in the encoder time_base, and the key flag.
- Count RAM as Σ`size` + 64 bytes padding per packet.

**Snapshot (microseconds)**
- Under the ring lock, take `av_packet_ref` copies of the selected packets into a `Vec`. These are refcount bumps, not copies.
- **Do not use `Packet::clone()`** (§1.3).

**Session parameters**
- Store one `codec::Parameters` (codecpar with extradata and colour) per encoder session, taken right after `avcodec_open2`.
- **A save never spans two sessions** with different extradata. Cut at the newest session boundary, or write two files.
- The CPU-side ring survives device loss and TDR; only the GPU state is rebuilt.

### 5.2 Cut and rebase

1. Find the last video packet with `AV_PKT_FLAG_KEY` and `pts ≤ T−pre`. Call it K. With `bf=0`, decode order equals presentation order.
2. Video: take the packets from K up to `T+post`, with `pts' = pts − K.pts` and `dts' = dts − K.pts`.
3. Audio, for each stream, with t0 = K.pts / fps:
   - take packets from the first AAC packet whose `pts ≥ t0·48000` (the brief's rule; the offset is 0 to 21.3 ms);
   - set `pts' = pts − round(t0·48000)`.
   - Choosing the frame *nearest* to t0 halves the worst case to ±10.7 ms.
4. Write the packets in global dts order. Compare across time bases with `av_compare_ts`.

### 5.3 Muxer options

| Option | Value | Source |
|---|---|---|
| `movflags` | `+faststart` | "Run a second pass to put the index (moov atom) at the beginning". The shift re-opens the URL for reading through `s->io_open(…, AVIO_FLAG_READ)` (`mux_utils.c:~90`), so **write to a real file path** |
| `use_editlist` | `0` (with `bf=0`) | default −1 → 1 for non-fragmented output (`movenc.c:8203-8213`) |
| `avoid_negative_ts` | leave auto. With `use_editlist=0` movenc switches it to `make_zero` (`:8222-8224`) | one global offset for all streams (libavformat mux.c) |
| `moov_size` (optional, later) | estimated bytes | Reserves room for the moov at the front and skips the second pass. **If the estimate is too small, the trailer fails** with "reserved_moov_size is too small" (`:9087-9094`). Start with faststart |

### 5.4 Stream setup

**Per stream**
- Create with `octx.add_stream(ffmpeg_next::codec::Id::None)` (null codec), then `st.set_parameters(session_params.clone())`.
- Set `codecpar->codec_tag = 0` so movenc picks `avc1`/`hvc1`/`av01`/`mp4a`.
- Give `set_time_base` a hint: video `1/fps`, audio `1/48000`.

**After `write_header_with`, re-read each `st.time_base()`**
- movenc sets the video timescale to `time_base.den`, doubled until ≥ 10000: 60 → 15360, 144 → 18432 (`:8492-8494`).
- Audio uses `sample_rate` (`:8542`).
- `avpriv_set_pts_info` applies them (`:8630`).
- `pkt.rescale_ts(src_tb, st_tb)`.

**Audio tracks**
- Set `disposition = AV_DISPOSITION_DEFAULT` on the **mixed track only**.
- movenc enables the tracks marked default, or the first track of each type when none is marked (`enable_tracks`, `:7885-7925`).
- All audio tracks share `alternate_group = 1`. They get per-stream groups only if more than one audio track is enabled (`:4009-4014`).

**Names**
- Stream metadata `title` becomes the per-track `udta/name` (`:4372-4373`). `handler_name` becomes the `hdlr` name (`:3624`). Also set `language` (`und`/`eng`).
- Suggested titles: "Mix", "Game", "Mic", "Voice chat".

### 5.5 The `colr` atom

- movenc writes `colr nclx` (MP4) when `color_primaries`, `color_trc` **and** `color_space` are all not `UNSPECIFIED` (`:2944-2953`).
- The box holds `primaries, transfer, matrix` (16-bit each) and a byte `full_range<<7` (`:2611-2658`).
- Setting the four colour fields on the encoder before `avcodec_open2` propagates them through `avcodec_parameters_from_context`.
- `movflags=+write_colr` forces a box even when unspecified; it is marked experimental.
- For later HDR10, `clli` and `mdcv` come from side data (`:2957-2958`).

### 5.6 Metadata

| Key (format level) | MP4 atom | Use |
|---|---|---|
| `title` | `©nam` | "VALORANT – 2026-10-21 21:14" |
| `comment` | `©cmt` | "Captured with Kairo" |
| `encoding_tool` | `©too` | "Kairo 0.1.0 (Lavf 63.1.102)". It replaces the default Lavf ident (`:4854-4857`) |
| `description` | `desc` | JSON: `{"kairo":1,"game":"…","exe":"…","session":"…","tracks":[…]}`. This mirrors the library for a rescan |
| `date` | `©day` | ISO date |
| `creation_time` | `mvhd`/`tkhd`/`mdhd` creation time | UTC capture time |

- **Keep iTunes-style tags.** `movflags=+use_metadata_tags` *replaces* them with QuickTime `mdta` keys (`:4957-4961`), which Explorer is unlikely to show **[M]**. There is no generic freeform `----` writer.

### 5.7 Native AAC encoder (LGPL, part of libavcodec)

**Encoder facts**
- Input: `AV_SAMPLE_FMT_FLTP` (`aacenc.c:1317`); `frame_size = 1024` (`:1164`).
- Options: `aac_coder` (twoloop, the default, or fast), `aac_ms`, `aac_is`, `aac_pns`, `aac_tns`, `aac_pce` (`:1278-1287`); profile `aac_low` (default).
- Codec default `b=0`, so always set the bitrate.

**Kairo settings**
- 48 kHz, stereo 160 kbps per stereo track (mic: mono 96–128 kbps), `ch_layout` STEREO/MONO, time_base 1/48000.
- Chunk PCM into exact 1024-sample frames, with your own planar FIFO or `ffi::av_audio_fifo_*`.
- 4 parallel encoders with twoloop cost about 0.5–1% of an 8-core CPU **[Estimate, measure]**. Drop to `aac_coder=fast` if CPU is too high.
- AAC patent licensing is out of scope (brief §8.6).

### 5.8 Mux skeleton

```rust
use ffmpeg_next::{self as ff, ffi, packet::Mut as _, packet::Ref as _};

fn write_clip(tmp: &Path, snap: &Snapshot, meta: &ff::Dictionary) -> anyhow::Result<()> {
    let mut octx = ff::format::output_as(tmp, "mp4")?;
    for (i, s) in snap.streams.iter().enumerate() {
        let mut st = octx.add_stream(ff::codec::Id::None)?;
        st.set_parameters(s.params.clone());
        st.set_time_base(s.time_base);
        st.set_metadata(s.tags.clone());                    // title, handler_name, language
        unsafe {
            (*(*st.as_mut_ptr()).codecpar).codec_tag = 0;
            (*st.as_mut_ptr()).disposition = if s.is_default { ffi::AV_DISPOSITION_DEFAULT as i32 } else { 0 };
        }
    }
    octx.set_metadata(meta.clone());
    let mut o = ff::Dictionary::new();
    o.set("movflags", "+faststart");
    o.set("use_editlist", "0");
    let left = octx.write_header_with(o)?;
    anyhow::ensure!(left.iter().count() == 0, "unused muxer options");
    let tbs: Vec<ff::Rational> = (0..octx.nb_streams() as usize).map(|i| octx.stream(i).unwrap().time_base()).collect(); // AFTER header
    for p in snap.packets_dts_order() {
        let mut pkt = ff::Packet::empty();
        unsafe { ffi::av_packet_ref(pkt.as_mut_ptr(), p.pkt.as_ptr()); }   // ref, not Packet::clone()
        pkt.set_stream(p.out_index);
        pkt.set_pts(Some(p.pts - p.base)); pkt.set_dts(Some(p.dts - p.base));
        pkt.set_duration(p.duration);
        pkt.rescale_ts(p.time_base, tbs[p.out_index]);
        pkt.write_interleaved(&mut octx)?;                   // takes ownership of this ref
    }
    octx.write_trailer()?;                                   // runs the faststart shift
    Ok(())                                                   // then fsync (File::sync_all) + std::fs::rename into the library
}
```

- **Paths.** FFmpeg's `file` protocol on Windows converts UTF-8 to wide characters, with long-path handling, so Unicode game names are fine **[M]**. Pass `&Path` and keep the temp file on the same volume as the library, so the final `rename` is atomic.

---

## 6. Building and linking

### 6.1 FFmpeg 9 LGPL shared builds (BtbN) [H]

**Naming and retention** (release `latest` and the dated `autobuild-2026-10-20-13-08`)
- Floating: `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip` (77 MB).
- Pinned: `ffmpeg-n9.0.2-29-g6dfaba896f-win64-lgpl-shared-9.0.zip`, built from the release/9.0 branch at 6dfaba896f, which is 29 commits after n9.0.2.
- Retention: "the last build of each month is kept for two years; the last 14 daily builds are kept".
- → **Pin a month-end autobuild, and mirror the zip plus its matching source in Kairo's own GitHub Releases.**

**Contents of the 2026-10-20 9.0 lgpl-shared zip**
- `bin/`: `avcodec-63.dll` 86.9 MB, `avformat-63.dll` 21.7, `avfilter-12.dll` 30.1, `avdevice-63.dll` 4.7, `avutil-61.dll` 2.9, `swscale-10.dll` 2.2, `swresample-7.dll` 0.7. That is **149 MB of DLLs**, plus small `ffmpeg.exe`, `ffprobe.exe` and `ffplay.exe`.
- `lib/`: **MSVC import libraries** `avcodec.lib`, `avformat.lib`, `avutil.lib`, `swresample.lib`, `swscale.lib`, `avfilter.lib`, `avdevice.lib`, plus MinGW `.dll.a`, `.def` and pkg-config files (`variants/windows-install-shared.sh` copies `bin/*.lib`).
- `include/` includes `libavutil/hwcontext_d3d11va.h` and `hwcontext_amf.h`.
- `LICENSE.txt` is **LGPLv3**: `defaults-lgpl.sh` sets `--enable-version3` and `LICENSE_FILE=COPYING.LGPLv3`.

**Library majors** (`version*.h`): avcodec 63, avformat 63, avutil 61, swresample 7, swscale 10, avfilter 12, avdevice 63. libpostproc is gone.

**NVENC**
- BtbN builds FFmpeg ≥ 9.0 against nv-codec-headers at commit eddcea9e, "in-dev 13.1.15.1" (`scripts.d/50-ffnvcodec.sh`).
- Its README says SDK 13.1.15, "Windows: 610.0 or newer".
- The README floors per tag: n13.0.19.0 → 570.0; n12.2.72.0 → 551.76; n12.1.14.0 → 531.61.
- FFmpeg 9.0.2 configure accepts `ffnvcodec >= 12.1.14.0` (or 12.0.16.1, or 11.1.5.3) (`configure:7214-7216`).
- **A custom build can pick 12.1.14.0 to support drivers from 531.61.**

**Platform.** BtbN: "The minimum supported version is Windows 10 22H2". The builds need the UCRT.

### 6.2 Native build on the Windows PC

```powershell
winget install LLVM.LLVM                         # libclang for bindgen
setx LIBCLANG_PATH "C:\Program Files\LLVM\bin"
# unzip the pinned BtbN zip to C:\deps\ffmpeg-9.0-lgpl-shared
setx FFMPEG_DIR "C:\deps\ffmpeg-9.0-lgpl-shared"   # contains include\ and lib\
cargo build -p kairo-engine --release
Copy-Item C:\deps\ffmpeg-9.0-lgpl-shared\bin\*.dll target\release\   # implicit-link DLLs next to the exe
```

**How ffmpeg-sys-next uses this [H]**
- With `FFMPEG_DIR` set, it adds `cargo:rustc-link-search=native=%FFMPEG_DIR%\lib` and `rustc-link-lib=dylib=avcodec` etc., which links the `.lib` import libraries (`build.rs:1159-1189`).
- Its `check.c` feature probe uses cc-rs's host MSVC detection.
- rust-ffmpeg's wiki says the same for MSVC: install LLVM, set `LIBCLANG_PATH`, set `FFMPEG_DIR`, add `bin` to `PATH`.

**Where the DLLs must live.** Ship them next to `kairo-engine.exe`. The loader searches the application directory first. Don't rely on `PATH`, where other apps' `avcodec-63.dll` could collide **[M]**.

### 6.3 Compile-check from the Mac with cargo-xwin

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --locked                 # 0.23.1 (2026-08-13), MSRV 1.89
brew install llvm
export PATH="$(brew --prefix llvm)/bin:$PATH" LIBCLANG_PATH="$(brew --prefix llvm)/lib"
# copy include/ and lib/ (not bin/) of the same BtbN zip to ~/kairo-deps/ffmpeg-9.0-lgpl-shared
FFMPEG_DIR=~/kairo-deps/ffmpeg-9.0-lgpl-shared cargo xwin check --target x86_64-pc-windows-msvc -p kairo-engine
```

**Why this should work (each step checked in source)**
1. **Compiler and linker.** cargo-xwin sets `CC_x86_64_pc_windows_msvc=clang-cl`, `TARGET_CC` and the `lld-link` linker, but **not** the host `CC` (`src/compiler/common.rs:116-127`).
2. **Feature probe.** ffmpeg-sys-next compiles `check.c` with `cc::Build::new().target(HOST)` ("don't cross-compile this") against the FFmpeg headers only, then runs it to emit `ffmpeg_9_0` and similar cfgs (`build.rs:895-930`). The headers are plain C; `avconfig.h` comes from the Windows build.
3. **Target.** bindgen 0.73 adds `--target=<TARGET>` from the `TARGET` environment variable (`lib.rs:753-803`).
4. **Include paths.** cargo-xwin exports `BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc="-I…/crt/include -I…/sdk/include/ucrt -I…/um -I…/shared -I…/winrt"` (`clang_cl.rs:111-115`). This is the fix for cargo-xwin issue #32. So `__has_include(<d3d11.h>)` is true and the D3D11VA structs are generated.
5. **`check` never links.** `cargo xwin build` links against the BtbN `.lib` files with lld-link.

**[Unverified end-to-end]** Run it in S0. Fallbacks if bindgen under xwin misbehaves:
- (a) Write the hwcontext mirrors yourself (§2.2), so only the core headers matter.
- (b) `[patch.crates-io] ffmpeg-sys-next = { path = "vendor/ffmpeg-sys-next" }`, with a small `build.rs` change: if `KAIRO_FFMPEG_BINDINGS` is set, copy that file to `OUT_DIR/bindings.rs` and print the `ffmpeg_*`/`ff_api_*` cfgs from a checked-in list. Generate the file on the Windows PC.
- `rusty_ffmpeg`'s `FFMPEG_BINDING_PATH` is not an option until it supports FFmpeg 9.

### 6.4 Public release: a minimal custom LGPL build

- **Why:**
  - 149 MB of BtbN DLLs;
  - LGPL compliance for every external LGPL library BtbN links into them (ffmpeg.org checklist item 17);
  - control over the NVENC header and driver floor.
- **Sketch** (cross-compile with mingw-w64 in GitHub Actions on `ubuntu-latest`; all flags exist in 9.0.2's `configure`):

```sh
./configure --target-os=mingw32 --arch=x86_64 --cross-prefix=x86_64-w64-mingw32- \
  --enable-shared --disable-static --enable-version3 --disable-autodetect --disable-everything \
  --disable-network --disable-doc --extra-version=kairo \
  --enable-d3d11va --enable-amf --enable-ffnvcodec --enable-nvenc --enable-libvpl --enable-mediafoundation \
  --enable-encoder=h264_amf,hevc_amf,av1_amf,h264_nvenc,hevc_nvenc,av1_nvenc,h264_qsv,hevc_qsv,av1_qsv,h264_mf,hevc_mf,aac \
  --enable-decoder=h264,hevc,av1,aac --enable-parser=h264,hevc,av1,aac \
  --enable-muxer=mp4,mov,mpegts --enable-demuxer=mov --enable-protocol=file \
  --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,aac_adtstoasc,dump_extra,extract_extradata \
  --enable-swresample --enable-hwaccel=h264_d3d11va,hevc_d3d11va,av1_d3d11va   # decode for exports later
```

- **Requirements in 9.0.2's configure:** AMF headers ≥ 1.5.2 (`configure:7903-7904`); nv-codec-headers ≥ 12.1.14.0; libvpl ≥ 2.6 (`:7386-7388`).
- **Licences of the bundled pieces:** AMF headers MIT, nv-codec-headers MIT, libvpl MIT.
- Build `ffprobe.exe` for the test harness only; don't ship it.

### 6.5 LGPL compliance checklist (FFmpeg DLLs inside a GPL-3.0 app)

**The ffmpeg.org/legal.html checklist [H], applied to Kairo**
- [ ] Build without `--enable-gpl` and `--enable-nonfree`. Link dynamically; keep the `avcodec-63.dll`-style names (a prefix or suffix is allowed).
- [ ] Distribute the **exact corresponding source** as a tarball on the same server as the binaries (GitHub Releases), containing:
  - the FFmpeg tree at the build commit (6dfaba896f for the build above);
  - `git diff > changes.diff` (empty if unmodified);
  - a text file with the configure line;
  - for BtbN builds, the BtbN scripts at that date and the source of every library linked into the DLLs.
- [ ] About box: "This software uses libraries from the FFmpeg project under the LGPLv3". The checklist template says LGPLv2.1; BtbN lgpl builds are v3.
- [ ] Download page: an FFmpeg credit with links to the licence and the source.
- [ ] No reverse-engineering ban. Kairo's GPL-3.0 already satisfies this.
- [ ] No GPL libraries (x264/x265).

**LGPLv3 specifics**
- Users must be able to replace the DLLs. Never refuse to load modified or unsigned FFmpeg DLLs; signing them for SmartScreen is fine.
- Include the LGPLv3 and GPLv3 texts. LGPLv3 builds on GPLv3, and Kairo ships GPLv3 anyway.

---

## 7. Recommended crates and versions

| Crate | Version (pin) | Use | Licence |
|---|---|---|---|
| `ffmpeg-next` | `=9.0.0` (`default-features=false`, `codec`,`format`,`software-resampling`) | encoders, packets, muxing | WTFPL |
| `ffmpeg-sys-next` | `=9.0.1` (`avcodec`,`avformat`,`swresample`) | raw ffi: hwcontext, `av_opt_set`, `av_packet_ref`, log callback | WTFPL |
| `windows` | `=0.62.2` (stay off 0.100 for now; brief R6) | D3D11, DXGI, `D3DCompile`, DisplayConfig | MIT/Apache |
| `bindgen` (transitive) | 0.73.2 | via the sys crate (needs libclang) | BSD-3 |
| `cargo-xwin` (tool) | 0.23.1 | Mac compile check | MIT |
| FFmpeg | 9.0.2 line (BtbN `n9.0.2-29-g6dfaba896f` lgpl-shared for the beta) | DLLs | LGPLv3 |
| OBS shader code | `color.effect`, `format_conversion.effect` (master, 2024-11-11 last change to color.effect) | ported HLSL | GPL-2.0-or-later |

---

## 8. Gotchas

1. **AMF aborts on a foreign frames context** (`av_assert0`, `amfenc.c:411`).
2. **QSV + texture array + `BIND_RENDER_TARGET` = mapping failure** for slices other than 0. Use `initial_pool_size=0`. The brief's `scale_d3d11 … hwmap=derive_device=qsv` line probably hits this **[M]**.
3. **QSV needs `AV_PIX_FMT_QSV` frames.** It has no D3D11 input in FFmpeg 9.0.2. The derived device follows the D3D11 adapter LUID, so the device must be on the Intel GPU. Legacy Media SDK runtimes (pre-Tiger Lake) are untested with dynamic pools; use `h264_mf` there.
4. **`set_flags()` overwrites codec defaults.** AMF `+loop` keeps deblocking on and QSV `+cgop` keeps the GOP closed. OR the flags in instead.
5. **Unknown encoder options are silently ignored** by ffmpeg-next `open_with`. Use `av_opt_set` and check it.
6. **`Packet::clone()` deep-copies** shared payloads. Snapshot with `av_packet_ref`.
7. **The muxer stream time_base changes in `write_header`** (60 fps → 1/15360). Rescale after the header.
8. **`colr` needs all three colour fields.** One `UNSPECIFIED` and no box is written.
9. **NVENC with GLOBAL_HEADER puts no SPS/PPS in the bitstream.** That is fine for MP4. TS segments later need `dump_extra`.
10. **Always set `bf`.** NVENC and QSV `-1` lets the preset or runtime choose; AMF only takes its private `bf`. Hidden B-frames make dts ≠ pts and break no-edit-list output.
11. **`async_depth` defaults to 16 on AMF.** That means 16 pool textures held, about 50 MB VRAM at 1080p. Lower it **[H for the default; VRAM is an estimate]**.
12. **`h264_mf`:** pass `hw_encoding=1`; it uses the first hardware MFT (may be the wrong adapter); it writes no colour VUI. Verify with ffprobe.
13. **Multithread protection is not set by FFmpeg for user devices.** Call `ID3D11Multithread::SetMultithreadProtected(true)` on the **immediate context**.
14. **The D3D11VA device context Releases `device`.** Hand FFmpeg an owned reference (`clone().into_raw()`). `lock_ctx` is not released by FFmpeg.
15. **NV12 width and height must be even.** Some drivers break NV12 plane rendering. Run OBS's `HasBadNV12Output`-style self-test.
16. **faststart needs a re-openable file URL.** It won't work with an in-memory AVIO without a custom `io_open`.
17. **AV1 on RDNA3 may code 1080 lines as 1082**, with `clap`. Test before offering AV1.
18. **The BtbN NVENC build needs driver ≥ 610.** Older NVIDIA drivers fail at encoder open. Report it clearly; a custom build can lower the floor.
19. **`links="ffmpeg"`: one sys crate only.** No `ez-ffmpeg` alongside `ffmpeg-sys-next` 9.
20. **BtbN deletes daily builds after 14 days.** Mirror the pinned zip, or CI breaks.
21. **windows-capture 2.0.1 makes its own device** on the default adapter, with no multithread protection (§2.1).
22. **AMD Adrenalin Instant Replay is running on the user's PC** and shares VCN with Kairo. Benchmark with it both on and off **[Estimate: throughput contention only]**.

---

## 9. Code skeletons not shown above

**QSV mapping (per frame)**

```rust
const MAP_READ: i32 = 1; const MAP_DIRECT: i32 = 8;          // hwcontext.h:515-531
let mut qdev = ptr::null_mut();
check(ffi::av_hwdevice_ctx_create_derived(&mut qdev, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_QSV, d3d_dev.0, 0))?;
let mut qfr = ptr::null_mut();
check(ffi::av_hwframe_ctx_create_derived(&mut qfr, ffi::AVPixelFormat::AV_PIX_FMT_QSV, qdev, d3d_frames.0 /* pool 0 */, 0))?;
// encoder: pix_fmt = Pixel::QSV; (*c).hw_frames_ctx = av_buffer_ref(qfr); flags |= GLOBAL_HEADER (+cgop already default)
let mut q = ffmpeg_next::frame::Video::empty();
(*q.as_mut_ptr()).format = ffi::AVPixelFormat::AV_PIX_FMT_QSV as i32;
(*q.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(qfr);
check(ffi::av_hwframe_map(q.as_mut_ptr(), d3d_frame.as_ptr(), MAP_READ | MAP_DIRECT))?; // keeps a ref to the D3D11 frame
(*q.as_mut_ptr()).pts = tick;
enc.send_frame(&q)?;
```

**FFmpeg log → tracing**

```rust
unsafe extern "C" fn ff_log(avcl: *mut c_void, level: c_int, fmt: *const c_char, vl: ffi::va_list) {
    if level > ffi::AV_LOG_INFO { return; }
    let (mut buf, mut prefix) = ([0 as c_char; 1024], 1);
    ffi::av_log_format_line2(avcl, level, fmt, vl, buf.as_mut_ptr(), buf.len() as c_int, &mut prefix);
    let line = CStr::from_ptr(buf.as_ptr()).to_string_lossy();
    match level { l if l <= ffi::AV_LOG_ERROR => tracing::error!(target: "ffmpeg", "{}", line.trim_end()),
                  l if l <= ffi::AV_LOG_WARNING => tracing::warn!(target: "ffmpeg", "{}", line.trim_end()),
                  _ => tracing::info!(target: "ffmpeg", "{}", line.trim_end()) }
}
unsafe { ffi::av_log_set_callback(Some(ff_log)); }
```

- **[Unverified]** The exact `va_list` type path in the generated bindings. Mirror whatever bindgen emits for `av_log_set_callback`'s callback type.

---

## 10. Tests (run on the Windows PC over SSH; GUI-bound pieces via the `schtasks /it` launcher)

### 10.1 Unit and GPU tests (`cargo test -p kairo-engine -- --include-ignored`)

1. **Shader compile.** Every entry point compiles with `D3DCompile`.
2. **NV12 matrix.** Render 75% bars and the black/white patches through Pass B into a pool frame. `CopyResource` to a staging NV12 texture, `Map` it, and check the §3.2 values ±1. The Y plane is `RowPitch × H` bytes, followed by UV (`RowPitch × H/2`).
3. **Chroma siting.** A 1-px vertical white line on black at an even column must give a symmetric [1 2 1] chroma response.
4. **HDR tone-map.** A synthetic `R16G16B16A16_FLOAT` texture with scRGB values 1.0 (80 nits), 3.0 (240), 12.5 (1000) and Lw=1000, Lmax=240 must give sRGB codes for 100 → 100 nits, 240 → 193/240 relative and 1000 → 1.0. Compute the expected values with the formulas in §3.4.
5. **Bad-NV12 self-test.** It passes on the AMD card. Log the result.
6. **Encoder probe.** For every available encoder (`h264_amf`, `hevc_amf`, `av1_amf`; and `h264_mf` as fallback): it opens; the first packet is KEY; keyframes come exactly every 60 packets; `extradata_size > 0`.

### 10.2 ffprobe checks on a saved clip (`ffprobe.exe` from the same BtbN build)

```powershell
# streams, colour, disposition, titles
ffprobe -v error -show_entries stream=index,codec_name,profile,width,height,pix_fmt,color_range,color_space,color_transfer,color_primaries,r_frame_rate,time_base,start_time,duration,sample_rate,channels:stream_disposition=default:stream_tags=title,handler_name,language -of json clip.mp4
#   expect: h264 High, yuv420p, tv/bt709/bt709/bt709, start_time 0; a:0 default=1, others default=0; 48000 Hz AAC LC

# keyframe cadence + pts==dts (bf=0)
ffprobe -v error -select_streams v:0 -show_entries packet=pts,dts,flags -of csv=p=0 clip.mp4
#   expect: first row "0,0,K__"; a K every 60 rows; pts==dts on every row

# every keyframe is an IDR (nal_unit_type 5)
ffmpeg -v error -i clip.mp4 -map 0:v -c copy -bsf:v trace_headers -f null - 2>&1 | Select-String "nal_unit_type"

# moov before mdat, colr present, no edit list
ffprobe -v trace clip.mp4 2>&1 | Select-String "type:'(ftyp|moov|mdat|colr|elst)'"
#   expect: ftyp, then moov (with colr inside), then mdat; no 'elst'

# A/V start offset
ffprobe -v error -show_entries stream=index,start_time -of csv=p=0 clip.mp4   # |audio - video| < 0.022 s

# decodes cleanly, every track
ffmpeg -v error -i clip.mp4 -map 0 -f null -
# format tags
ffprobe -v error -show_entries format_tags -of json clip.mp4          # title, comment, encoder (©too), description
```

### 10.3 End-to-end

**Synthetic run**
- The brief's D3D11 test-pattern window, showing a frame counter and timed beeps.
- Save over the pipe, then:
  - decode the middle frame to `rgb24` and compare against the pattern with a PSNR threshold;
  - check the beep onset against the flash frame: offset < 40 ms;
  - check the duration is 30 s ± 1 GOP.

**Players**
- The S6 matrix (WMP, Movies & TV, Explorer thumbnail, Chrome/Edge/WebView2, Discord, Premiere, Resolve, VLC).
- Check that only the mixed track plays by default and that the colours look right: the bars clip should look identical side by side in each player.

**HDR**
- With HDR on, on the 360 Hz OLED, capture a known HDR scene and the Windows desktop.
- Compare a clip screenshot with a Game Bar SDR screenshot.

**Vendor matrix**
- AMD on the user's PC.
- NVENC on the GitHub GPU runner (brief §3.5), using the same synthetic test.
- QSV on an Intel tester's PC before declaring QSV supported.

---

## 11. Plan impacts (for the implementation plan being written in parallel)

1. **Dependencies.** Pin `ffmpeg-next =9.0.0` + `ffmpeg-sys-next =9.0.1` + `windows =0.62.2` as Windows-only dependencies. Budget one `unsafe` FFI module (hwcontext, options, packet refs, log) of about 300–500 lines in S2.
2. **Frames.** One frames path for all vendors: `AVHWFramesContext` (D3D11, NV12, `initial_pool_size=0`, `BIND_RENDER_TARGET`), with the shader rendering straight into `av_hwframe_get_buffer` frames. AMF, NVENC and MF consume them directly. QSV maps them to QSV frames.
3. **Encoder order and gating.**
   - AMF (user's PC, S2 first), then NVENC (CI GPU runner), then QSV (**experimental until tested on Intel hardware**), then `h264_mf` with `hw_encoding=1`.
   - Gate each with a real 5-frame probe.
4. **Default `bf=0` and `use_editlist=0` in v1.** This replaces "B-frames 2 where supported" in brief D3 and §2.8. Add an S2 VMAF test for `bf=2` and the edit-list player matrix before changing it.
5. **HDR.** Change the brief's HDR note: OBS uses Reinhard for display capture, and the EETF is a Kairo choice. Add an S1 task comparing the EETF, clip and Reinhard on the HDR OLED, and expose the result as a setting.
6. **Shaders.** Runtime `D3DCompile` with embedded HLSL. No `build.rs` shader step. A Windows-only shader test. OBS attribution files go in `THIRD_PARTY_NOTICES`.
7. **NV12 output check.** A capability check plus the bad-NV12 self-test at startup, with a BGRA-into-encoder fallback.
8. **Device.** Kairo creates its own D3D11 device on the encode adapter, with multithread protection. Reconcile this with the capture plan: windows-capture creates its own device.
9. **Save path.** A per-save muxer with `+faststart`, timestamps rescaled after the header, dts-ordered writes from `av_packet_ref` snapshots, mixed track default, track titles, iTunes tags plus a JSON `description`, `colr` checked. Store `codec::Parameters` per encoder session. **No save across sessions.**
10. **Beta FFmpeg.** A pinned BtbN month-end `lgpl-shared` build (LGPLv3, NVIDIA driver ≥ 610), mirrored with its source in Kairo releases. **M3:** a CI job for a minimal custom LGPL build (nv-codec-headers 12.1.14.0, so drivers ≥ 531.61), smaller than the 149 MB BtbN DLL set **[Estimate: a few MB per DLL]**.
11. **S0 additions.**
    - Install Rust, cargo-xwin and brew llvm on the Mac, then run `FFMPEG_DIR=… cargo xwin check`.
    - On Windows, install LLVM, set `LIBCLANG_PATH` and `FFMPEG_DIR`, and copy the DLLs.
    - If xwin plus bindgen fails, fall back to vendoring the sys crate with pre-generated bindings (§6.3).
12. **Capability report additions:** `avcodec_configuration()`, `av_version_info()`, the AMF runtime version, refused options, the NV12 self-test result, encoder probe results, and display `MaxLuminance` and SDR white.

---

## Open items / [Unverified]

- End-to-end `cargo xwin check` of `ffmpeg-sys-next` on macOS: Rust is not installed yet.
- **QSV:**
  - whether 1920×1080 textures with `Info.Height=1088` are accepted;
  - dynamic pools on legacy runtimes;
  - whether in-band SPS/PPS appear at IDRs.
- AMF's `header_spacing` default, and whether VCN reads the D3D11 texture directly or AMF copies it internally.
- `h264_mf` IDR and closed-GOP behaviour per vendor MFT.
- Windows/Media Foundation support for `ctts` v1 (`negative_cts_offsets`) and `clap`.
- The exact name of the `va_list` type in the generated bindings.
- Quality cost of `bf=0` against `bf=2` at the default bitrates.

---

## Sources

**Crates (crates.io API and `.crate` sources, read 2026-10-21)**
- https://crates.io/api/v1/crates/ffmpeg-next · https://crates.io/api/v1/crates/ffmpeg-sys-next · https://static.crates.io/crates/ffmpeg-sys-next/ffmpeg-sys-next-9.0.1.crate · https://static.crates.io/crates/ffmpeg-next/ffmpeg-next-9.0.0.crate
- https://crates.io/api/v1/crates/ffmpeg-the-third · https://crates.io/api/v1/crates/ffmpeg-sys-the-third · https://crates.io/api/v1/crates/rsmpeg · https://crates.io/api/v1/crates/rusty_ffmpeg · https://crates.io/api/v1/crates/ez-ffmpeg · https://crates.io/api/v1/crates/video-rs · https://crates.io/api/v1/crates/ac-ffmpeg
- https://crates.io/api/v1/crates/windows (0.62.2) · https://static.crates.io/crates/windows/windows-0.62.2.crate (Fxc `D3DCompile`, `ID3D11Multithread`) · https://static.crates.io/crates/windows-link/windows-link-0.2.1.crate (raw-dylib) · https://static.crates.io/crates/windows-core/windows-core-0.62.2.crate
- https://crates.io/api/v1/crates/bindgen (0.73.2, `lib.rs` TARGET handling) · https://crates.io/api/v1/crates/cargo-xwin (0.23.1, `src/compiler/clang_cl.rs`, `common.rs`) · https://static.crates.io/crates/windows-capture/windows-capture-2.0.1.crate (`src/d3d11.rs`)

**GitHub**
- https://github.com/zmwangx/rust-ffmpeg-sys/commits/master (369976d9 "Expose `hwcontext` structs", aff42027 "Add support for FFmpeg 9.0")
- https://github.com/zmwangx/rust-ffmpeg/pull/274
- https://github.com/zmwangx/rust-ffmpeg/wiki/Notes-on-building
- https://github.com/larksuite/rsmpeg/pull/256 · https://github.com/CCExtractor/rusty_ffmpeg/pull/154 · https://github.com/shssoichiro/ffmpeg-the-third
- https://github.com/rust-cross/cargo-xwin/issues/32

**FFmpeg 9.0.2 source and docs**
- Tarball: https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz. Files below are on GitHub at https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/…
- `libavutil/hwcontext_d3d11va.h`, `libavutil/hwcontext_d3d11va.c`, `libavutil/hwcontext.h`, `libavutil/hwcontext.c`, `libavutil/hwcontext_qsv.c`, `libavutil/hwcontext_amf.c`, `libavutil/hwcontext_amf.h`, `libavutil/mem.c`
- `libavcodec/amfenc.c`, `amfenc_h264.c`, `amfenc_hevc.c`, `amfenc_av1.c`, `nvenc.c`, `nvenc.h`, `nvenc_h264.c`, `qsvenc.c`, `qsvenc.h`, `qsvenc_h264.c`, `qsv.c`, `qsv_internal.h`, `mfenc.c`, `aacenc.c`, `profiles.h`, `avcodec.h`, `packet.h`, `bsf/dump_extradata.c`, `bsf/trace_headers.c`
- `libavformat/movenc.c`, `libavformat/mux_utils.c`, `libavformat/mov.c`, `libavformat/avformat.h`
- `libavfilter/vf_scale_d3d11.c`, `vsrc_gfxcapture_winrt.cpp`, `vsrc_ddagrab.c`
- `configure`, `Changelog`, `doc/filters.texi`
- https://ffmpeg.org/legal.html

**Builds and NVIDIA headers**
- https://github.com/BtbN/FFmpeg-Builds (README, `variants/defaults-lgpl.sh`, `variants/windows-install-shared.sh`, `scripts.d/50-ffnvcodec.sh`, `scripts.d/50-amf.sh`)
- https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-10-20-13-08 (`ffmpeg-n9.0.2-29-g6dfaba896f-win64-lgpl-shared-9.0.zip`, contents listed)
- https://github.com/FFmpeg/nv-codec-headers (README at n12.1.14.0, n12.2.72.0, n13.0.19.0, n13.1.15.0, eddcea9e)

**OBS Studio (master)**
- https://github.com/obsproject/obs-studio/blob/master/libobs/data/color.effect
- https://github.com/obsproject/obs-studio/blob/master/libobs/data/format_conversion.effect
- https://github.com/obsproject/obs-studio/blob/master/libobs/data/default.effect
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/duplicator-monitor-capture.c
- https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-subsystem.cpp
- https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-shader.cpp
- https://github.com/obsproject/obs-studio/blob/master/plugins/obs-ffmpeg/texture-amf.cpp
- https://github.com/obsproject/obs-studio/blob/master/README.rst · https://github.com/obsproject/obs-studio/blob/master/COPYING
- https://github.com/obsproject/obs-studio/commits/master/libobs/data/color.effect

**Microsoft Learn**
- https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format (NV12, P010)
- https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_sdr_white_level
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11_4/nn-d3d11_4-id3d11multithread
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_create_device_flag

**Secondary (d3dcompiler_47 in the OS)**
- https://www.vogons.org/viewtopic.php?p=577406 · https://code.videolan.org/videolan/vlc/-/merge_requests/6779
