# 15 · Kairo implementation cookbook

*2026-10-21 · Lead-engineer synthesis of research 08–13 (code level), reconciled with `plan/PLAN.md` and the 2026-10-21 reality check. Audience: Claude Code building Kairo. Nothing is built yet.*

**Tags.** [V] verified in source or vendor docs (research + reality check) · **[RC]** changed or added by the reality check · **[CK]** sketch compiled for this cookbook · **[U]** unverified, settle on the PC · **[E]** estimate · **[D]** decision taken here where research or plan disagree.

**Compile evidence** (Rust 1.99.1; harnesses in `prototypes/`). Type-checked is not run.

| Harness | Command | Covers | Result |
|---|---|---|---|
| `wincheck` | `cargo check --target x86_64-pc-windows-msvc` | `windows` =0.62.2, `wasapi` =0.26.0, tokio 1.54.0, crash-handler 0.8.1, minidumper 0.11.0, sentry 0.49.3, file-rotate 0.8.0: device, GpuLock, WGC start/consume, interval rule, HDR, VP fallback, D3DCompile, GPU timers, pipe server/client + DACL, hotkeys, mouse poll, elevation, job object, mutex, EcoQoS, WASAPI capture + notifications, PDH, CF_HDROP, SHDoDragDrop, panic/crash hook, Sentry outbox, logging | 0 errors, 0 warnings |
| `ffcheck` | `cargo check` on the Mac against Homebrew FFmpeg 8.0.1 headers | `ffmpeg-next` =9.0.0 + `ffmpeg-sys-next` =9.0.1: hwcontext mirrors, encoder open/options/drain, AAC, mux, log callback | 0 errors (API names only; 9.0.2 behaviour per research 09) |
| `corecheck` | `cargo test` | ring + cut + audio start, aligner (rubato 5.0.1), MinUpdateInterval rule | 5/5 pass |
| `kcapcheck` (research 08) | `cargo check --target …msvc` | DDA, monitors, targets, HDR | reused as is |

**Bugs the compiler found in earlier research sketches** (fixed below):
1. `wasapi::WasapiRes` is `pub(crate)` (research 10 §2.1, §2.6). Use `Result<T, wasapi::WasapiError>`.
2. `Output::write_header_with` returns a `Dictionary<'_>` that borrows the muxer (research 09 §5.8). Consume it in the same statement.
3. `sentry::ClientOptions` is `#[non_exhaustive]` (research 11 §5.4). Use `default()` and assign fields.
4. ulid 3.0.0 has `Ulid::generate()`, not `Ulid::new()`.
5. Feature gates: `CRITICAL_SECTION` needs `Win32_System_Kernel`; `SetWinEventHook` needs `Win32_UI_Accessibility`; `DISPLAYCONFIG_COLOR_ENCODING` lives in `Win32::Graphics::Gdi`.

---

## 0. Summary and plan impacts

### 0.1 The engine in eight lines

