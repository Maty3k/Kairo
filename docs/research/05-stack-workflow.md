# Kairo research 05: Desktop stack and developer workflow

*Researched 2026-10-21 for Kairo, a Windows-only instant-replay clipper. Developer profile: solo PHP/Laravel + JS developer on an Apple M2 Mac with 8 GB RAM, plus a Windows gaming PC (AMD or Intel GPU) used for all capture testing. Claude Code writes much of the code.*

**Confidence tags used below**
- **[H]**: verified today against a primary source (vendor docs, release page, source code, package registry).
- **[M]**: from a secondary source, or reasoned from primary facts.
- **[L]**: an estimate, or something I could not verify. Measure it before you rely on it.

My web-search budget ran out partway through. After that I used direct fetches of known URLs, package-registry APIs and source files on GitHub. Section 8 lists the items left unverified.

---

## 0. Verdict

| Rank | Stack | One-line reason |
|---|---|---|
| **#1** | **Tauri 2 UI (TypeScript) + a standalone, always-on Rust engine.** They talk JSON-RPC over a per-user named pipe. The engine uses WGC/DXGI capture, FFmpeg (LGPL DLLs) for AMF/QSV/NVENC encoding, and WASAPI for audio. | Smallest footprint while gaming. Rust is the only native language in both the engine and the UI shell. Mature plugins exist for tray, hotkeys, autostart, updater and single instance. The compiler catches many mistakes in AI-written code. There is a path to a macOS port. |
| **#2** | **Same Tauri/TypeScript UI + a C#/.NET 10 engine** compiled with NativeAOT. It uses CsWinRT for WGC, Vortice for D3D11/MF, NAudio 3 for WASAPI and process loopback, FFmpeg.AutoGen 9 for encoding, and StreamJsonRpc for IPC. | C# is much easier than Rust for a PHP developer to read and own. The libraries are excellent and current (all updated in 2026). Choose this if the Rust spike proves painful. Nothing in the UI changes. |
| **#3 (conditional)** | **A libobs-based engine**, run as a separate process, behind the same UI. | This is the fastest route to battle-tested multi-vendor capture, encoding and a replay buffer, and it is how Streamlabs and Overwolf/Outplayed record. It only makes sense if you accept GPL obligations, in practice open-sourcing at least the engine. |
| Avoid | NativePHP Desktop, Electron for any always-on part, C++/Qt or Win32 for this developer, WPF/WinUI 3 as the main UI. | See sections 2c, 2e and 3. In short: extra runtimes and RAM, no native engine solved, a steep learning curve, or no way to develop the UI on the Mac. |

**The key structural insight:** put a named-pipe JSON-RPC contract between the engine and the UI. That makes the **UI decision** (Tauri + TypeScript, decide now) independent of the **engine-language decision**. Make the engine decision after a 1–2 week spike on the gaming PC: build it in Rust first, and fall back to C# if needed.

---

## 1. Facts that constrain every stack

### 1.1 Windows API floors (these matter more than the framework)

| Fact | Detail | Tag |
|---|---|---|
| Hiding the cursor in WGC | `GraphicsCaptureSession.IsCursorCaptureEnabled` was introduced in **10.0.19041** (Windows 10 2004). | [H] MS Learn |
| Removing the yellow WGC capture border | `IsBorderRequired` was introduced in **10.0.20348** ("Windows 10, version 2104", UniversalApiContract v12). You must call `GraphicsCaptureAccess.RequestAccessAsync(GraphicsCaptureAccessKind.Borderless)` first; if the user denies it, the border stays. OBS, an unpackaged Win32 app, does exactly this, gated by `ApiInformation.IsPropertyPresent`. | [H] MS Learn, OBS `libobs-winrt/winrt-capture.cpp` |
| What that means on Windows 10 22H2 | Windows 10 22H2 is build 19045, below 20348, so **WGC captures on Windows 10 always show the yellow border**. For full-monitor capture on Windows 10, use **DXGI Desktop Duplication**, which draws no border. | [M] (from the build numbers) |
| WGC frame pacing and dirty regions | `MinUpdateInterval` and `DirtyRegionMode` exist only from **build 26100** (Windows 11 24H2). | [H] MS Learn monikers |
| Per-process ("app audio") loopback | Microsoft documents `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` as requiring build **20348**. OBS registers its Application Audio Capture source from **19041**, with the code comment *"MS says 20348, but process filtering seems to work earlier"*. That means it works on Windows 10 22H2 in practice. | [H] MS sample (2026-02-19), OBS `plugins/win-wasapi/plugin-main.cpp` |
| Services can't capture | Services run in isolated, non-interactive **Session 0**. The engine must therefore be a **per-user process in the interactive session**, not a Windows service. | [H/M] Microsoft Session 0 isolation docs |
| WebView2 availability | The Evergreen WebView2 runtime is part of Windows 11. "The vast majority of Windows 10 devices" already have it, but you should still check for it. **Edge and WebView2 will keep updating on Windows 10 22H2 until at least October 2028.** | [H] MS Learn (2026-08-31, 2026-10-02) |
| Windows 10 lifecycle | General support ended 2025-10-14. Microsoft's consumer ESU page (fetched today) says the consumer ESU programme runs until **2027-10-12**. | [H] |

### 1.2 What a replay buffer architecturally needs

- **OBS's replay buffer is the reference design.** It keeps **encoded packets** in an in-memory deque, bounded by `max_time_sec` and `max_size_mb`. It purges old packets but always keeps at least 2 keyframes. On save, it spawns a **separate process, `obs-ffmpeg-mux.exe`**, and pipes the packets to it for muxing. [H] (`plugins/obs-ffmpeg/obs-ffmpeg-mux.c`)
  - **So Kairo's engine needs packet-level encoder output.** That means FFmpeg `libavcodec`, Media Foundation encoder MFTs (`IMFTransform`), or the vendor SDKs.
  - High-level "record to file" APIs are not enough. That includes WinRT `MediaTranscoder`, which the `windows-capture` crate uses (section 2a).
- **RAM cost of an in-memory buffer** (simple arithmetic):
  - 30 s at 50 Mbit/s ≈ 188 MB.
  - 60 s at 25 Mbit/s ≈ 188 MB.
  - 120 s at 50 Mbit/s ≈ 750 MB.
  - A disk-backed ring saves RAM but writes continuously: 50 Mbit/s ≈ 22.5 GB per hour of play.
  - Picking between these is a product decision, not a stack decision.
