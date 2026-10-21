# Kairo research 08: Screen and window capture in Rust (WGC first, DDA later)

*Prepared 2026-10-21 for the implementation cookbook. Scope: the engine's capture layer, from "pick a target" to "a D3D11 texture plus a QPC timestamp on the shared device". Colour conversion, encoding, the replay ring and audio are covered in other reports. Nothing is built yet.*

**Method and honesty notes**

- I read source code, not only docs:
  - the published crate tarballs of `windows-capture` 2.0.1 (commit `c7d10644`), `windows` 0.62.2 (generated bindings), `windows-core` 0.62.2, `windows-future` 0.3.2, `windows-result` 0.4.1, `tao` 0.37.1, `tauri` 2.12.4 and `embed-manifest` 1.5.1;
  - FFmpeg `n9.0.2` (`vsrc_gfxcapture_winrt.cpp`, `vsrc_ddagrab.c`, `hwcontext_d3d11va.c/.h`, `doc/filters.texi`);
  - OBS `master` (`libobs-winrt/winrt-capture.cpp`, `libobs-d3d11/d3d11-subsystem.cpp`, `d3d11-duplicator.cpp`, `plugins/win-capture/duplicator-monitor-capture.c`);
  - Sunshine `master` (`display_wgc.cpp`), Qt Multimedia `dev` (`qffmpegwindowcapture_uwp.cpp`);
  - Microsoft Learn API pages, and the Windows SDK `wingdi.h` (via the win32metadata mirror).
- **Every Rust sketch in this report was type-checked** with `cargo check --target x86_64-pc-windows-msvc` (Rust 1.99.1, `windows = "=0.62.2"`) on the Mac. The full harness, about 1,090 lines, is at `prototypes/kcapcheck/`. A second harness, `prototypes/kwcapcheck/`, checks the `windows-capture` 2.0.1 API. Type-checked is not the same as run: all behaviour still needs the PC.
- Tags: **[H]** primary source (MS Learn, SDK header, crate or OS-project source); **[M]** a single report or a secondary source; **[L]** my inference; **[Estimate]** arithmetic; **[Unverified]** could not be confirmed.

---

## 0. Answers in brief

1. **`windows-capture` 2.0.1 is a good library with the wrong shape for Kairo. Do not depend on it.**
   - It does give zero-copy access to the WGC surface (`Frame::as_raw_texture()`).
   - It exposes MinUpdateInterval, IsBorderRequired, DirtyRegionMode, IncludeSecondaryWindows, cursor and FP16 (`ColorFormat::Rgba16F`). Since 2.0 it also has a DDA module.
   - But its public API **always creates its own `ID3D11Device`**:
     - default adapter;
     - only `D3D11_CREATE_DEVICE_BGRA_SUPPORT`, with no `VIDEO_SUPPORT`;
     - no multithread protection.
   - Its WGC pool is the DispatcherQueue-bound `Create` with **one buffer**, and it never calls `RequestAccessAsync(Borderless)`.
   - The DDA module has no cursor, no adapter choice and an `unreachable!()` panic path.
   - An open PR (#216) would move it to windows-rs 0.100 "focused crates", which breaks its public types. [H]
2. **Recommendation: write a small `capture` module directly on `windows = "=0.62.2"`.** That is about 250 lines for WGC and 200 for DDA, plus monitors, targets and health. Use `windows-capture` (MIT) and FFmpeg/OBS as reference code. All sketches in §3, §6 and §7 compile against 0.62.2.
3. **WGC session settings** [H unless tagged]:

   | Setting | Value |
   |---|---|
   | Pool | `Direct3D11CaptureFramePool::CreateFreeThreaded`, **2 buffers**. Format `B8G8R8A8UIntNormalized`, or `R16G16B16A16Float` when the monitor is in HDR. |
   | Cursor | `SetIsCursorCaptureEnabled` |
   | Border | `GraphicsCaptureAccess::RequestAccessAsync(Borderless)` once at startup (log the status), then `SetIsBorderRequired(false)` on every session. |
   | Frame rate | `SetMinUpdateInterval(8.33 ms)` for a 60 fps output on 26100+ (half the output frame time, as OBS PR #12759 and Apollo PR #785 do). [M] |
   | Secondary windows | `SetIncludeSecondaryWindows(false)` |
   | Dirty regions | `DirtyRegionMode` is left at its default. |

   Gate every property with `ApiInformation::IsPropertyPresent`.
4. **Threading: one "GPU thread" owns the immediate context.**
   - `FrameArrived` fires on the pool's worker thread. It only calls `TryGetNextFrame` and parks the frame in a one-slot mailbox (newest wins).
   - The GPU thread copies the frame into its own ring texture with `CopyResource`, `Close()`s it at once, and paces on QPC.
   - The device is created with `SetMultithreadProtected(true)` because WGC, our code and libavcodec all touch it.
5. **DDA:** no maintained Rust crate fits.
   - The candidates: `windows-capture` (limits above); `win_desktop_duplication` 0.10.11 (2024, windows 0.57); `rusty-duplication` 0.6.1 and `dxgi-capture-rs` 1.2.2 (legacy `DuplicateOutput`, so BGRA only and no HDR); `dxgcap`/`captrs` (AGPL, 2020).
   - Implement it directly: `IDXGIOutput5::DuplicateOutput1` with `[R16G16B16A16_FLOAT, B8G8R8A8_UNORM]`, under a per-monitor-v2 thread DPI context.
   - Poll `AcquireNextFrame(0)` once per pacer tick.
   - Release and re-create on `DXGI_ERROR_ACCESS_LOST`, and composite the pointer yourself.
6. **Targets:**
   - **Window mode:** `GetForegroundWindow` → `GetAncestor(GA_ROOT)` → filters (visible, not cloaked, not a tool window, not ours) → `IGraphicsCaptureItemInterop::CreateForWindow`. Retry on `E_INVALIDARG`, as Qt does.
   - **Screen mode:** `MonitorFromWindow` → `CreateForMonitor`.
   - **Monitor list:** `EnumDisplayMonitors` + `QueryDisplayConfig` / `DisplayConfigGetDeviceInfo`, for the friendly name and the stable `monitorDevicePath`.
7. **HDR per monitor:** HDR is on when `IDXGIOutput6::GetDesc1().ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`.
   - SDR white in nits = `DISPLAYCONFIG_SDR_WHITE_LEVEL.SDRWhiteLevel / 1000 × 80`.
   - On 24H2+, `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2` (type 15) also tells HDR apart from WCG/ACM. It is **missing from windows 0.62.2**, so define the struct yourself (§7.3).
   - Re-check when `IDXGIFactory1::IsCurrent()` returns FALSE.
8. **Health:** poll `SHQueryUserNotificationState` (the shell sends no notification on fullscreen enter or exit).
   - Flag "possibly frozen" when no new frame timestamp has arrived for 3 s while a game is in the foreground.
   - Flag "exclusive fullscreen" when, in addition, the state is `QUNS_RUNNING_D3D_FULL_SCREEN`.
   - Flag an "ACCESS_LOST loop" at ≥5 re-creates per second (the OBS #14021 pattern).
   - Add a 1 Hz 16×9 readback for black or frozen content.
9. **Kairo's own windows:**
   - The engine calls `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` on its own toast window. The API only works on top-level windows **owned by the calling process**.
   - The Tauri UI uses `"contentProtected": true` or `WebviewWindow::set_content_protected(true)`. Tauri's runtime (tao 0.37.1) maps that to `WDA_EXCLUDEFROMCAPTURE`.
   - Tauri 2.12.4 depends on `windows ^0.62`, so `WebviewWindow::hwnd()` returns the same `HWND` type the engine uses. [H]

---

## 1. Version and API facts the implementation hinges on

| Item | Fact | Tag |
|---|---|---|
| `windows` crate | **0.62.2** (2025-10-06) is the latest `windows` umbrella; MSRV 1.82. The windows-rs "0.100.0" family (`windows-core`, `windows-future`, `windows-result`, …) shipped 2026-09-03 *without* a `windows` 0.100 umbrella. Its non-WinRT APIs move to header-derived metadata, with different signatures and modules. **Pin `windows = "=0.62.2"`** (and `windows-future = "=0.3.2"` if you name its types). Never mix 0.100 types with 0.62 types. | [H] crates.io, release `74`, issue #4867 |
| `windows-capture` | **2.0.1** (2026-08-08), MIT, depends on `windows 0.62.2` + `windows-future 0.3.2` + `parking_lot` + `rayon`. 2.19M downloads (1.63M in the last 90 days), 22 reverse deps (e.g. `scap`, `zed-scap`). One maintainer; 21 open issues. | [H] crates.io, GitHub API |
| Tauri | 2.12.4 (2026-10-14) depends on `windows ^0.62`. `WebviewWindow::hwnd() -> tauri::Result<HWND>`. `content_protected(bool)` / `set_content_protected(bool)` → tao 0.37.1 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE \| WDA_NONE)`. | [H] source |
| `IGraphicsCaptureItemInterop` | `CreateForWindow<T: Interface>(&self, HWND) -> Result<T>`; `CreateForMonitor<T>(&self, HMONITOR) -> Result<T>` (needs feature `Win32_Graphics_Gdi`). Min client 1903 (18362). | [H] bindings, MS Learn |
| `CreateDirect3D11DeviceFromDXGIDevice` | `unsafe fn<P0: Param<IDXGIDevice>>(P0) -> Result<IInspectable>`; cast to `IDirect3DDevice`. Feature `Win32_System_WinRT_Direct3D11` + `Win32_Graphics_Dxgi`. | [H] |
| `IDirect3DDxgiInterfaceAccess` | `unsafe fn GetInterface<T: Interface>(&self) -> Result<T>` | [H] |
| `Direct3D11CaptureFramePool::CreateFreeThreaded` | `(device: Param<IDirect3DDevice>, DirectXPixelFormat, i32 buffers, SizeInt32)`. "the dependency on the DispatcherQueue is removed and the FrameArrived event is raised on the frame pool's internal worker thread". Since 1809. | [H] MS Learn |
| `FrameArrived` | `pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move \|sender, _\| …))` → `i64` token; the closure must be `Fn + Send + 'static`. | [H] |
| `TryGetNextFrame` with no frame | Returns `Err(e)` with `e.code() == HRESULT(0)`, because windows-rs maps a null interface to `Error::empty()`. Handle it as "no frame". | [H] windows-core/result source |
| `SystemRelativeTime` | `TimeSpan { Duration: i64 }` in 100 ns: "The QPC … time at which the compositor rendered the frame". | [H] |
| Version gates | `IsCursorCaptureEnabled` 19041; `IsBorderRequired` + `GraphicsCaptureAccess` 20348 (contract v12); `MinUpdateInterval`, `IncludeSecondaryWindows`, `DirtyRegionMode` 26100 (contract v19). `IncludeSecondaryWindows` defaults to false and "can be changed during the capture". | [H] |
| `GraphicsCaptureAccess::RequestAccessAsync` | Behind feature `Security_Authorization_AppCapabilityAccess`; returns `IAsyncOperation<AppCapabilityAccessStatus>`; block with `.join()` (windows-future 0.3.2). Status values: DeniedBySystem 0, NotDeclaredByApp 1, DeniedByUser 2, UserPromptRequired 3, Allowed 4. | [H] |
| `ID3D11Multithread` | `SetMultithreadProtected(&self, bool) -> BOOL` (returns the previous state). MS: "obtained by querying it from an immediate device context … By default, multithread protection is turned off." | [H] |
| `IDXGIOutput5::DuplicateOutput1` | `<P0: Param<IUnknown>>(&self, device, flags: u32, &[DXGI_FORMAT]) -> Result<IDXGIOutputDuplication>`. Device "must be created from the adapter to which the output is connected". Max 4 duplicating apps. | [H] |
| `AcquireNextFrame` | `(&self, timeout_ms: u32, *mut DXGI_OUTDUPL_FRAME_INFO, *mut Option<IDXGIResource>) -> Result<()>`. `LastPresentTime`/`LastMouseUpdateTime` are raw QPC `i64`. | [H] |
| `DXGI_OUTDUPL_POINTER_SHAPE_INFO::Type` | **`u32`** in 0.62.2, while the constants are `DXGI_OUTDUPL_POINTER_SHAPE_TYPE(i32)`. Compare via `.0`. | [H] |
| `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2` | Not in windows 0.62.2. The SDK header defines it under `NTDDI_WIN11_GA` with request type **15** and `activeColorMode` (0 SDR, 1 WCG, 2 HDR). | [H] wingdi.h |
| `SetWindowDisplayAffinity` | `(HWND, WINDOW_DISPLAY_AFFINITY) -> Result<()>`. "The window must belong to the current process"; top-level only; `WDA_EXCLUDEFROMCAPTURE` = 0x11 from Win10 2004, acts as `WDA_MONITOR` before that. | [H] |
| `SHQueryUserNotificationState` | `() -> Result<QUERY_USER_NOTIFICATION_STATE>`; `QUNS_RUNNING_D3D_FULL_SCREEN` = 3 ("A full-screen (exclusive mode) Direct3D application is running"). "no notifications sent when the user starts or stops a full-screen application." | [H] |