1. **Capture:** Kairo's own WGC module on `windows` =0.62.2 (free-threaded 2-buffer pool, borderless, refresh-aware `MinUpdateInterval`); DDA for Win10 and as the fallback. Frames are copied into a ring texture and released at once.
2. **One device:** a D3D11 device on the adapter that scans out the target monitor (`BGRA_SUPPORT | VIDEO_SUPPORT`), `SetMultithreadProtected(TRUE)` (FFmpeg does not do it for supplied devices **[RC]**), and one recursive `GpuLock` (a `CRITICAL_SECTION`) shared with FFmpeg through `AVD3D11VADeviceContext.lock/unlock`. `ID3D11DeviceContext` is `Send + Sync` in 0.62.2 **[RC]**, so the compiler will not stop unsynchronised use.
3. **Convert:** two-pass HLSL (scale/tone-map, then Y and UV) rendered straight into FFmpeg's D3D11 NV12 pool frames. Runtime `D3DCompile`. Never BGRA into AMF.
4. **Encode:** `h264_amf` on the same device (zero copy), `usage=transcoding`, Pre-Analysis off, `bf=0`, explicit `gop_size = fps` (AMF's own IDR default is 240 **[RC]**), `async_depth=4`, peak ≤50 Mbps; its own thread; lazy probe; output-stall watchdog.
5. **Ring and save:** encoded packets (video + all audio tracks) in RAM with a keyframe index and epochs; a save snapshots packet refs, then muxes `+faststart`, `use_editlist=0`, mix track default.
6. **Audio:** `wasapi` =0.26.0 behind a `LoopbackMode` enum; event-driven shared mode with both durations 0; per-source aligners (rubato) in `kairo-core`; `TIMESTAMP_ERROR` counted, never a restart; seamless A/B refresh.
7. **Shell:** plain Win32 loop on T0, direct `RegisterHotKey`, tokio named pipes (server and client) with a user-SID DACL, u32 big-endian framing ≤1 MiB, panic hook that dumps before the `__fastfail` **[RC]**.
8. **Measure:** PresentMon 2.6.0 with `--session_name`, PDH video-engine sampling, ABBA ×3, pass = mean loss ≤3% and upper 95% CI ≤4%.

### 0.2 Plan impacts (concrete edits for PLAN.md)

| # | Change | PLAN.md place | Why / source |
|---|---|---|---|
| P1 | Add the **GpuLock**: one `CRITICAL_SECTION` passed to FFmpeg as `AVD3D11VADeviceContext.lock/unlock/lock_ctx`; every multi-call immediate-context sequence on Kairo's side holds it; never hold it across `avcodec_send_frame`/`receive_packet`. Plus `SetMultithreadProtected(TRUE)` on the shared device | §2.2 Device row; M3 deliverables | **[RC]** context is Send + Sync; FFmpeg's `d3d11va_device_init` skips multithread protection for user devices; default lock is a private `CreateMutex` |
| P2 | Replace "NV12 self-test with a BGRA fallback" with: self-test → `ID3D11VideoContext::VideoProcessorBlt` BGRA→NV12 **[U]** → last resort: Pass B into plain R8/R8G8 textures, read back, system-memory NV12 frames into the encoder. **Never feed BGRA/RGB to AMF** | §2.2 Convert row; M3 risks | AMF #574 (DCC gridlines on RX 7000 from 25.10.2), FFmpeg PR #24688 (research 13) |
| P3 | AMF private options: `usage=transcoding quality=balanced rc=vbr_peak vbaq=1 preanalysis=0 high_motion_quality_boost_enable=0 preencode=0 bf=0 frame_skipping=0 filler_data=0 header_spacing=60 async_depth=4`; set `gop_size=fps` explicitly; each via checked `av_opt_set` | §2.2 Encode row | 13 §1.2; **[RC]** FFmpeg sets `IDR_PERIOD` only if `gop_size != -1` |
| P4 | "High" preset: 1440p60 **36/50 Mbps** (was 40/60) | §2.2 presets | AMF #614 stall at ~100 Mbps (RX 9060 XT), never seen at 50 |
| P5 | Encoder probe: lazy and sequential, H.264 only unless HEVC/AV1 opted in, ≥1 s between sessions, cached per (driver store version, AMF runtime version), run inside media | §2.2 "Per-adapter probe" | Sunshine #5385: back-to-back sessions hang an RX 9070 XT (26.6.4) |
| P6 | Add an **encoder governor + output-stall watchdog**: no packet for >2 s while frames go in → recreate the session (new epoch); video engine >85% for 5 s → step quality down; `encoderPressure` notification naming the competing process | §2.2 Health; M5 | AMF #614, OBS #13670, OBS #10594 |
| P7 | `MinUpdateInterval` start value **(k − 0.5)/refresh, k = max(1, ⌊refresh/(1.2·fps)⌋)**: 360 Hz → 12.5 ms (72/s), 180 Hz → 8.33 ms (90/s), 60 Hz → 8.33 ms (60/s). SP2 sweeps it against 1/(2·fps); log the OS default read back | §2.2 WGC row ("1/(2·fps)") | **[D]** merges 08 and 13; **[RC]** OBS PR #12759 still open; default undocumented |
| P8 | Save audio start: the AAC packet **covering** the IDR time plus one pre-roll packet, written with true negative pts, `use_editlist=0`. movenc's make_zero shift then holds video frame 0 ≤42.7 ms and A/V is exact afterwards. The "nearest packet" rule can start audio *after* the IDR; movenc then pulls that track's first sample to 0 and lengthens it (movenc.c:7282-7290), so players that play audio back-to-back run the whole track up to 10.7 ms early | §2.2 save flow step 3 | **[D]** from movenc source; **[U]** confirm with testbed flash/beep in M3 |
| P9 | Mux: DEFAULT disposition on the mix only (more than one default per type switches movenc to per-stream groups, all enabled); temp file is a real path on the library volume (faststart re-opens it by URL); kairo-verify checks both | §2.2 save flow; M1 kairo-verify | **[RC]** |
| P10 | Tauri: no `appDirectoriesOverride`. Tauri's defaults already resolve to `%LOCALAPPDATA%\io.github.maty3k.kairo\` (EBWebView included), which the NSIS "delete app data" step removes; `%LOCALAPPDATA%\Kairo` is the install dir | §2.6, §1.5 | **[RC]** research 12 suggested `$LOCALDATA/Kairo/ui` |
| P11 | Drag-out on Windows: own `SHDoDragDrop` command (CK); `tauri-plugin-drag` only on macOS (target-gated) | §2.7 | **[D]** removes `windows` 0.52 + `windows-core` 0.58 builds and open issues #72/#76/#94 from the shipped app |
| P12 | UI shell connects with tokio `ClientOptions` (Windows) / `UnixStream` (Mac); research 12's `interprocess` client sketch is replaced | §2.1 IPC table | plan already rejects the interprocess client; **[RC]** no SQOS flags there |
| P13 | Dev tasks: `Register-ScheduledTask` with `-Priority 4 -ExecutionTimeLimit 0`. Never benchmark through a `schtasks /create /it` task: it runs at priority 7 = `BELOW_NORMAL_PRIORITY_CLASS` | §3.1 | **[RC]** |
| P14 | Crash path: panic hook → `CrashHandler::simulate_exception(Some(0xE0004B50))` → `exit(EXIT_PANIC)`; dev PC also registers WER LocalDumps for `kairo-engine.exe` | §2.9 | **[RC]** abort, `panic=abort` and nounwind panics are `__fastfail`; WER LocalDumps still sees them (job interaction **[U]**) |
| P15 | Sentry: set `ClientOptions.server_name` explicitly | §2.9 | **[RC]** sentry-contexts fills in the hostname (PII) |
| P16 | Media opts out of EcoQoS (`SetProcessInformation(ProcessPowerThrottling)`, `EXECUTION_SPEED \| IGNORE_TIMER_RESOLUTION`, StateMask 0); pacer and mouse poll use `CreateWaitableTimerExW(HIGH_RESOLUTION)` | §2.1 lifecycle | research 11 E11 (missing from the plan) |
| P17 | `pc bench`: PresentMon `--session_name KairoBench --qpc_time --track_gpu_video --track_frame_type`; PDH video-engine sampler matching "video" case-insensitively; gate adds "upper 95% CI ≤4%"; budget stated "with AMD Instant Replay off" | §3.2 | 13 §5; **[RC]** AMD engine-type name uncertain; Adrenalin ships its own PresentMon |
| P18 | SP9: read the installed driver's `HKLM\SOFTWARE\AMD\DVR\*Hotkey` values; do not trust published AMD default lists | M2 SP9 | **[RC]** only Ctrl+Shift+O/L confirmed by AMD |
| P19 | Pop-up: create once, `SW_HIDE` when idle, `SW_SHOWNOACTIVATE` ≤2 s; never a transparent topmost window over a game; test with AMD's metrics overlay on | §2.5 | 13 §2.4 (Composed Flip), AMD 26.10.1 known overlay issues |
| P20 | Pins: `tauri` =2.12.5 with `tauri-build` =2.7.4 (they move together: 2.12.5 needs `tauri-build` ^2.7.4), `@tauri-apps/cli` 2.12.4, `@tauri-apps/api` 2.12.4 (reviewed in M0 on 2026-10-23, §12 item 22); pin `webdriverio@9.32.0` (npm `latest` is now 10.1.0, which breaks `@wdio/tauri-service` 1.5.0's `^9` peer) | §2.11 | **[RC]** |
| P21 | No host-side CMake build dependencies in the workspace: cargo-xwin exports non-target-specific `CL_FLAGS`, `RCFLAGS`, `LIB`, `CMAKE_GENERATOR=Ninja`, `CMAKE_SYSTEM_NAME=Windows` (Sentry stays on `native-tls`, so no `aws-lc-sys`) | §3.1 | **[RC]** |
| P22 | UI launch: `Kairo.exe` started without `--from-engine` hands off with `ui.open {waitPid}` and exits; control waits for that PID, then spawns `Kairo.exe --from-engine` as its child (covers Start-menu, updater and double-click launches without an NSIS shortcut hook) | §2.1 UI bullet | **[D]** single-instance mutex race avoided by waiting on the PID |

### 0.3 Where the research disagreed

| Topic | Positions | Decision |
|---|---|---|
| WGC library | 11's thread map uses `windows-capture`; 08 rejects it | Own module (08, plan) |
| `MinUpdateInterval` | 08: 1/(2·fps); 13: k/refresh | (k − 0.5)/refresh, SP2 decides **[D]** |
| Framing | 11: u32 LE, 1 MiB; 12: u32 BE, 16 MiB | u32 BE (tokio-util default = StreamJsonRpc), 1 MiB (plan) |
| Pipe client | 12: interprocess; 11: tokio | tokio (plan) |
| Contract crate | `kairo-protocol` (12) vs `kairo-rpc` (11, plan) | `kairo-rpc` |
| App data | 12: `appDirectoriesOverride` → `%LOCALAPPDATA%\Kairo\ui`; 11: identifier dir | identifier dir (plan) |
| FFmpeg lock | 09: `ID3D11Multithread::Enter/Leave` (recursion unverified); 08: reentrant mutex | `CRITICAL_SECTION` (recursive by definition) **[D]** |
| NV12 fallback | 09: BGRA into the encoder; 13: never on AMD | VideoProcessorBlt, then R8/R8G8 render + readback to system-memory NV12 **[D]** |
| Audio start at save | 09: first packet ≥ IDR; 10: pre-roll + edit list; plan: nearest | covering packet + 1 pre-roll, negative pts, no edit list **[D]** |
| Full-frame copies | 08: copy on arrival; 13: convert on arrival candidate | copy on arrival in M3; A/B in SP1/M5 |
| Drag-out | 12: plugin + fallback | `SHDoDragDrop` on Windows, plugin on macOS **[D]** |

---

## 1. Repo and crate layout

```
Kairo/                        (public, GPL-3.0; "-only" vs "-or-later" is Q18)
├─ Cargo.toml                 workspace, resolver "3", edition 2024, rust-version 1.95 (rusqlite_migration); rust-toolchain.toml = 1.99.1
├─ .cargo/config.toml         [env] TS_RS_EXPORT_DIR = { value = "apps/ui/src/gen", relative = true }, TS_RS_LARGE_INT = "number"
├─ crates/
│  ├─ kairo-core/             pure Rust, tested on the Mac: ring + cut planning, pacer maths, aligner, mixer/limiter/meters,
│  │                          restart policy, game matching, settings, export size-fit
│  ├─ kairo-rpc/              serde + ts-rs + schemars types, rpc_methods! registry, PROTOCOL_VERSION, codec(), pipe_name(), fixtures
│  ├─ kairo-engine/           bin kairo-engine: control (default) | --media | --ensure | --shutdown | --open
│  ├─ kairo-net/              the only crate with sockets (update check, Sentry transport)
│  ├─ kairo-cli/  kairo-mock-engine/  kairo-verify/  kairo-testbed/  kairo-probes/  kairo-devrun/
├─ apps/ui/                   Vite + React; apps/ui/src-tauri = crate kairo-ui
├─ shaders/                   kairo_nv12.hlsl, pointer.hlsl (include_str!, compiled at runtime)
├─ tools/mac/pc  tools/pc/*.ps1  data/games.json  docs/{research,adr,spikes,milestones}
```

`kairo-engine/src` (everything below is `#[cfg(windows)]`; the crate builds a stub elsewhere):

| Module | Files | Owner thread |
|---|---|---|
| control | `shell.rs` (T0 loop, tray, hotkeys, WinEvent, power), `ipc/` (pipe server, sessions, media link), `detect/`, `db.rs`, `crash/server.rs`, `logging.rs`, `lifecycle.rs` (mutex, job, Run key, restart policy) | T0 / tokio / detect / db |
| capture | `device.rs`, `wgc.rs`, `wgc_rate.rs`, `dda.rs`, `pointer.rs`, `monitors.rs`, `hdr.rs`, `target.rs`, `health.rs` | pool worker → GPU thread |
| gpu | `lock.rs` (GpuLock), `shaders.rs`, `convert.rs`, `nv12_selftest.rs`, `vp_fallback.rs`, `timer.rs` | GPU thread |
| encode | `ffi_hw.rs`, `video.rs`, `amf_opts.rs`, `probe.rs`, `governor.rs`, `aac.rs`, `log.rs` | encoder thread |
| save | `mux.rs`, `verify.rs` | save thread |
| audio | `capture.rs`, `loopback.rs`, `devices.rs`, `proctree.rs`, `watchdog.rs` (aligner, mixer in kairo-core) | one thread per source, mixer, watchdog |

**Dependencies** (versions, dates and licences read from crates.io/npm on 2026-10-21):

| Crate | Version | Features | Licence | Purpose |
|---|---|---|---|---|
| `windows` | =0.62.2 | per module; at least the set compiled in `wincheck/Cargo.toml` | MIT OR Apache-2.0 | Win32 + WinRT. 0.100 has no umbrella crate **[RC]** |
| `windows-future` | =0.3.2 | — | MIT OR Apache-2.0 | `IAsyncOperation::join()` |
| `windows-registry` | 0.6.1 | — | MIT OR Apache-2.0 | Run key, AMD hints (not 0.100) |
| `ffmpeg-next` | =9.0.0 | `default-features=false`, `codec`, `format`, `software-resampling` | WTFPL | encoders, packets, mux (9.0.1 is git-only **[RC]**) |
| `ffmpeg-sys-next` | =9.0.1 | `default-features=false`, `avcodec`, `avformat`, `swresample` | WTFPL | raw FFI; `links="ffmpeg"`: one sys crate only |
| FFmpeg DLLs | BtbN `autobuild-2026-10-20-13-08`, `n9.0.2-29-g6dfaba896f` lgpl-shared | the engine links avcodec-63, avformat-63, avutil-61, swresample-7; the export `ffmpeg.exe` also needs avfilter-12, swscale-10, avdevice-63 | LGPL-3.0 | mirrored with source in Kairo releases (zip 77,233,322 B; 7 DLLs 149.2 MiB **[RC]**) |
| `wasapi` | =0.26.0 | — | MIT | WASAPI capture |
| `rubato` | =5.0.1 | — | MIT OR Apache-2.0 | drift resampler |
| `rtrb` / `crossbeam-channel` / `bytemuck` | 0.3.5 / 0.5.17 / 1.25.2 | — | MIT OR Apache-2.0 / same / Zlib OR Apache-2.0 OR MIT | SPSC audio, sync channels, slice casts |
| `tokio` / `tokio-util` | 1.54.0 / 0.7.20 | `rt-multi-thread macros net io-util sync time process` / `codec` | MIT | pipes, runtime, framing |
| `futures`, `bytes` | 0.3.34, 1.12.1 | — | MIT OR Apache-2.0, MIT | |
| `serde` / `serde_json` | 1.0.229 / 1.0.154 | `derive` / `raw_value` | MIT OR Apache-2.0 | |
| `ts-rs` / `schemars` | 12.0.1 / 1.2.3 | `serde-json-impl` / — | MIT | TS + JSON Schema from one registry |
| `tracing` / `-subscriber` / `-appender` / `-log` | 0.1.44 / 0.3.23 / 0.2.5 / 0.2.0 | subscriber: `env-filter fmt json registry` | MIT | logs; `tracing-log` bridges wasapi's `log` |
| `file-rotate` | 0.8.0 | — | MIT | 10 × 10 MB rotation |
| `sentry` | 0.49.3 | `default-features=false`, `backtrace contexts panic debug-images reqwest native-tls` | MIT | opt-in crash reports, EU |
| `crash-handler` / `minidumper` / `minidump-writer` | 0.8.1 / 0.11.0 / 0.13.0 | — | MIT OR Apache-2.0 / same / MIT | dumps (media out of process) |
| `rusqlite` / `rusqlite_migration` | 0.40.2 / 2.6.0 | `bundled` (SQLite 3.53.2) | MIT / Apache-2.0 | library index |
| `tray-icon` / `muda` | 0.26.1 / 0.21.2 | `default-features=false` | MIT OR Apache-2.0 | tray |
| `steamlocate` / `roxmltree` | 2.1.1 / 0.21.1 | — | MIT / MIT OR Apache-2.0 | Steam, `MicrosoftGame.config` |
| `ulid`, `anyhow`, `thiserror`, `dirs`, `semver` | 3.0.0, 1.0.104, 2.0.21, 7.0.0, 1.0.28 | — | MIT / MIT OR Apache-2.0 | |
| `embed-resource` (build) | 3.0.12 | — | MIT | manifest (PerMonitorV2, asInvoker) + icons; `llvm-rc` on the Mac |
| `tauri` / `tauri-build` | =2.12.5 / =2.7.4 | no `protocol-asset`, no `unstable` | Apache-2.0 OR MIT | UI shell; move both together (P20) |
| Tauri plugins | single-instance 2.5.4, window-state 2.5.0, opener 2.7.2, dialog 2.8.3; drag 2.1.1 (macOS only); wdio-webdriver 1.5.0 (`e2e` feature only, MIT) | — | Apache-2.0 OR MIT | |
| `http-range` / `percent-encoding` / `arboard` | 0.1.6 / 2.3.2 / 3.7.0 (macOS) | — | MIT / MIT OR Apache-2.0 / MIT OR Apache-2.0 | `kairo-media`, Mac clipboard parity |
| Tools | cargo-xwin 0.23.1 (MIT), dump_syms 2.3.9, minidump-stackwalk 0.27.0, PresentMon 2.6.0, LLVM (libclang) | | | |

**npm (apps/ui):** react 19.3.1 (MIT), `@tauri-apps/api` 2.12.4 / cli 2.12.4 (Apache-2.0 OR MIT; the CLI's macOS build also contains `tauri-macos-sign` under MPL-2.0, dev only), TanStack Query 5.104.5 / Virtual 3.14.21 / Router 1.170.45 (MIT), zustand 5.0.15, radix-ui 1.8.1, tailwindcss 4.3.3, motion 14.5.0, sonner 2.0.8, i18next 26.5.4 (MIT), lucide-react 1.61.0 and react-image-crop 11.1.2 (ISC), wavesurfer.js 8.0.3 (BSD-3-Clause), `@fontsource-variable/inter` 5.3.0 (OFL-1.1), typescript **6.0.3** (Apache-2.0), vite 8.3.5, vitest 5.0.5, webdriverio **9.32.0** + `@wdio/tauri-service` 1.5.0 (MIT), axe-core 4.14.0 (MPL-2.0, dev only). `@crabnebula/tauri-plugin-drag` 2.1.0 has no licence field on npm (the crate is Apache-2.0 OR MIT).

**Not used:** `windows-capture`, `global-hotkey`, `auto-launch`, the `interprocess` client, `sentry-rust-minidump` (deprecated), `specta`/`tauri-specta` (RC), `jsonrpsee`, `cpal`, `ez-ffmpeg` (pulls a second FFmpeg sys crate), `tauri-plugin-updater` (updates live in control; `deny.toml` bans it), `tauri-plugin-process` (it only paired with the updater; `exit()` is in `@tauri-apps/api/app` since 2.12.0, behind `core:app:allow-exit`).

---

## 2. Capture

### 2.1 Rules

| Item | v1 rule |
|---|---|
| Backend | WGC when `GraphicsCaptureSession::IsSupported()` and `IsBorderRequired` is present (Win11). DDA on Win10, after 2 failed WGC creations, or when the watchdog says WGC is stale and DDA is not |
| Target | Game: window (`CreateForWindow`); on `Closed` re-resolve by PID (games swap windows on mode changes); elevated, black or frozen → the game's monitor. Screen mode: monitor persisted by `monitorDevicePath` |
| Pool | `CreateFreeThreaded`, **2 buffers**; `B8G8R8A8UIntNormalized` (87), or `R16G16B16A16Float` (10) when the monitor is in HDR |
| Border | `GraphicsCaptureAccess::RequestAccessAsync(Borderless)` once, on an MTA thread, log the status; always `SetIsBorderRequired(false)` (ignored, not an error, if denied **[RC]**) |
| Cursor | off in window mode, on in screen mode (`SetIsCursorCaptureEnabled`) |
| Rate | `MinUpdateInterval` = (k − 0.5)/refresh, k = max(1, ⌊refresh/(1.2·fps)⌋) **[D][E]** |
| Secondary windows | `SetIncludeSecondaryWindows(false)` |
| Clock | `SystemRelativeTime` (100 ns QPC, the presentation time); pace on it, never on arrival |

Version gates **[RC]**: `IsCursorCaptureEnabled` 19041 · `IsBorderRequired` + `GraphicsCaptureAccess` 20348 (contract v12) · `MinUpdateInterval`, `IncludeSecondaryWindows` (default false), `DirtyRegionMode` 26100 (contract v19). Gate each with `ApiInformation::IsPropertyPresent`; setting a missing one throws.

### 2.2 Threads

```
WGC pool worker (OS)            GPU thread (ours, MTA, owns all immediate-context work under GpuLock)
  FrameArrived:                   loop on the QPC pacer (high-res waitable timer):
    TryGetNextFrame → Mailbox       frame = mailbox newest (older ones Close()d by the handler)
    (newest wins, older Close())    consume(): ContentSize check → Recreate | CopyResource → ring[i]; frame.Close()
  item.Closed → mailbox.closed      at tick: newest ring slot with ts ≤ tick (else repeat) → convert → NV12 pool frame → encoder channel
```

- `IDirect3DDevice` (WinRT) is not `Send`: build `GpuDevice` on the GPU thread. WGC runtime classes are `Send + Sync` **[RC]**.
- `HWND`/`HANDLE` are `!Send`: pass `hwnd.0 as isize`.
- Copy-on-arrival is the M3 default. Convert-on-arrival (shader straight from the WGC surface into NV12) is the bandwidth-saving A/B in SP1/M5 (13 §2.4).

### 2.3 Device **[CK]**

```rust
pub struct GpuDevice { pub device: ID3D11Device, pub context: ID3D11DeviceContext, pub winrt: IDirect3DDevice, pub luid: LUID }

/// The adapter that scans out `mon` (DDA requires it; never assume adapter 0: Ryzen iGPUs add one).
pub fn adapter_for_monitor(f: &IDXGIFactory1, mon: HMONITOR) -> Result<Option<IDXGIAdapter1>> {
    for a in 0.. {
        let Ok(adapter) = (unsafe { f.EnumAdapters1(a) }) else { return Ok(None) }; // DXGI_ERROR_NOT_FOUND ends
        for o in 0.. {
            let Ok(out) = (unsafe { adapter.EnumOutputs(o) }) else { break };
            if unsafe { out.GetDesc()? }.Monitor == mon { return Ok(Some(adapter)); }
        }
    }
    Ok(None)
}

pub fn create_device(adapter: &IDXGIAdapter1) -> Result<GpuDevice> {
    let a: IDXGIAdapter = adapter.cast()?;
    let (mut dev, mut ctx) = (None, None);
    unsafe {
        D3D11CreateDevice(&a, D3D_DRIVER_TYPE_UNKNOWN /* explicit adapter */, HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, // FFmpeg QIs ID3D11VideoDevice
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION,
            Some(&mut dev), None, Some(&mut ctx))?;
    }
    let (device, context) = (dev.unwrap(), ctx.unwrap());
    let mt: ID3D11Multithread = context.cast()?;            // query it from the immediate context
    let _was = unsafe { mt.SetMultithreadProtected(true) }; // FFmpeg does NOT do this for user devices [RC]
    let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)? }.cast()?;
    let luid = unsafe { adapter.GetDesc1()? }.AdapterLuid;
    Ok(GpuDevice { device, context, winrt, luid })
}
```

### 2.4 WGC session **[CK]**

```rust
pub fn min_update_interval(refresh_hz: f64, clip_fps: f64) -> Duration {          // wgc_rate.rs
    let k = (refresh_hz / (1.2 * clip_fps)).floor().max(1.0);
    Duration::from_secs_f64((k - 0.5) / refresh_hz)        // half a vblank short of k vblanks: jitter can't add one
}

let item = create_item(&target)?;  // CreateForWindow retried 10 × 100 ms on E_INVALIDARG (Qt); CreateForMonitor
let size = item.Size()?;
let fmt = if fp16 { DirectXPixelFormat::R16G16B16A16Float } else { DirectXPixelFormat::B8G8R8A8UIntNormalized };
let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&dev.winrt, fmt, 2, size)?;
let session = pool.CreateCaptureSession(&item)?;
if caps.cursor { session.SetIsCursorCaptureEnabled(cursor)?; }
if caps.border { let _ = session.SetIsBorderRequired(false); }
if caps.min_interval {
    let default: Duration = session.MinUpdateInterval()?.into();           // undocumented: log it
    tracing::info!(default_ms = default.as_secs_f64() * 1e3, "WGC MinUpdateInterval default");
    session.SetMinUpdateInterval(TimeSpan::from(min_update_interval(refresh_hz, fps)))?;
}
if caps.secondary { session.SetIncludeSecondaryWindows(false)?; }
let m = mailbox.clone();
let frame_tok = pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
    let Some(pool) = sender.as_ref() else { return Ok(()) };
    let frame = match pool.TryGetNextFrame() {
        Ok(f) => f,
        Err(e) if e.code().is_ok() => return Ok(()),   // "no frame" = Err with HRESULT(0) [RC]
        Err(e) => return Err(e),
    };
    if let Some(old) = m.slot.lock().unwrap().replace(frame) { let _ = old.Close(); } // newest wins
    m.cv.notify_one();
    Ok(())
}))?;
// item.Closed(..) sets mailbox.closed; then session.StartCapture()?
// Drop order (OBS): RemoveFrameArrived(tok), RemoveClosed(tok), pool.Close(), session.Close(), close any mailbox frame.
```

GPU-thread `consume` (under the GpuLock) **[CK]**: `ContentSize` ≤ 0 → drop; ≠ pool size → `Close()`, `pool.Recreate(&dev.winrt, fmt, 2, content)`, re-create ring textures, drop; else `SystemRelativeTime().Duration`, `frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface::<ID3D11Texture2D>()`, compare `GetDesc` (size, format) with the ring texture ("ContentSize is not reliable", OBS), `CopyResource`, `Close()`. Window resizes never re-open the encoder: the canvas is fixed per session and only the scale/letterbox rectangle changes.

### 2.5 DDA (Win10 default, Win11 fallback)

Full type-checked code: research 08 §6, `kcapcheck/src/dda.rs`.
- `DuplicateOutput1(&dev.device, 0, &[R16G16B16A16_FLOAT, B8G8R8A8_UNORM])` (BGRA only when SDR), wrapped in `SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2)`; also declare PMv2 in the manifest. Drop the old duplication first (else `E_INVALIDARG`). The device must be on the output's adapter; at most 4 duplicators.
- Per pacer tick: `ReleaseFrame()` (treat `INVALID_CALL` as benign), then `AcquireNextFrame(0, &mut info, &mut res)`. `WAIT_TIMEOUT` = no change.
- `LastPresentTime == 0` (or `AccumulatedFrames == 0`) = pointer- or metadata-only update; keep the last copy (its format can flicker) **[RC]**. `LastPresentTime` is raw QPC: convert with `ticks × 10⁷ / QPF`.
- `ACCESS_LOST` → re-duplicate at once; `E_ACCESSDENIED` (secure desktop) → retry on `EVENT_SYSTEM_DESKTOPSWITCH`; `UNSUPPORTED` → retry on `WM_DISPLAYCHANGE` or every 3 s; `NOT_CURRENTLY_AVAILABLE` → tell the user; ≥5 ACCESS_LOST/s for 3 s → "exclusive fullscreen".
- Pointer: draw it yourself when the pointer is a separate plane; `PointerShapeInfo.Type` is `u32` vs `i32` constants (compare `.0`); monochrome height is `Height/2`; position is the shape's top-left; ring textures need `BIND_RENDER_TARGET`. In FP16, linearise the SDR pointer and scale by SDR white/80.
- Surfaces are unrotated: read `DXGI_OUTDUPL_DESC.Rotation`.

### 2.6 Monitors and HDR **[CK]**

- List: `EnumDisplayMonitors` + `QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)` + `DisplayConfigGetDeviceInfo` (`GET_SOURCE_NAME` matched to `MONITORINFOEXW.szDevice`, `GET_TARGET_NAME` → `monitorFriendlyDeviceName`, `monitorDevicePath`), refresh from `targetInfo.refreshRate`.

```rust
let d = unsafe { output.cast::<IDXGIOutput6>()?.GetDesc1()? };
let hdr = d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;     // + d.MaxLuminance, d.BitsPerColor
// SDR white: DISPLAYCONFIG_SDR_WHITE_LEVEL (type GET_SDR_WHITE_LEVEL) → nits = SDRWhiteLevel / 1000 × 80
// 24H2+: hand-declared, not in windows 0.62.2 [RC]
#[repr(C)] #[derive(Clone, Copy, Default)]
pub struct AdvancedColorInfo2 { pub header: DISPLAYCONFIG_DEVICE_INFO_HEADER, pub bits: u32,
    pub color_encoding: DISPLAYCONFIG_COLOR_ENCODING /* Win32::Graphics::Gdi */, pub bits_per_color_channel: u32,
    pub active_color_mode: i32 /* 0 SDR, 1 WCG (ACM), 2 HDR */ }
pub const GET_ADVANCED_COLOR_INFO_2: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(15);
```
- DXGI cannot tell ACM/WCG SDR from plain SDR; only `active_color_mode` can. FP16 pool only when HDR is active.
- Poll `IDXGIFactory1::IsCurrent()` at 1 Hz; when false, create a **new** factory, re-read, and `Recreate` the pool with the other format. Win+Alt+B toggles HDR for tests.

### 2.7 Health, fullscreen, own windows

| Signal | Rule [E] | Result |
|---|---|---|
| No new timestamp 3 s, game foreground, not minimized | `now − last_ts > 3 s` | "possibly frozen"; with `SHQueryUserNotificationState() == QUNS_RUNNING_D3D_FULL_SCREEN`: "exclusive fullscreen, switch to borderless" |
| Minimized (`IsIconic`, `EVENT_SYSTEM_MINIMIZESTART/END`) | — | repeat the last frame, log, not "frozen" |
| Identical content 3 s / mean luma ≈ 0 for 3 s | 1 Hz 16×9 readback | "possibly frozen" / "black: DRM or capture-excluded" |
| DDA ACCESS_LOST ≥15 in 3 s | sliding window | exclusive fullscreen; retry 1/s |

- Mark ring segments `possibly_frozen`; never block a save. `SHQueryUserNotificationState` has no notifications: poll 1 Hz and on foreground changes **[RC]**.
- Engine pop-up: `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` works only on top-level windows of the calling process (acts as `WDA_MONITOR` before Win10 2004) **[RC]**. Tauri windows protect themselves (`contentProtected` → tao 0.37.1 → same call); off by default for the library window. Whether DDA honours it is **[U]**.

### 2.8 Capture gotchas

1. Pin `windows` =0.62.2; never mix 0.100 types (bridge with `as_raw`/`from_raw`).
2. `TryGetNextFrame` "no frame" is `Err` with `code() == HRESULT(0)`.
3. `CreateForWindow`, `CreateForMonitor`, `CreateDirect3D11DeviceFromDXGIDevice`, `GetInterface` are `unsafe fn` **[RC]**; `FrameArrived` closures are `Fn(Ref<..>, Ref<IInspectable>) -> Result<()> + Send + 'static` and return an `i64` token.
4. `RequestAccessAsync(..).join()` only on an MTA thread; never `.join()` on T0 (STA).
5. Two pool buffers; never hold a WGC frame across ticks.
6. Never set `MinUpdateInterval` to exactly 1/fps (≈45 fps on 60 Hz) or 0 on the 360 Hz panel.
7. `Recreate` on any `ContentSize` change; never with 0×0.
8. `D3D_DRIVER_TYPE_UNKNOWN` with an explicit adapter; `IDXGIAdapter` (not `…1`) for the parameter.
9. Elevated games: window capture may fail (0x80070424/0x80070057) or go black → monitor fallback.
10. Device removed (driver update, TDR): rebuild device, pool (`Recreate(newDevice)`), DDA, ring textures, FFmpeg contexts; start a new epoch.
11. Benchmark capture with AMD Instant Replay **off**; test coexistence separately.

---

## 3. GPU conversion and HDR

### 3.1 Locking discipline **[CK]**

```rust
pub struct GpuLock(Box<UnsafeCell<CRITICAL_SECTION>>); // boxed: must never move. Features: Win32_System_Threading + Win32_System_Kernel
unsafe impl Send for GpuLock {}
unsafe impl Sync for GpuLock {}
impl GpuLock {
    pub fn new() -> Self { let cs = Box::new(UnsafeCell::new(CRITICAL_SECTION::default())); unsafe { InitializeCriticalSection(cs.get()) }; Self(cs) }
    pub fn enter(&self) -> GpuGuard<'_> { unsafe { EnterCriticalSection(self.0.get()) }; GpuGuard(self) }
    pub fn ffmpeg_ctx(&self) -> *mut c_void { self.0.get().cast() } // AVD3D11VADeviceContext.lock_ctx (never freed by FFmpeg)
}
impl Drop for GpuLock { fn drop(&mut self) { unsafe { DeleteCriticalSection(self.0.get()) } } }
pub struct GpuGuard<'a>(&'a GpuLock);
impl Drop for GpuGuard<'_> { fn drop(&mut self) { unsafe { LeaveCriticalSection(self.0 .0.get()) } } }
pub unsafe extern "C" fn ff_lock(ctx: *mut c_void) { unsafe { EnterCriticalSection(ctx.cast()) } }   // "must be recursive": it is
pub unsafe extern "C" fn ff_unlock(ctx: *mut c_void) { unsafe { LeaveCriticalSection(ctx.cast()) } }
```

Rules:
- `SetMultithreadProtected(TRUE)` serialises single calls (covers WGC and AMF internals). The GpuLock covers *sequences*: Map/Unmap, state + draw, CopyResource batches.
- Hold the GpuLock for `consume`, Pass A/B draws and `Flush()`. **Never** across `send_frame`/`receive_packet` (vendor runtimes use the same immediate context on their own threads).
- Re-bind the full pipeline state on every draw; AMF and VPL also use the immediate context.
- The GpuLock must outlive every FFmpeg object (FFmpeg keeps `lock_ctx`).

### 3.2 Pipeline

```
capture tex (BGRA8 | RGBA16F scRGB) ─Pass A: scale (+ tone-map if HDR)→ RGBA8 intermediate at output size
                                    ─Pass B1: Y  → RTV R8_UNORM   (W×H)       on the FFmpeg pool texture data[0]
                                    ─Pass B2: UV → RTV R8G8_UNORM (W/2×H/2)   (same texture; slice data[1] = 0)
```
- Skip Pass A when SDR and sizes match. Fullscreen triangle: `IASetInputLayout(None)`, `TRIANGLELIST`, `Draw(3, 0)`. `ctx.Flush()` after Pass B (OBS does the same before AMF).
- BT.709 limited range (Kr 0.2126, Kb 0.0722), signal 1/1/1:

```
Y  =  0.182585882 R + 0.614230588 G + 0.062007059 B + 16/255
Cb = -0.100643732 R - 0.338571954 G + 0.439215686 B + 128/255
Cr =  0.439215686 R - 0.398942163 G - 0.040273524 B + 128/255
```
Pixel test (±1 code): black 16/128/128 · white 235/128/128 · 75% yellow 168/44/136 · 75% blue 28/212/120.

### 3.3 HLSL (not compiled here; the Windows `#[test]` compiles every entry point)

```hlsl
// kairo_nv12.hlsl. Portions derived from OBS Studio libobs/data/color.effect and format_conversion.effect,
// GPL-2.0-or-later, used under GPL-3.0; modified for Kairo 2026-10 (keep this notice; list in THIRD_PARTY_NOTICES).
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut VSMain(uint id : SV_VertexID) { VSOut o; float2 t = float2((id << 1) & 2, id & 2);
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1); o.uv = t; return o; }
Texture2D<float4> src : register(t0); SamplerState linear_clamp : register(s0);
cbuffer Tone : register(b0) { float scrgb_to_pq_unit; float lw_nits; float lmax_nits; float rel_scale; }; // 80/10000, Lw, Lmax, 10000/Lmax
float4 PS_ScaleSdr(VSOut i) : SV_Target { return float4(src.SampleLevel(linear_clamp, i.uv, 0).rgb, 1); }
float4 PS_ScaleHdrToSdr(VSOut i) : SV_Target {           // helpers ported from OBS color.effect (EETF: research 09 §3.4)
    float3 c = max(rec709_to_rec2020(src.SampleLevel(linear_clamp, i.uv, 0).rgb * scrgb_to_pq_unit), 0.0);
    if (lw_nits > lmax_nits) c = maxRGB_eetf_linear_to_linear(c, lw_nits, lmax_nits);
    return float4(srgb_linear_to_nonlinear(saturate(rec2020_to_rec709(c) * rel_scale)), 1);
}
cbuffer Yuv : register(b1) { float4 cy; float4 cu; float4 cv; float2 texel; float2 pad; }; // xyz = row, w = offset; texel = 1/W, 1/H
float  PS_Y (VSOut i) : SV_Target { return dot(cy.xyz, src.Load(int3(i.pos.xy, 0)).rgb) + cy.w; }
float2 PS_UV(VSOut i) : SV_Target {                      // "left" siting: [1 2 1]/4 horizontally, [1 1]/2 vertically
    float3 c = 0.5 * (src.SampleLevel(linear_clamp, float2(i.uv.x - texel.x, i.uv.y), 0).rgb + src.SampleLevel(linear_clamp, i.uv, 0).rgb);
    return float2(dot(cu.xyz, c) + cu.w, dot(cv.xyz, c) + cv.w);
}
```

Compile at engine start **[CK]**: `D3DCompile(src.as_ptr().cast(), src.len(), s!("kairo_nv12.hlsl"), None, None::<&ID3DInclude>, entry, s!("ps_5_0"), D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS, 0, &mut code, Some(&mut errs as *mut _))`. `windows` 0.62.2 links `d3dcompiler_47.dll` via raw-dylib, so cargo-xwin needs no import lib; fxc is not available on the Mac. Optional cache: `%LOCALAPPDATA%\io.github.maty3k.kairo\cache\shaders\<hash>.cso`.

### 3.4 HDR → SDR

| Curve (setting, chosen in SP3) | Formula | SDR white lands at | Highlights |
|---|---|---|---|
| **Natural** (default candidate) | BT.2390 maxRGB EETF, Lw = `MaxLuminance` (clamp 400–4000), Lmax = SDR white | ≈80% linear (240-nit white, 1000-nit peak: knee ≈114 nits) | roll off |
| Match SDR apps | `rgb × 80 / SDR_white`, clip | 100% | clip hard |
| Reinhard (OBS display-capture default **[V]**) | `DrawMultiplyTonemap` | ≈75% code value | compressed |

Tone-map in Rec.2020 and clamp, then convert to 709. The encoder stays NV12 8-bit BT.709 in v1; an HDR toggle only swaps the pool format and the Pass A shader.

### 3.5 NV12 capability, self-test, fallback

1. `CheckFormatSupport(DXGI_FORMAT_NV12)` must include `TEXTURE2D | RENDER_TARGET`; skip WARP (0x1414:0x8c).
2. Self-test at startup (OBS `HasBadNV12Output`): render 1.0 into Y and (1,1) into UV of a 128×128 NV12 texture, stage, check UV bytes ≠ 0. Log in the capability report.
3. Fallback **[U]**: video processor BGRA→NV12 into the pool texture **[CK]**:

```rust
// every D3D call below sits in an unsafe block in vp_fallback.rs
let vdev: ID3D11VideoDevice = dev.cast()?;  let vctx: ID3D11VideoContext1 = ctx.cast()?;
let en = vdev.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC { InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: rate, InputWidth: iw, InputHeight: ih, OutputFrameRate: rate, OutputWidth: ow, OutputHeight: oh,
        Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED })?;
let vp = vdev.CreateVideoProcessor(&en, 0)?;
vctx.VideoProcessorSetStreamColorSpace1(&vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
vctx.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);   // BT.709 limited
// per frame: CreateVideoProcessorInputView(src, &en, &desc, Some(&mut iv)); CreateVideoProcessorOutputView(dst, ..);
// D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: ManuallyDrop::new(iv), ..Default::default() };
// vctx.VideoProcessorBlt(&vp, ov.as_ref().unwrap(), 0, std::slice::from_ref(&stream)); then drop ManuallyDrop::into_inner(stream.pInputSurface)
```
4. Still broken → last resort: Pass B into separate `R8_UNORM` and `R8G8_UNORM` textures (always renderable), `CopyResource` to staging, `Map`, fill system-memory NV12 frames; reopen the encoder with `pix_fmt = NV12`, no `hw_frames_ctx`, and `hw_device_ctx = av_buffer_ref(<our D3D11VA device>)` so AMF stays on Kairo's adapter **[U]**. Costs ≈190 MB/s of readback at 1080p60 [E]. Never `sw_format=BGRA` into AMF.

### 3.6 GPU timing **[CK]**

`D3D11_QUERY_TIMESTAMP_DISJOINT` + two `D3D11_QUERY_TIMESTAMP` around Pass A/B. windows-rs maps `S_FALSE` ("not ready") to `Ok(())` **[RC]**, so poll through the vtable:

```rust
let hr = unsafe { (Interface::vtable(ctx).GetData)(Interface::as_raw(ctx), q.as_raw(), &mut v as *mut T as *mut _,
                  size_of::<T>() as u32, D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32) };
(hr == S_OK).then_some(v)   // S_FALSE: poll again next tick
```

---

## 4. Encode and mux

### 4.1 Bindings

- `ffmpeg-next` for the safe plumbing; `ffmpeg_next::ffi` for hwcontext, `av_opt_set`, `av_packet_ref`, `hw_frames_ctx`, disposition, log callback. ~300–500 lines of `unsafe` in `encode/ffi_hw.rs` and friends.
- Hand-mirror the D3D11VA structs (below) so no build depends on libclang seeing `<d3d11.h>`. Under cargo-xwin it does see it **[RC]**, so `ffi::AVD3D11VADeviceContext` exists too; keep the mirror anyway.
- Traps: `set_flags()` overwrites codec defaults (OR into `(*ctx).flags`); `open_with` silently drops unknown options (use `av_opt_set` and check); `Packet::clone()` deep-copies shared payloads (use `av_packet_ref`); `write_interleaved` consumes the ref you pass.

### 4.2 Device and frames contexts **[CK]**

```rust
#[repr(C)] #[allow(non_snake_case)]
pub struct AVD3D11VADeviceContext {           // FFmpeg 9.0.2 layout [RC]
    pub device: *mut c_void, pub device_context: *mut c_void, pub video_device: *mut c_void, pub video_context: *mut c_void,
    pub lock: Option<unsafe extern "C" fn(*mut c_void)>, pub unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub lock_ctx: *mut c_void, pub BindFlags: u32, pub MiscFlags: u32,
}
#[repr(C)] #[allow(non_snake_case)]
pub struct AVD3D11VAFramesContext { pub texture: *mut c_void, pub BindFlags: u32, pub MiscFlags: u32, pub texture_infos: *mut c_void }

/// device_addrefd = gpu.device.clone().into_raw() (FFmpeg Releases device/context/video_* on uninit)
pub unsafe fn d3d11va_device(device_addrefd: *mut c_void, lock_ctx: *mut c_void,
    lock: unsafe extern "C" fn(*mut c_void), unlock: unsafe extern "C" fn(*mut c_void)) -> anyhow::Result<HwRef> { unsafe {
    let mut r = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
    anyhow::ensure!(!r.is_null(), "av_hwdevice_ctx_alloc");
    let d = &mut *((*((*r).data as *mut ffi::AVHWDeviceContext)).hwctx as *mut AVD3D11VADeviceContext);
    d.device = device_addrefd;
    (d.lock, d.unlock, d.lock_ctx) = (Some(lock), Some(unlock), lock_ctx);   // else a private CreateMutex lock
    if let Err(e) = check(ffi::av_hwdevice_ctx_init(r)) { ffi::av_buffer_unref(&mut r); return Err(e); }
    Ok(HwRef(r))
}}

pub unsafe fn nv12_frames(dev: &HwRef, w: i32, h: i32) -> anyhow::Result<HwRef> { unsafe {
    let mut r = ffi::av_hwframe_ctx_alloc(dev.0);
    anyhow::ensure!(!r.is_null(), "av_hwframe_ctx_alloc");
    let fc = &mut *((*r).data as *mut ffi::AVHWFramesContext);
    fc.format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
    fc.sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
    (fc.width, fc.height) = (w, h);              // even
    fc.initial_pool_size = 0;                    // dynamic pool: one texture per buffer (QSV maps it; arrays break QSV [RC])
    (*(fc.hwctx as *mut AVD3D11VAFramesContext)).BindFlags = 0x20; // D3D11_BIND_RENDER_TARGET: our shader writes the planes
    if let Err(e) = check(ffi::av_hwframe_ctx_init(r)) { ffi::av_buffer_unref(&mut r); return Err(e); }
    Ok(HwRef(r))
}}
// per tick: av_hwframe_get_buffer(frames, f.as_mut_ptr(), 0); data[0] = ID3D11Texture2D* (borrow with
// ID3D11Texture2D::from_raw_borrowed), data[1] = slice (0); RTVs R8_UNORM / R8G8_UNORM; draw; (*f).pts = tick.
```
`HwRef` = `AVBufferRef*` newtype with `Drop → av_buffer_unref` and `unsafe impl Send`. VRAM ≈ peak in-flight frames × 3.1 MB (1080p NV12) [E]; `async_depth=4` bounds it.

### 4.3 Encoders (FFmpeg 9.0.2)

| Encoder | Input | Setup | Watch out |
|---|---|---|---|
| `h264_amf` (default on AMD) / `hevc_amf` / `av1_amf` | D3D11 frames directly (AMF device derived, `InitDX11(device, AMF_DX11_1)`) | `pix_fmt=D3D11`, `hw_frames_ctx = av_buffer_ref(frames)` | **`av_assert0`: frames from any other ctx abort the process [RC]**; `async_depth` default 16; `receive_packet` may `Sleep(1)` when the queue is full |
| `h264_nvenc` (CI GPU runner, friends) | D3D11 frames, ≤64 registrations cached | same | BtbN 9.0 builds need driver ≥610; `GLOBAL_HEADER` ⇒ no in-band SPS/PPS |
| `h264_qsv` (experimental) | `AV_PIX_FMT_QSV` (or system NV12) **[RC]** | derived QSV device by adapter LUID, `av_hwframe_ctx_create_derived`, `av_hwframe_map(READ \| DIRECT)` per frame | dynamic pool only; 16-aligned surface Info **[U]** for 1080 |
| `h264_mf` (fallback) | D3D11 frames | `hw_encoding=1` (in practice required for D3D input **[RC]**), `rate_control=pc_vbr`, `scenario=camera_record` | first hardware MFT, no LUID filter; writes no colour VUI |

### 4.4 Opening and driving the encoder **[CK]**

```rust
pub const H264_AMF: &[(&str, &str)] = &[
    ("usage", "transcoding"), ("quality", "balanced"), ("rc", "vbr_peak"), ("vbaq", "1"),
    ("preanalysis", "0"), ("high_motion_quality_boost_enable", "0"), ("preencode", "0"),
    ("bf", "0"), ("frame_skipping", "0"), ("filler_data", "0"), ("profile", "high"), ("coder", "cabac"),
    ("header_spacing", "60"), ("async_depth", "4"),
];

pub unsafe fn open_video(name: &str, cfg: &VideoCfg, frames: &HwRef, opts: &[(&str, &str)]) -> anyhow::Result<ff::encoder::video::Encoder> { unsafe {
    let codec = ff::encoder::find_by_name(name).ok_or_else(|| anyhow::anyhow!("{name} not in this FFmpeg build"))?;
    let mut v = ff::codec::Context::new_with_codec(codec).encoder().video()?;   // applies codec defaults (+loop, g=-1)
    v.set_width(cfg.width); v.set_height(cfg.height); v.set_format(ff::format::Pixel::D3D11);
    v.set_time_base((1, cfg.fps)); v.set_frame_rate(Some((cfg.fps, 1)));
    v.set_gop(cfg.fps as u32);   // 1 s; AMF sets IDR_PERIOD only if gop_size != -1 (AMF default 240) [RC]
    v.set_max_b_frames(0);       // NVENC/QSV default -1 = driver decides
    v.set_bit_rate(cfg.bitrate); v.set_max_bit_rate(cfg.maxrate);
    v.set_color_range(ff::color::Range::MPEG); v.set_colorspace(ff::color::Space::BT709);
    v.set_color_primaries(ff::color::Primaries::BT709);
    v.set_color_transfer_characteristic(ff::color::TransferCharacteristic::BT709);   // all three ⇒ MP4 colr nclx
    let c = v.as_mut_ptr();
    (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;   // OR: set_flags() would drop "+loop"/"+cgop"
    (*c).rc_buffer_size = cfg.maxrate as i32;
    (*c).hw_frames_ctx = ffi::av_buffer_ref(frames.0);
    for (k, val) in opts {
        let (ck, cv) = (CString::new(*k)?, CString::new(*val)?);
        let r = ffi::av_opt_set((*c).priv_data, ck.as_ptr(), cv.as_ptr(), 0);
        anyhow::ensure!(r >= 0, "{name}: option {k}={val} rejected: {}", ff::Error::from(r));
    }
    Ok(v.open_as(codec)?)
}}

/// AMF/NVENC/QSV submit inside receive_packet: always drain after send_frame.
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
/// Encoder thread fed by a 2–4 slot channel; the pacer never calls send_frame itself.
pub fn encode_thread(mut enc: ff::encoder::video::Encoder, rx: std::sync::mpsc::Receiver<ff::frame::Video>,
                     mut out: impl FnMut(ff::Packet)) -> anyhow::Result<()> {
    for f in rx { enc.send_frame(&f)?; drain(&mut enc, &mut out)?; }
    enc.send_eof()?;
    drain(&mut enc, &mut out)
}
```

- Read back options (`av_opt_get` + `av_free`) and log them; record `av_version_info()`, `avcodec_configuration()`, `AVAMFDeviceContext.version` (not the `amfrt64.dll` file version).
- Session parameters for muxing: `ff::codec::Parameters::from(&enc)` right after open (extradata + colour), stored per epoch.
- Probe: open, send ~5 rendered frames, drain; require a KEY packet and `extradata_size > 0`. `avcodec_open2` alone proves nothing.
- `AV_PKT_FLAG_KEY` = IDR only (AMF H.264/HEVC, NVENC, QSV); index the ring by it.

### 4.5 GOP and headers

| | IDR at GOP start | Closed GOP | In-band SPS/PPS with GLOBAL_HEADER | Extradata after open |
|---|---|---|---|---|
| AMF H.264 | yes, once `gop_size` set | yes | with `header_spacing` | always |
| AMF HEVC | `gops_per_idr=1` | yes | `header_insertion_mode=idr` | always |
| NVENC | `idrPeriod = gopLength` | yes | no | yes |
| QSV | `idr_interval=0` | `+cgop` default | yes **[U]** | always |
| MF | MFT-dependent **[U]** | MFT-dependent | MFT-dependent | ≤70 ms after first frame |

MP4 needs only extradata (`avcC`). A future TS disk ring needs `dump_extra` (freq=keyframe).

### 4.6 AAC **[CK]**

```rust
let codec = ff::encoder::find_by_name("aac").ok_or_else(|| anyhow::anyhow!("aac missing"))?; // native, LGPL; aac_mf per SP7 ADR
let mut a = ff::codec::Context::new_with_codec(codec).encoder().audio()?;
a.set_rate(48_000); a.set_channel_layout(ff::ChannelLayout::STEREO);           // mic track: MONO
a.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
a.set_bit_rate(160_000); a.set_time_base((1, 48_000));                         // default bit rate is 0: always set
unsafe { (*a.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32; }
let aac = a.open_as(codec)?;
```
1024-sample frames, `initial_padding = 1024` (first packet pts = input − 1024; never subtract again). 160 kbps stereo / 128 kbps mono; `aac_coder=fast` if CPU is high.

### 4.7 Mux **[CK]**

```rust
pub fn write_clip(tmp: &Path, streams: &[OutStream], packets: &[SnapPacket], meta: ff::Dictionary) -> anyhow::Result<()> {
    let mut octx = ff::format::output_as(tmp, "mp4")?;     // real file path: faststart re-opens it [RC]
    for s in streams {
        let mut st = octx.add_stream(ff::codec::Id::None)?;
        st.set_parameters(s.params.clone());               // epoch codecpar (extradata, colour)
        st.set_time_base(s.tb);                            // hint; movenc rewrites it
        let mut md = ff::Dictionary::new(); md.set("title", s.title); md.set("handler_name", s.title); md.set("language", "und");
        st.set_metadata(md);
        unsafe {
            (*(*st.as_mut_ptr()).codecpar).codec_tag = 0;
            (*st.as_mut_ptr()).disposition = if s.default { ffi::AV_DISPOSITION_DEFAULT as i32 } else { 0 }; // mix only [RC]
        }
    }
    octx.set_metadata(meta);
    let mut o = ff::Dictionary::new(); o.set("movflags", "+faststart"); o.set("use_editlist", "0");
    let unused = octx.write_header_with(o)?.iter().count();   // drop the borrowing Dictionary right here
    anyhow::ensure!(unused == 0, "unused muxer options");
    let tbs: Vec<ff::Rational> = octx.streams().map(|s| s.time_base()).collect(); // AFTER the header: 60 fps → 1/15360
    for p in packets {                                        // global dts order (av_compare_ts across time bases)
        let mut pkt = ff::Packet::empty();
        unsafe { check(ffi::av_packet_ref(pkt.as_mut_ptr(), p.pkt.as_ptr()))?; }
        pkt.set_stream(p.out_index);
        pkt.set_pts(Some(p.pts)); pkt.set_dts(Some(p.dts)); pkt.set_duration(p.duration);
        pkt.rescale_ts(p.tb, tbs[p.out_index]);
        pkt.write_interleaved(&mut octx)?;
    }
    octx.write_trailer()?;                                    // runs the faststart shift
    Ok(())                                                    // then File::sync_all + std::fs::rename
}
```
- Metadata: `title` (©nam), `comment` "Captured with Kairo", `encoding_tool` "Kairo x.y.z" (replaces Lavf), `description` = JSON mirror of the library row, `date`, `creation_time`. Do not set `+use_metadata_tags`.
- Track titles: "Mix", "Game", "Mic", "Voice chat". Share copies: mix only.

### 4.8 Build and link

| Where | Steps |
|---|---|
| PC (native) | `winget install LLVM.LLVM`; `setx LIBCLANG_PATH "C:\Program Files\LLVM\bin"`; unzip the pinned BtbN zip to `C:\deps\ffmpeg-9.0-lgpl-shared`; `setx FFMPEG_DIR …`; `cargo build --locked`; copy the four DLLs next to `kairo-engine.exe` (never rely on PATH) |
| Mac (check) | `brew install llvm`; `LIBCLANG_PATH=$(brew --prefix llvm)/lib`; copy `include/` + `lib/` of the same zip to `~/kairo-deps/ffmpeg-9.0-lgpl-shared`; `FFMPEG_DIR=… cargo xwin check --target x86_64-pc-windows-msvc -p kairo-engine` **[U end to end; M0 exit 5]** |
| Fallback | hand mirrors (already) or a vendored `ffmpeg-sys-next` reading bindings generated on the PC |

LGPL checklist: no `--enable-gpl/nonfree`; dynamic DLLs with standard names; exact source (FFmpeg at 6dfaba896f, configure line, BtbN scripts, bundled libs) next to the binaries; About box "uses libraries from the FFmpeg project under the LGPLv3"; never refuse modified DLLs. Own trimmed build in M11 (nv-codec-headers 12.1.14.0 → drivers ≥531.61).

---

## 5. Replay ring and save

### 5.1 Model

- **One clock.** Engine epoch `E` = QPC (100 ns) at media start. Video pts = pacer tick `k` (time base 1/fps); audio pts = 48 kHz sample index (time base 1/48000); fps ∈ {30, 60, 120} so a tick is a whole number of samples (800 at 60 fps).
- **Epochs.** A new encoder session (target change, resize policy, HDR swap if it reopens, device loss, stall recovery) starts an epoch with its own `codec::Parameters`. Ticks keep counting. A save never spans epochs.
- **Retention.** `keep_ticks` = longest binding + post-roll + 1 GOP; byte cap = peak bitrate × keep × 1.1; always ≥2 GOPs; pinned GOPs survive.

| Preset | 30 s at target / peak | 2 min at target / peak |
|---|---|---|
| 1080p60 H.264 25/40 Mbps | 94 / 150 MB | 375 / 600 MB |
| 1440p60 H.264 36/50 Mbps | 135 / 188 MB | 540 / 750 MB |
| Audio, 4 tracks (mix 160 + game 160 + mic 128 + voice 160 kbps) | 2.3 MB | 9.1 MB |

### 5.2 Ring and cut (kairo-core) **[CK, tested]**

```rust
pub struct Pkt<P> { pub pts: i64, pub dts: i64, pub dur: i64, pub key: bool, pub epoch: u32, pub bytes: usize, pub payload: P }
pub struct CutPlan { pub epoch: u32, pub start_pts: i64, pub end_pts: i64, pub a0: i64, pub audio_first: Vec<Option<i64>>, pub truncated: bool }

impl<P: Clone> Ring<P> {
    /// Drop whole GOPs from the front while older than keep_ticks or over the byte cap; keep ≥2 GOPs and pins.
    fn evict(&mut self) {
        let Some(newest) = self.video.back().map(|p| p.pts) else { return };
        loop {
            let keys: Vec<usize> = self.video.iter().enumerate().filter(|(_, p)| p.key).map(|(i, _)| i).take(3).collect();
            if keys.len() < 3 { break; }
            let next = &self.video[keys[1]];
            if !(newest - next.pts >= self.keep_ticks || self.bytes > self.cap_bytes) { break; }
            if self.pins.values().any(|&pin| pin < next.pts) { break; }
            let cut_pts = next.pts;
            for p in self.video.drain(..keys[1]) { self.bytes -= p.bytes; }
            let a_keep = cut_pts * (48_000 / self.fps) - 2 * 1024;            // keep 2 AAC packets of pre-roll
            for t in &mut self.audio { while t.front().is_some_and(|p| p.pts + p.dur <= a_keep) { self.bytes -= t.pop_front().unwrap().bytes; } }
        }
    }
    /// `t` = hotkey tick (QPC from T0). If the newest epoch began after t − pre, start at its first IDR (truncated).
    pub fn plan_cut(&self, t: i64, pre_ticks: i64, post_ticks: i64) -> Option<CutPlan> {
        let epoch = self.video.back()?.epoch;
        let in_epoch = || self.video.iter().filter(|p| p.key && p.epoch == epoch);
        let (k, truncated) = match in_epoch().filter(|p| p.pts <= t - pre_ticks).last() {
            Some(k) => (k, false), None => (in_epoch().next()?, true) };
        let a0 = k.pts * (48_000 / self.fps);                                 // the IDR's exact sample time
        Some(CutPlan { epoch, start_pts: k.pts, end_pts: t + post_ticks, a0,
                       audio_first: self.audio.iter().map(|tr| audio_start(tr, a0)).collect(), truncated })
    }
}
/// The packet covering a0 plus one pre-roll packet: the AAC warm-up frame lands before the IDR.
pub fn audio_start<P>(track: &VecDeque<Pkt<P>>, a0: i64) -> Option<i64> {
    let i = track.iter().rposition(|p| p.pts <= a0)?;
    Some(track[i.saturating_sub(1)].pts)
}
```
Tests that pass (`corecheck`): GOP-aligned eviction keeps 31–32 s for a 31 s window; the cut starts on an IDR ≤ t − 30 s; the first audio packet starts 1024–2047 samples before the IDR on every track; epochs are never spanned; pins block eviction and the byte cap leaves ≥2 GOPs.

### 5.3 Save sequence

1. T0 stamps QPC on `WM_HOTKEY` → `clip.save {qpc, lengthS, origin}` → media converts to a tick (`round((qpc − E) × fps / 10⁷)`). IPC latency never moves the clip.
2. `plan_cut`, `pin(save, start_pts)`, play the cue.
3. Wait until video has passed `end_pts` and the audio mixer has passed `end_pts × (48000/fps) + 1024` samples (≤ ~250 ms; the mixer runs 200 ms behind).
4. Snapshot under the ring lock: `av_packet_ref` of video `[K, end]` and each audio track `[audio_first, end]`; copy the epoch's `Parameters`. Unpin.
5. Rebase: video `pts' = pts − K.pts` (duration 1 tick); audio `pts' = pts − a0` (first packet negative, second ≤ 0; duration 1024).
6. Save thread (below normal): `write_clip` to `<library>\<Game>\.kairo-<ulid>.mp4.tmp` → `File::sync_all` → `std::fs::rename` to `YYYY-MM-DD_HH-MM-SS_<Game>.mp4`.
7. Verify in-process (`avformat_open_input` + `find_stream_info`): duration, IDR at 0, track count, moov before mdat. On failure retry once from the snapshot; then error cue + `clip.failed`.
8. Poster from the first IDR → DB row → `clip.saved`.

What step 5 does in the file (`use_editlist=0` ⇒ `avoid_negative_ts` auto → make_zero **[RC]**): every stream shifts by the earliest dts (the audio pre-roll), movenc moves the video track's first sample to 0 and lengthens it (movenc.c:7282-7290), so video frame 0 shows for ≤42.7 ms longer and every later frame is exactly in sync with audio. No `elst` is written. Audio from the IDR on decodes bit-exactly (research 10's experiment). **[U]** confirm with testbed flash/beep: |offset| < 10 ms.

### 5.4 Checks (kairo-verify / ffprobe)

```powershell
ffprobe -v error -show_entries stream=index,codec_name,profile,pix_fmt,color_range,color_space,color_transfer,color_primaries,start_time:stream_disposition=default:stream_tags=title -of json clip.mp4
#  h264 High, yuv420p, tv/bt709/bt709/bt709; a:0 (Mix) default=1, others 0; AAC LC 48 kHz
ffprobe -v error -select_streams v:0 -show_entries packet=pts,dts,flags -of csv=p=0 clip.mp4   # K every 60, pts==dts
ffprobe -v trace clip.mp4 2>&1 | Select-String "type:'(ftyp|moov|mdat|colr|elst)'"              # ftyp, moov(colr), mdat; no elst
```

---

## 6. Audio

### 6.1 Sources (plan §2.3)

| Track | Source | Mode |
|---|---|---|
| T1 Mix (default) | Σ gain × enabled stems → limiter −1 dBFS | — |
| T2 Game | process loopback, include-tree on the game's root (walk up within its install dir, stop at launchers) + extra clients for pinned helpers | `LoopbackMode::IncludeTree` |
| T3 Mic | console-role default capture device, mono, never auto-enabled on Bluetooth (HFP) | endpoint capture |
| T4 Voice (off, consent) | include-tree on root `Discord.exe` (+ editable list) | `IncludeTree` |
| T5 Everything else | Plan A per-process clients; plan B exclude-tree on control (UI must be its child, P22); Win10: endpoint loopback | `ExcludeTree` |

### 6.2 Capture thread **[CK]**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopbackMode { IncludeTree, ExcludeTree }
/// wasapi 0.26.0: include_tree=false means EXCLUDE (its doc comment is wrong) [RC]. Blocks on a Condvar with
/// no timeout: the watchdog abandons the thread after 3 s.
pub fn open_process_loopback(pid: u32, mode: LoopbackMode) -> Result<wasapi::AudioClient, wasapi::WasapiError> {
    wasapi::AudioClient::new_application_loopback_client(pid, matches!(mode, LoopbackMode::IncludeTree))
}

pub fn capture_main(spec: SourceSpec, ch: usize, pcm: &mut rtrb::Producer<f32>, meta: &mut rtrb::Producer<PacketMeta>,
                    shutdown: HANDLE, ready: &crossbeam_channel::Sender<Result<(), String>>) -> anyhow::Result<()> {
    wasapi::initialize_mta().ok()?;                        // caller runs wasapi::deinitialize() after this returns
    let mut task = 0u32;
    let _mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task) }.ok().map(Mmcss); // never "Pro Audio"
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, 48_000, ch, None);
    let is_mic = matches!(spec, SourceSpec::Endpoint { is_mic: true, .. });
    let mut client = open_source(&spec)?; // open_process_loopback(pid, mode), or DeviceEnumerator::new()?
                                          //   .get_default_device_for_role(&dir, &Role::Console)?.get_iaudioclient()?
    client.initialize_client(&fmt, &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })?;
    if is_mic { mic_no_ducking(&client); }                 // before start: the opt-out applies only after stop/start [RC]
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    let mut buf = vec![0f32; 48_000 * ch];                 // 1 s: a short slice makes read_from_device drop the packet
    client.start_stream()?;
    let _ = ready.send(Ok(()));
    let handles = [event.as_handle(), shutdown];
    let mut first = true;
    loop {
        if unsafe { WaitForMultipleObjects(&handles, false, 20) }.0 == WAIT_OBJECT_0.0 + 1 { break; }
        while let Some(n) = capture.get_next_packet_size()? {
            if n == 0 { break; }
            let (frames, info) = capture.read_from_device(bytemuck::cast_slice_mut::<f32, u8>(&mut buf))?;
            if frames == 0 { break; }
            let samples = frames as usize * ch;
            if info.flags.silent { buf[..samples].fill(0.0); }               // SILENT: zero it yourself
            let mut flags = 0u8;
            if info.flags.data_discontinuity && !(first && info.index == 0) { flags |= F_DISC; }
            if info.flags.timestamp_error { flags |= F_TSERR; }              // count; never restart (OBS 0f498133a)
            first = false;
            let m = PacketMeta { ts_hns: info.timestamp, arrival_hns: qpc::now_hns(), device_pos: info.index, frames, flags };
            if pcm.push_entire_slice(&buf[..samples]).is_ok() { let _ = meta.push(m); }
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

fn mic_no_ducking(client: &wasapi::AudioClient) {      // process-loopback clients have no session control (E_NOINTERFACE) [RC]
    if let Ok(sc) = client.get_audiosessioncontrol() { let _ = sc.set_ducking_preference(true); }
    unsafe { if let Ok(dc) = client.as_iaudioclient().GetService::<IAudioClientDuckingControl>() {
        let _ = dc.SetDuckingOptionsForCurrentStream(AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS); } }
}
```

| Rule | Detail |
|---|---|
| Flags reaching `Initialize` | `EVENTCALLBACK \| AUTOCONVERTPCM \| SRC_DEFAULT_QUALITY` (+ `LOOPBACK` for render/process clients with `Direction::Capture`); both durations 0 (Microsoft) |
| Not implemented on process loopback | `get_mixformat`, `is_supported`, `get_device_period`, `get_current_padding`; `get_audiosessioncontrol` → E_NOINTERFACE; `get_next_packet_size` works |
| Threading | every `wasapi`/COM object is `!Send`: create, use, drop on its capture thread; `deinitialize()` last |
| Never | `read_from_device_to_deque` (unwraps `ReleaseBuffer`, allocates); restart on `TIMESTAMP_ERROR` |
| Errors | `AUDCLNT_E_DEVICE_INVALIDATED`/`RESOURCES_INVALIDATED`/`SERVICE_NOT_RUNNING` → reconnect 0.25 → 1 → 3 → 10 s; `DEVICE_IN_USE` → "blocked (exclusive)" retry 30 s; `E_ACCESSDENIED` → privacy **[U]** |
| Logging | `wasapi` logs via `log` at trace per packet: bridge with `tracing-log`, filter `wasapi=warn` |

Device notifications **[CK]**: `DeviceEnumerator::register_notification_callback(DeviceEventCallbacks)` with `set_default_device_callback(Fn(Direction, Role, Option<String>))`, `set_device_removed_callback(Fn(String))`, `set_device_state_callback(Fn(String, DeviceState))`; closures only `try_send`; keep the `DeviceEventRegistration` on a long-lived MTA thread (its `Drop` unregisters and can wait on a running callback); expect three callbacks per change (one per role), filter and debounce 300 ms.

### 6.3 Process trees

- Snapshot: `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)` + `Process32FirstW/NextW`. Parent PIDs can be reused: walk up only while `GetProcessTimes` creation(parent) ≤ creation(child), with `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`; stop at launchers (`steam.exe`, `epicgameslauncher.exe`, `battle.net.exe`, `riotclientservices.exe`, `explorer.exe`, shells, `svchost.exe`).
- Game `same_app`: parent image under the same install directory. Discord: same image name (`Discord.exe`, PTB, Canary).
- Pin targets with `OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)` and poll `WaitForSingleObject(h, 0)`; fall back to the 2 s snapshot if access is denied (anti-cheat **[U]**).

### 6.4 Timeline, aligner, mixer (kairo-core)

- Timeline index `s = (t_100ns − E) × 48000 / 10⁷` in i128. Device QPC for loopback; for mics compare with `arrival − packet duration` and switch to arrival time if they disagree >50 ms persistently [E].
- `SourceAligner` **[CK, tested]**: rubato `Async::<f32>::new_poly(1.0, 1.01, PolynomialDegree::Septic, 480, ch, FixedAsync::Input)`; PI loop on placement error (`ALPHA` 0.02, τ 15 s, `KP = 2/(τ·48000)`, `KI = KP/τ`, innovation clip ±96 samples, ±1000 ppm, `set_resample_ratio_relative(r, true)`); hard re-anchor on discontinuity or >50 ms error; `TIMESTAMP_ERROR` packets are placed by prediction only. Test: a 100 ppm fast source with ±1.5 ms jitter locks to within 20 ppm and <0.5 ms after 120 s. Full code: `corecheck/src/aligner.rs` (= research 10 §5.4, compiled).
- Mixer every 10 ms of QPC, `D` = 200 ms behind real time; gains ramped per block; limiter on the mix only (−1 dBFS, 5 ms look-ahead, 100 ms release, sliding-min + box filter); stems hard-clipped to ±1; meters at 20 Hz (`audio.levels` only while subscribed).
- AAC grid: all tracks start at the same `G` (multiple of 1024), gapless 1024-frame planar frames, `pts` = timeline index.

### 6.5 Watchdog and OS policy

| Signal | Action |
|---|---|
| Activation not `ready` within 3 s | abandon the thread (parked forever, logged), retry later |
| No packets >2 s while a Windows session meter of the target tree is >−50 dBFS for 1 s | restart via A/B swap |
| Captured stem all-zero >5 s while the session meter is active | same |
| Target exited (handle signalled) | stop; "waiting for game" |
| Proactive refresh due | A/B swap while the stem is quiet: start B, wait until locked with >200 ms, crossfade 20 ms at `mixer_pos + 50 ms`, drop A |

| Windows build | Game/voice | Everything else | Refresh |
|---|---|---|---|
| ≥26100 (24H2+) | process loopback | exclude-tree on control | 60 min (off if the 8 h soak is clean) |
| 22000–22631 | process loopback | exclude-tree or endpoint (soak decides) | 10 min |
| 19041–19045 (Win10) | process loopback, "best effort" | endpoint loopback | 10 min |

Activation success is not proof of audio on old Win10: play a −30 dBFS tone in Kairo's own process for 300 ms and require it in an include-tree(self) capture (the "per-app audio works" capability bit).

---

## 7. Engine shell

### 7.1 Processes and threads

```
HKCU Run "Kairo" (quoted) ─► kairo-engine.exe  CONTROL (unelevated, interactive session)
   T0 shell (STA): hidden top-level tool window · tray-icon + muda · RegisterHotKey · WinEvent hook · power/WTS/endsession · cue · pop-up
   tokio (2 workers): core actor · pipe server \\.\pipe\kairo-<SID> · media link (stdio) · export jobs
   detect (MTA) · db (sole rusqlite owner) · crashsrv (minidumper::Server) · mouse poll (only while bound)
   Job Object (KILL_ON_JOB_CLOSE | DIE_ON_UNHANDLED_EXCEPTION | BREAKAWAY_OK) ─► kairo-engine.exe --media
       capture (pool worker) → GPU thread (MTA) → encoder thread → ring ← audio-cap-* (MMCSS "Audio") → mixer → AAC
       save thread (below normal) · link thread (stdin/stdout JSON-RPC; media never logs to stdout)
   spawns Kairo.exe --from-engine outside the job (P22)