- **Zero-copy GPU path.** FFmpeg's `h264_amf` and `h264_nvenc` accept D3D11 hardware frames directly (QSV needs an extra hwmap step). Do BGRA→NV12 conversion on the GPU, with the D3D11 VideoProcessor or FFmpeg's `scale_d3d11` (added in FFmpeg 8.0). [H]/[M]
- **A prototype is possible on day 1 with no code.** FFmpeg **8.1** (2026-03-16) added **`gfxcapture`**, a Windows.Graphics.Capture source that outputs `d3d11` hardware frames. Its documentation includes examples feeding `hevc_nvenc` and `h264_amf` directly. FFmpeg **9.0 "Lei"** is current (9.0.2 shipped 2026-09-18). It extends AMF (frame-rate converter, HDR VPP, "AMF hardware memory mapping") and removes NVENC support for SDKs older than 11.1. [H] (FFmpeg Changelog, `filters.texi`, download page)

---

## 2. Stack-by-stack evaluation

### 2a. Tauri 2 (Rust + WebView2)

**Status (2026-10-21)**

| Item | Version / fact | Tag |
|---|---|---|
| Tauri | **2.12.4** (2026-10-14); 2.12.0 shipped 2026-09-26. MSRV is now Rust **1.90**, and Windows 7 was dropped. 2.12 also made the `asset://` protocol async and fixed multi-range responses, which helps `<video>` seeking of local clips. | [H] crates.io, tauri.app/release |
| Tauri 3 | **3.0.0-alpha.5** (2026-10-14), including an optional **`tauri-runtime-cef`** (Chromium Embedded) alpha. Expect a major-version migration during 2027. Stay on 2.x for now. | [H] GitHub releases |
| Tray | Built into core via `tray-icon` (crate 0.26.1, 2026-10-07). | [H] |
| `tauri-plugin-global-shortcut` | 2.4.1. Uses `global-hotkey` 0.8, which calls Win32 **`RegisterHotKey`**. | [H] (source) |
| `tauri-plugin-autostart` | 2.7.0. Uses `auto-launch` 0.6, which writes **`HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`** and honours `StartupApproved\Run`. | [H] (source) |
| `tauri-plugin-updater` | 2.14.0. Updates must be signed with a Tauri key pair; signing can't be disabled. Works with NSIS and MSI artifacts. Install modes are `passive`, `basicUi` and `quiet`. Supports a static JSON manifest (e.g. on GitHub Releases) or a dynamic server. | [H] |
| `tauri-plugin-single-instance` | 2.5.4. Must be registered first. The callback receives `args` and `cwd`. | [H] |
| Sidecars | `bundle.externalBin`, with a `-<target-triple>` filename suffix. Spawned through `tauri-plugin-shell` (2.5.0), which emits stdout events and can write to stdin. | [H] |
| Windows installer | NSIS installs per-user into `%LOCALAPPDATA%` by default, with no admin needed. It supports **installer hooks** (`NSIS_HOOK_PREINSTALL` and friends), which you can use to stop and restart the engine during updates. MSI builds need Windows. | [H] |
| WebView2 bootstrapping cost | `downloadBootstrapper` adds 0 MB (default); `embedBootstrapper` about 1.8 MB; `offlineInstaller` about 127 MB; `fixedVersion` about 180 MB. Microsoft's own docs say the Fixed Version binaries are over 250 MB. | [H] |

**Rust capture, encode and audio libraries**

| Need | Crate (version, date) | Assessment | Tag |
|---|---|---|---|
| Win32/WinRT bindings | `windows` **0.62.2** (2025-10-06). The windows-rs **0.100.0** family shipped 2026-09-03. | **Churn risk.** 0.100 moves native APIs to new header-derived metadata. Signatures change (raw `HRESULT`, pointer/count pairs, header-named modules), the MSRV for the new crates is 1.95, and the `windows` umbrella crate may be republished later. Pin 0.62 for now and budget a migration. AI models know the older APIs best. | [H] crates.io, windows-rs newsletter #4867 |
| WGC + DXGI capture | `windows-capture` **2.0.1** (2026-08-08). MIT, about 2.19M downloads. | Covers WGC (cursor, border, secondary windows, `MinUpdateInterval`, dirty regions) and, new in 2.x, DXGI Desktop Duplication. Its encoder wraps **WinRT `MediaTranscoder`** fed with D3D11 surfaces (H.264, HEVC, VP9 …) and writes to a file or stream. It exposes **no packet access**, so use it for capture and screenshots and do the encoding yourself. | [H] (source) |
| Encode, packet level | `ffmpeg-next` **9.0.0** (2026-08-05, WTFPL); `rsmpeg` 0.18.0+ffmpeg.8.0 (2025-08-24, MIT); `ffmpeg-sidecar` 2.7.0 (CLI wrapper). | ffmpeg-next is self-described as "**maintenance mode**". Its README still says it supports FFmpeg 3.4–8.0, and 9.0.0 landed the day after FFmpeg 9 shipped [M]. rsmpeg lags one FFmpeg major. ffmpeg-sidecar pipes the CLI, so it can't do zero-copy D3D11 frames and is fine only for prototypes. You could also call Media Foundation MFTs directly through `windows`: no extra DLLs, but verbose COM code. | [H] |
| Audio | `wasapi` **0.26.0** (2026-10-11, MIT). | Includes `AudioClient::new_application_loopback_client(pid, include_tree)`, so per-game audio is covered, plus normal loopback and microphone capture. | [H] (source example) |
| IPC | `interprocess` **2.4.6** (2026-10-16). | "Local sockets" are named pipes on Windows and Unix domain sockets on macOS, with Tokio support. The same code runs a mock engine on the Mac. | [H] |
| Other | `velopack` 1.2.161, `sentry` 0.49.3, `ts-rs` 12.0.1 (generates TypeScript types from Rust structs). | | [H] |

**Assessment**

- **Idle cost while gaming.**
  - The engine is a native process with no runtime. Expect roughly **15–40 MB plus the replay buffer** [L].
  - **On Windows, WebView2 *is* Chromium**, so an open Tauri window costs about as much as Electron. In a CI benchmark of an empty app on Windows x64, Tauri used about 307 MB and Electron about 279 MB, measured as the median sum of the app's process tree in a single run [M]. Treat these as order-of-magnitude figures only.
  - The real win is that the UI process can be **completely closed** while gaming.
- **Installer size.**
  - An empty Tauri build is about 3 MB, versus about 385 MB for Electron in the same benchmark [M].
  - The total depends on FFmpeg: a trimmed LGPL build with only the needed encoders and muxers might be roughly 15–60 MB [L].
