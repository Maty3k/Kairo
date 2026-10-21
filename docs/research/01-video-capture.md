# Kairo research 01: Windows video capture, hardware encoding and the replay-buffer pipeline

Prepared 2026-10-21 for the Kairo planning team. Scope: Windows 10 22H2 and Windows 11 (23H2 to 26H2) on AMD, Intel and NVIDIA GPUs. Nothing is built yet; this report feeds architecture decisions.

**Method and honesty notes**

- Facts were checked against Microsoft Learn, vendor SDK repositories and pages (AMD AMF, Intel media-driver/libvpl, NVIDIA developer pages), and source code fetched on 2026-10-21: OBS Studio `master`, FFmpeg tag `n9.0.2`, Sunshine `master`, Chromium `main`.
- The session's web-search budget ran out partway through. After that, I fetched only known primary URLs and read source code. Points I could not re-verify are marked **unverified**.
- Numbers marked **estimate** come from engineering arithmetic, not measurement. They must be confirmed in the spike on the user's PC (section 9).
- Confidence tags: **[H]** = primary source or source code; **[M]** = credible secondary source, or a single report; **[L]** = inference.

---

## 0. Executive summary

1. **Use DWM-based capture in v1. Do not use injection.** On Windows 11, use Windows.Graphics.Capture (WGC). On Windows 10, use DXGI Desktop Duplication (DDA). Both deliver D3D11 textures into the same downstream pipeline. Hook/injection "game capture" (the OBS approach) is the most capable method, but it brings anti-cheat, stability and maintenance risk that a solo-dev v1 should not take on. [H/M]
2. **Windows 10 22H2 cannot hide WGC's yellow capture border.** `IsBorderRequired` and `GraphicsCaptureAccess` start at build 20348, which on client PCs means Windows 11. Win10 22H2 is build 19045. An always-on replay buffer on Win10 via WGC would draw a permanent yellow frame around the screen, so **Win10 needs the DDA path** (DDA draws no border). [H]
3. **WGC frame pacing is not CFR, and its rate is hard to control before Win11 24H2.** `MinUpdateInterval` exists only on build 26100 and later. OBS (PR #12759), Apollo (PR #785) and FFmpeg's own code all treat WGC as about 60 fps-limited without it. Even with it, delivery is irregular. **Pace frames yourself** on a QPC clock (QueryPerformanceCounter, Windows' high-resolution timer) and resample to constant frame rate. [H for API, M for the 60 fps behaviour]
4. **True exclusive-fullscreen games are a known gap for DWM-based capture on 24H2+.** An Oct-2026 OBS report shows both DDA and WGC freezing on exclusive-fullscreen OpenGL games. Maintainers attributed it to the Microsoft API. Most modern DX11/DX12 games run flip-model or fullscreen-optimized and are capturable. [M]
5. **Hardware encoders:**
   - Every in-scope vendor has hardware H.264 and HEVC encoding.
   - **Hardware AV1 encoding is limited to:** NVIDIA RTX 40/50; AMD VCN 4/5 (RX 7000/9000, Ryzen 8000 "Phoenix" APUs); Intel Arc and Core Ultra (Meteor/Arrow/Lunar/Panther Lake).
   - **Some GPUs have no encoder at all** (RX 6300/6400/6500 XT, GeForce MX450), so Kairo needs capability probing and a fallback.
   - **The NVENC consumer session limit is now 12** per NVIDIA's matrix, so it is a non-issue. [H]
6. **FFmpeg:**
   - `ddagrab` (DDA) has existed since **6.0**.
   - `gfxcapture` (WGC) arrived in **8.1 (2026-03-16)**, not 8.0.
   - Current stable is **9.0.2 (2026-09-18)**.
   - D3D11 frames go zero-copy into `*_nvenc` and `*_amf`, and into `*_qsv` via `hwmap`.
   - **No FFmpeg version has a WASAPI input device**, so audio must be captured natively.
   - Verdict: the FFmpeg CLI is fine for a prototype. libav* used as a library is viable for production; Sunshine does this. [H]
7. **Replay buffer: copy the OBS model**, verified from source:
   - A RAM ring of *encoded* packets, bounded by seconds and MB.
   - Trimmed one whole GOP (group of pictures, the frames between keyframes) at a time, so it always starts on a keyframe.
   - On save, snapshot the packet references and mux on a background thread.
   - RAM is roughly bitrate × seconds / 8: **30 s at 25 Mbps is about 95 MB, 2 min about 380 MB, 5 min about 950 MB.** [H]
8. **Suggested v1 defaults:**
   - Video: 1080p60 CFR, H.264 High, VBR 25 Mbps target / 40 Mbps peak, 1 s closed GOP, 2 B-frames where supported, BT.709 limited range.
   - Audio: AAC 48 kHz.
   - Buffer: 30 s in RAM.
   - Screenshots: PNG; on HDR displays, JXR plus a tone-mapped PNG.
9. **Expected cost:** about 1 to 5% FPS, based on LTT Labs (Aug 2026), who measured 1 to 5 fps average loss for Game Bar, NVIDIA's and AMD's tools, and OBS on NVIDIA. Low single-digit CPU. Roughly 100 to 300 MB VRAM (estimate). About 95 MB RAM for the buffer, plus engine overhead.
10. **Run a 2 to 3 day spike on the user's Windows PC before committing.** Use FFmpeg 9.0.2 plus PresentMon to measure WGC vs DDA cost, presentation-mode changes, encoder load and HDR behaviour (section 9).

---

## 1. OS landscape on 2026-10-21 (changes the brief's assumptions)

| Version | Build | GA | Home/Pro servicing | Notes |
|---|---|---|---|---|
| Windows 11 26H2 | 26300 | 2026-09-29 | until 2028-10-10 | Enablement package on the 24H2/25H2 platform. **Not in the brief's list; it is now GA.** |
| Windows 11 26H1 | 28000 | 2026-02-10 | until 2028-03-14 | New devices only; not offered as an in-place update. |
| Windows 11 25H2 | 26200 | 2025-09-30 | until 2027-10-12 | |
| Windows 11 24H2 | 26100 | 2024-10-01 | **end of updates (Home/Pro) as of 2026-10-13** | Enterprise until 2027-10-12. |
| Windows 11 23H2 | 22631 | 2023-10-31 | **end of updates (Home/Pro)** | Enterprise until 2026-11-10. Lacks all of the 24H2 WGC APIs. |
| Windows 10 22H2 | 19045 | - | End of support 2025-10-14; ESU available | Microsoft's consumer ESU page (fetched today) says coverage runs "through October 12, 2027". |

Source: Microsoft Learn release-health page, updated 2026-10-13. [H]

**Windows 10 is still a quarter of gamers.** Steam Hardware Survey for Sep 2026: Windows 11 is 71.34% of all users and Windows 10 is 23.61% (about 25% of Windows users). [H]

**Practical reading:** Windows 11 users will overwhelmingly be on builds 26100, 26200 or 26300, so the 24H2 WGC APIs (`MinUpdateInterval`, `IncludeSecondaryWindows`, `DirtyRegionMode`) can be treated as available on Win11 with a runtime check. Windows 10 22H2 is the real "second platform".

---

## 2. Capture methods compared

### 2.1 Comparison table

| Aspect | Windows.Graphics.Capture (WGC) | DXGI Desktop Duplication (DDA) | Hook/injection (OBS "Game Capture") |
|---|---|---|---|
| Minimum OS | API from Win10 1803 (17134). Programmatic `CreateForWindow`/`CreateForMonitor` via `IGraphicsCaptureItemInterop` from **1903 (18362)** [H] | Win8. `DuplicateOutput1` (HDR/native formats) needs DXGI 1.5 (Win10) [H] | Any; depends on the hooked graphics API |
| Targets | One window, or one monitor per item | One monitor (output) per duplication | Only the hooked game's swap chain |
| Yellow border | Always on Win10. Removable on Win11 via `IsBorderRequired=false` (build 20348+) [H] | None | None |
| Cursor | `IsCursorCaptureEnabled` toggle from 2004 (19041) [H] | Pointer shape delivered separately; you composite it yourself [H] | Not captured; drawn separately (OBS does this) |
| Frame-rate control | `MinUpdateInterval` on 24H2+ (26100) only [H]; irregular delivery [M] | Frames only when the desktop updates; you pace and duplicate yourself [H] | At the game's Present rate |
| Exclusive fullscreen (true FSE) | Not reliable; freezes reported on 24H2+ [M] | Loses access (`DXGI_ERROR_ACCESS_LOST`) on fullscreen/mode switches and must re-create [H]; repeated ACCESS_LOST loops on 24H2+ [M] | Works; this is its main advantage |
| Flip-model / FSO games | Works (DWM-composed) | Works | Works |
| Minimized window | No new frames (games usually stop presenting anyway) [M] | Shows whatever is on screen | Usually no presents |
| Occluded window | Window capture still gets the window's own content [M] | Captures what is visible on the monitor (includes overlays and notifications) | Unaffected |
| Multi-monitor | One item per HMONITOR; combining monitors is your job | One duplication per output; device per adapter; **max 4 duplicating processes per session** [H] | N/A |
| Hybrid-GPU laptops | Frames delivered on the D3D device you pass; OS handles cross-adapter (OBS auto-selects WGC here) [H/L] | **Fails with `DXGI_ERROR_UNSUPPORTED` if run on the dGPU of a hybrid system**; must use the adapter that owns the output (usually the iGPU) [H] | Game GPU; OBS falls back to shared memory across adapters [H] |
| HDR | FP16 scRGB frame pool (`R16G16B16A16Float`) [H] | `DuplicateOutput1` with FP16 or R10G10B10A2 [H] | Captures the game's backbuffer format |
| Protected/DRM content | Black or omitted (`WDA_MONITOR` / `WDA_EXCLUDEFROMCAPTURE`) [H] | Same [H] | Hook lives in the game; N/A |
| Runs as a Windows service | No (Sunshine: WGC is "not compatible with the Sunshine service") [H] | Yes (secure desktop needs LOCAL_SYSTEM) [H] | No |
| Anti-cheat risk | Very low (OS API) [L] | Very low [L] | **High for a new, unknown injector** (unverified for specific anti-cheats) |
| Engineering effort | Low to medium | Medium (ACCESS_LOST, pointer compositing, hybrid quirks) | High (D3D8-12, OpenGL, Vulkan layer, 32/64-bit helpers, signing) |

### 2.2 WGC details

**API version history** (Microsoft Learn) [H]:

| API | Minimum build |
|---|---|
| `GraphicsCaptureSession` | 1803 (17134) |
| `IGraphicsCaptureItemInterop` | 1903 (18362) |
| `IsCursorCaptureEnabled` | 2004 (19041) |
| `IsBorderRequired` and `GraphicsCaptureAccess.RequestAccessAsync(Borderless)` | "Windows 10, version 2104 (10.0.20348.0)", the Server 2022 build; on clients this means Windows 11 |
| `MinUpdateInterval`, `DirtyRegionMode`, `IncludeSecondaryWindows` | 24H2 (26100) |

`IncludeSecondaryWindows` is described as "Windows 11, version 24H2 (introduced in 10.0.26100.0)".

**Border removal on Windows 11** [H, with one unverified point]:

- The docs say the system needs user consent via `RequestAccessAsync(GraphicsCaptureAccessKind.Borderless)`. That call "displays a prompt" and needs the `graphicsCaptureWithoutBorder` capability for packaged apps.
- OBS, an unpackaged Win32 app, calls `RequestAccessAsync(Borderless).get()` and then `IsBorderRequired(false)` whenever the property exists.
- FFmpeg's `gfxcapture` sets `IsBorderRequired` directly through `IGraphicsCaptureSession3`. Its code comment notes the call "can return failure but still work".
- An OBS maintainer (discussion #9590) states border removal is "limited exclusively to Windows 11".
- **Unverified:** whether an unpackaged app on Win11 25H2/26H2 sees any consent prompt. The spike must check this.

**Capabilities if Kairo ships as MSIX/Store** [H]: `graphicsCaptureProgrammatic` (to create items from window/display IDs without the picker) and `graphicsCaptureWithoutBorder`. Both are listed under general-use capabilities.

**Frame pool and session behaviour:**

- Use `Direct3D11CaptureFramePool.CreateFreeThreaded` so no DispatcherQueue is needed. Both OBS and FFmpeg use a pool of 2 buffers.
- Copy the frame out immediately with `CopyResource`, then release the frame.
- Handle `ContentSize` changes with `Recreate`. Content outside the frame is clipped, and the area beyond `ContentSize` is undefined. [H]

**Timestamps:** `Direct3D11CaptureFrame.SystemRelativeTime` "is QPC time that can be used to synchronize other media elements". [H]

**Frame rate and pacing:**

- OBS PR #12759 is titled "Prevent hard cap of 60 fps for WGC capture". It was approved Jul 2026 and was still open at last check.
- Apollo PR #785 (merged Jun 2025) fixed a "60 fps lock" by setting `MinUpdateInterval`.
- FFmpeg logs "framerate may be limited" when `IGraphicsCaptureSession5` is unavailable.
- With `MinUpdateInterval` set to 1/60 s on a 60 Hz panel, one Win11 24H2 user measured about 45 fps; 1/90 s gave about 60 fps (windows-capture #190, Feb 2026).
- On Windows 10, OBS #13893 (Sep 2026) reports WGC delivery following the refresh rate of the display the mouse is on, with uneven pacing.
- **Design consequence:** treat WGC as a "latest frame" source. Set `MinUpdateInterval` to about 1/(1.5 × target fps) on 26100+, and drive encoding from your own QPC pacer. [M]

**HDR:** use an FP16 pool (`DXGI_FORMAT_R16G16B16A16_FLOAT`). Microsoft notes that captured HDR needs "HDR-to-SDR tone mapping (for example, SDR screenshot copy for internet sharing) and content saving with proper format (for example, JPEG XR)". OBS picks FP16 only when the monitor is HDR. [H]

**Threading:** FFmpeg enables `ID3D10Multithread::SetMultithreadProtected(TRUE)` on the D3D11 device shared with WGC callbacks. [H]

### 2.3 DDA details

- **Adapter rule:** the device "must be created from the adapter to which the output is connected".
- **Return codes from `DuplicateOutput`:**
  - `E_ACCESSDENIED` on the secure desktop (UAC, lock screen) unless running as LOCAL_SYSTEM.
  - `DXGI_ERROR_UNSUPPORTED` for unsupported modes.
  - `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` when four processes are already duplicating.
- **`DXGI_ERROR_ACCESS_LOST`** on desktop switch, mode change, "Switch from DWM on, DWM off, or other full-screen application". Recover by releasing and re-creating the duplication. [H]
- **Formats:** `DuplicateOutput` always returns BGRA. It converts fullscreen surfaces, which costs performance. `DuplicateOutput1` can hand back the fullscreen app's original format, including FP16 and R10G10B10A2. FFmpeg sets `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` on the thread before calling it. [H]
- **Cursor:** you draw it from `GetFramePointerShape` and `PointerPosition`. Games that draw a software cursor are always captured with it. [H]
- **24H2 behaviour change:** an MPO-related change on 24H2 (robmikh #83) was fixed by KB5046617. This shows DDA behaviour can shift with OS updates. [M]
- **Hybrid laptops:** Microsoft KB3019314 says DDA "does not support being run against the discrete GPU on a Microsoft Hybrid system". The workaround is to run on the integrated GPU. [H]

### 2.4 Hook-based capture (OBS Game Capture), for reference only

- OBS injects `graphics-hook` into the game process. It hooks D3D8, D3D9, D3D10, D3D11, D3D12, OpenGL and Vulkan; Vulkan uses an implicit layer, `obs-vulkan64.json`. Frames reach OBS through a shared texture, or shared memory when textures cannot be shared across adapters. [H]
- **Injection methods:**
  - Direct: CreateRemoteThread-style via `inject-helper`, with obfuscated function names.
  - "Anti-cheat compatibility hook": `SetWindowsHookEx(WH_GETMESSAGE)` on the game's thread. [H]
- **Pros:** works in true exclusive fullscreen; lowest overhead; game-only image (no notifications).
- **Cons for Kairo:**
  - Kernel anti-cheats are likely to block or flag an unknown injector. This is **unverified for specific titles**, but it is the reason OBS ships a compatibility mode.
  - Hook bugs crash the *game*.
  - Every graphics API and bitness must be maintained.
  - The DLLs need code signing.
- Recommendation: v2+ at the earliest, and only if the spike shows many target games use true exclusive fullscreen.

### 2.5 Exclusive fullscreen, fullscreen optimizations (FSO) and independent flip

- **Fullscreen optimizations** (Windows 10+): exclusive-fullscreen games run as an optimized borderless window. "When an overlay such as the Game Bar is present, the DWM reassumes control of the display" (DirectX blog, Dec 2019). [H]
- **Independent flip:** flip-model swap chains that cover the screen can bypass composition. "If other desktop contents come on top, the DWM can either seamlessly transition back to composed mode, efficiently 'reverse compose'... or leverage MPO." Microsoft points to PresentMon to see which mode is active. [H]
- **"Optimizations for windowed games"** (Windows 11 setting): moves DX10/DX11 blt-model games to flip model, lowering latency and enabling Auto HDR and VRR. Turning on Auto HDR enables it automatically. This makes more games composable and capturable. [H]
- **Unverified:** whether an active WGC/DDA capture demotes a game from "Hardware: Independent Flip" to "Composed: Flip". That would cost about one frame of latency and possibly VRR behaviour. The spike must measure this with PresentMon (`PresentMode` column).
- **True FSE on 24H2+:** OBS #14021 (2026-10-11, Win11 26300) reports a freeze on exclusive-fullscreen OpenGL titles (Minecraft Java, osu!) with both DXGI (ACCESS_LOST loop) and WGC. It persisted with FSO disabled, MPO off and older drivers. Maintainers closed it as a Microsoft API issue. [M]
- **Product consequence:** Kairo should detect "frozen" captures (no new frame timestamps while the game presents), warn the user, and suggest borderless fullscreen.

### 2.6 Hybrid-GPU laptops

- **WGC:** OBS's display capture defaults to DXGI. It switches to WGC automatically when the monitor is not on OBS's adapter, or when the PC runs on battery and has two or more adapters (from `duplicator-monitor-capture.c`). [H]
- **Cross-adapter copy cost:** 1080p60 BGRA is about 0.5 GB/s, 1440p60 about 0.9 GB/s, 2160p60 about 2.0 GB/s. FP16 doubles each. [estimate]
- **Encoder placement:** choose the encode adapter explicitly (dGPU NVENC vs iGPU Quick Sync), and create the capture device on that adapter so the copy happens once.
- **DDA:** must run on the output-owning adapter, typically the iGPU. Then either encode on the iGPU (Quick Sync / AMD APU VCN) or copy to the dGPU.

### 2.7 Protected content

- `WDA_MONITOR` windows show "no content" in captures.
- `WDA_EXCLUDEFROMCAPTURE` (Win10 2004+) windows "do not appear at all". It degrades to `WDA_MONITOR` on older builds.
- Protection only works while DWM composes. [H]
- Kairo should set `WDA_EXCLUDEFROMCAPTURE` on its own overlay and toast windows so they never appear in clips.
- Hardware-DRM video (for example, browser streaming services) typically records black. [M]
- Whether specific cloud-gaming clients use protected paths is **unverified**; test them.

### 2.8 Overhead: what is measured vs estimated

**Measured anchor** (LTT Labs, published 2026-08-24; reported by GIGAZINE 2026-08-26) [M, secondary source]:

- Setup: Ryzen 7 7800X3D, H.264 at 6 Mbps, Cyberpunk 2077 / F1 24 / RDR2. Resolution and baseline FPS are not in the text.

| Tool | Avg FPS loss, RTX 5060 | Avg FPS loss, RX 9060 XT |
|---|---|---|
| Game Bar | -1 | -1 |
| NVIDIA recording | -3 | n/a |
| AMD recording | n/a | -5 |
| OBS | -5 | -16 |
| Streamlabs | -12 | -22 |

**Estimates for a zero-copy DWM-capture pipeline** [estimate]:

| Cost | 1080p60 | 4K60 / FP16 |
|---|---|---|
| GPU copy bandwidth (BGRA) | 0.5 GB/s; trivial against 270+ GB/s VRAM bandwidth on current mid-range cards | 2.0 GB/s at 4K60; FP16 doubles any of these |
| Colour conversion / scale shader | about 0.1 to 0.3 ms GPU per frame | higher |
| VRAM | about 50 to 150 MB (frame pool, NV12 pool, encoder reference frames, bitstream buffers) | about 200 to 400 MB |

- The hardware encoder is a separate ASIC (VCN / NVENC / Quick Sync) and does not take 3D-engine time. [L]
- NVIDIA lists features that use CUDA internally: high-quality two-pass, look-ahead, all AQ modes, weighted prediction, RGB-input encoding, temporal filter. NVIDIA says their impact "on overall CUDA or graphics performance is minimal". [H]
- CPU: low single-digit percent of an 8-core for capture, pacing, audio AAC and muxing. It rises sharply if frames are ever downloaded to system memory. [estimate]
- **Hook capture:** one extra GPU copy inside the game's frame. It does not involve DWM, which is why OBS treats it as the efficient path. No primary benchmark was found (search budget exhausted).

### 2.9 What existing products use

| Product (version) | Capture | Encode | Buffer |
|---|---|---|---|
| OBS Studio 32.2.2 (2026-08-14; 33.0 RC3 2026-10-20) | Game Capture (hook, as above); Window Capture (BitBlt or WGC; WGC auto-chosen for Chrome/Firefox/UWP/Office/SDL/Roblox window classes); Display Capture (DXGI default; WGC on multi-adapter laptops) [H] | Native texture encoders: `obs-nvenc` (NVENC SDK), `texture-amf` (AMF SDK), `obs-qsv11` (VPL), x264. No Media Foundation encoders in the plugin list [H] | RAM ring of encoded packets (section 6.1) [H] |
| NVIDIA App (ShadowPlay) | Not documented. NVIDIA's public NvFBC "has been deprecated on Windows 10 and above" (last supported 1803) [H] | NVENC | Disk temp files (community reports; users build RAM disks to spare SSDs) [M] |
| AMD Software: Adrenalin (Instant Replay) | Not documented. AMF ships an AMD-proprietary DisplayCapture with "lower latency and lower impact on the CPU and GPU" than its DDA-based component; plausibly related [L] | AMF / VCN | Disk by default, optional system memory (user reports; AMD page timed out) [M/unverified] |
| Xbox Game Bar | OS "Game DVR". The historical buffer is exposed to third parties only for *their own app* via `AppRecordingManager.RecordTimeSpanToFileAsync` [H] | OS | HDR screenshots saved as 16-bit .jxr plus tone-mapped 8-bit .png (Microsoft Q&A; GDK `wdcapture` saves .png or .jxr) [M] |
| Steam Game Recording | Through the Steam overlay (works for non-Steam games where the overlay works) [M] | "dedicated video encoding hardware of your AMD or nVidia GPUs" (Intel not mentioned) [H] | "Background Recording: continuously saved to your preferred drive, never exceeding your specified duration and storage limits" [H]. Press: default 120 min, 12 Mbps [M] |
| Medal | Not documented; modes Full screen / Display / Window [M] | GPU (NVENC/AMF/QSV) or CPU; H.264 "recommended default", H.265 for 1440p+, AV1 early access (RTX 40+, RX 7000+, Arc); 24-144 fps, 3-100 Mbps [H, Medal support] | Not documented |
| Sunshine (stream host, GPL-3.0) | DDA default (`ddx`); WGC "beta", not usable as a service [H] | libavcodec `h264/hevc/av1_amf`, `h264/av1_qsv`, `h264_mf` fallback, plus its own NVENC [H] | n/a |

---

## 3. FFmpeg as an engine

### 3.1 Versions [H]

| Release | Date | Relevant contents |
|---|---|---|
| 6.0 | 2023 | `ddagrab` (DDA capture filter) |
| 7.1 | 2024 | D3D12VA HEVC encoder; Vulkan H.264/H.265 encoders; mastering-display/CLL metadata in `hevc_nvenc`/`av1_nvenc` |
| 8.0 "Huffman" | 2025-08-22 | `scale_d3d11` (ID3D11VideoProcessor scale and convert to nv12/p010); Vulkan AV1 encoder |
| 8.1 "Hoare" | 2026-03-16 | **`gfxcapture` (WGC)**; D3D12 H.264 and AV1 encoders |
| 9.0 "Lei" | tagged 2026-08-03 | AMF HDR colour converter, AMF memory mapping; removed pre-11.1 NVENC SDK support |
| 9.0.2 | 2026-09-18 | current stable; 8.1.3 was 2026-09-21 |

- The user's Mac has "ffmpeg 8". These filters are Windows-only anyway. On the Windows PC, use a current build: BtbN builds have included `gfxcapture` since at least the 2025-10-28 master build (issue #549).

### 3.2 Filter options (FFmpeg 9.0.2 docs) [H]

**`ddagrab`**
- Options: `output_idx`, `draw_mouse` (hardware cursors only), `framerate` (default **30**), `video_size`, `offset_x`/`offset_y`, `output_fmt` (`auto`, `8bit`/`bgra`, `10bit`/`x2bgr10`; source also has `16bit`/`rgbaf16`), `dup_frames` (default true; duplicates frames to approximate a constant rate).
- Output: D3D11 hardware frames only.

**`gfxcapture`**
- Selection: `window_title` / `window_class` / `window_exe` (ECMAScript regexes), `monitor_idx` (or `window`), `hwnd`, `hmonitor`.
- Other options: `capture_cursor` (on), `capture_border` (off), `display_border` (off; has no effect on Win10, where the border is always drawn [L]), `max_framerate` (default **60**), `width`/`height`, `crop_*`, `premultiplied`, `resize_mode` (`crop`/`scale`/`scale_aspect`), `scale_mode` (`point`/`bilinear`/`bicubic`), `output_fmt` (`bgra`, `x2bgr10`, `rgbaf16`).
- Behaviour: "This source does NOT hold a stable FPS... add an fps filter". EOF when the window closes.
- Implementation: `CreateFreeThreaded` pool of 2; pts from `SystemRelativeTime`; `MinUpdateInterval` set from `max_framerate` when available.

### 3.3 Zero-copy encoding paths (prototype commands; flags verified in 9.0.2 source, pipelines not yet run)

```
# AMD (AMF accepts D3D11 frames directly, including BGRA/X2BGR10/RGBAF16 sw formats)
ffmpeg -filter_complex "gfxcapture=monitor_idx=0:max_framerate=90,fps=60" \
  -c:v h264_amf -usage transcoding -quality quality -rc vbr_peak -b:v 25M -maxrate 40M \
  -g 60 -bf 2 -forced_idr 1 out.mp4

# NVIDIA (NVENC accepts D3D11 frames)
... -c:v h264_nvenc -preset p5 -tune hq -rc vbr -b:v 25M -maxrate 40M -g 60 -bf 2 \
  -b_ref_mode middle -spatial-aq 1 -forced-idr 1 out.mp4

# Intel (convert on GPU, then map to QSV)
... "gfxcapture=...,fps=60,scale_d3d11=format=nv12,hwmap=derive_device=qsv,format=qsv" \
  -c:v h264_qsv -preset medium -b:v 25M -maxrate 40M -g 60 -bf 2 out.mp4

# Rolling on-disk buffer for a prototype: 2-s segments, ring of 45 (~90 s)
... -f segment -segment_time 2 -segment_wrap 45 -segment_format mpegts -reset_timestamps 1 buf%03d.ts
# On hotkey: concat the newest N segments with -c copy -movflags +faststart clip.mp4
```

- The Intel `hwmap` device-derivation incantation is the least certain line. Validate it in the spike.

### 3.4 Gaps

- **No WASAPI capture in libavdevice.** The 9.0.2 device list has `dshow`, `gdigrab`, `lavfi`, `openal`, `vfwcap`, among others. System audio needs a loopback-capable dshow device (for example "Stereo Mix"), which many PCs lack. [H]
- **The CLI has no in-RAM ring buffer.** The segment muxer writes to disk, and "save last N seconds" has to be scripted outside FFmpeg.
- Pacing, HDR tone-mapping control, device-loss recovery, hotkeys and process supervision all sit outside the CLI.
- **Licensing:**
  - FFmpeg is LGPL-2.1+ unless built with `--enable-gpl` (x264/x265) or nonfree.
  - `*_amf`, `*_nvenc`, `*_qsv` and `*_mf` work in LGPL builds.
  - BtbN offers both LGPL and GPL builds.

### 3.5 Verdict

- **Prototype: yes.** Within a day on the Windows PC, the CLI can validate capture quirks, encoder quality and load, and HDR behaviour.
- **Production: libav* as a library, yes.** Kairo owns WGC/DDA capture, the pacer, WASAPI audio and the ring buffer. libavcodec does encoding through D3D11 hwframes, and libavformat muxes. Sunshine ships exactly this mix of libavcodec hardware encoders. Rust bindings exist: `ffmpeg-next` 9.0.0 / `ffmpeg-sys-next` 9.0.1.
- **The FFmpeg CLI as the production engine: no.** No RAM ring, no WASAPI, and fragile process control.

---

## 4. Hardware encoders

### 4.1 AMD AMF (SDK v1.5.3, 2026-09-29; MIT)

Codec support by VCN version (AMF wiki, edited 2025-12-05) [H]:

| VCN | GPUs / APUs | Encode AVC | Encode HEVC | Encode AV1 |
|---|---|---|---|---|
| 2.0 | RX 5000 (Navi10/14), Ryzen 5000 "Cezanne" | 8-bit, 4K | 8/10-bit, 4K | - |
| 3.0 / 3.1 | RX 6600-6950 (Navi21-23); Ryzen 6000 "Rembrandt" (3.1); Ryzen 7000 desktop iGPU (3.1); Ryzen 9000 desktop iGPU (3.0) | 8-bit, 4K | 8/10-bit, 8K | - (decode only) |
| 3.0 decode-only | **RX 6300/6400/6500 XT (Navi24): no encoder** | - | - | - |
| 4.0 | RX 7000 (Navi31-33), Ryzen 8000 "Phoenix" | 8-bit, 4K | 8/10-bit, 8K | 8/10-bit, 8K |
| 5.0 | RX 9060/9070 (Navi44/48) | 8-bit, 4K | 8/10-bit, 8K | 8/10-bit, 8K |

- AV1 runs on only one VCN instance, except on Strix Halo. [H]
- **RDNA3 AV1 alignment:** AMF's `AV1_ALIGNMENT_MODE` has `64X16_ONLY`, `64X16_1080P_CODED_1082`, `NO_RESTRICTIONS` and `8X2_ONLY`. 1920×1080 is coded as **1920×1082 with two black lines**. Query `CAP_WIDTH/HEIGHT_ALIGNMENT_FACTOR`. RDNA4 relaxing this is reported but unverified. [H/M]
- **B-frames:**
  - AVC: capability-gated (`AMF_VIDEO_ENCODER_CAP_BFRAMES`); cannot be combined with LTR.
  - **HEVC: the AMF HEVC API has no B-frame properties.**
  - AV1: added in SDK 1.4.36 (2025-04), gated by `AV1_CAP_BFRAMES`.
  [H]
- **Rate control modes:** CQP, CBR ("not optimal for recording"), peak-constrained VBR, latency-constrained VBR, QVBR (level 1-51, default 23), HQ-VBR, HQ-CBR. **QVBR, HQ-VBR and HQ-CBR require Pre-Analysis and are 8-bit only.** [H]
- **Licensing notice:** AMD's AMF docs disclaim codec patent licenses: "you will pay any royalties due". [H]

### 4.2 Intel Quick Sync via oneVPL / Intel VPL

Encode support by platform (intel/media-driver README; hardware capability is OS-independent) [H]:

| Platform | AVC | HEVC 8-bit | HEVC 10-bit | AV1 8/10-bit |
|---|---|---|---|---|
| Broadwell+ | ✓ | | | |
| Skylake+ | ✓ | ✓ | | |
| Kaby Lake+ (incl. Tiger/Rocket/Alder/Raptor Lake iGPUs, DG1) | ✓ | ✓ | ✓ | decode only |
| Arc A-series (DG2), Meteor/Arrow Lake (Core Ultra 1/2), Lunar Lake, Arc B-series (Battlemage), Panther Lake, Nova Lake | ✓ | ✓ | ✓ | **encode** |

- **Runtime:**
  - The `libvpl` dispatcher is at v2.17.0 (2026-06-25; MIT).
  - The `vpl-gpu-rt` runtime is at 26.3.5 (2026-09-30) and covers Tiger Lake and newer.
  - Pre-Tiger Lake GPUs go through the legacy Media SDK runtime, limited to the MSDK API. [H]
- **Zero-copy:** VPL API 2.10+ `mfxMemoryInterface::ImportFrameSurface` imports an `ID3D11Texture2D` (`MFX_SURFACE_TYPE_D3D11_TEX2D`) for encode or VPP, with or without a copy. [H]

### 4.3 NVIDIA NVENC (Video Codec SDK 13.1 is latest)

From NVIDIA's support matrix, fetched 2026-10-21 (page undated) [H]:

| Capability | GPUs |
|---|---|
| H.264 4:2:0 | Maxwell+ |
| HEVC 10-bit | Pascal+ |
| HEVC B-frames | 7th-gen NVENC: Turing RTX 20, GTX 1650 SUPER/1660, and newer. **The plain GTX 1650 and GTX 1630 have 6th-gen NVENC without HEVC B-frames.** |
| AV1 | Ada (RTX 40) and Blackwell (RTX 50) only |
| H.264/HEVC 4:2:2 | Blackwell only |
| No NVENC at all | GeForce MX450, MX110/130, 920MX-940MX |

- **Encoder engine counts:**
  - RTX 4060-4070 SUPER: 1; RTX 4070 Ti-4090: 2.
  - RTX 5050-5070: 1; RTX 5070 Ti/5080: 2; RTX 5090: 3.
- **Max concurrent sessions on GeForce: 12** for every listed GeForce with NVENC. The limit is driver-enforced and was historically lower.
- NVENC needs features such as look-ahead (`enableLookahead`, depth up to 32). NVIDIA "strongly recommends" B-frames-as-reference (middle) when using multiple B-frames. [H]

### 4.4 Media Foundation encoder MFTs (vendor-neutral API, not vendor-neutral behaviour)

- **Hardware MFTs** must use the asynchronous model (`MF_TRANSFORM_ASYNC`, `METransformNeedInput` / `METransformHaveOutput`) and support dynamic format changes. Pass D3D through `MFT_MESSAGE_SET_D3D_MANAGER` with an `IMFDXGIDeviceManager`. [H]
- **Each GPU driver registers its own MFTs** (enumerate with `MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE ...)`). Names and capabilities vary by driver.
- **Chromium's MF encoder is full of per-vendor workarounds:**
  - Intel HEVC rate-control trust.
  - NVIDIA skipped for some temporal-layer modes (crbug 1088650).
  - Qualcomm AV1 HMFT incompatible with software rate control.
  - AMD H.264 software rate control disabled by GPU workaround.
  This is direct evidence of behaviour drift between vendors. [H]
- **AV1 MFT availability is inconsistent.**
  - Chromium enumerates AV1 HMFTs, and wcap encodes AV1 through MF. [H]
  - But a 2026-07 Intel community report on an Arc 140T (driver 32.0.101.8860, Win11 26200) found `av1_qsv` works while `av1_mf` fails with "Function not implemented". There was no AV1 MFT registered. [M]
  - **Always probe at runtime.**
- **Software MFTs:** Microsoft's H.264 encoder is built in. Software HEVC needs the Store "HEVC Video Extensions" (8-bit only per wcap). [M]
- **Windows N editions** ship without Media Foundation unless the user installs the Media Feature Pack. That affects the MF AAC encoder, MF H.264 and the MP4 sink. [M, general knowledge]
- **Trade-off summary:**
  - For: zero extra dependencies, OS-maintained, decent latency.
  - Against: fewer knobs (look-ahead/AQ/B-frame control is spotty across vendors; unverified per vendor) and per-vendor quirks.
  - OBS does not use MF encoders; Sunshine uses `h264_mf` only as a fallback.

### 4.5 Emerging vendor-neutral paths

- D3D12 Video Encode (FFmpeg `hevc_d3d12va` since 7.1, `h264_d3d12va`/`av1_d3d12va` since 8.1).
- Vulkan Video encode (FFmpeg H.264/H.265 since 7.1, AV1 since 8.0).
- Promising, but driver maturity on Windows is unproven for this use. **Not for v1.** [H for versions; L for maturity]

### 4.6 Zero-copy path: capture texture to encoder

1. **Capture texture.** BGRA8, or FP16 scRGB for HDR. Copy it out of the WGC/DDA pool into Kairo's own "latest frame" texture.
2. **Scale and convert on the GPU** to **NV12** (SDR) or **P010** (HDR10). Two options:
   - **`ID3D11VideoProcessor`:** `VideoProcessorBlt`, with `ID3D11VideoContext1::VideoProcessorSetStreamColorSpace1` / `SetOutputColorSpace1` (Windows 10+), for example `DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709` to `DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709`. This is what FFmpeg's `scale_d3d11` does.
   - **An HLSL/compute shader:** what OBS does. It gives exact, vendor-independent control of matrix, range and tone-mapping. **Recommended**, because VideoProcessor colour handling varies by driver [L].
3. **Hand the NV12/P010 texture to the encoder:**
   - NVENC: register the D3D11 resource.
   - AMF: `InitDX11` plus a DX11-native surface.
   - VPL: `ImportFrameSurface`.
   - MF: `MFCreateDXGISurfaceBuffer`.
   - libavcodec: an `AV_PIX_FMT_D3D11` hwframes context.

- Everything should share one D3D11 device (or use shared NT handles plus fences).
- NVENC and AMF can take RGB directly and convert internally (NVENC uses CUDA for this). That is convenient, but Kairo loses control over matrix and range. [H/L]

### 4.7 Fallback when there is no usable hardware encoder

- Affected: RX 6300/6400/6500 XT, GeForce MX, very old GPUs.
- Options:
  - Microsoft software H.264 MFT: no extra licensing; CPU cost is high at 1080p60.
  - OpenH264 (Cisco binary).
  - x264: GPL, which would force a GPL or commercial licence.
- Medal falls back to CPU recording in this case. [H, Medal support]
- **Decision needed** (see questions): support CPU encoding at 720p/30-60 in v1, or declare these GPUs unsupported.

---

## 5. Recommended encoder settings for gameplay clips

Benchmarks for context:
- **YouTube's recommended *upload* bitrates (SDR):** 1080p60 12 Mbps; 1440p60 24 Mbps; 2160p60 53-68 Mbps. HDR: 15 / 30 / 66-85 Mbps. YouTube also asks for H.264 High, closed GOP of half the frame rate, 2 consecutive B-frames, BT.709, MP4 with "moov atom at the front", and no edit lists. [H]
- Local clips should sit **above** those figures, because hardware encoders are less efficient than offline x264 and clips get re-encoded on upload.

### 5.1 Recommended bitrates (VBR target / peak) [recommendation, to tune in the spike]

| Output | H.264 | HEVC | AV1 |
|---|---|---|---|
| 1080p60 | 25 / 40 Mbps | 16 / 25 Mbps | 12 / 20 Mbps |
| 1440p60 | 40 / 60 Mbps | 25 / 40 Mbps | 20 / 32 Mbps |
| 2160p60 | 75 / 100 Mbps (check H.264 4K limits per vendor; AMD AVC encode is limited to 4K) | 50 / 75 Mbps | 40 / 60 Mbps |
| 120 fps | about 1.5× the 60 fps values | | |

### 5.2 Other settings

| Setting | Recommendation | Why |
|---|---|---|
| Rate control | **VBR with peak cap**. NVENC: `rc=vbr` (optionally a quality target via `cq`). AMF: `vbr_peak` (QVBR/HQVBR need Pre-Analysis, 8-bit only). QSV: VBR. Avoid CBR. | Bounded size keeps the RAM ring predictable. CQP/QVBR/ICQ give better quality per bit but unbounded size; OBS's Simple mode uses CQP 23 plus an MB cap. |
| Keyframe interval | **1 s (= fps frames), closed GOP, every keyframe an IDR** (`forced-idr`, `strict_gop` / `idr_interval`). Repeat SPS/PPS on IDR. | Clip start precision ≤1 s, and every GOP is independently decodable. Cost is a few % bitrate vs 2 s [L]. |
| B-frames | **2 where supported, else 0.** NVENC H.264: yes. NVENC HEVC: 7th-gen and newer. AMF AVC: if `CAP_BFRAMES`. AMF HEVC: none. AMF AV1: if cap. QSV: yes. NVENC B-ref middle. | Quality per bit. Latency is irrelevant for a replay buffer. Muxer must handle pts≠dts. |
| Look-ahead / multipass | **Off by default.** Optional "quality" preset: NVENC look-ahead 16-32, AMF PA, QSV LA. | Uses CUDA/compute and VRAM in the game's GPU; keep the default lean. |
| Adaptive quantization | NVENC spatial AQ on (temporal AQ off); AMF VBAQ where available; QSV defaults. | Cheap visible gains. |
| Preset | NVENC P4-P5 with `tune hq`; AMF `quality=balanced/quality`, `usage=transcoding`; QSV `medium` (`low_power` default on Gen12+). | Balance quality against encoder occupancy. Validate occupancy at 1440p/4K. |
| Profile | H.264 High; HEVC Main (Main10 for HDR); AV1 Main | Compatibility |
| Colour (SDR) | **BT.709 primaries/transfer/matrix, limited (TV) range, NV12**, signalled in the VUI and in MP4 `colr` (nclx) | OBS default (NV12/709/Partial). Full range commonly mis-plays [M]. |
| HDR desktop → SDR clip (default) | Capture FP16 scRGB. Tone-map in a shader. Write BT.709. | See next row. |
| SDR clip tone-map details | BT.2390 EETF on maxRGB (OBS implements it in `libobs/data/color.effect`, plus Reinhard). Read the user's **SDR white level** via `DISPLAYCONFIG_SDR_WHITE_LEVEL` (nits = value/1000×80). OBS defaults: SDR white 300 nits, HDR nominal peak 1000 nits. Windows scRGB 1.0 = 80 nits. | Shareable clips look right on phones and Discord. |
| HDR10 clip (opt-in) | HEVC Main10 or AV1 10-bit from **P010**, BT.2020 + SMPTE ST 2084 (PQ), limited range, with mastering-display and MaxCLL metadata. FFmpeg supports this for `hevc_nvenc`/`av1_nvenc` since 7.1. AMF QVBR modes are unavailable at 10-bit. | For HDR-to-HDR sharing (for example YouTube HDR). Looks wrong on SDR players that don't tone-map. |
| Default codec | **H.264.** Offer HEVC/AV1 per capability probe. | Medal also recommends H.264 as the default. AV1 encode is limited to recent GPUs. |

---

## 6. Replay-buffer design

### 6.1 How OBS's replay buffer works (verified in `plugins/obs-ffmpeg/obs-ffmpeg-mux.c`, `master`) [H]

**Structure**
- Output type `replay_buffer`.
- Encoded packets (video plus every audio track, reference-counted) are pushed into a `deque`.
- Settings `max_time_sec` and `max_size_mb`. Output-level defaults are 15 s / 500 MB; the **UI defaults are 20 s / 512 MB**.

**Trimming (`replay_buffer_purge`)**
- Purging happens while `(newest.dts_usec - oldest.dts_usec) > max_time` or `size > max_size`.
- `purge()` pops from the front. If it pops a keyframe, it keeps popping until the next video keyframe is at the front. **The buffer therefore always starts on a keyframe**, and it never purges while it holds ≤2 keyframes.
- Consequence: the saved span is ≤ `max_time` and can be up to one GOP shorter. Kairo should set its internal cap to clip length + GOP.

**Saving**
1. `save()` records `save_ts = now`.
2. When the first packet with `sys_dts_usec >= save_ts` arrives, OBS:
   - copies references to all packets;
   - rebases video timestamps to the first video packet and each audio track to its own first packet;
   - sorts by dts;
   - spawns a mux thread that pipes into the **separate `obs-ffmpeg-mux.exe` process** (libavformat);
   - emits the `saved` signal.
3. If a previous save is still muxing, the new save waits.
- OBS has **no post-hotkey ("post-roll") feature**: the clip ends at the hotkey.

**Formats**
- OBS's fragmented formats use `movflags=frag_keyframe+empty_moov+delay_moov`.
- OBS 30.2+ also has a native GPLv2 "Hybrid MP4/MOV" muxer (`plugins/obs-outputs/mp4-mux.c`, 2024).

### 6.2 RAM math

RAM ≈ (video Mbps + audio) × seconds / 8. Audio assumed: 2 AAC tracks × 192 kbps.

| Profile | Mbps | 30 s | 2 min | 5 min | Disk writes if disk-backed |
|---|---|---|---|---|---|
| 1080p60 H.264 (Kairo default) | 25 | 95 MB | 381 MB | 952 MB | 11.4 GB/h |
| 1080p60 high / 1440p60 H.264 | 40 | 151 MB | 606 MB | 1.51 GB | 18.2 GB/h |
| 1440p60 HEVC/AV1 | 25 | 95 MB | 381 MB | 952 MB | 11.4 GB/h |
| 2160p60 HEVC/AV1 | 50 | 189 MB | 756 MB | 1.89 GB | 22.7 GB/h |
| 2160p60 H.264 | 80 | 301 MB | 1.21 GB | 3.01 GB | 36.2 GB/h |

- Raw frames are not an option for buffering: 1080p NV12 is 3.1 MB per frame, so 60 fps for 30 s would be about 5.6 GB. Only encoded packets are buffered.

### 6.3 In-RAM vs on-disk buffers

| | In-RAM packet ring (OBS) | On-disk segmented (Steam; NVIDIA/AMD by reports) |
|---|---|---|
| Save latency | Instant snapshot; mux about 0.1-0.5 s for a 100-400 MB clip on NVMe [estimate] | Concat/remux of segments; similar |
| Crash safety | Lost on crash (acceptable for "the last 30 s") | Segments survive (fMP4/MPEG-TS) |
| Long buffers | RAM-bound (5 min at 50 Mbps is about 1.9 GB) | Minutes to hours (Steam's default is reported as 120 min) |
| SSD wear | None | 11-36 GB/h written continuously. Example: 3 h/day at 25 Mbps is about 12.5 TB/year. Users noticed this with ShadowPlay and moved temp files to RAM disks [M]. |
| Complexity | Low | Medium (segment rotation, cleanup, disk-full handling) |

**Recommendation:**
- **v1: RAM ring** with caps on seconds and MB, default 30 s and maximum about 2 min.
- **v1.x:** an optional disk-backed mode for long buffers. Use 2-4 s keyframe-aligned fMP4 or TS segments in a temp folder on a user-chosen drive, delete the oldest, and on save remux the needed segments.
- FFmpeg's `hybrid_fragmented` movflag (fragmented while writing, converted to a regular MP4 at the end) suits *manual* long recordings.

### 6.4 Capturing N seconds after the hotkey

- On hotkey at time T, create `SaveJob{start = keyframe ≤ T−pre, end = T+post}`.
- Pin the GOP containing `start` so trimming cannot drop it.
- Mux once a packet with dts ≥ `end` arrives.
- Size the ring for pre + post + 1 GOP.
- Queue overlapping saves instead of dropping them.
- Show a "saving in 5 s" toast.

### 6.5 A/V sync with QPC

**Common clock:** 100 ns QPC units (QPC × 10^7 / QPF).

| Source | Timestamp | Units |
|---|---|---|
| WGC frames | `SystemRelativeTime` | 100 ns QPC [H] |
| DDA frames | `DXGI_OUTDUPL_FRAME_INFO.LastPresentTime` | raw QPC ticks (FFmpeg uses it) [H] |
| WASAPI audio | `IAudioCaptureClient::GetBuffer` → `pu64QPCPosition` ("converts the counter value to 100-nanosecond units") | 100 ns [H] |

**Video**
- The pacer ticks at the target fps on QPC.
- Each tick encodes the newest captured frame with timestamp ≤ the tick; if none is newer, it re-encodes the previous one.
- pts = tick time, so video and audio share one clock.

**Audio**
- On `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`, insert silence. [H]
- Loopback streams typically deliver no packets while nothing is playing, so synthesize silence to keep the timeline continuous. [M]
- Per-game audio: process loopback (`AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`) is documented as build 20348+, but OBS enables it from 19041 because "MS says 20348, but process filtering seems to work earlier". So it should work on Win10 22H2. [H]

**At save**
- The clip starts at t0 = the first video keyframe pts.
- Drop audio before t0. Residual offset is at most one AAC frame (21.3 ms at 48 kHz) unless an edit list is used. YouTube asks for no edit lists, so accept the small offset or trim PCM before encoding.

### 6.6 Variable frame rate

- WGC/DDA deliver irregularly. WGC pacing issues are documented in OBS #13893, where OBS uses acquisition time rather than presentation time.
- MP4 *can* store VFR (per-sample durations), but editors and some players handle it poorly.
- **Emit CFR from the pacer.** Duplicate frames cost almost no bits.

### 6.7 MP4 muxing options

| Option | Notes |
|---|---|
| **libavformat** (`mov`/`mp4`) | `+faststart` = "a second pass moving the index (moov atom) to the beginning". Fragmentation via `frag_keyframe`/`empty_moov` etc. `hybrid_fragmented` for recoverable recordings. Battle-tested (OBS replay saves go through it). Rust: `ffmpeg-next` 9.0.0. **Recommended for v1.** |
| Media Foundation Sink Writer / MPEG-4 sink | Accepts compressed samples (passthrough). `MF_MPEG4SINK_MOOV_BEFORE_MDAT` (Win8+) writes moov first but "involves an additional file copying/remuxing". Missing on N editions without the Media Feature Pack. |
| Pure Rust | `muxide` 0.5.0 ("pure-Rust MP4 muxer for recording applications", updated 2026-10-19; young). `mp4-atom` 0.16.2 (box-level). `shiguredo_mp4` 2026.5.0. The `mp4` crate is stale (0.14.0, 2023-08). |
| C# | No first-party MP4 muxer. Use the MF Sink Writer via interop (ScreenRecorderLib, MIT, v7.0.1 2026-09-05, is MF-based) or FFmpeg bindings. |
| Custom writer | The replay save has all packets in hand, so a writer can emit `ftyp`→`moov`→`mdat` in **one pass** (faststart without a second pass). Needs stsz/stco/stts/ctts/stss plus avcC/hvcC/av1C/esds. A later optimisation. |

### 6.8 Save flow (v1)

1. **Hotkey.** Create a `SaveJob` (pre/post).
2. **At `end`:** under a lock, copy packet references for [keyframe ≤ start, end] across all tracks (microseconds of work).
3. **Worker thread:**
   - libavformat MP4 into a temp file;
   - `+faststart`;
   - fsync, then rename to `Kairo/Clips/<game>/<date>.mp4`;
   - extract a thumbnail (decode the first keyframe, or use the latest captured texture);
   - write metadata (game, timestamps) to the library database;
   - notify the UI.
4. **Release references.** The ring keeps running throughout; saving never stalls capture.

---

## 7. Screenshots from the same pipeline

**Source**
- On the screenshot hotkey, take the **latest captured texture at native resolution**, before any downscale.
- `CopyResource` it to a `D3D11_USAGE_STAGING` texture, `Map` it, and encode on a worker thread.
- This adds no new capture session and no extra GPU work during gameplay beyond one copy.

**Formats via WIC** (built into Windows):
- PNG (lossless; **default**), JPEG (offer q≈90-95 as a "small" option), TIFF, BMP, GIF, and **JPEG XR** (supports 16-bit float, which is what HDR needs).
- WebP and AVIF have WIC *decode* via Store extensions but **no built-in encoder** [M]. For WebP/AVIF output, bundle libwebp or libavif.
- JPEG XL arrived as a Store extension for Windows 11 24H2 (press reports). Not for v1.

**HDR screenshots**
- Capture FP16 scRGB.
- Save **.jxr** (HDR, as Game Bar does) **plus** an SDR **.png** tone-mapped with the same EETF and SDR-white-level logic as video.
- Game Bar writes a 16-bit .jxr and an 8-bit tone-mapped .png [M]. Microsoft explicitly suggests JPEG XR for HDR capture saving [H].

**Expected size**
- 1080p game PNG is roughly 2-6 MB; 4K roughly 10-25 MB [estimate].
- Encode PNG off the game's critical path. WIC PNG at 4K can take 100+ ms of CPU [estimate].

---

## 8. Recommended v1 pipeline

```
                    ┌────────────── Kairo engine (native process, user session) ──────────────┐
 Win11: WGC  ──┐    │                                                                          │
 (monitor or   │    │  Capture thread: CopyResource → "latest frame" tex (BGRA8 | FP16) + QPC  │
  game window) ├──► │                      │                         │                         │
 Win10: DDA ───┘    │   Screenshot hotkey ─┘                         ▼                         │
                    │   (staging → WIC PNG / JXR+PNG)   Pacer @60 Hz (QPC): pick newest ≤ tick │
                    │                                                │                         │
                    │                      HLSL convert/scale/tonemap → NV12 (P010 for HDR10)  │
                    │                                                │                         │
                    │      HW encoder (libavcodec d3d11 hwframes: *_nvenc / *_amf / *_qsv;     │
                    │                  MF or CPU fallback)                                     │
                    │                                                │ H.264 packets           │
 WASAPI loopback ──►│  Audio threads (float PCM + QPC) → AAC 48 kHz ─┼──► ReplayRing           │
 WASAPI mic ───────►│                                                │   (seconds+MB caps,     │
                    │                                                │    whole-GOP trim)      │
                    │  Save hotkey → SaveJob(pre, post) → snapshot refs → mux thread           │
                    │  (libavformat MP4 +faststart) → clip.mp4 + thumbnail → UI notify         │
                    └──────────────────────────────────────────────────────────────────────────┘
```

### 8.1 Components

**Capture backends behind one interface**
- `WgcSource`: Win11 default. Borderless. `MinUpdateInterval` on 26100+. Cursor toggle.
- `DdaSource`: Win10 default, and a fallback. Re-create on ACCESS_LOST. Composite the pointer. Use the iGPU on hybrid laptops.
- Both output `{ID3D11Texture2D, qpc100ns, format, isHDR}`.
- **Game-window capture** (WGC `CreateForWindow` on the foreground game) is the cleaner, more private option on Win11: no notifications or Discord pop-ups in clips. Offer it as a mode, chosen by a product decision (see questions).

**Freeze watchdog**
- If the source stops delivering new timestamps while the game is foreground, mark the clip window "possibly frozen" and tell the user to switch to borderless.

**Encoder layer**
- Probe at startup: vendor, codecs, 10-bit, B-frame capability, AV1 alignment.
- Choose libavcodec `*_nvenc` / `*_amf` / `*_qsv`. Fall back to `*_mf`, then CPU.
- Persist the capability report so beta issues can be triaged.

**ReplayRing**
- Arena allocator for packet payloads.
- Index of keyframes; caps on seconds and MB; ≥2 GOP floor; pinning for in-flight saves.

**Muxer**
- libavformat MP4 with `+faststart`. Writes the `colr` atom. AAC in esds.

### 8.2 Default settings

| Setting | v1 default | Options |
|---|---|---|
| Source | Win11: WGC monitor of the foreground game. Win10: DDA. | Game-window mode (Win11). Specific monitor. |
| Output resolution | min(native, 1080p), aspect-preserving | 720p / 1080p / 1440p / native |
| Frame rate | 60 CFR | 30 / 60 / 120 (120 needs Win11 26100+ or DDA; spike) |
| Codec | H.264 High 8-bit | HEVC / AV1 by probe |
| Bitrate | VBR 25 Mbps target / 40 Mbps peak (scale by resolution per section 5.1) | Quality presets |
| GOP | 1 s closed, IDR | 2 s |
| B-frames | 2 if supported | - |
| Look-ahead | Off | "Max quality" preset |
| Colour | BT.709 limited range. HDR displays: tone-mapped SDR. | HDR10 opt-in |
| Audio | AAC-LC 48 kHz stereo 192 kbps. Track 1 system/game, track 2 mic. | Per-app audio (process loopback) |
| Buffer | 30 s in RAM (cap 120 s) | Disk mode later |
| Post-roll | 0 s | 0-10 s |
| Container | MP4, moov first | - |
| Screenshot | PNG native resolution. HDR: JXR + PNG. | JPEG |

### 8.3 Expected cost at defaults

| Resource | Expected cost | Basis |
|---|---|---|
| RAM | about 95 MB buffer + about 50-150 MB engine | Arithmetic plus estimate; a UI process such as Electron would add more |
| VRAM | about 100-200 MB | Estimate |
| CPU | about 1-3% of an 8-core | Estimate |
| GPU 3D | well under 1 ms/frame for copy and convert at 1080p | Estimate |
| Encoder ASIC | Moderate occupancy at 1080p60 | Measure in spike |
| FPS | Target ≤3%, comparable to vendor tools (LTT Labs: NVIDIA -3 fps, AMD -5, Game Bar -1) | Measured anchor |

---

## 9. Spike plan (on the user's Windows gaming PC, about 2-3 days)

1. **Install tools.** FFmpeg 9.0.2 (BtbN LGPL build), PresentMon, GPU-Z / Task Manager "Video Encode" graph.
2. **Run each game with no capture, then `gfxcapture`, then `ddagrab`.** Use three or more games: one DX11/DX12 FSO game, one borderless, one Vulkan/OpenGL exclusive-fullscreen game. Record:
   - average FPS and 1% lows;
   - PresentMon `PresentMode` (does capture demote independent flip?);
   - CPU and encoder load.
3. **Check WGC delivery rate** with and without `MinUpdateInterval` on 60/144 Hz, plus frame-pacing histograms.
4. **Check the border and consent prompt** (unpackaged, Win11 build) and Win10 behaviour (VM or second machine).
5. **Test encoders.** For the user's GPU (AMD or Intel), run H.264/HEVC/AV1 at 1080p60 and 1440p60, look at quality, and check the RDNA3 1082-line issue if relevant.
6. **Test HDR.** Desktop HDR on: verify FP16 capture, tone-mapping, and the JXR screenshot.
7. **Test the rolling buffer** with segment ring and concat, and measure save latency.

---

## 10. Risks

- **True exclusive-fullscreen games may produce frozen clips** with WGC/DDA on 24H2+. Fixing this needs hook capture, which brings anti-cheat risk.
- **Win10 split:** a separate DDA path (no window capture without a border, hybrid-laptop iGPU constraint) costs extra engineering for about 25% of the audience.
- **WGC pacing quirks** (60 fps ceiling before 26100, irregular delivery) can make 60 fps clips judder unless the pacer is good.
- **Encoder fragmentation:**
  - AV1 only on recent GPUs.
  - AMD HEVC has no B-frames; RDNA3 AV1 codes 1080p as 1082.
  - Some GPUs have no encoder at all.
  - MF MFT behaviour drifts by vendor and driver.
- **Test coverage:** the developer has only one Windows PC (AMD *or* Intel). NVIDIA and the other vendor need beta testers or borrowed hardware, plus Win10 and Win11 machines.
- **RAM growth** with long or high-bitrate buffers, and **SSD wear** if a disk-backed buffer is added.
- **OS updates can change capture behaviour** (24H2 DDA/MPO change, later fixed). A CI-less solo project needs a "capture self-test" in the app.
- **Media Foundation pieces missing on Windows N editions** (AAC, MP4 sink, MF encoders) without the Media Feature Pack.
- **Codec patents:** AMD's SDK disclaims codec patent licences. Shipping H.264/HEVC encoding in a free app is usually fine via OS/GPU encoders, but this is unverified legally; flag for the legal/business research.
- **GPL contamination:** using libobs or x264 forces GPL. FFmpeg must be an LGPL build, dynamically linked.
- **The Mac dev machine cannot run or test any of this.** All capture work happens on the Windows PC (remote dev or Windows-side builds).

## 11. Questions for the user (product decisions this research surfaces)

1. Should Windows 10 22H2 be fully supported in v1 (requires a second capture backend, DDA), or should v1 be Windows 11-only? About 25% of Windows gamers are still on Win10.
2. Should clips capture the whole monitor (includes notifications, overlays, Discord pop-ups) or only the game window (cleaner and more private, but can miss some fullscreen modes)? Or should Kairo auto-pick?
3. What default clip length (15/30/60/120 s), and should Kairo also record a few seconds *after* the hotkey?
4. Default quality: 1080p60 for everyone, or native resolution? Do you want 120/144 fps clips?
5. Where will clips mostly be shared (Discord, WhatsApp, TikTok/Shorts, YouTube)? This decides H.264-only vs offering HEVC/AV1.
6. What exact GPU and CPU are in your PC, which Windows build, and do you use an HDR monitor?
7. Should Kairo support PCs without a hardware encoder (CPU encoding at lower quality), or declare them unsupported?
8. Which games do you play (some run true exclusive fullscreen or use strict anti-cheat)? Is "inject into the game for best capture" ever acceptable?
9. For console games: is a capture card in scope for v1? For cloud gaming: which services (they appear as normal windows, but need testing)?
10. Distribution: a plain installer (simplest for capture) or the Microsoft Store/MSIX (needs capability declarations)?
11. Licensing stance: is shipping LGPL FFmpeg DLLs acceptable? Is a GPL Kairo ever acceptable (that would allow reusing OBS code)?

---

## Sources

Microsoft (Learn, support, release health)
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.minupdateinterval
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.includesecondarywindows
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.dirtyregionmode
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.configurationiteration
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess.requestaccessasync
- https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nn-windows-graphics-capture-interop-igraphicscaptureiteminterop
- https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture (redirects to /windows/apps/develop/media-authoring-processing/screen-capture)
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_5/nf-dxgi1_5-idxgioutput5-duplicateoutput1
- https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/error-when-dda-capable-app-is-against-gpu
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model
- https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/
- https://support.microsoft.com/en-us/windows/optimizations-for-windowed-games-in-windows-11-3f006843-2c7e-4ed0-9a5e-f9389e535952
- https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range
- https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_sdr_white_level
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11videocontext-videoprocessorblt
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11_1/nf-d3d11_1-id3d11videocontext1-videoprocessorsetstreamcolorspace1
- https://learn.microsoft.com/en-us/windows/win32/medfound/hardware-mfts
- https://learn.microsoft.com/en-us/windows/win32/medfound/mf-mpeg4sink-moov-before-mdat
- https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params
- https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer
- https://learn.microsoft.com/en-us/uwp/api/windows.media.apprecording.apprecordingmanager
- https://learn.microsoft.com/en-us/windows/win32/wic/native-wic-codecs
- https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information
- https://learn.microsoft.com/en-us/windows/whats-new/extended-security-updates
- https://www.microsoft.com/en-us/windows/extended-security-updates
- https://learn.microsoft.com/en-us/answers/questions/3958070/taking-screenshoots-with-xbox-controller-on-pc-sav
- https://learn.microsoft.com/en-us/gaming/gdk/docs/tools/tools-pc/commandlinetools/gr-wdcapture

Vendor SDKs and docs
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF (releases; amf/doc/AMF_Video_Encode_API.md, AMF_Video_Encode_HEVC_API.md, AMF_Video_Encode_AV1_API.md, AMF_Display_Capture_API.md)
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/GPU-and-APU-HW-Features-and-Support
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/Rate-Control-Methods
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/AV1-Encoder
- https://github.com/intel/media-driver (README codec/platform table)
- https://github.com/intel/libvpl (and api/vpl/mfxmemory.h)
- https://github.com/intel/vpl-gpu-rt
- https://developer.nvidia.com/video-encode-decode-support-matrix
- https://docs.nvidia.com/video-technologies/video-codec-sdk/index.html
- https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html
- https://developer.nvidia.com/capture-sdk
- https://github.com/microsoft/DirectX-Specs/tree/master/d3d (D3D12 video encoding specs)

FFmpeg
- https://ffmpeg.org/index.html#news
- https://ffmpeg.org/download.html
- https://github.com/FFmpeg/FFmpeg/blob/master/Changelog
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/doc/filters.texi (ddagrab, gfxcapture)
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/doc/muxers.texi (mp4 faststart, fragmentation, hybrid_fragmented, segment)
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavfilter/vsrc_gfxcapture_winrt.cpp
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavfilter/vsrc_ddagrab.c
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavfilter/vf_scale_d3d11.c
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/amfenc.c
- https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavdevice/alldevices.c
- https://github.com/BtbN/FFmpeg-Builds/issues/549

OBS Studio (source and project)
- https://github.com/obsproject/obs-studio/blob/master/libobs-winrt/winrt-capture.cpp
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/duplicator-monitor-capture.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/window-capture.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/game-capture.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/plugin-main.c
- https://github.com/obsproject/obs-studio/blob/master/shared/obs-inject-library/inject-library.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/obs-ffmpeg/obs-ffmpeg-mux.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/obs-outputs/mp4-output.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/plugin-main.cpp
- https://github.com/obsproject/obs-studio/blob/master/frontend/widgets/OBSBasic.cpp
- https://github.com/obsproject/obs-studio/blob/master/frontend/utility/SimpleOutput.cpp
- https://github.com/obsproject/obs-studio/blob/master/libobs/data/color.effect
- https://github.com/obsproject/obs-studio/discussions/9590
- https://github.com/obsproject/obs-studio/pull/12759
- https://github.com/obsproject/obs-studio/issues/13893
- https://github.com/obsproject/obs-studio/issues/14021
- https://github.com/obsproject/obs-studio/releases
- https://obsproject.com/kb/window-capture-sources (via archive; live site returned 502 on 2026-10-21)

Other projects and reports
- https://github.com/robmikh/Win32CaptureSample
- https://github.com/robmikh/Win32CaptureSample/issues/82
- https://github.com/robmikh/Win32CaptureSample/issues/83
- https://github.com/ClassicOldSong/Apollo/issues/676
- https://github.com/ClassicOldSong/Apollo/pull/785
- https://github.com/NiiightmareXD/windows-capture (and issue #190)
- https://github.com/mmozeiko/wcap
- https://handmade.network/forums/t/8701-wcap/2
- https://github.com/LizardByte/Sunshine/blob/master/docs/configuration.md
- https://github.com/LizardByte/Sunshine/blob/master/src/video.cpp
- https://github.com/chromium/chromium/blob/main/media/gpu/windows/media_foundation_video_encode_accelerator_win.cc
- https://github.com/chromium/chromium/blob/main/media/gpu/windows/mf_video_encoder_shared_state.cc
- https://community.intel.com/t5/Graphics/AV1-encoding-not-available-Poor-video-encoding/m-p/1754079
- https://github.com/sskodje/ScreenRecorderLib
- https://crates.io/crates/windows-capture ; https://crates.io/crates/muxide ; https://crates.io/crates/mp4-atom ; https://crates.io/crates/shiguredo_mp4 ; https://crates.io/crates/mp4 ; https://crates.io/crates/ffmpeg-next ; https://crates.io/crates/windows
- https://gigazine.net/gsc_news/en/20260826-pc-screen-recorder/ (LTT Labs results)
- https://store.steampowered.com/hwsurvey/Steam-Hardware-Software-Survey-Welcome-to-Steam
- https://store.steampowered.com/gamerecording
- https://support.medal.tv/support/solutions/articles/48001159618-video-quality-settings-guide
- https://support.medal.tv/support/solutions/articles/48001272506-why-your-amd-gpu-isn-t-showing-up
- https://support.google.com/youtube/answer/1722171
- https://www.sweclockers.com/forum/post/20133225 (ShadowPlay temp files / RAM-disk community thread)