```
M3 runs the pipeline in-process behind a `Pipeline` trait; M5 splits media out (plan).

| Cross-thread rule | Why |
|---|---|
| Wake T0 with `PostMessageW(hwnd, WM_APP_WAKE)` + a crossbeam queue, never `PostThreadMessageW` | thread messages are lost in modal loops (tray menu) |
| Shell window is top-level `WS_EX_TOOLWINDOW \| WS_EX_NOACTIVATE`, never `HWND_MESSAGE` | message-only windows miss TaskbarCreated, `WM_ENDSESSION`, power and display broadcasts |
| `RegisterHotKey`, `ShutdownBlockReasonCreate` on T0 only | they require the window's creating thread |
| `catch_unwind` in every `WNDPROC`/`WINEVENTPROC`; `try_borrow_mut` on thread-local state | a panic across `extern "system"` is a `__fastfail` with no dump; window procs re-enter |
| Encoders never on T0 | `h264_mf`/`aac_mf` fail on STA threads |

Messages on T0: `WM_HOTKEY` (stamp QPC first) · TaskbarCreated (tray-icon re-adds) · `WM_WTSSESSION_CHANGE` (lock → pause; `WTSRegisterSessionNotification` may fail with `RPC_S_INVALID_BINDING` right after logon: retry) · `WM_POWERBROADCAST` (decode `lParam` synchronously) · `WM_QUERYENDSESSION`/`WM_ENDSESSION` (`ShutdownBlockReasonCreate` while a save runs) · `WM_DISPLAYCHANGE` (media re-evaluates monitors).

### 7.2 Hotkeys, mouse, elevation **[CK]**

```rust
pub fn set(hwnd: HWND, id: i32, b: Binding, was_registered: bool) -> HotkeyStatus {
    if !(0..=0xBFFF).contains(&id) { return HotkeyStatus::Invalid; }
    if b.vk == VK_F12.0 || b.win { return HotkeyStatus::Reserved; }
    if was_registered { let _ = unsafe { UnregisterHotKey(Some(hwnd), id) }; }   // same (hwnd,id) stacks otherwise
    let mut mods = MOD_NOREPEAT;
    if b.ctrl { mods |= MOD_CONTROL; } if b.alt { mods |= MOD_ALT; } if b.shift { mods |= MOD_SHIFT; }
    match unsafe { RegisterHotKey(Some(hwnd), id, mods, b.vk as u32) } {
        Ok(()) => HotkeyStatus::Registered,
        Err(e) if e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() => HotkeyStatus::InUse,   // 1409
        Err(e) => HotkeyStatus::Failed { hresult: e.code().0 },
    }
}
```
- On a failed change, re-register the old binding. Display names: `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` → `GetKeyNameTextW`. AMD's in-game hook means `RegisterHotKey` success does not prove there is no clash: parse `HKLM\SOFTWARE\AMD\DVR\*Hotkey` (names **[U]**, SP9).
- Mouse side buttons (opt-in): 8 ms `CreateWaitableTimerExW(.., CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)` + `SetWaitableTimer(timer, &-80_000, 8, ..)`; `GetAsyncKeyState(VK_XBUTTON1/2) & 0x8000` only (0 under UIPI); ignored while a Kairo window is in front.
- Elevation: `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken(TOKEN_QUERY)` → `GetTokenInformation(TokenElevation)`; access denied = Unknown, not Elevated. Cache per PID.

### 7.3 Game detection

- T0: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(cb), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)` (feature `Win32_UI_Accessibility`); the callback only `try_send`s `hwnd.0 as isize` when `idObject == 0`. Also `EVENT_SYSTEM_MINIMIZEEND`.
- Detect thread: PID → `QueryFullProcessImageNameW` → elevation → longest install-dir prefix match; Toolhelp poll every 3 s for exits.
- Index: Steam (`steamlocate` 2.1.1), Epic `*.item` manifests (skip invalid JSON), GOG and Ubisoft registry, Battle.net uninstall entries, Xbox via `PackageManager::FindPackagesByUserSecurityIdWithPackageTypes(&HSTRING::new(), PackageTypes::Main)` (windows-rs name for `FindPackagesForUser`; empty SID = current user **[RC]**) + `MicrosoftGame.config` (roxmltree), curated `data/games.json`, user marks. Launchers are never games.
- Icons: `SHDefExtractIconW(path, 0, 0, Some(&mut large), None, 256 | (16 << 16))` → WIC `CreateBitmapFromHICON` → PNG; `DestroyIcon`.