- **Learning curve.** The UI side is easy (TypeScript, Vite, Vue or React). The Rust engine is the hardest of the options for a PHP developer: ownership, `unsafe` COM interop and async.
- **Suitability for AI-assisted work.**
  - Good. When Rust compiles, memory and thread safety are largely guaranteed, which matters when the human can't review deeply.
  - `cargo xwin check` lets Claude Code compile-check Windows-only code on the Mac.
  - The main risk is version churn in windows-rs.
- **Maintainability.** Good. One native language covers the engine and the UI shell, and the UI is web code.
- **macOS later.** The UI and shell port as-is. Only the engine's capture and encode layer needs rewriting (ScreenCaptureKit, plus VideoToolbox through FFmpeg).

### 2b. C#/.NET

**Platform status**

| Item | Status | Tag |
|---|---|---|
| .NET 10 | **LTS**, released 2025-11-11, supported until **2028-11-14**. .NET 11 RC2 shipped 2026-10-13; GA is expected at .NET Conf, 10–13 Nov 2026. | [H] |
| WPF | Mature, but **not trim- or NativeAOT-compatible**: "trimming support for WPF is currently disabled in the .NET SDK". A community fork (`lezzi/wpfaot`) is only a proof of concept. | [H] MS Learn |
| WinUI 3 (Windows App SDK) | **2.0** went GA on 2026-04-29; **2.5.1** shipped 2026-09-16. 1.8 left servicing on 2026-09-24. Native AOT has been supported since 1.6. It is backward-compatible to Windows 10 1809. | [H] |
| Avalonia | **12.1.4** (2026-10-09); 12.0 shipped 2026-04-07. Cross-platform, runs on the Mac, AOT-capable. | [H] NuGet |
| Building on the Mac | `EnableWindowsTargeting=true` lets you compile `net*-windows`, WPF and WinForms projects on macOS **but not run them**. I expect WinUI 3 still needs Windows to build because of its XAML compiler. | [H] / [L] for WinUI |

**Libraries**

| Need | Package (version, date) | Notes | Tag |
|---|---|---|---|
| WGC / WinRT | `Microsoft.Windows.CsWinRT` **2.3.1** (2026-07-22), or just a `net10.0-windows10.0.19041.0` TFM. | Projections have been AOT-compatible since CsWinRT 2.1.1. | [H] |
| Win32 P/Invoke | `Microsoft.Windows.CsWin32` 0.3.366 (2026-10-19). | Source-generated: `RegisterHotKey`, pipes, job objects and so on. | [H] |
| D3D11, DXGI, MF | `Vortice.Direct3D11` / `Vortice.MediaFoundation` **3.8.3** (2026-03-04). | Thin, current bindings. | [H] |
| Audio | **NAudio 3.3.0** (2026-10-19; 3.0 shipped 2026-08-15). Requires net9.0+. | New `WasapiRecorder` handles "system audio and **per-process loopback**". The README says the core is "Native-AOT compatible". | [H] |
| Encode | `FFmpeg.AutoGen` **9.0.1.1** (2026-08-16). | Bindings for FFmpeg 9. | [H] |
| All-in-one recorder | `ScreenRecorderLib` **7.0.1** (2026-09-05, MIT). | **C++/CLI** (`CLRSupport=true`), so no trimming or NativeAOT. Encodes H.264/H.265 through Media Foundation. `RecorderApi` is DesktopDuplication or WindowsGraphicsCapture. Needs the VC++ redistributable. **No replay-buffer feature** and a single maintainer. Fine for experiments, not as Kairo's core. | [H] (source) |
| IPC | `StreamJsonRpc` 2.26.150 (2026-10-19, Microsoft). | JSON-RPC over named pipes. | [H] |
| Tray, installer, updates | `H.NotifyIcon` 2.4.1; `Velopack` 1.2.161. | | [H] |

**Assessment**