---

## 2. `windows-capture` 2.0.1: evaluation

### 2.1 Question by question

| Question | Answer (2.0.1 source) |
|---|---|
| Zero-copy `ID3D11Texture2D`? | **Yes.** `Frame::as_raw_texture() -> &ID3D11Texture2D` is the WGC pool surface, obtained via `IDirect3DDxgiInterfaceAccess::GetInterface`. It is valid only inside `on_frame_arrived`. Also `as_raw_surface()`, `timestamp()` (`SystemRelativeTime`), `dirty_regions()`, `device()`, `device_context()`. DDA: `DxgiDuplicationFrame::texture()`, `frame_info()`. |
| Supply our own `ID3D11Device`? | **No, not through the public handler API.** `GraphicsCaptureApiHandler::start()` and `start_free_threaded()` both call `d3d11::create_d3d_device()`, which is `D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, …, D3D11_CREATE_DEVICE_BGRA_SUPPORT, [11_1…9_1])`. That means the default adapter, no `VIDEO_SUPPORT` and no multithread protection. The device reaches your handler as `Context { device, device_context }`, so you could share it "backwards" with the encoder. FFmpeg's `d3d11va_device_init` QueryInterfaces `ID3D11VideoDevice` and fails if that is unavailable, and FFmpeg's own `device_create` passes `D3D11_CREATE_DEVICE_VIDEO_SUPPORT`. Whether this device works for FFmpeg is **[Unverified]**. `GraphicsCaptureApi::new(d3d_device, d3d_device_context, …, thread_id, …)` is `pub` and takes a device, but you would re-implement `start()`: DispatcherQueue, message loop, WM_QUIT plumbing. `DxgiDuplicationApi::new` / `new_options` always create their own device. |
| MinUpdateInterval | `MinimumUpdateIntervalSettings::Custom(Duration)` → `SetMinUpdateInterval`. Its docs (2.0.1): "an OS-side throttle, not a target frame rate". |
| IsBorderRequired | `DrawBorderSettings::WithoutBorder` → `SetIsBorderRequired(false)`. It **never calls `RequestAccessAsync`**; its `windows` features do not even include `Security_Authorization_AppCapabilityAccess`. |
| DirtyRegionMode | `DirtyRegionSettings::ReportOnly` / `ReportAndRender`; `Frame::dirty_regions()`. |
| IncludeSecondaryWindows | `SecondaryWindowSettings::Include` / `Exclude`. |
| Cursor | WGC: `CursorCaptureSettings::WithCursor` / `WithoutCursor`. DDA: **no cursor** (issue #196, open). |
| FP16 for HDR | `ColorFormat::Rgba16F` (DirectXPixelFormat 10). The default is `Rgba8`, not BGRA. The image-save helpers reject `Rgba16F`. |
| Old Windows builds | Any non-`Default` setting the OS lacks is a **hard error** (`MinimumUpdateIntervalUnsupported`, `BorderConfigUnsupported`, …). On Win10 the session will not start unless you pass `Default`. |
| DDA? | **Yes, since 2.0:** `dxgi_duplication_api::DxgiDuplicationApi`. It calls `DuplicateOutput1` with `[FP16, RGBA8, BGRA8]` (BGRA always appended), and has `acquire_next_frame(timeout_ms)`, `Error::AccessLost` → `recreate()`, and `Error::Timeout`. Limits: outputs are searched **only on the default adapter** (others give `OutputNotFound`; adapter-choice issue #191 was closed with no change); it calls the process-wide `SetProcessDpiAwarenessContext(PMv2)`; `format()` has `_ => unreachable!()`. |
| Threading model | `start()` blocks the calling thread and `start_free_threaded()` spawns one. Either way the thread runs `RoInitialize(RO_INIT_MULTITHREADED)` + `CoIncrementMTAUsage` and `CreateDispatcherQueueController(DQTYPE_THREAD_CURRENT, DQTAT_COM_NONE)`. It creates the pool with **`Direct3D11CaptureFramePool::Create` and 1 buffer**, then pumps `GetMessageW`. Your handler sits in `Arc<parking_lot::Mutex<T>>` and runs on that thread; stopping posts `WM_QUIT`. |
| Encoder | WinRT `MediaTranscoder` (`VideoEncoder`) with no packet access (report 05). Not usable for the replay ring. |
| Licence | MIT, compatible with a GPL-3.0 Kairo. Attribute it if you copy code. |
| Maintenance | 2.0.0 shipped 2026-04-14 and 2.0.1 on 2026-08-08. In Aug 2026 issues got fixes within days (#190 docs, #197, #187, #210). Dependabot PRs are open. **PR #216 (open since 2026-09-17) migrates to windows-rs 0.100 "focused crates + private generated bindings"; "Windows-facing type changes require a breaking release".** After that, `as_raw_texture()` would no longer be the same `ID3D11Texture2D` type as yours. |

### 2.2 Verdict

- **Do not take the dependency.**
  - Kairo's requirements are exactly the parts the crate hides: one device shared with libavcodec (with `VIDEO_SUPPORT`, on the adapter that owns the monitor, multithread-protected); a 2-buffer free-threaded pool; borderless consent; and a DDA path with cursor and adapter choice.
  - The parts it adds (an encoder, image saving, a Python binding, the picker) are unused.
- **Borrow code (MIT, with attribution in third-party notices):**
  - `monitor.rs` for QueryDisplayConfig names;
  - `window.rs` for window filters and title-bar height;
  - `dxgi_duplication_api.rs` for the "release the old duplication before `DuplicateOutput1`" lesson.

### 2.3 What using it would look like (type-checked against 2.0.1, for comparison only)

```rust
// kwcapcheck/src/lib.rs (compiles with windows-capture = "=2.0.1", windows = "=0.62.2")
impl GraphicsCaptureApiHandler for KairoHandler {
    type Flags = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        // Only option: adopt the crate's device (default adapter, BGRA_SUPPORT only).
        let mt: ID3D11Multithread = ctx.device_context.cast()?;
        unsafe { let _ = mt.SetMultithreadProtected(true); }
        Ok(Self { device: ctx.device, context: ctx.device_context, latest: None })
    }
    fn on_frame_arrived(&mut self, frame: &mut Frame, _c: InternalCaptureControl) -> Result<(), Self::Error> {
        let _ts_100ns = frame.timestamp()?.Duration;       // SystemRelativeTime
        let src: &ID3D11Texture2D = frame.as_raw_texture(); // zero-copy, valid only in this callback
        if let Some(dst) = &self.latest { unsafe { self.context.CopyResource(dst, src) }; }
        Ok(())
    }
}
let settings = Settings::new(monitor, CursorCaptureSettings::WithCursor, DrawBorderSettings::WithoutBorder,
    SecondaryWindowSettings::Default, MinimumUpdateIntervalSettings::Custom(Duration::from_micros(8_333)),
    DirtyRegionSettings::Default, ColorFormat::Bgra8, ());
let control = KairoHandler::start_free_threaded(settings)?; // CaptureControl<KairoHandler, _>
```

---

## 3. Recommended: WGC directly on `windows` 0.62.2

### 3.1 Cargo

```toml
[dependencies]
windows-future = "=0.3.2"   # only to name IAsyncOperation; .join() works without it
[dependencies.windows]
version = "=0.62.2"
features = [
  "Foundation", "Foundation_Metadata",
  "Graphics", "Graphics_Capture", "Graphics_DirectX", "Graphics_DirectX_Direct3D11",
  "Security_Authorization_AppCapabilityAccess",           # GraphicsCaptureAccess::RequestAccessAsync
  "Win32_Foundation", "Win32_Devices_Display",
  "Win32_Graphics_Direct3D", "Win32_Graphics_Direct3D11",
  "Win32_Graphics_Dxgi", "Win32_Graphics_Dxgi_Common",
  "Win32_Graphics_Dwm", "Win32_Graphics_Gdi",
  "Win32_System_Com", "Win32_System_Performance", "Win32_System_Threading",
  "Win32_System_WinRT", "Win32_System_WinRT_Direct3D11", "Win32_System_WinRT_Graphics_Capture",
  "Win32_UI_Accessibility", "Win32_UI_HiDpi", "Win32_UI_Shell", "Win32_UI_WindowsAndMessaging",
]
```

- Each feature name was checked against `windows-0.62.2/Cargo.toml`.
- On the Mac, plain `cargo check --target x86_64-pc-windows-msvc` was enough for this crate: windows-rs needs no C toolchain to type-check.
- Use `cargo xwin check` once crates with C build scripts join, such as the FFmpeg `-sys` bindings. [H, verified here]

### 3.2 Thread and ownership model

```
WGC pool worker thread (OS)          GPU thread (ours; owns the immediate context, MTA)
  FrameArrived                         loop {
    TryGetNextFrame ──► Mailbox ──┐      frame = mailbox.wait_frame(next_tick)   // Condvar
    (older frame Close()d:        │      consume(): ContentSize check → Recreate | CopyResource → ring[i]; frame.Close()
     newest wins)                 └──►   at tick: pick newest ring slot with ts ≤ tick (else re-use previous)
  Closed ─► mailbox.closed=true             → convert/scale shader → NV12 → libavcodec (same device)
                                       }
```

- **Why copy instead of holding the WGC frame** (Sunshine holds it):
  - It decouples the pool from pacer timing and from screenshots.
  - With 2 buffers, a frame held across ticks starves WGC.
  - The copy costs about 0.05–0.1 ms of GPU time at 1440p BGRA. [Estimate]
- **Why the handler does not touch D3D:** all of Kairo's immediate-context work stays on one thread. Multithread protection then only guards against WGC- and FFmpeg-internal use.
- **`IDirect3DDevice` is not `Send`** in windows 0.62.2 (no `unsafe impl Send` is generated). `ID3D11DeviceContext` is `Send` but not `Sync`. Create the `GpuDevice` *on* the GPU thread; `windows-capture` wraps the device in an `unsafe impl Send` newtype for the same reason. WGC classes are `Send + Sync` (agile). [H]

### 3.3 Device (shared with the encoder)

```rust
// kcapcheck/src/device.rs (type-checked)
pub struct GpuDevice { pub device: ID3D11Device, pub context: ID3D11DeviceContext,
                       pub mt: ID3D11Multithread, pub winrt: IDirect3DDevice }

pub fn factory() -> Result<IDXGIFactory1> { unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) } }

/// The adapter (and output) that scans out `monitor`: required for DDA, best for WGC.
pub fn adapter_for_monitor(f: &IDXGIFactory1, monitor: HMONITOR) -> Result<Option<(IDXGIAdapter1, IDXGIOutput)>> {
    let mut a = 0;
    loop {
        let adapter = match unsafe { f.EnumAdapters1(a) } {
            Ok(x) => x, Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => return Ok(None), Err(e) => return Err(e) };
        let mut o = 0;
        loop {
            match unsafe { adapter.EnumOutputs(o) } {
                Ok(output) => { if unsafe { output.GetDesc()? }.Monitor.0 == monitor.0 { return Ok(Some((adapter, output))); } o += 1; }
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e),
            }
        }
        a += 1;
    }
}

pub fn create_device(adapter: Option<&IDXGIAdapter1>) -> Result<GpuDevice> {
    let adapter: Option<IDXGIAdapter> = adapter.map(|a| a.cast()).transpose()?;
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut got = D3D_FEATURE_LEVEL::default();
    unsafe { D3D11CreateDevice(adapter.as_ref(),
        if adapter.is_some() { D3D_DRIVER_TYPE_UNKNOWN } else { D3D_DRIVER_TYPE_HARDWARE }, // explicit adapter => UNKNOWN
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, // VIDEO: FFmpeg QIs ID3D11VideoDevice
        Some(&levels), D3D11_SDK_VERSION, Some(&mut device), Some(&mut got), Some(&mut context))?; }
    let (device, context) = (device.unwrap(), context.unwrap());
    let mt: ID3D11Multithread = context.cast()?;           // MS: query it from the immediate context
    let _prev = unsafe { mt.SetMultithreadProtected(true) };
    let dxgi: IDXGIDevice = device.cast()?;
    let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
    Ok(GpuDevice { device, context, mt, winrt })
}
```

**Handing the device to libavcodec.** The fields below are from FFmpeg 9.0.2 `hwcontext_d3d11va.h`. [H]

- `AVD3D11VADeviceContext.device` is "the only mandatory field", and "Deallocating the AVHWDeviceContext will always release this interface". So pass an **owned** reference: `gpu.device.clone().into_raw()`.
- FFmpeg's `lock`/`unlock` callbacks "protect accesses to device_context and video_context calls" and "must be recursive". If unset, FFmpeg uses a private mutex that Kairo cannot see.
- Recommendation: give FFmpeg `lock`/`unlock` that take a `parking_lot::ReentrantMutex<()>`, and have the GPU thread take the same lock around multi-call sequences (Map/Unmap, draw sequences). [L]
- Whether `ID3D11Multithread::Enter/Leave` is recursive, which would allow using it directly, is **[Unverified]**.

### 3.4 Capture item and session

```rust
// kcapcheck/src/wgc.rs (type-checked; excerpts)
pub fn create_item(target: Target) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    match target {
        Target::Monitor(hmon) => unsafe { interop.CreateForMonitor(hmon) },
        Target::Window(hwnd) => {
            let mut last = None;           // QTBUG-116025: transient E_INVALIDARG, Qt retries 10 x 100 ms
            for _ in 0..10 {
                match unsafe { interop.CreateForWindow::<GraphicsCaptureItem>(hwnd) } {
                    Ok(item) => return Ok(item),
                    Err(e) if e.code() == E_INVALIDARG => { last = Some(e); std::thread::sleep(Duration::from_millis(100)); }
                    Err(e) => return Err(e),
                }
            }
            Err(last.unwrap())
        }
    }
}

// WgcSource::start
let item = create_item(target)?;
let size = item.Size()?;
let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&dev.winrt, DirectXPixelFormat(cfg.format.0), 2, size)?;
let session = pool.CreateCaptureSession(&item)?;
if caps.cursor_toggle       { session.SetIsCursorCaptureEnabled(cfg.cursor)?; }
if caps.border_toggle       { let _ = session.SetIsBorderRequired(false); }      // FFmpeg: "can return failure but still work"
if caps.min_update_interval { session.SetMinUpdateInterval(TimeSpan::from(Duration::from_nanos(1_000_000_000 / (2 * cfg.target_fps as u64))))?; }
if caps.secondary_windows   { session.SetIncludeSecondaryWindows(false)?; }

let mb = mailbox.clone();
let frame_token = pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
    let Some(pool) = sender.as_ref() else { return Ok(()) };
    let frame = match pool.TryGetNextFrame() {
        Ok(f) => f,
        Err(e) if e.code().is_ok() => return Ok(()),   // null frame => windows-rs "empty" error (code S_OK)
        Err(e) => return Err(e),
    };
    let mut slot = mb.slot.lock().unwrap();
    if let Some(older) = slot.replace(frame) { let _ = older.Close(); } // newest wins; buffer back to pool
    drop(slot);
    mb.cv.notify_one();
    Ok(())
}))?;
let mb = mailbox.clone();
let closed_token = item.Closed(&TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
    mb.closed.store(true, Ordering::Release); mb.cv.notify_one(); Ok(())       // window closed/replaced, monitor gone
}))?;
session.StartCapture()?;
```

Capability probes, done once and stored in the capability report. Prefer these to build-number checks, as OBS does:

```rust
let t = h!("Windows.Graphics.Capture.GraphicsCaptureSession");
let caps = WgcCaps {
    supported: GraphicsCaptureSession::IsSupported()?,
    cursor_toggle:       ApiInformation::IsPropertyPresent(t, h!("IsCursorCaptureEnabled"))?,
    border_toggle:       ApiInformation::IsPropertyPresent(t, h!("IsBorderRequired"))?,
    min_update_interval: ApiInformation::IsPropertyPresent(t, h!("MinUpdateInterval"))?,
    secondary_windows:   ApiInformation::IsPropertyPresent(t, h!("IncludeSecondaryWindows"))?,
    dirty_regions:       ApiInformation::IsPropertyPresent(t, h!("DirtyRegionMode"))?,
};
let borderless = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)?.join()?; // MTA thread only
```

### 3.5 GPU-thread consumption

```rust
pub enum Consumed { Frame { ts_hns: i64, content: SizeInt32 }, Resized(SizeInt32), Dropped }

pub fn consume(&mut self, dev: &GpuDevice, frame: Direct3D11CaptureFrame, dst: &ID3D11Texture2D) -> Result<Consumed> {
    let content = frame.ContentSize()?;
    if content.Width <= 0 || content.Height <= 0 { let _ = frame.Close(); return Ok(Consumed::Dropped); }
    if content.Width != self.pool_size.Width || content.Height != self.pool_size.Height {
        let _ = frame.Close();                                             // surface still has the old size
        self.pool.Recreate(&dev.winrt, pixel_format(self.format), 2, content)?; // discards queued frames
        self.pool_size = content;
        return Ok(Consumed::Resized(content));                             // caller re-creates ring textures
    }
    let ts = frame.SystemRelativeTime()?.Duration;                         // 100 ns on the QPC timeline
    let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
    let src: ID3D11Texture2D = unsafe { access.GetInterface()? };
    let (mut s, mut d) = (D3D11_TEXTURE2D_DESC::default(), D3D11_TEXTURE2D_DESC::default());
    unsafe { src.GetDesc(&mut s); dst.GetDesc(&mut d); }                   // OBS: "ContentSize is not reliable"
    if (s.Width, s.Height, s.Format) != (d.Width, d.Height, d.Format) { let _ = frame.Close(); return Ok(Consumed::Dropped); }
    unsafe { dev.context.CopyResource(dst, &src) };
    drop(src);
    frame.Close()?;
    Ok(Consumed::Frame { ts_hns: ts, content })
}
```

- **Teardown** (in `Drop`): `pool.RemoveFrameArrived(token)`, `item.RemoveClosed(token)`, `pool.Close()`, `session.Close()`, then close any frame still in the mailbox. OBS revokes FrameArrived, then closes the pool, then the session.
- **Encoder canvas:** keep the encoder canvas fixed, e.g. 1920×1080 per save session. A window-mode `Resized` only changes the scale/letterbox rectangle in the convert shader; the encoder is never re-opened mid-ring.

---

## 4. WGC pitfalls

| Pitfall | What to do | Evidence |
|---|---|---|
| **Apartment** | WGC runtime classes are Agile (`MarshalingBehavior(Agile)`, `Threading(Both)`), so any thread may use them. Call `RoInitialize(RO_INIT_MULTITHREADED)` on the GPU thread (an `S_FALSE` result is fine). `RPC_E_CHANGED_MODE` means the thread is already STA (a UI thread): never capture there, spawn a dedicated thread. windows-rs silently calls `CoIncrementMTAUsage` when `RoGetActivationFactory` returns `CO_E_NOTINITIALIZED`, so missing init "works" by accident. Do not rely on that. Never `.join()` an async WinRT call on an STA thread. | [H] MS class page, windows-core `factory_cache.rs`; [L] for the STA advice |
| **`Create` vs `CreateFreeThreaded`** | `Create` ties FrameArrived to the creating thread's DispatcherQueue (`windows-capture` creates one and pumps messages). `CreateFreeThreaded` needs neither. | [H] |
| **Buffers** | Use 2 (OBS, FFmpeg, Sunshine all do). With 1 (`windows-capture`), WGC cannot render while you hold the frame. | [H] sources |
| **Multithread protection** | Set it on the shared device (FFmpeg gfxcapture does `ID3D10Multithread::SetMultithreadProtected(TRUE)` on the device it gives WGC; FFmpeg's own D3D11VA device does the same). It serialises immediate-context calls. Multi-call sequences still need your own lock. | [H] |
| **ContentSize changes** | The surface is always the pool size. Larger content is clipped and smaller content leaves "undefined data" beyond `ContentSize`. `Recreate` discards outstanding frames. Recreate on any change and drop that frame (`windows-capture` 2.0.1, Qt), and validate with `GetDesc` (OBS). The first frame after start can differ from `item.Size()` (`windows-capture` #197). | [H] MS screen-capture guide; sources |
| **Item `Closed`** | Raised when the window closes, the monitor disappears (FFmpeg returns EOF on both), **or when "some apps … seamlessly replace the targeted window with a new one"**. Games do this on mode or resolution changes. On `Closed`: re-resolve the game's top-level window by PID and re-create; fall back to its monitor. | [H] MS Learn remark, FFmpeg docs |
| **Minimized windows** | Expect **no new frames** while a captured window is minimized. In robmikh's sample issue #13 a user had to un-minimize windows to snapshot them. Detect with `IsIconic` + `EVENT_SYSTEM_MINIMIZESTART/END`, keep duplicating the last frame, and log the gap, but do not report it as "frozen". Window mode never shows the desktop. | [M] |
| **Cloaked / tool windows** | Skip `DWMWA_CLOAKED != 0` (Qt rejects `DWM_CLOAKED_SHELL`), `WS_EX_TOOLWINDOW` and invisible windows when choosing targets. | [H] Qt, `windows-capture` `window.rs` |
| **Border on Win11** | MS: consent via `RequestAccessAsync(Borderless)` "displays a prompt"; packaged apps need `graphicsCaptureWithoutBorder`; "If the user denies access, setting this property to false will succeed, but the value will be ignored". OBS (unpackaged) calls `RequestAccessAsync(Borderless).get()` and then `IsBorderRequired(false)`. FFmpeg 9.0.2, Sunshine and Qt only set `IsBorderRequired(false)`. mmozeiko (wcap): "you don't need that RequestAccessAsync". Whether an unpackaged Kairo sees a prompt on 25H2/26H2 is **[Unverified]**: test it. Win10 has no `IsBorderRequired`, so Win10 uses DDA. | [H] MS Learn, OBS/FFmpeg/Sunshine/Qt source; [M] handmade.network |
| **`TryGetNextFrame` returns no frame** | In Rust this is `Err` with `code() == S_OK`, not `Ok(None)`. Treat it as benign. | [H] |
| **Timestamps** | `SystemRelativeTime` is 100 ns on the QPC timeline. The OBS #13893 reporter found it is the frame's *presentation* time: "sometimes … late" and sometimes ahead of the arrival time. On a mixed 360 Hz + 165 Hz setup (Win10), deltas followed the monitor whose content was updating. Pace by timestamp (newest `ts ≤ tick`), never by arrival time. | [H] MS; [M] OBS #13893 |
| **HDR toggled mid-capture** | The pool keeps the format you asked for. A BGRA8 pool on an HDR desktop gives over-clipped, "washed out" frames (MS). Poll `IDXGIFactory1::IsCurrent()` about once a second. When it returns FALSE, re-create the factory, re-read `GetDesc1`, and `pool.Recreate(&dev.winrt, R16G16B16A16Float, 2, size)` (or the reverse). OBS compares each frame's `desc.Format` with the monitor's current HDR state and re-initialises on mismatch. | [H] MS HDR article, OBS source |
| **Elevated games** | Forum reports: OBS window capture fails across an elevation mismatch (`CreateForWindow` HRESULTs 0x80070424 / 0x80070057), and other tools show black. If `CreateForWindow` fails or frames are black, fall back to monitor capture of the game's monitor. Test it. | [M] OBS forum; [Unverified] for monitor capture of elevated windows |
| **Device removed** (driver update, TDR) | MS: `Recreate` exists for "providing a new Direct3D device". On `DXGI_ERROR_DEVICE_REMOVED`, rebuild the device, recreate the pool (or the session), the ring, the DDA duplication and the FFmpeg contexts. OBS has `device_loss_release` / `rebuild` for WGC. | [H] |
| **Session 0 / services** | WGC does not work from a service (Sunshine). Run the engine in the user session; over SSH, launch it with a `/IT` scheduled task (§11). | [H] report 01 |
| **Default MinUpdateInterval** | Undocumented. `windows-capture` #190 says the default behaves like 16.67 ms, and OBS, Apollo and Sunshine comments describe a 60 fps cap without it. Read back `session.MinUpdateInterval()` on the PC and log it. | [M] / [Unverified] |
| **Frame GPU sync** | Every reference implementation copies and then releases at once (OBS, FFmpeg, `windows-capture`). If tearing ever shows up, `Flush()` before `Close()`, or hold the frame until the next one arrives (Sunshine). | [L] |

---

## 5. Frame delivery on the 360 Hz OLED: semantics, cost and the interval to use

**What MinUpdateInterval is.**
- MS documents only the `TimeSpan` property (26100+). [H]
- `windows-capture` 2.0.1's docs, rewritten after #190: "an OS-side throttle, not a target frame rate. A custom interval limits how frequently updates are eligible for delivery, but it does not make Windows produce frames periodically or guarantee a constant rate." [H]
- FFmpeg's `gfxcapture` docs: "does NOT hold a stable FPS … returns frames at whatever rate the compositor provides them, only capped by `max_framerate`" (default 60). [H]
- Frames arrive only when DWM composes a change. A static screen gives no frames, so the pacer duplicates.

**Measured behaviour** (`windows-capture` #190, Win11 24H2, a 60 fps frame-counter video) [M]:

| Display | MinUpdateInterval | Delivered |
|---|---|---|
| 60 Hz | 1/60 s (16.67 ms) | ~45 fps |
| 60 Hz | 1/90 s | ~60 fps (78 when the mouse moved) |
| 120 Hz | 1/60 | ~52 |
| 120 Hz | 1/90 | ~58 |
| 120 Hz | 1/120 | ~59 |

Setting exactly one output frame time undershoots. My reading is that eligibility is measured from the previous delivery, and delivery snaps to the next composition, so jitter pushes frames one vblank later. **[L]**

**What others use** [H]:

| Project | MinUpdateInterval |
|---|---|
| OBS PR #12759 (approved, unmerged) | frame interval / 2 |
| Apollo PR #785 (merged 2025-06) | `1/(2·fps)` |
| Sunshine | 4 ms ("250Hz") |
| FFmpeg gfxcapture | `1/max_framerate` |

**Recommendation for a 60 fps output:**
- **8.33 ms** (1/120 s). On the 360 Hz OLED that is 3 vblanks, so at most 120 deliveries per second. The pacer takes every other frame, which gives an even cadence.
- **Refinement [L], to adopt only if the probe shows aliasing:** pick a whole number of refresh periods `k = max(1, floor((T_out/2) / T_refresh))` and set the interval to `(k − 0.5)·T_refresh`. That sits robustly between vblanks.
  - 360 Hz: 6.94 ms → 120 fps.
  - 240 Hz: 6.25 ms → 120.
  - 180 Hz: 2.78 ms → 180, a multiple of 60; 1/120 would give 90, a 2-1-2-1 judder pattern.
  - 60 Hz: 8.33 ms → 60.
  - With VRR or independent-flip games, composition follows the game's presents, so treat this only as a ceiling.
- **Never set 0 on a 360 Hz panel without a reason.** A CS2 or Valorant player at 400+ fps would push about 360 deliveries per second into your callback.

**Cost per delivered frame at 2560×1440** [Estimate]:

| | BGRA8 (14.7 MB) | FP16 (29.5 MB) |
|---|---|---|
| Our `CopyResource` (read + write) | 29.5 MB → ≈0.05 ms GPU | 59 MB → ≈0.1 ms |
| At 120 deliveries/s | 3.5 GB/s ≈ 0.6% of a 624 GB/s RX 7800 XT | 7.1 GB/s ≈ 1.1% |
| At 360 deliveries/s | 10.6 GB/s ≈ 1.7% | 21 GB/s ≈ 3.4% |
| CPU (event, QI, mutex, condvar, submit) | ≈10–50 µs per frame → <1% of one core at 120/s | same |

- DWM also does its own capture copy per delivered frame. It shows up under `dwm.exe` in Task Manager → GPU, and it is part of why the cap matters.
- Whether an active WGC session demotes a game from "Hardware: Independent Flip" to "Composed: Flip" (latency, VRR) is **[Unverified]**. Measure it with PresentMon (§11).

---

## 6. DDA in Rust (Windows 10 path and Win11 fallback)

### 6.1 Crate landscape (2026-10-21)

| Crate | Version / date | `windows` | DuplicateOutput1 (HDR) | Cursor | Own device / adapter | Verdict |
|---|---|---|---|---|---|---|
| `windows-capture` | 2.0.1 / 2026-08 | 0.62.2 | Yes (FP16, RGBA8, BGRA8) | No (#196) | No / default adapter only | Reference only |
| `win_desktop_duplication` | 0.10.11 / 2024-08 | 0.57 | Yes | GDI (`GetCursorInfo` + `DrawIconEx`) | Adapter choice yes; creates its own device | Stale; old `windows` |
| `rusty-duplication` | 0.6.1 / 2025-02 | 0.59 | No (`DuplicateOutput`) | Shape API | — | No HDR |
| `dxgi-capture-rs` | 1.2.2 / 2026-04 | 0.61.3 | No | — | — | No HDR |
| `dxgcap`, `captrs` | 0.2.4 / 0.3.1 (2020) | old | No | — | — | AGPL, dead |

**Implement it directly.** It is about 200 lines plus a pointer shader. [H crates.io and source]

### 6.2 Flow (type-checked excerpt)

```rust
// kcapcheck/src/dda.rs
pub fn duplicate(&mut self, dev: &GpuDevice) -> std::result::Result<(), DupError> { // Result = windows::core::Result elsewhere
    self.dupl = None;  // release the old duplication FIRST: "already duplicating this output" => E_INVALIDARG
    let prev = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let r = unsafe { self.output.DuplicateOutput1(&dev.device, 0, &self.formats) }; // formats: [FP16, BGRA8] if HDR else [BGRA8]
    if !prev.0.is_null() { unsafe { SetThreadDpiAwarenessContext(prev) }; }
    match r {
        Ok(d) => { self.desc = unsafe { d.GetDesc() }; self.dupl = Some(d); Ok(()) }
        Err(e) if e.code() == E_ACCESSDENIED => Err(DupError::AccessDenied),            // secure desktop (UAC, lock)
        Err(e) if e.code() == DXGI_ERROR_UNSUPPORTED => Err(DupError::Unsupported),     // mode, hybrid dGPU, no PMv2
        Err(e) if e.code() == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE => Err(DupError::TooManyDuplicators), // 4 max
        Err(e) if e.code() == DXGI_ERROR_SESSION_DISCONNECTED => Err(DupError::SessionDisconnected),
        Err(e) => Err(DupError::Other(e)),
    }
}

/// Called once per pacer tick on the GPU thread; timeout 0 = "newest desktop image, now".
pub fn poll(&mut self, dev: &GpuDevice, dst: &ID3D11Texture2D, timeout_ms: u32) -> Result<Poll> {
    let Some(dupl) = self.dupl.clone() else { return Ok(Poll::AccessLost) };
    if self.holding {                       // MS: release just before the next AcquireNextFrame
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
    if info.LastMouseUpdateTime != 0 { self.update_pointer(&dupl, &info)?; }
    if info.LastPresentTime == 0 || info.AccumulatedFrames == 0 { return Ok(Poll::NoChange); } // pointer-only update
    let Some(res) = res else { return Ok(Poll::NoChange) };
    let tex: ID3D11Texture2D = res.cast()?;
    // (format check vs self.formats elided here; never panic on an unexpected format)
    unsafe { dev.context.CopyResource(dst, &tex) };
    Ok(Poll::NewImage { qpc: info.LastPresentTime })   // raw QPC ticks -> qpc_to_hns(ticks, QPF)
}
```

**Notes, all [H] unless tagged:**
- **Polling at the pacer tick is natural for DDA.** DXGI accumulates updates, and `AcquireNextFrame(0)` hands back the latest composed image. OBS calls `AcquireNextFrame(0, …)` once per render tick. Microsoft's guidance is to release a frame right before the next acquire, so the OS only tracks dirty regions while you hold it.
- **Pointer-only updates.** `LastPresentTime == 0` and `AccumulatedFrames == 0` mean only the pointer changed. FFmpeg notes the resource format "can change/flicker randomly" in that case, so keep using your last copy.
- **First frames** can be empty (`windows-capture` example comment). Probe until `LastPresentTime != 0`.
- **Rotation.** The surface is always unrotated: a portrait 768×1024 desktop arrives as a 1024×768 surface with the image rotated inside. Read `DXGI_OUTDUPL_DESC.Rotation` and rotate in the shader.
- **Timestamps.** `LastPresentTime` is raw `QueryPerformanceCounter`. Convert it to 100 ns with `ticks × 10⁷ / QPF` so it shares a clock with WGC and WASAPI.

### 6.3 ACCESS_LOST and re-creation policy

| Event | What you see | Action |
|---|---|---|
| Mode change, HDR toggle, resolution, "DWM on/off or other full-screen application", desktop switch | `DXGI_ERROR_ACCESS_LOST` from `AcquireNextFrame`/`ReleaseFrame` | Drop the duplication; call `duplicate()` again at once. Re-read `GetDesc()` (`ModeDesc` size/format, `Rotation`) and re-create the ring. Count it for the watchdog. |
| UAC prompt, lock screen, Ctrl+Alt+Del (secure desktop) | `E_ACCESSDENIED` on re-create | Retry on `EVENT_SYSTEM_DESKTOPSWITCH`, or every 250 ms–1 s. Only LOCAL_SYSTEM can capture the secure desktop. |
| Unsupported mode, hybrid laptop on the dGPU (KB3019314), not PMv2-aware | `DXGI_ERROR_UNSUPPORTED` | MS: wait for `WM_DISPLAYCHANGE` or a desktop switch, then retry. OBS simply retries every 3 s (`RESET_INTERVAL_SEC 3.0f`). |
| Too many duplicators | `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` | Tell the user (four is the default limit), back off. |
| ACCESS_LOST at ≥5/s for 3 s | The OBS #14021 pattern (true exclusive fullscreen on 24H2+: about 15 re-creates/s, 0–1 frames each) | Mark the capture "exclusive fullscreen", show the borderless hint, and slow retries to 1/s. |

### 6.4 Pointer compositing

- **When to draw.** "If the graphics adapter overlays the mouse pointer on top of the desktop image, AcquireNextFrame reports that a separate pointer is visible. So, your client app must draw the mouse pointer shape." Games that draw their own cursor are already in the image.
- **Where to draw.** `PointerPosition.Position` is the shape's top-left, not the hotspot.
- **When to fetch the shape.** Fetch it only when `PointerShapeBufferSize > 0` and cache it.
- **Shape types:**

  | Type | Format | How to draw |
  |---|---|---|
  | `COLOR` | 32-bpp ARGB | Straight alpha blend |
  | `MASKED_COLOR` | 32-bpp | Alpha 0 means replace; alpha 0xFF means XOR with the screen |
  | `MONOCHROME` | 1-bpp AND mask stacked on a 1-bpp XOR mask | **Real height = `Height/2`** |

- **FFmpeg's method** (verified in `vsrc_ddagrab.c`): convert to an RGBA texture plus an "XOR" texture on the CPU, then draw two quads:
  - normal pass: `SrcBlend = D3D11_BLEND_SRC_ALPHA`, `DestBlend = D3D11_BLEND_INV_SRC_ALPHA`;
  - XOR approximation: `SrcBlend = D3D11_BLEND_INV_DEST_COLOR`, `DestBlend = D3D11_BLEND_INV_SRC_COLOR`.
- **Multi-monitor:** follow MS's sample logic (only update position from the output that last reported it visible).
- **Ring textures need `D3D11_BIND_RENDER_TARGET`** as well as `SHADER_RESOURCE` so the pointer can be drawn onto the copy.
- **Bindings gotcha:** `DXGI_OUTDUPL_POINTER_SHAPE_INFO.Type` is `u32` in windows 0.62.2; compare with `DXGI_OUTDUPL_POINTER_SHAPE_TYPE_*.0`.
- **HDR (FP16 scRGB) duplication:** the pointer is SDR sRGB. Linearise it and scale by `sdr_white_nits/80` before blending, or it will look dim. [L]

```hlsl
// Pointer quad over the desktop copy (straight alpha). XOR pixels use the second blend state.
Texture2D<float4> ptr : register(t0);  SamplerState pt : register(s0);
cbuffer P : register(b0) { float2 posPx; float2 sizePx; float2 targetPx; float sdrScale; float isScRgb; };
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut vs(uint id : SV_VertexID) {                    // 4-vertex triangle strip
    float2 uv = float2(id & 1, id >> 1);
    float2 px = posPx + uv * sizePx;
    VSOut o; o.pos = float4(px / targetPx * float2(2, -2) + float2(-1, 1), 0, 1); o.uv = uv; return o;
}
float3 SrgbToLinear(float3 c) { return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4); }
float4 ps(VSOut i) : SV_Target {
    float4 c = ptr.Sample(pt, i.uv);                  // CPU-converted, straight-alpha RGBA8
    if (isScRgb > 0.5) c.rgb = SrgbToLinear(c.rgb) * sdrScale;   // sdrScale = SDR white nits / 80
    return c;
}
```

This HLSL was **not compiled** here [Unverified]. The per-component `?:` is fine for `fxc` with `vs_5_0`/`ps_5_0`; with DXC and HLSL 2021, write `select(c <= 0.04045, …, …)`. Draw it as a 4-vertex `D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP` with no vertex buffer.

**Simpler alternative.** OBS's DXGI display capture draws its own cursor with `GetCursorInfo` + `CopyIcon` + `GetIconInfo` (`plugins/win-capture/cursor-capture.c`). For WGC, OBS uses `IsCursorCaptureEnabled` when it is available. The GDI route needs no shape decoding but costs CPU on every cursor change. For Kairo's DDA path, prefer the DXGI shape data above. [H] source; the comparison is [L].

### 6.5 The per-monitor-v2 DPI requirement

- **The requirement is not in the `DuplicateOutput1` docs.**
  - A Stack Overflow thread reports `DXGI_ERROR_UNSUPPORTED` until the caller sets `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`. [M]
  - go-d3d's docs require PMv2 too. [M]
  - FFmpeg 9.0.2 wraps the call in `SetThreadDpiAwarenessContext(PMv2)` / restore. [H]
- **Kairo should do both:**
  - Declare PMv2 in the engine's manifest. `embed-manifest` 1.5.1's `new_manifest("Kairo.Engine")` defaults to `PerMonitorV2Only` and works when cross-compiling; on MSVC it passes `/MANIFEST:EMBED` options to the linker.
  - Also wrap the call in `SetThreadDpiAwarenessContext`, as above.
- **Why PMv2 matters beyond DDA:** it also makes `GetWindowRect`, `DWMWA_EXTENDED_FRAME_BOUNDS` and `MonitorFromWindow` report physical pixels. Window-mode cropping and the fullscreen heuristic need that.
- **Avoid process-wide DPI calls in libraries.** `windows-capture`'s DDA module calls `SetProcessDpiAwarenessContext`. It returns `E_ACCESSDENIED` once awareness is already set, which the crate ignores.

### 6.6 Windows 10 specifics

- The border cannot be removed on Win10 (`IsBorderRequired` starts at 20348), so **Win10 = DDA, monitor mode only**.
- `IsCursorCaptureEnabled` exists (19041), but it is irrelevant there.
- Process loopback and the rest are covered in report 02.

---

## 7. Target selection, monitor list and HDR state

### 7.1 Window mode (Win11 default candidate)

```rust
// kcapcheck/src/target.rs (type-checked)
pub fn is_capturable(hwnd: HWND) -> bool {
    unsafe {
        if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() { return false; }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 { return false; }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4).is_ok() && cloaked != 0 { return false; }
    }
    true
}
pub fn foreground_candidate(own_pid: u32) -> Option<(HWND, u32)> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() { return None; }
    let root = unsafe { GetAncestor(fg, GA_ROOT) };
    if !is_capturable(root) { return None; }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
    if pid == own_pid { None } else { Some((root, pid)) }
}
pub fn monitor_of(hwnd: HWND) -> HMONITOR { unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) } }
// Foreground changes without injection (needs a GetMessageW loop on the hooking thread):
// unsafe extern "system" fn on_foreground(h: HWINEVENTHOOK, ev: u32, hwnd: HWND, id_obj: i32, id_child: i32, thread: u32, time: u32)
let hook: HWINEVENTHOOK = unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None,
    Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) };   // UnhookWinEvent(hook) on exit
```

- **Game check:** match the PID's image path against the game database (report 03) before switching capture to a window.
- **Window capture includes borders and the title bar.** For windowed games, crop the client area. Compute it from `GetClientRect` + `ClientToScreen` against `DWMWA_EXTENDED_FRAME_BOUNDS`, as FFmpeg (`wgc_calculate_client_area`) and OBS (`get_client_box`) do.
- **Borderless-fullscreen heuristic:** the window rect covers the monitor rect (`covers_monitor` in the harness).
- **`HWND` is not `Send`** in windows-rs. Pass `hwnd.0 as isize` across threads and rebuild it with `HWND(raw as *mut _)`.

### 7.2 Screen mode: monitor list with friendly names (type-checked)

```rust
// kcapcheck/src/monitors.rs (excerpt; the API calls below sit inside unsafe blocks in the harness)
unsafe extern "system" fn push_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) }.push(h); TRUE
}
EnumDisplayMonitors(None, None, Some(push_monitor), LPARAM(&mut hmons as *mut _ as isize));
GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).ok()?;
QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).ok()?;
// per HMONITOR: GetMonitorInfoW(MONITORINFOEXW) -> szDevice "\\.\DISPLAY1"; then for each path:
let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME { header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
    adapterId: p.sourceInfo.adapterId, id: p.sourceInfo.id }, viewGdiDeviceName: [0; 32] };
DisplayConfigGetDeviceInfo(&mut src.header)  // == 0 and viewGdiDeviceName == szDevice => this path
// DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME with p.targetInfo.{adapterId,id}:
//   monitorFriendlyDeviceName (UI label), monitorDevicePath (PERSIST THIS, not HMONITOR/\\.\DISPLAYn/index)
// refresh: p.targetInfo.refreshRate.Numerator / Denominator
```

### 7.3 HDR state and SDR white level (type-checked)

```rust
// helper: header::<T>(kind, path, target=true) fills r#type, size_of::<T>(), path.targetInfo.{adapterId,id}
// SDR white (OBS GetSdrMaxNits does exactly this):
let mut wl = DISPLAYCONFIG_SDR_WHITE_LEVEL {
    header: header::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, p, true),
    SDRWhiteLevel: 1000 };
if unsafe { DisplayConfigGetDeviceInfo(&mut wl.header) } == 0 { nits = wl.SDRWhiteLevel as f32 / 1000.0 * 80.0; }

// Legacy advanced-colour bits (Win10 1709+): bit0 advancedColorSupported, bit1 advancedColorEnabled
let mut aci = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();   // needs feature Win32_Graphics_Gdi
let bits = unsafe { aci.Anonymous.value };

// 24H2+: not in windows 0.62.2 - define it (layout from SDK 26100 wingdi.h)
#[repr(C)] #[derive(Clone, Copy, Default)]
struct AdvancedColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    bits: u32,                       // b0 supported, b1 active, b3 limitedByPolicy, b4 hdrSupported,
                                     // b5 hdrUserEnabled, b6 wideColorSupported, b7 wideColorUserEnabled
    color_encoding: DISPLAYCONFIG_COLOR_ENCODING,
    bits_per_color_channel: u32,
    active_color_mode: i32,          // DISPLAYCONFIG_ADVANCED_COLOR_MODE: 0 SDR, 1 WCG, 2 HDR
}
const GET_ADVANCED_COLOR_INFO_2: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(15);

// DXGI view (what OBS uses to pick FP16):
let d1 = unsafe { output.cast::<IDXGIOutput6>()?.GetDesc1()? };
let hdr_active = d1.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;   // d1.MaxLuminance, d1.BitsPerColor
```

- **DXGI cannot tell an ACM/WCG SDR display from plain SDR:** both report `G22_NONE_P709`. Only `_INFO_2.activeColorMode` can. [H]
- **The meaning of 1.0 in FP16 frames depends on the mode** [H MS HDR article]:
  - HDR mode: scRGB 1.0 = 80 nits.
  - SDR Advanced Colour (WCG) mode: 1.0 = the display's reference white.
- **Capture format rule:** FP16 only when `hdr_active` (`activeColorMode == 2` on 24H2+), otherwise BGRA8.
- **Stale values:** DXGI output descriptors are cached by the factory. When `IDXGIFactory1::IsCurrent()` returns FALSE, create a new factory before re-reading (MS recommends checking every frame; OBS re-creates the factory). [H]
- **Toggle on the PC:** Win+Alt+B toggles HDR (Game Bar shortcut). [M]

---

## 8. Exclusive fullscreen, freeze watchdog, Kairo's own windows

### 8.1 Exclusive-fullscreen detection

- **`SHQueryUserNotificationState()` → `QUNS_RUNNING_D3D_FULL_SCREEN` (3)**, "A full-screen (exclusive mode) Direct3D application is running".
- **`QUNS_BUSY` (2):** "A full-screen application is running or Presentation Settings are applied".
- **No notifications** are sent on fullscreen start or stop, so **poll every 1 s and on each foreground change**. [H]
- **[Unverified]:** what QUNS reports for a game that *requests* exclusive mode but is promoted to borderless flip by fullscreen optimizations. Do not use QUNS alone to decide "capture will freeze"; use it to word the hint.

### 8.2 Freeze watchdog (harness: `fullscreen.rs`)

| Signal | Rule (suggested; tune on the PC) | Verdict shown to the user |
|---|---|---|
| No new frame timestamp for 3 s while a known game is foreground and not minimized | `now − last_ts > 3 s` | "Capture looks frozen". If QUNS == RUNNING_D3D_FULL_SCREEN as well: "This game runs in exclusive fullscreen. Switch it to borderless." |
| DDA ACCESS_LOST ≥15 in 3 s | sliding window | Same, plus a slower retry |
| Content hash unchanged for 3 s while the game is foreground (frames arrive but are identical; WGC "froze after the first frame" in OBS #14021) | 1 Hz: scale the newest frame to 16×9 with the existing scale shader, read back, hash and average | "possibly frozen" |
| Mean luminance ≈ 0 for 3 s | same readback | "Black capture: DRM-protected or capture-excluded window" (`WDA_MONITOR` content, `DXGI_OUTDUPL_FRAME_INFO.ProtectedContentMaskedOut`) |

- **Mark, don't block.** Mark the affected ring segment `possibly_frozen` so a saved clip carries the flag (brief §2.2); never block saving.

### 8.3 `WDA_EXCLUDEFROMCAPTURE` for Kairo's windows

- **Engine-owned toast or overlay** (the engine shows it, so the UI can stay closed while gaming): after `CreateWindowExW`, call `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)?`. The window must be **top-level** and **owned by the calling process**. Verify with `GetWindowDisplayAffinity`. It works only "when the DWM is composing" (always true on Win10/11), and it is "not … a security feature". [H]
- **Tauri UI windows:** the engine *cannot* do this across processes. In the Tauri app, choose one:
  - `tauri.conf.json`: `"app": { "windows": [ { "label": "toast", "contentProtected": true, … } ] }` (tauri-utils `WindowConfig.content_protected`, serialised camelCase; the alias `content-protected` is also accepted);
  - Rust: `WebviewWindowBuilder::new(...).content_protected(true)`, or `window.set_content_protected(true)?`;
  - raw: `let hwnd = window.hwnd()?; unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)? }`. Same `windows 0.62` `HWND` type as the engine.
- **Product caveat:** a content-protected main window also disappears from the user's own screenshots and Discord screen-shares of Kairo. Protect toasts and overlays always; protect the main window only while a capture is running in screen mode (product decision). [L]
- **[Unverified]:** that DDA honours `WDA_EXCLUDEFROMCAPTURE` the same way WGC does. Teams/Zoom-style tools rely on it, but test both backends on the PC.

---

## 9. Recommended approach (summary for the plan)

**Module layout** (engine crate, `windows = "=0.62.2"`):

```
capture/
  mod.rs        CaptureSource trait { start, wait/poll, stop, health }, CapturedFrame { ring_slot, ts_hns, size, format }
  device.rs     GpuDevice: adapter that owns the target monitor (fallback EnumAdapterByGpuPreference HIGH_PERFORMANCE),
                BGRA|VIDEO_SUPPORT, SetMultithreadProtected(true), IDirect3DDevice wrapper, IsCurrent watcher
  wgc.rs        WgcSource (§3)                       -- Win11 default
  dda.rs        DdaSource + pointer.rs/pointer.hlsl  -- Win10 default, Win11 fallback (WGC unsupported/failing)
  monitors.rs   list + names + device path + refresh + HDR + SDR white (+ ADVANCED_COLOR_INFO_2)
  target.rs     foreground game window, monitor of window, WinEvent hook, minimize tracking
  health.rs     QUNS polling + FreezeWatchdog + 1 Hz black/frozen probe
  probe.rs      `kairo-capture-probe` CLI for the spike and the capability report
```

**Decision rules:**

- **Backend:**
  - WGC when `IsSupported() && border_toggle`, i.e. Windows 11.
  - DDA otherwise (Windows 10).
  - Also DDA when WGC creation fails twice, or the watchdog says WGC is stale while DDA is not.
- **Target:**
  - Window mode (Win11 only) captures the foreground *game* window. On `Closed` → re-resolve by PID; if that fails (elevation, black) → monitor mode.
  - Screen mode captures `MonitorFromWindow(game)`, or the monitor the user pinned by `monitorDevicePath`.
- **Format:** FP16 iff the target monitor is in HDR (re-evaluated on `IsCurrent() == FALSE`), else BGRA8.
- **WGC rate:** `MinUpdateInterval = 1/(2·fps_out)`. Log the default read back and the delivered-rate histogram in the capability report.
- **Effort [Estimate]:**

  | Piece | Days |
  |---|---|
  | WGC path + device + monitors + probe | 1.5–2 |
  | DDA + pointer | 1.5–2 (can come after the Win11 beta) |
  | Watchdog / QUNS / WDA | 0.5 |

  This fits the 7-day Rust engine skeleton budget if DDA is deferred.

---

## 10. Gotchas (checklist for the coding agent)

1. **Pin `windows = "=0.62.2"`.** Do not let any crate pull `windows-core`/`windows-future` 0.100 types into capture APIs; they are different types. If a dependency moves to 0.100, bridge with `Interface::as_raw()` / `from_raw()`.
2. **`TryGetNextFrame()`'s "no frame" is `Err` with `code() == S_OK`.**
3. **`IDirect3DDevice` (WinRT) is `!Send`** in 0.62.2, and `ID3D11DeviceContext` is `!Sync`. Build the device on the GPU thread.
4. **`HWND` is `!Send`.** Pass `isize` between threads.
5. **Use `CreateFreeThreaded`.** `Create` needs a DispatcherQueue and a message pump on the creating thread.
6. **Use 2 pool buffers**, never 1, and never hold a frame across pacer ticks.
7. **Recreate on `ContentSize` change, then validate with `GetDesc`.** `CopyResource` needs identical size and format, so re-create the ring textures too. Never `Recreate` with a 0×0 size.
8. **An explicit adapter needs `D3D_DRIVER_TYPE_UNKNOWN`** in `D3D11CreateDevice`, and `Option<&IDXGIAdapter>` (not `IDXGIAdapter1`) for the parameter.
9. **Do not assume adapter 0.** Ryzen 7000/9000 desktop CPUs have an iGPU that may be enabled. Pick the adapter that owns the monitor (`adapter_for_monitor`); DDA *requires* this.
10. **`D3D11_CREATE_DEVICE_VIDEO_SUPPORT`** on the shared device (FFmpeg D3D11VA init QIs `ID3D11VideoDevice`). Hand FFmpeg an AddRef'd pointer (`clone().into_raw()`), because it Releases on free.
11. **`SetMultithreadProtected(true)`** on the immediate context. Also give FFmpeg `lock`/`unlock` callbacks backed by a reentrant lock that Kairo's GPU thread also takes.
12. **`RequestAccessAsync(...).join()`** only on an MTA worker thread. Log the `AppCapabilityAccessStatus`, then always `SetIsBorderRequired(false)` and ignore its error, as FFmpeg does.
13. **Gate every 24H2 property with `ApiInformation::IsPropertyPresent`.** Setting one on an older build throws.
14. **Do not set MinUpdateInterval to exactly 1/fps** (≈45 fps on 60 Hz). Use 1/(2·fps), and never 0 on 360 Hz without a reason.
15. **Pace on `SystemRelativeTime`** (presentation QPC), not arrival time. Frames can carry future timestamps.
16. **DDA: release the old duplication before `DuplicateOutput1`.** Call `ReleaseFrame` right before the next `AcquireNextFrame`; treat `INVALID_CALL` there as benign.
17. **DDA pointer-only frames** (`LastPresentTime == 0`) are not new images. Their resource format may be bogus.
18. **DDA monochrome pointer height = `Height/2`.** `Type` is `u32` vs `i32` constants.
19. **DDA `DuplicateOutput1` needs PMv2.** Use the manifest (`embed-manifest`) plus a `SetThreadDpiAwarenessContext` wrap. Avoid process-wide DPI calls in library code.
20. **`DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2` is missing from windows 0.62.2.** Use your own `#[repr(C)]` struct and type 15. Non-zero returns before 24H2 are expected.
21. **Re-create the DXGI factory when `IsCurrent()` is FALSE**, or HDR and colour-space data stay stale.
22. **Persist monitors by `monitorDevicePath`**, not `\\.\DISPLAYn`, HMONITOR or index.
23. **`SetWindowDisplayAffinity` only works on your own process's top-level windows.** The Tauri UI must protect its own windows.
24. **`SHQueryUserNotificationState` has no change notification.** Poll it.
25. **`GraphicsCaptureItem.Closed` also fires when a game swaps its window.** Re-resolve by PID instead of stopping.
26. **Elevated games:** window capture may fail or go black. Fall back to monitor capture.
27. **Device removed** (driver update, TDR): rebuild everything that hangs off the device (pool via `Recreate(newDevice)`, DDA, ring, FFmpeg).
28. **Test with AMD Adrenalin Instant Replay disabled.** It also captures and encodes, which skews FPS and encoder measurements; then test coexistence separately.

---

## 11. How to test each piece on the PC

**Build and launch (the SSH session is non-interactive; WGC/DDA need the user's desktop).**

```powershell
# on the PC (or via SSH): build the probe
cargo build --release -p kairo-capture-probe
# run it in the logged-on user's interactive session
$exe = "C:\dev\kairo\target\release\kairo-capture-probe.exe"
schtasks /Create /TN KairoProbe /TR "`"$exe`" wgc --monitor primary --seconds 30 --fps 60 --json C:\dev\kairo\probe.json" /SC ONCE /ST 23:59 /IT /F
schtasks /Run /TN KairoProbe
while ((schtasks /Query /TN KairoProbe /FO CSV | ConvertFrom-Csv).Status -eq 'Running') { Start-Sleep 1 }
Get-Content C:\dev\kairo\probe.json
# elevated variant for the "elevated game" test: add /RL HIGHEST to a second task
```

On the Mac: `cargo check --target x86_64-pc-windows-msvc` is enough for the capture crate (verified). Use `cargo xwin check` once FFmpeg bindings are in the workspace.

**PresentMon** (v2.6.0, 2026-09-21): `PresentMon.exe --process_name <game>.exe --output_file pm.csv --timed 30 --terminate_after_timed --stop_existing_session`. Compare the `PresentMode` column and FPS / 1% lows with Kairo off, with WGC monitor capture, with WGC window capture, and with DDA.

| # | Piece | Test | Pass criteria |
|---|---|---|---|
| 1 | Capability report | `probe caps` prints `WgcCaps`, the build (`windows-version` 0.1.7 `OsVersion::current().build`), the `RequestAccessAsync(Borderless)` status, the default `MinUpdateInterval()` read back, and adapters / outputs / monitors (§7) | All 24H2 props present on 26200/26300; status logged; the two monitors listed with correct names, 360/180 Hz, HDR flags matching Settings |
| 2 | Border | Run WGC monitor capture 10 s. Grab a DDA frame at the same time (`ffmpeg -f lavfi -i ddagrab=output_idx=0:framerate=5 -frames:v 1 border.png`) and check the edge pixels for yellow. Repeat in window mode. | No yellow border; no prompt (record whether one appears) |
| 3 | Delivery rate on 360 Hz | Fullscreen 360 fps test content (e.g. a high-fps game or a frame-counter video). Probe histograms of `SystemRelativeTime` deltas for MinUpdateInterval = default, 16.67, 11.1, 8.33, 6.94, 2.78 ms. Repeat on the 180 Hz monitor. | 8.33 ms gives ~120/s with ≤8.4 ms staleness at 60 fps ticks; no 90 fps aliasing (otherwise adopt the refresh-aware rule) |
| 4 | Cost | Same runs: Task Manager GPU for `dwm.exe` and the probe (3D + Copy engines); PresentMon FPS / 1% lows of the game | ≤1% FPS delta at 8.33 ms; PresentMode unchanged or documented |
| 5 | HDR | Toggle HDR (Win+Alt+B) during capture | `IsCurrent()` flips, pool switches BGRA↔FP16 within 1 s, no crash; FP16 white UI ≈ SDRWhiteLevel/1000 in scRGB |
| 6 | ContentSize | Window mode on a resizable window and on a game resolution change | `Resized` events, ring re-created, encoder canvas stable, no `CopyResource` debug-layer errors (run once with `D3D11_CREATE_DEVICE_DEBUG` + Graphics Tools installed) |
| 7 | Closed / window swap | Close the target; switch a game windowed↔fullscreen | `Closed` handled; re-acquired by PID within 1 s |
| 8 | Minimize | Minimize the game in window mode | No frames, watchdog says "minimized" (not "frozen"), resumes on restore |
| 9 | Exclusive fullscreen | An exclusive-fullscreen title (or "Disable fullscreen optimizations" on a DX11 game) | QUNS value logged; watchdog flags "exclusive fullscreen" if frames freeze; DDA ACCESS_LOST rate logged |
| 10 | DDA basics | `probe dda` on each monitor: UAC prompt, Ctrl+Alt+Del, resolution change, HDR toggle | Recovers from every case; `E_ACCESSDENIED` during the secure desktop; pointer drawn correctly for arrow, I-beam (monochrome) and resize/colour cursors |
| 11 | Elevated target | Run a test window or game as admin | Window capture fails or goes black → automatic fallback to monitor capture works |
| 12 | WDA | Show the engine toast and a `contentProtected` Tauri window during WGC monitor capture and DDA | Visible on screen, absent in both captures |
| 13 | Device lost | Restart the graphics driver (Win+Ctrl+Shift+B; whether it removes D3D devices is **[Unverified]**) or during an Adrenalin update | Engine rebuilds the device and resumes, or reports cleanly |
| 14 | Coexistence | AMD Instant Replay on vs off | Kairo still captures; note any DDA `NOT_CURRENTLY_AVAILABLE` or encoder contention |

---

## 12. Open items and unverified points

- Whether an unpackaged app gets a consent prompt or `Allowed` from `RequestAccessAsync(Borderless)` on 25H2/26H2, and whether `IsBorderRequired(false)` alone suffices. Test 2.
- The default `MinUpdateInterval` value; whether DWM still renders capture surfaces it then drops; whether capture demotes independent flip. Tests 1, 3, 4.
- Behaviour on a VRR 360 Hz OLED with a game below the refresh rate; whether WGC timestamps then follow the game's presents. Test 3.
- Minimized-window behaviour (assumed: no frames).
- What QUNS reports for FSO-promoted "exclusive" games.
- Whether the windows-capture device (no `VIDEO_SUPPORT`) passes FFmpeg's `d3d11va_device_init`. Moot if Kairo owns the device.
- Whether `ID3D11Multithread::Enter` is reentrant. The recommendation avoids depending on it.
- Whether DDA honours `WDA_EXCLUDEFROMCAPTURE`. Test 12.
- Whether `embed-manifest`'s `/MANIFEST:EMBED` link args work under `cargo xwin` (lld-link). Release builds on the PC with MSVC are unaffected.

---

## Sources

Crates and Rust sources (all fetched 2026-10-21)
- https://crates.io/crates/windows-capture (2.0.1, 2026-08-08, MIT) · https://docs.rs/windows-capture/2.0.1/windows_capture/
- https://github.com/NiiightmareXD/windows-capture (source at commit c7d106448eb9d9b251345c39047711e1cd408ae2: `src/capture.rs`, `src/graphics_capture_api.rs`, `src/d3d11.rs`, `src/frame.rs`, `src/settings.rs`, `src/dxgi_duplication_api.rs`, `src/monitor.rs`, `src/window.rs`, `src/winrt.rs`)
- windows-capture issues/PRs: https://github.com/NiiightmareXD/windows-capture/issues/190 · /issues/191 · /issues/196 · /issues/197 · /issues/187 · /issues/210 · /issues/215 · /pull/216 · commit 1d9897b4 ("Fix capture session recreation lifetimes")
- https://crates.io/crates/windows (0.62.2) · https://github.com/microsoft/windows-rs/releases/tag/74 · https://github.com/microsoft/windows-rs/issues/4867 (0.100 newsletter)
- windows-core 0.62.2 (`src/imp/factory_cache.rs`, `src/type.rs`, `src/param.rs`), windows-result 0.4.1 (`src/error.rs`), windows-future 0.3.2 (`src/join.rs`), windows-version 0.1.7: crate tarballs from https://static.crates.io/
- https://crates.io/crates/tauri (2.12.4) · tao 0.37.1 `src/platform_impl/windows/window.rs` · tauri-utils `src/config.rs` (`content_protected`)
- https://crates.io/crates/embed-manifest (1.5.1)
- DDA crates: https://crates.io/crates/win_desktop_duplication · https://crates.io/crates/rusty-duplication · https://crates.io/crates/dxgi-capture-rs · https://crates.io/crates/dxgcap

Microsoft Learn
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.minupdateinterval
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.includesecondarywindows
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturedirtyregionmode
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess.requestaccessasync
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.create
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframe.systemrelativetime
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureitem.closed
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureitem.trycreatefromwindowid
- https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture
- https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11_4/nn-d3d11_4-id3d11multithread
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_create_device_flag
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_5/nf-dxgi1_5-idxgioutput5-duplicateoutput1
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-releaseframe
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ns-dxgi1_2-dxgi_outdupl_frame_info
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ne-dxgi1_2-dxgi_outdupl_pointer_shape_type
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/error-when-dda-capable-app-is-against-gpu (KB3019314)
- https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range
- https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_sdr_white_level
- https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ne-wingdi-displayconfig_device_info_type
- https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate
- https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ne-shellapi-query_user_notification_state
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- Windows SDK `wingdi.h` (DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2, type 15): https://github.com/microsoft/win32metadata/blob/main/generation/WinSDK/RecompiledIdlHeaders/um/wingdi.h

FFmpeg n9.0.2
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavfilter/vsrc_gfxcapture_winrt.cpp
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavfilter/vsrc_ddagrab.c
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavutil/hwcontext_d3d11va.c · https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavutil/hwcontext_d3d11va.h
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/doc/filters.texi (gfxcapture, ddagrab)

OBS, Sunshine, Apollo, Qt
- https://github.com/obsproject/obs-studio/blob/master/libobs-winrt/winrt-capture.cpp
- https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-subsystem.cpp (GetSdrMaxNits, GetMonitorColorInfo)
- https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-duplicator.cpp
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/duplicator-monitor-capture.c
- https://github.com/obsproject/obs-studio/pull/12759 · https://github.com/obsproject/obs-studio/issues/13893 · https://github.com/obsproject/obs-studio/issues/14021
- https://github.com/LizardByte/Sunshine/blob/master/src/platform/windows/display_wgc.cpp
- https://github.com/ClassicOldSong/Apollo/pull/785
- https://github.com/qt/qtmultimedia/blob/dev/src/plugins/multimedia/ffmpeg/qffmpegwindowcapture_uwp.cpp

Community reports
- https://handmade.network/forums/t/8701-wcap/2 (IsBorderRequired without RequestAccessAsync)
- https://windows-hexerror.linestarve.com/q/so48207677-dxgi-15-duplicateoutput1-fails-with-dxgierrorunsupported-0x887a0004 · https://pkg.go.dev/github.com/kirides/go-d3d (PMv2 for DuplicateOutput1)
- https://github.com/robmikh/Win32CaptureSample/issues/13 (minimized windows)
- https://obsproject.com/forum/threads/unable-to-capture-window-as-admin-but-fine-as-non-admin.166767 (elevation)
- https://github.com/GameTechDev/PresentMon (v2.6.0, `README-ConsoleApplication.md`)