### 7.4 IPC **[CK]**

```rust
pub fn pipe_name(sid: &str) -> String { format!(r"\\.\pipe\kairo-{sid}") }       // SID: OpenProcessToken → TokenUser → ConvertSidToStringSidW
// DACL: D:P(A;;GA;;;SY)(A;;GA;;;<user SID>) via ConvertStringSecurityDescriptorToSecurityDescriptorW (the default lets Everyone read)
fn instance(name: &str, sec: &PipeSecurity, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sec.0 .0, bInheritHandle: false.into() };
    let mut o = ServerOptions::new();
    o.first_pipe_instance(first)      // default false [RC]: set it so a squatter makes startup fail (ERROR_ACCESS_DENIED)
     .reject_remote_clients(true)     // default true; keep explicit
     .max_instances(8);               // identical on every instance
    unsafe { o.create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut core::ffi::c_void) }
}
pub async fn serve(name: String, sec: PipeSecurity, on_conn: impl Fn(NamedPipeServer) + Send + 'static) -> std::io::Result<()> {
    let mut server = instance(&name, &sec, true)?;
    loop {
        server.connect().await?;
        let connected = std::mem::replace(&mut server, instance(&name, &sec, false)?); // next instance BEFORE hand-off
        on_conn(connected);
    }
}
pub async fn connect(name: &str) -> std::io::Result<NamedPipeClient> {          // UI shell, kairo-cli
    loop {
        match ClientOptions::new().open(name) {        // default SECURITY_IDENTIFICATION | SECURITY_SQOS_PRESENT
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {}
            Err(e) => return Err(e),                     // NotFound ⇒ engine not running
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
pub fn codec() -> LengthDelimitedCodec {               // u32 big-endian = StreamJsonRpc LengthHeaderMessageHandler
    LengthDelimitedCodec::builder().length_field_type::<u32>().max_frame_length(1 << 20).new_codec()
}
```