- **Idle cost.** A NativeAOT engine of perhaps **25–60 MB** [L], with no runtime to install. A GC-free hot path is easy because frames are GPU textures and packets are small.
- **Learning curve.** Moderate. Classes, interfaces, attributes and DI feel like Laravel; async/await feels like JavaScript.
- **Suitability for AI-assisted work.** Excellent. C# is heavily represented in training data, and the typing is strong.
- **Main costs.**
  - A second native language if the UI is Tauri (Rust glue plus C#).
  - A WPF or WinUI UI can't run on the Mac.
  - macOS UI portability requires Avalonia.

### 2c. Electron, or NativePHP Desktop (Laravel inside Electron), plus a native engine sidecar

**Electron**

- Latest stable is **45.0.0**, released 2026-10-20 (Chromium M156, Node 24.22.0, end of life 2027-04-27).
- Releases arrive about every 8 weeks, and only the latest 3 majors are supported. That is a constant upgrade treadmill. [H]
- The empty-app benchmark (Windows x64): about 385 MB on disk and about 279 MB RAM in the measured process tree. [M]
- Node can open named pipes natively, and `electron-updater` is mature. Electron is fine for a UI that you *close while gaming*, but it is wasteful for anything always-on.

**NativePHP Desktop, 2026 status**

| Item | Finding | Tag |
|---|---|---|
| Versions | **2.3.1** (2026-09-14); 2.3.0 (2026-09-05); 2.2.1 (2026-05-29); 2.2.0 (2026-04-29, Laravel 13); 2.1.0 (2026-01-17); **2.0.0 (2025-10-14)**. 2.0.0 moved to **Electron 38**, set `nodeIntegration` off and `contextIsolation` on by default, and merged the repos into `nativephp/desktop`. | [H] |
| Runtime | **Electron only.** The current v1 and v2 docs mention no Tauri runtime. A third-party article claiming Tauri support looks outdated or wrong. | [H] |
| Requirements | PHP 8.3+, Laravel 11+, Node 22+, Windows 10+. Bundled PHP is built with static-php-cli; php-bin 1.2.0 (2026-05-21) ships 8.3, 8.4 and 8.5. | [H] |
| Architecture | Electron main process, a PHP server (`artisan serve`-style) on 127.0.0.1 with a secret header, and an internal "Electron API server". 2.2.1 was a **hotfix binding that API server to loopback**. Optional queue workers and a scheduler. So there are always several processes plus local HTTP listeners. | [H] release notes / [M] details |
| Extra binaries | Put them in the project's `extras/` folder and read them through the read-only `extras` disk (`Storage::disk('extras')->path(...)`). Launch with `ChildProcess::start(cmd:, alias:, persistent: true)`, which restarts on crash. Stdout and stderr arrive as Laravel events. The reported pid is the Electron helper's, not the child's. | [H] |
| Global shortcuts / tray / updater | `GlobalShortcut::key(...)->event(...)->register()`; a `MenuBar` facade for the tray; an `AutoUpdater` supporting GitHub, S3, DigitalOcean Spaces and (since 2.1.0) R2, active only in production builds. Supports Azure Trusted (Artifact) Signing. Cross-building Windows from a Mac is "possible", with caveats. | [H] |
| Size and idle RAM | No published measurements. Expect at least Electron's footprint plus the PHP binary and `vendor/`, likely **150–300 MB on disk and 200–400 MB RAM with the UI open** [L]. | [L] |

**Verdict.** NativePHP lets you reuse Laravel skills for the *library UI* only. It still needs a native engine in Rust, C# or C++. It adds a third runtime (PHP) and local HTTP servers to a gaming-time tool, and it has a small community. I don't recommend it for Kairo, even for the beta. The one acceptable use is a throwaway clickable prototype of the library or editor.

### 2d. libobs-based engine

**What you get**

| Item | Detail | Tag |
|---|---|---|
| Release | OBS **32.2.2** (2026-08-14). | [H] |
| Capture | Game capture via an injected hook, WGC, DXGI and window capture. | [H] |
| Encoders | Native AMF, QSV and NVENC encoders. | [M] |
| Audio | Application audio capture, from build 19041. | [H] |
| Replay buffer | A ready-made replay-buffer output with a separate muxer process (section 1.2). | [H] |

**License**

- libobs is **GPL-2.0-or-later** (`libobs/obs.h` header). [H]
- If you link libobs, the program you distribute must be GPL, with source offered to recipients. The OBS forum has stated this position [M].
- The FSF's view is that pipes, sockets and command-line arguments *normally* indicate separate programs, but "exchanging complex internal data structures" can make them one combined work. [H] GPL FAQ #MereAggregation
- So a GPL engine process plus a proprietary UI over JSON-RPC is plausible but legally grey. Get advice before relying on it.

**Who uses it**

| Product | Detail | Tag |
|---|---|---|
| Streamlabs Desktop | **GPL-3.0**; Electron + **obs-studio-node**. | [H] |
| obs-studio-node | **GPL-2.0**, Windows and macOS only. Split into `obs-studio-client` and `obs-studio-server` using `lib-streamlabs-ipc`, i.e. libobs runs in a separate server process. | [H] (repo layout) / [M] (process model) |
| Overwolf | Its developer docs say OBS is **"the base of all recording apps on Overwolf"** and its log files are named `ow_obs_*`. Overwolf's blog describes an `ow-obs.exe` engine process. | [H] / [M] |
| Outplayed | An Overwolf app, so it inherits Overwolf's OBS-based recorder. | [M] |
| Insights Capture | A community article says it relied on OBS core supplied through Overwolf. | [L] |
| Rust bindings | `libobs` 5.0.1 / `libobs-wrapper` 9.0.4: GPL-3.0, about 10k downloads, last updated January 2026. Small, but it exists. | [H] |

**Costs**

- CMake + Visual Studio builds of OBS and its dependencies.
- A large binary set, perhaps 100–250 MB [L].
- C API version churn across OBS releases.
- If you use game-capture **DLL injection**, possible friction with anti-cheat. Use WGC and DXGI only at first.

**Verdict.** This is the strongest engine on day one, but GPL is a product decision for you to make, not a technical one.

### 2e. C++ with Qt or Win32

- **Pros.** Maximum control and performance. Every Microsoft sample and the whole OBS source tree are directly reusable references.
- **Cons for this developer.**
  - The steepest learning curve.
  - AI-written C++ compiles easily but can hide undefined behaviour, leaks and races that a PHP developer can't spot in review.
  - Qt adds its own framework and licensing: LGPLv3 with dynamic linking, or a commercial or Small Business subscription. I could not read the prices from qt.io today [L].
  - Win32 UI is dated.
- **Verdict.** Only sensible as "C++ engine + web UI" if you later hire a C++ specialist. Not recommended now.

---

## 3. Comparison matrix

Numbers marked [L] are order-of-magnitude estimates. Measure them in the week 1 spike (section 7.2).

| Criterion | Tauri + Rust engine | Tauri UI + C# NativeAOT engine | Electron/NativePHP UI + native engine | libobs engine | C++ / Qt |
|---|---|---|---|---|---|
| Always-on RAM while gaming, excluding the replay buffer and with the UI closed | **~15–40 MB** [L] | ~25–60 MB [L] | Same as the chosen engine, but only if the UI really exits | ~100–250 MB [L]: libobs keeps a D3D11 render pipeline | ~10–30 MB [L] |
| RAM with the UI open | +100–300 MB (WebView2 is Chromium) [M] | +100–300 MB (WebView2) [M] | Electron ~150–300 MB; NativePHP ~200–400 MB [L] | Depends on UI | Qt UI ~40–100 MB [L] |
| Idle CPU | ~0%. Capture and encode CPU depends on the zero-copy design, not the language | ~0%; GC negligible if the hot path avoids allocation | ~0% when idle | Low; OBS is tuned | ~0% |
| Installer | Small (Tauri ~3 MB empty) + FFmpeg DLLs [L] | NativeAOT engine ~5–20 MB [L] + FFmpeg | ~100–150 MB compressed / ~385 MB unpacked (Electron), more for NativePHP | Large | Qt DLLs + FFmpeg |
| Learning curve for PHP/JS | UI easy; **engine hard** | UI easy; engine **moderate** | UI easiest; engine still native | Hard (C/CMake) | Hardest |
| AI-assisted fit | Good: compiler guards; watch windows-rs churn | **Very good** | Good for UI; NativePHP niche | Medium | Medium–low (UB risk) |
| Library maturity | Medium: official bindings in migration; community crates active | Medium–high: CsWinRT, Vortice, NAudio 3, FFmpeg.AutoGen 9 | n/a (needs an engine) | **Very high** (OBS) | High (direct APIs) |
| Licensing | MIT/Apache crates + FFmpeg LGPL | MIT packages + FFmpeg LGPL | MIT + FFmpeg LGPL | **GPL** | LGPL Qt or commercial |
| Maintainability, solo | Good | Good | Two or three runtimes | Fork tracking | Hard |
| macOS port later | **Good** (UI as-is) | UI only with Avalonia or Tauri | Good | Good (OBS is cross-platform) | Good with Qt |

---

## 4. Licensing checklist

| Component | Obligation | Tag |
|---|---|---|
| **FFmpeg**, LGPL route | Don't configure with `--enable-gpl` or `--enable-nonfree`. **Link dynamically (DLLs)**. Ship the matching source and your configure line. Credit FFmpeg in the About box and EULA. Don't forbid reverse engineering. Don't obfuscate DLL names. Avoid GPL libraries such as libx264/libx265. **BtbN publishes `lgpl` variants** (no x264/x265), whose minimum supported OS is Windows 10 22H2. | [H] ffmpeg.org/legal, BtbN README |
| Codec patents | FFmpeg says commercial products may face pool licensing fees. **AMD's AMF SDK licence explicitly states that AMD grants no licence to codec IP (AVC/H.264, HEVC, AAC …) and "you will pay any royalties due".** A free beta is low-risk, but get legal review before you monetise, and decide on HEVC/AV1 defaults deliberately. | [H] AMF LICENSE.txt |
| AMD AMF SDK | MIT; v1.5.3 shipped 2026-09-29. The runtime ships in the driver. | [H] |
| Intel VPL (oneVPL) | `libvpl` is MIT (v2.17.0, 2026-06-25); the GPU runtime `vpl-gpu-rt` is MIT. | [H] |
| NVIDIA NVENC | `nvEncodeAPI.h` in nv-codec-headers is MIT. **n13.1.15.0 (2026-07-14) requires NVIDIA driver ≥ 610.0 on Windows.** Build FFmpeg against headers that match the oldest driver you support. | [H] |
| libobs | GPL-2.0-or-later (section 2d). | [H] |
| WebView2 Runtime | Evergreen, serviced by Microsoft. Long-running apps keep using the old runtime until restarted, which is another reason to keep the UI process short-lived. | [H] |

---

## 5. Engine ↔ UI architecture

### 5.1 Process model (recommended)

```
login ──HKCU\...\Run──► kairo-engine.exe   (per-user, interactive session, no webview)
                         ├─ tray (tray-icon) + hotkeys (RegisterHotKey via global-hotkey)
                         ├─ capture (WGC / DXGI dup) → GPU convert → HW encode (AMF/QSV/NVENC)
                         ├─ encoded-packet ring buffer (+ audio: WASAPI loopback / process loopback / mic)
                         ├─ on hotkey: mux last N s → MP4 (in-proc libavformat or helper process)
                         └─ named pipe server \\.\pipe\kairo-<user-SID>  (JSON-RPC 2.0)
user opens library ───► Kairo.exe (Tauri UI) ── connects to pipe ── exits completely when closed
```

- **Why the engine owns the tray and hotkeys.** It must work with zero UI processes, so nothing Chromium-based runs during gameplay. Kairo can then advertise near-zero overhead, a common complaint about Medal and Overwolf-style clippers [M].
- **Alternative B: the Tauri app itself is the always-on tray process.** Webviews are created on demand and the engine runs as a supervised sidecar. This reuses Tauri's plugins directly. Before choosing it, **measure** whether `msedgewebview2.exe` processes fully exit after the last window is destroyed. I couldn't verify this [L].

### 5.2 IPC options

| Option | Verdict |
|---|---|
| **Named pipe + JSON-RPC 2.0 (length-prefixed or Content-Length framing)** | **Recommended.** Native to Windows, no TCP port, and ACL-controllable. Call `CreateNamedPipe` with an **explicit DACL** for the current user, because the default DACL grants *read* to Everyone and anonymous. Use `PIPE_REJECT_REMOTE_CLIENTS`. `FILE_FLAG_FIRST_PIPE_INSTANCE` doubles as a single-instance guard. Libraries: Rust `interprocess`/Tokio; C# `System.IO.Pipes` + StreamJsonRpc; Node `net.connect('\\\\.\\pipe\\…')`. A webview can't open pipes itself, so the Tauri Rust core proxies via `invoke` and events. [H] MS Learn |
| Local WebSocket/HTTP on 127.0.0.1 | Convenient for browser-based UI development on the Mac. In production it needs a random port, a per-session token and an Origin check against CSRF and DNS rebinding from web pages. NativePHP's 2026 loopback hotfix shows how easy this is to get wrong. Use it only in dev. |
| stdin/stdout JSON-RPC | Good for parent→child supervision (e.g. engine → mux helper, or the Tauri sidecar in alternative B). It can't serve a UI that starts and stops independently of an always-on engine. |
| gRPC | Overkill for one machine and one client. It adds a protobuf toolchain. Skip it for v1. |

**Contract-first.** Define the RPC types once in Rust and generate TypeScript with `ts-rs` (or `specta`). The events Kairo needs (`bufferStatus`, `clipSaved`, `error`) map naturally onto JSON-RPC notifications.

### 5.3 Service vs background process, crash isolation, autostart, single instance, updates

- **Windows service: no.** Session 0 can't capture the desktop or receive hotkeys. Consider a service only for an optional elevated helper later, if ever.
- **Elevation.** Run unelevated. Hotkeys, capture and process loopback against **elevated or anti-cheat-protected games** must be tested in the spike [L].
- **Crash isolation.**
  - The UI and engine are separate processes.
  - Optionally move muxing to a helper process, as OBS does.
  - Put helpers in a **Job Object** with kill-on-close.
  - A watchdog restarts the engine (the UI if open, or a tiny scheduled task).
  - Collect minidumps (Sentry has Rust and .NET SDKs).
  - Add a "safe mode": after N crashes at startup, fall back to a different encoder or capture API.
- **Autostart.** `HKCU\...\Run` (what `auto-launch` does) respects the Task Manager / Settings "Startup apps" toggle via `StartupApproved\Run`.
- **Single instance.**
  - Engine: a named mutex in the `Local\` namespace, or the first-pipe-instance flag.
  - UI: `tauri-plugin-single-instance`, which focuses the existing window and forwards args such as a deep link to a clip.
- **Hotkeys.** `RegisterHotKey` fails if another app already owns the combination. GPU vendor overlays reserve their own. F12 is reserved and Win-key combinations belong to the OS. Use `MOD_NOREPEAT` [H]. Show the conflict in the UI and offer alternatives. Gamepad combos need XInput/GameInput polling, which `RegisterHotKey` does not cover.
- **Updates.** The Tauri updater runs in the UI. Its NSIS `NSIS_HOOK_PREINSTALL` hook asks the engine to exit (or kills it), and `NSIS_HOOK_POSTINSTALL` restarts it. Velopack is the alternative for a C# engine.

---

## 6. Developer workflow: M2 Mac (8 GB) + Windows gaming PC

### 6.1 Make the Windows PC the build and test machine

| Topic | Details | Tag |
|---|---|---|
| Tools to install | Git for Windows, Visual Studio Build Tools (MSVC + Windows SDK), rustup (`x86_64-pc-windows-msvc`), Node 22 + pnpm, FFmpeg 9 LGPL *shared* dev build (BtbN). For #2, also the .NET 10 SDK. | |
| Claude Code on Windows | Supports Windows 10 1809+ with 4 GB+ RAM, x64 or ARM64. Install with `irm https://claude.ai/install.ps1 \| iex` or WinGet. Git for Windows is *optional*: with it Claude Code uses Git Bash for its Bash tool, otherwise PowerShell. WSL2 adds sandboxing, but the engine needs the native Windows toolchain. | [H] code.claude.com |
| Remote access from the Mac | Enable the OpenSSH Server optional feature (available since Windows 10 1809). **VS Code Remote-SSH supports Windows hosts (Windows 10 / Server 2016/2019, 1803+) with the official OpenSSH server.** I could not confirm JetBrains Gateway support for Windows hosts. Running the IDE locally on the PC is the fallback. | [H] / [L] |

**Important gotcha with SSH.** `sshd` runs as a service. Processes it spawns are in a **non-interactive session**, so they can't show windows, receive hotkeys or capture the logged-in desktop. This follows from Session 0 isolation, but test it on day 1 [M].

To run capture tests from SSH or Claude Code, launch the engine **into the interactive session** of a logged-in user:

- Create a scheduled task once with `schtasks /create ... /it`.
- Trigger it with `schtasks /run`.

Alternatively, run Claude Code in Windows Terminal on the PC and view the screen through Parsec, Moonlight/Sunshine or Steam Remote Play. **Avoid RDP for capture tests**: it swaps in a virtual display adapter [M].

**Automated verification Claude Code can run by itself.** A test harness:

1. Opens a D3D11 test-pattern window that shows a frame counter and plays timed beeps.
2. Triggers a save over the pipe.
3. Checks the MP4 with `ffprobe`: duration, frame count, keyframe at the start, and audio offset.

This catches regressions without a human watching.

### 6.2 Cross-compiling from the Mac (compile checks only)

- **Rust: `cargo-xwin` v0.23.1 (2026-08-13).**
  - Needs LLVM/clang (`brew install llvm`).
  - Downloads the MSVC CRT and Windows SDK. **Using it means you accept Microsoft's licence** (go.microsoft.com/fwlink/?LinkId=2086102).
  - `cargo xwin check --target x86_64-pc-windows-msvc` lets Claude Code catch Windows-only compile errors on the Mac.
  - Tauri documents cross-building **NSIS-only** installers from macOS as a "last resort". Use CI for releases. [H]
- **.NET:** `EnableWindowsTargeting=true` compiles Windows-targeted projects on macOS; it can't run them. [H]
- **NativePHP:** `php artisan native:build win` "possible" from a Mac. [H]

### 6.3 GitHub Actions

| Item | Detail | Tag |
|---|---|---|
| Images | `windows-latest` is **Windows Server 2025**; `windows-2022` and `windows-11-arm` are also available. | [H] |
| Runner sizes | Public repos: 4 vCPU / 16 GB, free. Private repos: 2 vCPU / 8 GB. | [H] |
| Price | **$0.010/min** for Windows x64 2-core after the plan's included minutes: 2,000/month on Free, 3,000 on Pro/Team. | [H] |
| Self-hosted runners | Usage is **free**. GitHub postponed the announced $0.002/min fee in December 2025. | [H] |
| GPUs | **No GPU on standard runners.** A Windows **GPU larger runner** (Tesla T4, 16 GB VRAM, 4 vCPU / 28 GB) costs **$0.102/min** and can exercise **NVENC only**. AMF and QSV can't be tested in hosted CI, so test on your own hardware and on beta testers' machines. | [H] |
| Suggested use | CI builds the Rust/Tauri and .NET artifacts, runs unit tests, signs, and publishes updater JSON (e.g. with `tauri-action`). | |
| Self-hosted on the gaming PC | Possible, but it lets CI code run on your personal machine. Restrict it to private repos and trusted workflows. | |

### 6.4 Windows 11 ARM VM on the M2: not useful for Kairo's core

- Microsoft authorises **Parallels Desktop 18, 19 and 20** for Windows 11 Pro/Enterprise on Arm on M1–M3. [H]
- Parallels' own KB (reviewed 2026-08-25) lists 3D support only up to **DirectX 11.1 / OpenGL 4.3**, and no nested virtualisation, WSL2 or VBS. [H]
- **The VM has no AMD, Intel or NVIDIA silicon, so AMF, QSV and NVENC can't be tested.** WGC will probably function on the virtual display, but I didn't verify that [L].
- x64 binaries run under emulation, which is unrepresentative.
- VMware Fusion has been free for everyone, including commercial use, since 2024-11-11. [H]
- **With 8 GB of host RAM, a 4 GB VM leaves about 4 GB for macOS, the IDE, Claude Code and a browser. Skip the VM.** At most, use one occasionally to smoke-test the installer and UI.

### 6.5 Developing the UI on the Mac against a mocked engine

- Tauri runs natively on macOS.
- `interprocess` local sockets map to Unix sockets on the Mac, so a **mock engine in Rust or Node** can speak the exact same JSON-RPC: fake buffer status, fake clips from sample MP4s, and simulated errors.
- For pure frontend work and tests, `@tauri-apps/api/mocks` `mockIPC` (with event mocking since 2.7.0) runs the UI in a browser or in Vitest without Rust. [H]
- `<video>` playback check: WebView2 plays H.264/AAC MP4. HEVC and AV1 playback depend on OS codec extensions and hardware [L]. Verify on the PC before choosing a default recording codec.

### 6.6 Distribution prerequisite to start early

**Code signing** affects SmartScreen, and AV false positives are likely for a tray app with global hotkeys and capture.

- Microsoft **Artifact Signing** (formerly Trusted Signing) issues public-trust certificates to:
  - **organisations** in the US, Canada, EU, UK, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway and Israel;
  - **individual developers only in the US or Canada**.
- It needs a paid Azure subscription. Identity validation takes 1–20 business days.
- Tiers: Basic includes 5,000 signatures/month, Premium 100,000. The pricing page didn't render prices today; historically they were about $9.99 and $99.99 per month [L].
- This is separate from Tauri's mandatory updater-signing key. [H]

---

## 7. Recommendation

### 7.1 Ranking

**#1: Tauri 2 (TypeScript UI) + standalone Rust engine.**

1. **Lowest always-on cost.**
   - A native engine with no runtime to install.
   - No Chromium during gameplay.
   - The UI is a separate process that fully exits.
2. **One native language.** The Tauri shell, the engine, the sidecar and the mock engine are all Rust; the UI is the developer's own JS/TS.
3. **The pieces exist in 2026.**
   - Capture: `windows-capture` 2.0.1.
   - App audio: `wasapi` 0.26 with application loopback.
   - Encoding: `ffmpeg-next` 9 (maintenance mode, but current).
   - IPC: `interprocess` 2.4.
   - Tray, hotkeys and autostart: the same crates Tauri itself uses (`tray-icon`, `global-hotkey`, `auto-launch`).
   - Updater, single instance and installer hooks: Tauri's own.
4. **AI fit.**
   - The compiler enforces memory and thread safety in code the developer can't fully review.
   - `cargo xwin check` gives the agent a fast compile-check loop on the Mac.
5. **macOS optionality.** Only the capture layer would need to change.
6. **Costs to accept.**
   - Rust's learning curve, with mitigations: keep `unsafe` COM code in a small, well-tested module and have Claude Code explain the code it writes.
   - The windows-rs 0.62→0.100 migration (pin 0.62).
   - Tauri 3 arriving in 2027.

**#2: Tauri UI + C# .NET 10 NativeAOT engine.** Pick this if, after the spike, the developer wants to *read and own* the engine. The libraries are excellent and current, and the UI and contract stay identical.

**#3: libobs engine (GPL).** Pick this only if you decide Kairo's desktop app can be open source. You trade licence freedom and a lighter footprint for OBS's proven capture, encode and replay stack.

### 7.2 First two weeks: spikes with exit criteria (all on the gaming PC)

| Spike | What to do | Done when |
|---|---|---|
| **S1. Encoder smoke test (no code)** | Install FFmpeg 9. Run `ffmpeg -filter_complex gfxcapture=monitor_idx=0:max_framerate=60 -c:v h264_amf out.mp4` (or `hevc_nvenc` for NVIDIA). For Intel, add the QSV hwmap path. Record while a game runs and note CPU, GPU and RAM in Task Manager. | The user's exact GPUs are confirmed with H.264 and HEVC working. |
| **S2. Rust engine skeleton** | WGC → D3D11 BGRA→NV12 → `libavcodec` hardware encoder → packet ring (≥2 keyframes) → hotkey → MP4. Add WASAPI system loopback + process loopback + mic → AAC. | A 30 s clip saves in under 1 s, A/V offset is under 40 ms, and the engine stays under the budgets you set (e.g. under 60 MB plus buffer, under 3% CPU at 1080p60). The budget numbers are product decisions. |
| **S3. Process-model measurement** | With the Tauri UI open, then closed, compare the total `msedgewebview2` process RAM. | Decides between 5.1 (separate engine) and alternative B. |
| **S4. Hotkeys and edge cases** | Test in borderless and exclusive fullscreen games, an elevated game, and one with anti-cheat. Test conflicts with vendor overlays. On Windows 10, test the WGC border and DXGI fallback. | |
| **S5. Remote loop** | Confirm the SSH session limitation, then set up the `schtasks /it` launcher and the `ffprobe` verification harness. | Claude Code on the PC can build, run and verify a capture unattended. |

If S2 drags past about 7 working days, because Rust and COM friction is causing repeated agent loops, switch the engine to C# (#2). Nothing else changes.

---

## 8. Unverified or open items

1. Idle RAM figures for every stack are estimates, apart from one CI benchmark of empty apps (Elanis, single run, no GPU details). Measure them on the gaming PC.
2. Whether Tauri's core releases all WebView2 processes once the last window is destroyed (decides 5.1 vs alternative B).
3. Hotkeys (`RegisterHotKey`), WGC and process loopback when the game runs **elevated** or under anti-cheat.
4. WebView2 playback of HEVC/AV1 clips (depends on OS codec extensions and hardware).
5. JetBrains Gateway support for Windows *hosts* in 2026.
6. Whether WinUI 3 can be built on macOS (I expect not).
7. Qt Small Business pricing and Artifact Signing prices (neither page rendered prices).
8. Overwolf `ow-obs.exe` and Insights Capture's use of OBS come from secondary sources (the blog and community pages could not be fetched).
9. FFmpeg 9.0's exact release date (2026-08-04 per secondary sources; ffmpeg.org lists 9.0.2 on 2026-09-18), and whether `ffmpeg-next` 9.0.0 fully covers FFmpeg 9's API.
10. Codec patent exposure for a free versus monetised app needs legal review. AMD's SDK licence explicitly pushes royalties to the developer.

---

## Sources

Accessed 2026-10-21; page dates in parentheses where shown.

**Microsoft / Windows**
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired (updated 2026-07-23)
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.minupdateinterval
- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.dirtyregionmode
- https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/ (2026-02-19)
- https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params
- https://learn.microsoft.com/en-us/deployedge/microsoft-edge-supported-operating-systems (2026-08-31)
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance (2026-01-12)
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution (updated 2026-10-02)
- https://learn.microsoft.com/en-us/windows/whats-new/extended-security-updates (2025-11-17)
- https://www.microsoft.com/en-us/windows/extended-security-updates
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
- https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights
- https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
- https://learn.microsoft.com/en-us/previous-versions/windows/hardware/design/dn653293(v=vs.85) (Session 0 isolation)
- https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh-overview
- https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-channels (2026-09-29)
- https://blogs.windows.com/windowsdeveloper/2024/09/04/whats-new-in-windows-app-sdk-1-6/
- https://learn.microsoft.com/en-us/dotnet/core/deploying/trimming/incompatibilities
- https://dotnet.microsoft.com/en-us/platform/support/policy/dotnet-core
- https://learn.microsoft.com/en-us/dotnet/core/tools/sdk-errors/netsdk1100
- https://github.com/microsoft/CsWinRT/blob/master/docs/aot-trimming.md
- https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart (2026-05-21)
- https://learn.microsoft.com/en-us/azure/artifact-signing/faq (2026-05-14)
- https://azure.microsoft.com/en-us/pricing/details/artifact-signing/
- https://support.microsoft.com/windows/options-for-using-windows-11-with-mac-computers-with-apple-m1-and-m2-chips-cd15fd62-9b34-4b78-b0bc-121baa3c568c

**Tauri**
- https://crates.io/crates/tauri
- https://tauri.app/release/core/
- https://github.com/tauri-apps/tauri/releases
- https://v2.tauri.app/blog/
- https://v2.tauri.app/develop/sidecar/
- https://v2.tauri.app/distribute/windows-installer/
- https://v2.tauri.app/plugin/updater/
- https://v2.tauri.app/plugin/global-shortcut/
- https://v2.tauri.app/plugin/autostart/
- https://v2.tauri.app/plugin/single-instance/
- https://v2.tauri.app/develop/tests/mocking/
- https://github.com/tauri-apps/plugins-workspace
- https://github.com/tauri-apps/global-hotkey
- https://github.com/zzzgydi/auto-launch

**Rust crates**
- https://crates.io/crates/windows
- https://github.com/microsoft/windows-rs/issues/4867
- https://github.com/microsoft/windows-rs/releases
- https://crates.io/crates/windows-capture
- https://github.com/NiiightmareXD/windows-capture
- https://crates.io/crates/wasapi
- https://github.com/HEnquist/wasapi-rs
- https://crates.io/crates/ffmpeg-next
- https://github.com/zmwangx/rust-ffmpeg
- https://crates.io/crates/rsmpeg
- https://crates.io/crates/ffmpeg-sidecar
- https://crates.io/crates/interprocess
- https://docs.rs/interprocess/latest/interprocess/
- https://crates.io/crates/libobs
- https://crates.io/crates/libobs-wrapper
- https://crates.io/crates/tray-icon
- https://crates.io/crates/global-hotkey
- https://crates.io/crates/velopack
- https://crates.io/crates/sentry
- https://crates.io/crates/ts-rs
- https://github.com/rust-cross/cargo-xwin

**.NET packages**
- https://www.nuget.org/packages/NAudio
- https://github.com/naudio/NAudio
- https://www.nuget.org/packages/Vortice.Direct3D11
- https://www.nuget.org/packages/Vortice.MediaFoundation
- https://www.nuget.org/packages/ScreenRecorderLib
- https://github.com/sskodje/ScreenRecorderLib
- https://www.nuget.org/packages/Microsoft.WindowsAppSDK
- https://www.nuget.org/packages/Avalonia
- https://www.nuget.org/packages/Microsoft.Windows.CsWinRT
- https://www.nuget.org/packages/Microsoft.Windows.CsWin32
- https://www.nuget.org/packages/FFmpeg.AutoGen
- https://www.nuget.org/packages/StreamJsonRpc
- https://www.nuget.org/packages/Velopack
- https://www.nuget.org/packages/H.NotifyIcon
- https://github.com/lezzi/wpfaot

**Electron / NativePHP**
- https://releases.electronjs.org/schedule
- https://www.electronjs.org/docs/latest/tutorial/electron-timelines
- https://nativephp.com/docs/desktop/2/getting-started/releasenotes
- https://github.com/NativePHP/desktop/releases
- https://nativephp.com/blog/nativephp-for-desktop-v2-released (2025-10-14)
- https://nativephp.com/docs/desktop/2/getting-started/installation
- https://nativephp.com/docs/desktop/1/getting-started/installation
- https://nativephp.com/docs/desktop/2/digging-deeper/child-processes
- https://nativephp.com/docs/desktop/2/digging-deeper/files
- https://nativephp.com/docs/desktop/2/the-basics/global-hotkeys
- https://nativephp.com/docs/desktop/2/publishing/building
- https://nativephp.com/docs/desktop/2/publishing/updating
- https://nativephp.com/docs/desktop/2/digging-deeper/php-binaries
- https://github.com/Elanis/web-to-desktop-framework-comparison

**OBS / libobs / GPL**
- https://github.com/obsproject/obs-studio
- https://github.com/obsproject/obs-studio/blob/master/libobs/obs.h
- https://github.com/obsproject/obs-studio/blob/master/plugins/obs-ffmpeg/obs-ffmpeg-mux.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/plugin-main.cpp
- https://github.com/obsproject/obs-studio/blob/master/libobs-winrt/winrt-capture.cpp
- https://github.com/stream-labs/obs-studio-node
- https://github.com/stream-labs/desktop
- https://dev.overwolf.com/ow-native/guides/test-your-app/ow-logs/obs/
- https://medium.com/overwolf/a-deep-dive-into-overwolfs-process-infrastructure-2e99b6f59a93 (via search summary only)
- https://insights.gg/community/d/18-what-is-insights-capture (via search summary only)
- https://obsproject.com/forum/threads/obs-license-inquiry.58557/ (via search summary only)
- https://www.gnu.org/licenses/gpl-faq.html#MereAggregation

**FFmpeg and codec SDKs**
- https://ffmpeg.org/legal.html
- https://ffmpeg.org/download.html
- https://ffmpeg.org/index.html#news
- https://github.com/FFmpeg/FFmpeg/blob/release/9.0/Changelog
- https://github.com/FFmpeg/FFmpeg/blob/release/9.0/doc/filters.texi
- https://linuxiac.com/ffmpeg-9-0-released-with-animated-webp-decoding-and-new-hardware-acceleration/ (secondary)
- https://github.com/BtbN/FFmpeg-Builds
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF
- https://github.com/intel/libvpl
- https://github.com/intel/vpl-gpu-rt
- https://github.com/FFmpeg/nv-codec-headers

**Workflow, CI and VMs**
- https://code.claude.com/docs/en/setup
- https://code.visualstudio.com/docs/remote/ssh
- https://www.jetbrains.com/help/idea/prerequisites.html
- https://docs.github.com/en/billing/reference/actions-minute-multipliers
- https://docs.github.com/en/billing/concepts/product-billing/github-actions
- https://github.com/resources/insights/2026-pricing-changes-for-github-actions
- https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- https://docs.github.com/en/actions/reference/runners/larger-runners
- https://kb.parallels.com/en/129497 (reviewed 2026-08-25)
- https://blogs.vmware.com/cloud-foundation/2024/11/11/vmware-fusion-and-workstation-are-now-free-for-all-users/
- https://github.com/PowerShell/Win32-OpenSSH/wiki/Troubleshooting-Steps
- https://www.qt.io/pricing