- User SID, not logon SID, on purpose: the same user's SSH session (session 0) runs `kairo-cli`. Don't add a deny ACE for NETWORK **[U]**. The Mac mock uses `tokio::net::UnixListener` in `std::env::temp_dir()` (per user).
- Protocol: one JSON object per frame, no batches (`-32600`). `engine.hello` first (`protocol {major, minor}`, `features[]`, `safeMode`); major mismatch → `-32002`. Methods, notifications and errors `-32001…-32012`: plan §2.1 table. Long operations return `{jobId}`. Per-session outbound queue 256; `buffer.status` and `audio.levels` keep only the latest value; responses always wait.
- Media link: `tokio::process::Command` with piped stdin/stdout, `kill_on_drop(true)`, framed with `tokio::io::join(stdout, stdin)` and the same codec; assign the child to the Job Object. Heartbeat 1 s with progress counters; 10 s silence → external dump (`MiniDumpWriteDump`) + `TerminateProcess` + restart.
- ts-rs: `TS_RS_LARGE_INT=number`; keep ints < 2^53. CI: `cargo test -p kairo-rpc && git diff --exit-code apps/ui/src/gen`.

### 7.5 Logging **[CK]**

```rust
let rot = FileRotate::new(dir.join(format!("{process}.log")), AppendCount::new(10),
                          ContentLimit::BytesSurpassed(10 << 20), Compression::None, None);
let (w, guard) = tracing_appender::non_blocking(rot);        // lossy: a full buffer drops lines, never blocks capture
tracing_subscriber::registry()
    .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,kairo=debug,wasapi=warn")))
    .with(fmt::layer().json().with_writer(w))               // wrap w in the redacting MakeWriter (one event per write)
    .init();                                                // keep `guard` alive until WM_ENDSESSION
```
One file family per process (`control`, `media`, `ui`) under `%LOCALAPPDATA%\io.github.maty3k.kairo\logs\`. Redact `C:\Users\<name>`, user and host names, library root and SIDs; never window titles, typed keys or clipboard. FFmpeg logs: `av_log_set_callback` → `av_log_format_line2` → tracing (`ffi::va_list` compiled on macOS; **[U]** on MSVC). Rate-limit repeated warnings to 1/s per site.

### 7.6 Crashes **[CK]**

```rust
pub fn install_media(socket: &Path) -> anyhow::Result<()> {
    let client = Arc::new(minidumper::Client::with_name(minidumper::SocketName::Path(socket))?);
    let c = client.clone();
    let handler = CrashHandler::attach(unsafe {
        make_crash_event(move |cc: &CrashContext| CrashEventResult::Handled(c.request_dump(cc).is_ok()))
    })?;
    let _ = HANDLER.set(handler);
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(target: "panic", "{info}\n{}", std::backtrace::Backtrace::force_capture());
        if let Some(h) = HANDLER.get() { let _ = h.simulate_exception(Some(KAIRO_PANIC)); } // 0xE0004B50: dump BEFORE any abort
        std::process::exit(EXIT_PANIC);                                                   // classified exit, not 0xC0000409
    }));
    Ok(())
}
pub struct OutboxTransport { pub dir: PathBuf }                   // "Ask each time": nothing leaves until Send
impl sentry::Transport for OutboxTransport {
    fn send_envelope(&self, envelope: sentry::Envelope) {
        let path = self.dir.join(format!("{}.envelope", ulid::Ulid::generate()));
        if let Ok(f) = std::fs::File::create(&path) { let _ = envelope.to_writer(f); }
    }
}
let mut o = sentry::ClientOptions::default();                     // #[non_exhaustive]: assign fields
o.dsn = dsn.parse().ok();                                         // EU: https://<key>@o<org>.ingest.de.sentry.io/<project>
o.server_name = Some("redacted".into());                          // else the hostname is sent [RC]
o.before_send = Some(Arc::new(|mut e: sentry::protocol::Event<'static>| { e.server_name = None; e.user = None; e.request = None; Some(e) }));
o.transport = Some(Arc::new(Arc::new(OutboxTransport { dir })));  // Arc<T: Transport> is a TransportFactory
```
- `sentry::init` only after opt-in. On "Send": real client, `Envelope::from_path`, `client.send_envelope`, `flush(Some(5 s))`, delete. Native dumps become a `Level::Fatal` event with an `AttachmentType::Minidump` attachment.
- Exit codes: 0 clean, `EXIT_RESTART`, `EXIT_PANIC`, `0xC0000005`, `0xC00000FD`, `0xC0000409` (fastfail). Safe mode: backoff 0.5→30 s; ≥3 crashes in 10 min → L1 `h264_mf`, L2 DDA + `h264_mf`, L3 no capture.
- Dev only: WER LocalDumps for `kairo-engine.exe` (§10.4) catches fastfails **[RC]**; with the job's `DIE_ON_UNHANDLED_EXCEPTION` **[U]**.

### 7.7 Lifecycle **[CK]**

- Single instance: `CreateMutexW(None, true, &HSTRING::from(format!(r"Local\Kairo.Control.{sid}")))` then `GetLastError() == ERROR_ALREADY_EXISTS` with nothing in between → forward args over the pipe, exit 0. The pipe's first-instance flag guards across sessions.
- Job: `CreateJobObjectW(None, PCWSTR::null())`, `JOBOBJECT_EXTENDED_LIMIT_INFORMATION.BasicLimitInformation.LimitFlags = KILL_ON_JOB_CLOSE | DIE_ON_UNHANDLED_EXCEPTION | BREAKAWAY_OK`, `SetInformationJobObject(job, JobObjectExtendedLimitInformation, ..)`, `AssignProcessToJobObject`.
- Media EcoQoS opt-out: `SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, &PROCESS_POWER_THROTTLING_STATE { Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION, ControlMask: EXECUTION_SPEED | IGNORE_TIMER_RESOLUTION, StateMask: 0 }, ..)`.
- Autostart (after consent): `HKCU\…\Run\Kairo` = `"<path>\kairo-engine.exe" --autostart` (quoted; value name = productName so the NSIS uninstaller removes it **[RC]**); read but never write `StartupApproved\Run`.
- Data: `%LOCALAPPDATA%\io.github.maty3k.kairo\{settings.json, library.db, logs, crashes, outbox, state, cache, run}`; the uninstaller keeps it unless "delete app data" is ticked **[RC]**.
- SQLite: control is the only writer; `pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))` must return `wal`; `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout(5 s)`, `rusqlite_migration` on `user_version`. Never on network or OneDrive folders.

---

## 8. Tauri UI

### 8.1 Setup

- `pnpm create tauri-app@4.7.5 ui --template react-ts --manager pnpm --identifier io.github.maty3k.kairo --yes`; TypeScript **6.0.3** (7.x breaks typescript-eslint 8.72.0); never install `@next` tags; shadcn `init -b radix`.
- `tauri.conf.json` essentials:

```jsonc
{ "productName": "Kairo", "identifier": "io.github.maty3k.kairo",              // permanent (mutex, UDF, updater)
  "app": { "withGlobalTauri": false,                                            // no appDirectoriesOverride (P10)
    "windows": [{ "label": "main", "create": false, "visible": false, "backgroundColor": "#0B0C10", "theme": "Dark",
                  "width": 1280, "height": 800, "minWidth": 960, "minHeight": 600, "dragDropEnabled": true, "zoomHotkeysEnabled": true }],
    "security": { "csp": { "default-src": "'self'", "script-src": "'self'", "style-src": "'self' 'unsafe-inline'",
        "img-src": "'self' data: blob: kairo-media: http://kairo-media.localhost",
        "media-src": "'self' blob: kairo-media: http://kairo-media.localhost",
        "font-src": "'self' data:", "connect-src": "ipc: http://ipc.localhost" },
      "dangerousDisableAssetCspModification": ["style-src"] } },               // wavesurfer + sonner inject <style>
  "bundle": { "targets": ["nsis"], "externalBin": ["binaries/kairo-engine"] } }   // updater artifacts: release overlay (below)
```
- Updater artifacts come only from `release.yml`, through an overlay: `tauri build --config tauri.release.conf.json` with `{"bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"pubkey":"<tauri signer public key>"}}}`, and `TAURI_SIGNING_PRIVATE_KEY` only there. No updater plugin is compiled in: the CLI reads the pubkey only to check the signing key against it, but with `createUpdaterArtifacts` on and no `plugins > updater` it stops every bundling build, and without the private key it fails unless given `--no-sign` (tauri-cli 2.12.4 `interface/rust.rs`, `bundle.rs`). Control verifies the `.sig` (PLAN §2.1) **[V]**.
- `build.rs`: `tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(&["engine_call", "engine_state", "engine_start", "clip_copy", "clip_reveal", "clip_drag_win"])))` so the ACL applies; capabilities list `allow-engine-call` etc. (`_` → `-`). `core:window:default` is read-only: add `allow-show`, `allow-set-focus`, `allow-set-progress-bar`.
- Plugins: single-instance first; window-state without `StateFlags::VISIBLE`; show the window after the first paint (rAF ×2) plus a 3 s Rust fallback. One window, no tray, no hidden windows, never `prevent_exit()`: the last window's destruction raises `ExitRequested { code: None }` and wry's `Drop` calls `ICoreWebView2Controller::Close()` **[RC]**.

### 8.2 Engine client

```rust
#[cfg(windows)]
async fn connect() -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    let sid = kairo_rpc::current_user_sid().map_err(std::io::Error::other)?;
    kairo_rpc::transport::connect(&kairo_rpc::pipe_name(&sid)).await          // §7.4 (tokio ClientOptions)
}
#[cfg(unix)]
async fn connect() -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(std::env::temp_dir().join("kairo-engine.sock")).await
}
// then Framed::new(stream, kairo_rpc::codec()); request/response map, reconnect 0.25 → 5 s, handshake: research 12 §2.3
```
- One allow-listed command `engine_call(method, params)`; method must be in `kairo_rpc::METHODS`; async commands taking `State<'_>` return `Result<_, E: Serialize>`.
- Notifications: one event, `app.emit("engine:notification", {method, params})`. Event names allow only `[A-Za-z0-9-/:_]` **[RC]**. Connection state on `engine:state` (`connecting | connected | notRunning | accessDenied | incompatible`).
- TanStack Query: `staleTime: Infinity`, retry queries once, never mutations; `export.start` carries a ULID `requestId`.

### 8.3 Video: `kairo-media` (research 12 §3.3)

- `.register_asynchronous_uri_scheme_protocol("kairo-media", handle)`; reject `ctx.webview_label() != "main"`; GET/HEAD only; path = `v|poster|sprite|shot / <ULID>` (percent-decoded: `convertFileSrc` encodes `/`); resolve ID → path via the engine (`library.resolvePath`), cached, cleared on `library.changed`.
- `Range` present → `206` with at most **2 MiB** (`http_range::HttpRange::parse(r, len)`), `Content-Range: bytes s-e/len`, `Accept-Ranges: bytes`; no `Range` and > 8 MiB → `416` (never load a clip into RAM); open the file per request (no lingering handle).
- Why: tauri#4133 open; asset protocol caps ranges at 1000 KiB and reads whole files without `Range`; wry copies bodies into `SHCreateMemStream`; partial content works only on `http(s)://<scheme>.localhost` (Tauri's mapping) **[RC]**. Fallback: WebView2 virtual-host mapping via `with_webview`.
- Probe codecs at start with `navigator.mediaCapabilities.decodingInfo` (`avc1.640033`, `hvc1.1.6.L123.B0`, AV1); the player plays only the default (mix) track.

### 8.4 Clipboard and drag-out **[CK]**

- Copy: CF_HDROP + "Preferred DropEffect" on the UI thread (`app.run_on_main_thread`, `win.hwnd()` inside the closure): `GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT)`, `write_unaligned` a `DROPFILES { pFiles: 20, fWide: true }` (packed), UTF-16 list + double NUL, `GlobalUnlock` (Err code 0 = success), `OpenClipboard(Some(hwnd))` (NULL owner makes `SetClipboardData` fail), `EmptyClipboard`, `SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(h.0)))` (`CF_HDROP` is a u16 newtype **[RC]**), never free after success, no `\\?\` paths, never a temp file. Full code: `wincheck/src/misc.rs` (= research 12 §6.1, compiled).
- Drag (Windows, P11):

```rust
pub unsafe fn drag_file(hwnd: HWND, path: &Path) -> windows::core::Result<DROPEFFECT> { unsafe {
    let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None::<&IBindCtx>)?;
    let data: IDataObject = item.BindToHandler(None::<&IBindCtx>, &BHID_DataObject)?;
    SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY)   // modal; NULL source → Shell provides one
}}   // DROPEFFECT_NONE = cancelled; call inside run_on_main_thread; start it from pointer events, not HTML5 DnD
```
- `dragDropEnabled: true` (import by drop) disables HTML5 DnD inside the page on Windows: use pointer-based DnD.

### 8.5 Tests

- Vitest 5.0.5 + Testing Library + jsdom 30.1.3; `@tauri-apps/api/mocks`: `mockIPC(cb, { shouldMockEvents: true })` (no `emitTo`), `mockWindows('main')`, `mockConvertFileSrc('windows')`, `clearMocks()`.
- E2E: `webdriverio@9.32.0` (pinned; npm latest 10.1.0 breaks the peer **[RC]**) + `@wdio/tauri-service` 1.5.0 + `tauri-plugin-wdio-webdriver` 1.5.0 behind an `e2e` feature and an inline capability; CI checks release builds contain no `wdio`.
- PC checks in the interactive session: WebView2 exit timing script (research 12 §1.8), 600 MB seek p95 <150 ms, `powershell.exe -Command "Get-Clipboard -Format FileDropList"` (Windows PowerShell 5.1 only).

---

## 9. AMD tuning and measurement

### 9.1 What costs the game FPS

VCN encoding is separate hardware and nearly free. The game pays for 3D/compute work (Kairo's copy and shaders, **AMF Pre-Analysis on OpenCL by default [RC]**, DWM's capture composition), memory bandwidth (full-frame copies; HDR doubles it), board power and scheduling. Levers: PA off, NV12 only, capture rate tied to the clip (§2.4), fewer copies, GPU priority untouched (`SetGPUThreadPriority(-2)` only as an experiment), no topmost overlay when idle.

### 9.2 AMF settings (set every key; FFmpeg 9.0.2 option names)

| | H.264 `h264_amf` (default) | HEVC `hevc_amf` (opt-in) | AV1 `av1_amf` (opt-in) |
|---|---|---|---|
| Common | `usage=transcoding quality=balanced rc=vbr_peak preanalysis=0 high_motion_quality_boost_enable=0 preencode=0 async_depth=4` | same | same |
| AQ / B-frames | `vbaq=1`, `bf=0` | `vbaq=1`, none exist | `aq_mode=caq`, `bf=0` |
| Headers / drops | `header_spacing=60 frame_skipping=0 filler_data=0` | `gops_per_idr=1 header_insertion_mode=idr skip_frame=0 filler_data=0` | `header_insertion_mode=gop skip_frame=0 filler_data=0` |
| Other | `profile=high coder=cabac` | `profile=main bitdepth=8` | `latency=power_saving_real_time align=none` |
| 1080p60 target/peak/VBV | 25/40/40 Mbps | 16/25/25 | 12/20/20 |
| 1440p60 | **36/50/50** | 24/36/36 | 18/28/28 |

- `usage=high_quality` silently enables PA, VBAQ, HMQB and TAQ (lookahead 11); VBAQ is ignored when TAQ/PAQ are on **[RC]**. "Quality" profile = `quality=quality preencode=1`, still `preanalysis=0`.
- `av1_amf`: `amfenc_av1.c` sets the rate-control context variable to CBR whenever `bit_rate > 0` after the property is assigned; read the stream back **[U]**. RDNA3 codes 1080p as 1920×1082 + `clap` (confirmed by AMD 2026-10-14 **[RC]**); 1440p is aligned; VCN 5 lifting it **[U]**. Do not offer 1080p AV1 on RDNA3.
- VCN: RX 9000 has one VCN for encode and decode; RX 7700–7900 have two, RX 7600 one; no session cap; FFmpeg cannot select `INSTANCE_INDEX`. "One decode, one encode by default" on RX 7000 is unconfirmed **[RC]**. Kairo + Instant Replay roughly double encoder load.

### 9.3 Known AMD issues to design around

| Issue | Mitigation |
|---|---|
| Back-to-back H.264→HEVC→AV1 sessions hang RX 9070 XT (26.6.4, Sunshine #5385) | lazy sequential probe, cached per driver |
| `h264_amf` stops output ~20 s at ~100 Mbps (RX 9060 XT, AMF #614) | peak ≤50 Mbps; stall watchdog |
| Encoder overload never recovers (RX 9070 XT, OBS #13670) | governor + session recreate |
| `AMF_INPUT_FULL` → TDR after hours, 1440p OLED replay buffer, RX 7900 GRE, 26.8.1 (OBS #10594) | 4–8 h soaks, device-loss rebuild, TDR history |
| RGB render-target input corrupts (DCC, RX 7000 from 25.10.2, AMF #574) | NV12 only |
| HEVC 10-bit + QVBR/HQVBR init failure (#591); B-frames + HQVBR hang (#586) | 8-bit, no HQ RC modes, `bf=0` |
| 26.10.1: overlay open/close timeouts with AMD's metrics overlay; video on a secondary display + FG timeouts (RX 9000) | pop-up created once and hidden; test with a video on the 180 Hz screen |

### 9.4 Governor and stall watchdog (media, 1 Hz)

1. Inputs: PDH video-engine % for all processes (§9.6), Kairo backlog (frames sent − packets received), time blocked in `receive_packet`.
2. Total >85% for 5 s or backlog > `async_depth` → `quality` → `balanced` → `speed`; then, if allowed, 120 → 60 fps (never below 60). Emit `encoderPressure {competitor}`.
3. No packet for >2 s while frames go in → tear down and recreate the session; new epoch, IDR; count it.

### 9.5 Coexistence with Adrenalin

- Detect Instant Replay by **PDH** (a non-Kairo process with video-engine use >1% for 5 s; map PID → path, `C:\Program Files\AMD\CNext\…` **[U]**) and registry hints `HKLM\SOFTWARE\AMD\DVR\InstantReplayEnabled`, `HKCU\Software\AMD\DVR\DvrEnabled` (**[U]**; confirm with the before/after diff script, research 13 §4.2). Warn; never write AMD's settings.
- AMD default hotkeys: only Ctrl+Shift+O/L are confirmed by AMD; the rest vary by driver **[RC]**. Alt+F8/Alt+F6 appear in no AMD list seen.

### 9.6 Measuring

```powershell
& C:\kairo\tools\PresentMon-2.6.0-x64.exe --process_name $Exe --output_file "C:\kairo\bench\$Tag.csv" `
  --session_name KairoBench --stop_existing_session --delay 5 --timed 60 --terminate_after_timed `
  --no_console_stats --qpc_time --track_gpu_video --track_frame_type
```
- Needs admin (the owner's SSH token is elevated) or Performance Log Users. Read columns by header: `MsBetweenPresents` (avg FPS = frames ÷ Σ ms × 1000; 1% low = 1000 ÷ P99, nearest rank), `PresentMode` shares, `MsGPUBusy`/`MsGPUWait`/`MsGPUTime`, `FrameType` (`AMD AFMF`). `MsVideoBusy` is the game's, not Kairo's.
- PDH sampler **[CK]**: `PdhOpenQueryW` → `PdhAddEnglishCounterW(q, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut c)` → `PdhCollectQueryData` twice → `PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, ..)` (all return `u32`); instance `pid_<PID>_luid_…_engtype_<Type>`; keep types whose lower-cased, space-stripped name starts with `video` and log every type seen once (AMD naming **[RC uncertain]**; check `typeperf -q "GPU Engine"` on the PC).
- Protocol: environment frozen in `bench/env.json`; AMD features off globally and per game (IR, Record Desktop, overlay, AFMF, Chill, Boost, FRTC, Enhanced Sync, RSR); VSync off; 2 min warm-up; A/A noise floor (CV ≤1%); A/B **ABBA ×3** per workload, A = engine not running; pass = mean loss ≤3% and upper 95% CI ≤4%, 1% low ≤5%, present modes unchanged. Scripts `pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1`: research 13 §5.2–5.3.
- Workloads: `kairo-loadgen` (flip-model, tearing, ALU- and bandwidth-bound modes), 2–3 games with built-in benchmarks, the owner's play only for soaks.
- Also log: driver store version (`IDXGIAdapter1::CheckInterfaceSupport(&IDXGIDevice::IID)`), HAGS (`D3DKMTQueryAdapterInfo(KMTQAITYPE_WDDM_2_7_CAPS)`, diagnostics only), TDRs (`Get-WinEvent -FilterHashtable @{ProviderName='Display'; Id=4101}`, `C:\Windows\LiveKernelReports`).

---

## 10. Dev-loop scripts

### 10.1 Mac, once

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.99.1
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --locked --version 0.23.1
brew install llvm && export PATH="$(brew --prefix llvm)/bin:$PATH" LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export XWIN_ARCH=x86_64 CARGO_BUILD_JOBS=4          # 8 GB M2: halve the SDK download, cap parallelism
git remote add pc kairo-pc:C:/kairo/src && git config remote.pc.receivepack "git receive-pack"   # URL form [U]
FFMPEG_DIR=~/kairo-deps/ffmpeg-9.0-lgpl-shared cargo xwin check --target x86_64-pc-windows-msvc -p kairo-engine
```

### 10.2 PC, once (bootstrap, elevated)

- OpenSSH Server (`Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0`), `DefaultShell` = pwsh 7, key in `C:\ProgramData\ssh\administrators_authorized_keys` with `icacls … /inheritance:r /grant *S-1-5-32-544:F /grant *S-1-5-18:F`, `PasswordAuthentication no`, firewall rule on the Private profile.
- Toolchain: Git, VS Build Tools (C++ + Windows SDK), rustup msvc 1.99.1, LLVM (`LIBCLANG_PATH`), Node 24 + pnpm, the pinned FFmpeg zip (`FFMPEG_DIR`), PresentMon 2.6.0; owner in Performance Log Users.
- `git init C:\kairo\src; git -C C:\kairo\src config receive.denyCurrentBranch updateInstead; git -C C:\kairo\src config core.autocrlf false`.
- Tasks (interactive, unelevated, **normal priority**):

```powershell
# C:\kairo\tools\register-tasks.ps1
$prin = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$set  = New-ScheduledTaskSettingsSet -Priority 4 -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew `
          -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries          # schtasks defaults: priority 7 (below normal), 72 h [RC]
$eng = New-ScheduledTaskAction -Execute 'C:\kairo\builds\current\kairo-engine.exe' -Argument '--dev' -WorkingDirectory 'C:\kairo\builds\current'
Register-ScheduledTask -TaskPath '\Kairo\' -TaskName 'Engine' -Action $eng -Principal $prin -Settings $set -Force
$run = New-ScheduledTaskAction -Execute 'C:\kairo\builds\current\kairo-devrun.exe' -Argument 'C:\kairo\run\job.json' -WorkingDirectory 'C:\kairo\run'
Register-ScheduledTask -TaskPath '\Kairo\' -TaskName 'Run' -Action $run -Principal $prin -Settings $set -Force
$prinHi = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Highest
Register-ScheduledTask -TaskPath '\Kairo\' -TaskName 'RunElevated' -Action $run -Principal $prinHi -Settings $set -Force  # elevated test window, no UAC click
$ens = New-ScheduledTaskAction -Execute 'C:\kairo\builds\current\kairo-engine.exe' -Argument '--ensure'
$trig = @((New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"),
          (New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 5)))
Register-ScheduledTask -TaskPath '\Kairo\' -TaskName 'Ensure' -Action $ens -Trigger $trig -Principal $prin -Settings $set -Force
```
Whether a standard (non-admin) user may register the logon trigger, and whether `-RepetitionInterval` without `-RepetitionDuration` repeats indefinitely on this build, are **[U]** (M0). Ad-hoc equivalent of the task model (no priority control, so never for timing): `schtasks /create /tn "Kairo\Adhoc" /tr "C:\kairo\builds\current\kairo-probes.exe wgc --seconds 30" /sc once /st 00:00 /it /rl LIMITED /f` then `schtasks /run /tn "Kairo\Adhoc"`.

### 10.3 `tools/mac/pc` (bash; remote shell is pwsh 7)

```bash
#!/usr/bin/env bash
set -euo pipefail
PC=${KAIRO_PC:-kairo-pc}; R='C:\kairo'; DATA='$env:LOCALAPPDATA\io.github.maty3k.kairo'
r() { ssh "$PC" "$@"; }
case "${1:-}" in
  status) r "& $R\\builds\\current\\kairo-cli.exe call engine.status '{}'" ;;   # game, saveInFlight, userIdleMs [proposed]
  sync)   idx=$(mktemp); cp .git/index "$idx"; GIT_INDEX_FILE="$idx" git add -A
          tree=$(GIT_INDEX_FILE="$idx" git write-tree); rm -f "$idx"
          sha=$(git commit-tree "$tree" -p HEAD -m "pc-sync $(date -u +%FT%TZ)")
          git push -q -f pc "$sha:refs/heads/pc-sync" && echo "$sha" ;;            # working tree snapshot, no branch commits
  build)  r "Set-Location $R\\src; cargo build --locked --release -p kairo-engine -p kairo-cli -p kairo-probes 2>&1 | Select-Object -Last 60" ;;
  deploy) r "& $R\\tools\\deploy.ps1" ;;        # target\release + 4 FFmpeg DLLs → builds\<sha>, junction builds\current, keep 3
  engine) r "Start-ScheduledTask -TaskPath '\\Kairo\\' -TaskName Engine" ;;
  stop)   r "& $R\\builds\\current\\kairo-cli.exe call engine.shutdown '{\"reason\":\"dev\"}'; Start-Sleep 3; Stop-ScheduledTask -TaskPath '\\Kairo\\' -TaskName Engine" ;;
  run)    shift; r "& $R\\tools\\run.ps1 $*" ;;  # GUI-session job through \Kairo\Run (or RunElevated)
  cli)    shift; r "& $R\\builds\\current\\kairo-cli.exe $*" ;;
  logs)   r "Get-ChildItem $DATA\\logs\\${2:-control}*.log | Sort-Object LastWriteTime | Select-Object -Last 1 | Get-Content -Tail ${3:-200}" ;;
  dumps)  mkdir -p .dogfood/dumps && scp "$PC:AppData/Local/io.github.maty3k.kairo/crashes/*" .dogfood/dumps/ ;;
  bench)  shift; r "& $R\\tools\\bench.ps1 $*" ;;
  *) echo "usage: pc status|sync|build|deploy|engine|stop|run|cli|logs|dumps|bench"; exit 2 ;;
esac
```
Quoting through Win32-OpenSSH into pwsh is fragile (#752): keep logic in `C:\kairo\tools\*.ps1` and pass simple arguments **[U]**.

### 10.4 PC-side helpers

```powershell
# C:\kairo\tools\run.ps1 — run one program in the owner's session via \Kairo\Run, wait, print its result
param([Parameter(Mandatory)][string]$Exe, [string[]]$ArgList = @(), [int]$TimeoutSec = 120, [switch]$Elevated)
@{ steps = @(@{ exe = $Exe; args = $ArgList; cwd = 'C:\kairo\run'; timeoutSec = $TimeoutSec }); out = 'C:\kairo\run\result.json' } |
  ConvertTo-Json -Depth 5 | Set-Content C:\kairo\run\job.json -Encoding utf8
Remove-Item C:\kairo\run\result.json -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskPath '\Kairo\' -TaskName ($(if ($Elevated) { 'RunElevated' } else { 'Run' }))
$sw = [Diagnostics.Stopwatch]::StartNew()
while (-not (Test-Path C:\kairo\run\result.json) -and $sw.Elapsed.TotalSeconds -lt $TimeoutSec + 10) { Start-Sleep -Milliseconds 500 }
Get-Content C:\kairo\run\result.json -ErrorAction SilentlyContinue
```
- `kairo-devrun.exe` is GUI-subsystem: it runs each step with `CREATE_NO_WINDOW`, captures stdout/stderr to `C:\kairo\logs`, writes `result.json` (exit code, duration, tail). No console window may appear while the owner plays.
- WER LocalDumps (dev PC, admin SSH):

```powershell
$k = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\kairo-engine.exe'
New-Item $k -Force | Out-Null
New-ItemProperty $k -Name DumpFolder -PropertyType ExpandString -Value 'C:\kairo\dumps' -Force | Out-Null
New-ItemProperty $k -Name DumpCount  -PropertyType DWord -Value 10 -Force | Out-Null
New-ItemProperty $k -Name DumpType   -PropertyType DWord -Value 1 -Force | Out-Null   # 1 mini, 2 full
```
- Symbolicate on the Mac: `dump_syms kairo-engine.pdb > kairo-engine.sym` (Breakpad layout), then `minidump-stackwalk --json dump.dmp symbols/`.
- Elevated test windows: `pc run -Elevated …` (task registered by an admin starts elevated without a prompt). PresentMon runs straight from SSH (ETW is system-wide); after a dropped connection `--stop_existing_session` with the unique `--session_name` recovers it.
- Prerequisites for any capture run: owner logged in and unlocked, displays on, no RDP; `engine.status` reports `sessionLocked` and `displayOn`. Patterns and soaks on the 180 Hz monitor (OLED care).

---

## 11. Engine-skeleton spike checklist (M3)

Prerequisites (M0/M1, outside the exit-rule clock): Mac `cargo xwin check` of the workspace with `ffmpeg-sys-next` 9.0.1 passes; PC toolchain + tasks at priority 4; `pc run hello` shows a window in the owner's session; `kairo-cli ping` crosses sessions; `wgc-probe` produces traces on both monitors; `kairo-verify` catches every synthetic bad clip.

| # | Task | Done when | Budget [E] |
|---|---|---|---|
| 1 | Workspace + `kairo-rpc` v0.1 (`engine.hello/status/shutdown`, `clip.save`, `clip.saved/failed`, `buffer.status`), codec, fixtures; frozen by day 2 | `cargo test -p kairo-rpc` on the Mac; ts-rs drift check clean; mock answers `engine.hello` | 0.5 d |
| 2 | Control shell: T0 window, tray stub, Alt+F8 via `RegisterHotKey`, pipe server, `kairo-cli` | over SSH `kairo-cli call engine.hello` reaches the task-launched engine; Alt+F8 logs a QPC stamp; a helper holding Alt+F8 → `InUse`; `GetSecurityInfo` SDDL equals the expected DACL | 1 d |
| 3 | `GpuDevice` on the monitor's adapter + `GpuLock` + multithread protection | probe log: adapter LUID matches the output's, FL 11_1, `ID3D11VideoDevice` QI ok, `GetMultithreadProtected` true | 0.25 d |
| 4 | WGC capture (M1's `wgc-probe` module): caps, consent status, interval rule + default read back, mailbox, GPU-thread consume, QPC pacer at 60 fps | testbed at 360 fps, 10 min: ≥99.5% fresh ticks, no gap >2 ticks, callback CPU <1%, no yellow border; HDR toggle swaps the pool ≤1 s | 0.5 d |
| 5 | Shaders + NV12 self-test + Pass B | every entry point compiles via `D3DCompile` (Windows `#[test]`); 75% bars ±1 code from a staging readback; self-test logged | 0.75 d |
| 6 | FFmpeg D3D11VA device/frames with the GpuLock callbacks; `h264_amf` open + 5-frame probe | KEY packet + `extradata_size > 0`; options read back equal the table; `av_version_info()` = the pinned build; no `av_assert0` abort | 0.75 d |
| 7 | Encoder thread + ring (video) + save → MP4 (video only) | ffprobe: h264 High, tv/bt709 ×3, K every 60, pts==dts, ftyp/moov(colr)/mdat, no elst; save → file <1 s | 0.75 d |
| 8 | Audio: game (include-tree on the testbed PID) + mic → aligners → mix/limiter → 3 AAC tracks → ring (aligner, mixer, limiter already unit-tested in `kairo-core` on the Mac) | include/exclude tone test; activation-with-tone check; 10 min run with no gaps; meters move | 1 d |
| 9 | Save with audio (§5.3 rules) | kairo-verify: flash vs beep offset under 10 ms (≤21.3 ms gate), mix default only, titles, durations 30 s ± 1 GOP | 0.5 d |
| 10 | **Timed gate** | 20/20 saves via `kairo-cli` pass kairo-verify; save → file p95 <1 s; fresh-tick % unchanged while saving | 0.25 d |
| 11 | Untimed | 1 h run: engine ≤60 MB + ring, growth <5 MB/h; 3-pair PresentMon A/B shows no gross regression | — |

The PC-side budget sums to ≈6.25 days against a ~7-day exit rule, so do every Mac-only piece (task 1, the ring, aligner, mixer, cut maths) before the clock starts or in parallel, and stop gold-plating at the gate.

Exit rule (plan, decision 26): log every engine day; count PC-available days only; exclude problems that reproduce with the FFmpeg CLI or a C++ sample. Same Rust/COM error class unresolved after 3 attempts, or an `unsafe` crash unresolved after a day, is a loop signal. Day 4 without an end-to-end hard path → draft the C# plan; ~7 counted days without the gate for Rust reasons → C#/.NET 10 NativeAOT (UI, contract, fixtures, mock and harness stay).

---

## 12. Open risks and unverified items

| # | Item | Why it matters | Settle by |
|---|---|---|---|
| 1 | `cargo xwin check` of `ffmpeg-sys-next` end to end on the Mac | Mac-side compile loop for the encode crate | M0 exit 5 (fallback: PC-generated bindings) |
| 2 | Audio start rule (§5.3) in real players | A/V sync of every clip | M3 task 9 + SP8 player matrix |
| 3 | `MinUpdateInterval` OS default; delivered rate per interval on 360/180 Hz; VRR behaviour | duplicate ticks vs copy cost | SP2 |
| 4 | Whether WGC (window vs monitor) demotes Independent Flip; AFMF frames visible to capture | game latency/VRR; warning text | SP1, 13 §3.2 |
| 5 | Unpackaged `RequestAccessAsync(Borderless)` prompt/result on 25H2/26H2 | yellow border | SP2 |
| 6 | PDH engine-type name on AMD (`VideoCodec`?) and visibility of other processes to a standard user | IR detection, governor | SP9 (`typeperf -q "GPU Engine"`) |
| 7 | AMD registry names for IR and hotkeys; IR host process path | coexistence warning | SP9 diff script |
| 8 | VCN default split on RX 7000; VCN 5 AV1 alignment; `REQUESTED_THROUGHPUT` scope | headroom, AV1 offer | SP1/SP7 |
| 9 | `VideoProcessorBlt` fallback quality on AMD | NV12 fallback | only if the self-test fails |
| 10 | AMF `header_spacing` default; whether VCN reads the D3D11 texture directly | VRAM/copies | SP7, PDH Copy engine |
| 11 | QSV 1080 vs 1088 surfaces; legacy runtimes; `h264_mf` IDR/closed GOP per MFT | Intel and fallback users | M10 testers |
| 12 | Process loopback across default-device switches; grandchildren after a parent exits; packets during silence | game/voice stems | SP4 soak |
| 13 | `IAudioClientDuckingControl` on capture streams (speecher PR #270 unchecked **[RC]**); mic privacy failure mode | ducking, error states | SP4 ducking matrix |
| 14 | Win10 / pre-24H2 noise bug and the A/B swap's effectiveness | long-session audio | SP4, M11 Win10 soak |
| 15 | DDA honouring `WDA_EXCLUDEFROMCAPTURE` | pop-up in clips | M5 |
| 16 | WER LocalDumps with the job's `DIE_ON_UNHANDLED_EXCEPTION`; `CREATE_BREAKAWAY_FROM_JOB` under Task Scheduler | crash evidence; UI survival | M3 `--debug-crash=abort` |
| 17 | NETWORK SID in OpenSSH tokens vs the pipe DACL; git push URL form; first push into an unborn branch | dev loop | M0 |
| 18 | Standard user registering `\Kairo\Ensure` (logon trigger) | control restart | M0 |
| 19 | WebView2 exit timing; 600 MB seek p95; CF_HDROP paste into Discord/WhatsApp | M6/M7 gates | SP11, SP8 |
| 20 | `generate_context!` needing `frontendDist` during `cargo xwin check`; `ffi::va_list` alias on MSVC | Mac checks | M0 |
| 21 | `SetGPUThreadPriority(-2)` under HAGS; "10-bit pixel format" breaking DDA on 2026 drivers | experiments | SP1 test 6 |
| 22 | Tauri 2.12.5 changes. **Settled 2026-10-23:** bump to `tauri` =2.12.5 / `tauri-build` =2.7.4 / CLI 2.12.4 (P20). The `tauri` crate itself changes only a Linux doc comment. In the shipped Windows app only tauri-runtime-wry 2.12.3 changes: std `LazyLock` replaces `once_cell::sync::Lazy` in the DPI helpers of the resize code for undecorated, resizable windows (which will include the frameless tray popup, PLAN §2.1, unless it sets `resizable: false`), a like-for-like swap. CLI 2.12.4 moves to zip 9 and the fixed `rustls-webpki`/webpki roots. Nothing in §8 changes | pin bump | M0 (done) |
| 23 | tao#1393 (open; fix PR tao#1394 not merged): on Windows, `EventLoopProxy::send_event` posts its message before it queues the event, and the UI thread then blocks in `recv()` until the sender runs again. `run_on_main_thread` from other threads (for example the CF_HDROP copy, §8) goes through it | UI stalls of seconds next to a CPU-heavy game | SP11/M6 gate: UI responsiveness under full CPU load; watch tao#1394 |
| 24 | First Tauri build: check for a newer patch, new issues and [Tauri's own advisories](https://github.com/tauri-apps/tauri/security/advisories) (they can be missing from RustSec and GitHub's global database, so `cargo-deny` won't flag them: GHSA-w28w-mhc8-qvjv is in neither). Bundler 2.10.4 changed the NSIS template, so check `makensis` still builds | the companion crates float (tauri-runtime and tauri-runtime-wry ~2.12, tauri-macros and tauri-codegen ~2.7, tauri-utils ~2.10), as do wry 0.57.x, tao 0.37.x and webview2-com 0.39.x under them, so the pins hold only through `Cargo.lock` + `--locked`; review lockfile diffs | SP11 |
| 25 | tauri#16277 (open, needs more info): NSIS updates abort with `Failed to kill <App>` since the Restart Manager switch. NSIS bundling as the `SYSTEM` account fails with `error 0x2` until the CLI release queued in tauri#16365 | control's `/S` updates | M8: make sure `Kairo.exe` has exited before the installer starts; never bundle from a `SYSTEM` task |

---

## Sources

- Research reports 00 (brief), 08–13 in `` and `plan/PLAN.md` (2026-10-21).
- Reality check of 2026-10-21 (JSON supplied with this task): crates.io API, windows 0.62.2 / windows-core 0.62.2 / windows-result 0.4.1 sources, FFmpeg n9.0.2 sources, tokio 1.54.0, interprocess 2.4.6, wasapi 0.20–0.26, rubato 5.0.1, sentry 0.49.3, crash-handler 0.8.1, Rust 1.99.1 std, tauri 2.12.4 / tauri-bundler 2.10.3 / wry 0.57.0, Microsoft Learn, AMD AMF docs and issues, PresentMon v2.6.0.
- Checked for this cookbook (2026-10-21): crates.io API (versions, licences, dates for 58 crate versions; Tauri 2.12.5 published today; `ffmpeg-next` 9.0.1 absent), npm registry (25 packages; `webdriverio` dist-tags), FFmpeg 9.0.2 `libavformat/movenc.c` lines 3305-3436 (AAC `roll` groups) and 7282-7290, 8203-8224 (no-edit-list first-sample rule), `wasapi` 0.26.0 `src/lib.rs`, `src/api.rs:70`, `ulid` 3.0.0 `src/time.rs`, `sentry-core` 0.49.3 `clientoptions.rs` (`#[non_exhaustive]`), `windows` 0.62.2 `Win32/System/Threading`, `Graphics/Direct3D11` (video processor), `Graphics/Gdi`, `System/JobObjects`.
- Compile harnesses: `prototypes/{wincheck,ffcheck,corecheck}` and research 08's `kcapcheck`.
- Tauri 2.12.5 review (2026-10-23; P20, §12 items 22–25): crates.io and npm registries; the crate sources of tauri 2.12.4/2.12.5, tauri-runtime-wry 2.12.2/2.12.3, tauri-build and tauri-codegen 2.7.3/2.7.4, tauri-cli 2.12.3/2.12.4 and tauri-bundler 2.10.4; https://github.com/tauri-apps/tauri/compare/tauri-v2.12.4...tauri-v2.12.5; tauri#16277, #16365; tao#1393, #1394; https://github.com/tauri-apps/tauri/security/advisories; the RustSec advisory database.
