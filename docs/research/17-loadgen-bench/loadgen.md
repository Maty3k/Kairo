# kairo-loadgen on D3D11 with `windows` =0.62.2: design note (research 17)

*2026-11-14 · One part of research 17, gate 4's bench half (M1:29). Nothing in the repo was changed, built or run, and the PC was not touched. Snapshots: `main` at ddec2c9; the testbed is PR #31's branch `m1/testbed` at f88ed79 and the harness is PRs #34–35's branch `m1/harness-run` at 1cbfa9f (both unmerged, read in the scratch worktrees); `windows` 0.62.2 and `windows-link` 0.2.1 are read from the Mac's cargo registry. The PresentMon files from the earlier, cut-off run are tag v2.6.0 (`tagref.json`: `refs/tags/v2.6.0`, commit e13fce6; `Version.props`: 2.6.0); they were read as data only, and nothing from them or from `dl/` was run. `dl/` holds PowerShell sources for another part and isn't used here. Microsoft Learn pages were read on 2026-11-14.*

**Citation keys.**

| Key | File |
|---|---|
| `CL:n` | CLAUDE.md |
| `PLAN:n`, `M0:n`, `M1:n` | docs/PLAN.md, docs/milestones/M0.md, M1.md |
| `R01`, `R08`, `R13` `:n` | docs/research/NN-*.md |
| `CB:n` | docs/research/15-implementation-cookbook.md |
| `R16:n` | docs/research/16-verify-testbed/README.md |
| `ci:n`, `Cargo:n` | .github/workflows/ci.yml, the workspace Cargo.toml |
| `tb/…:n` | crates/kairo-testbed/… on PR #31 (f88ed79) |
| `disp/…:n` | crates/kairo-display/… (`main`; identical on PR #31) |
| `hs/…:n` | crates/kairo-harness/src/… on PRs #34–35 (1cbfa9f) |
| `core/…:n`, `devrun/…:n` | crates/kairo-core/src/…, crates/kairo-devrun/src/… (`main`) |
| `win/…:n` | windows 0.62.2, `src/Windows/…` |
| `wlink:n` | windows-link 0.2.1, `src/lib.rs` |
| `wincheck/…:n` | docs/research/prototypes/wincheck/… |
| `PM/…:n` | PresentMon v2.6.0: `PresentData/…`, `PresentMon/…`, `README-ConsoleApplication.md` |
| `MS-…` | Microsoft Learn and two Microsoft-authored pages, listed under Sources |

**Evidence labels.** As in research 16: **[Unverified]** not yet checked (§13 says how it gets settled), **[proposed]** a number Claude chose, to confirm with PC data, **[Estimate]** a planning number. **[MS]** marks what a Microsoft page says; **[src]** what a source file in this list shows.

---

## 0. Decisions at a glance

| # | Question | Decision |
|---|---|---|
| 1 | Code home | New crate `kairo-loadgen` (M1:57), binary `kairo-loadgen.exe`, which `pc deploy` already accepts (`kairo*.exe`, M1:81). The Windows plumbing the testbed already has (window, QPC, display-on, foreground, stdin stop, device on the LUID adapter, ready-file writer) moves into a small shared library after PR #31 merges (§1.3). `unsafe` only in `gpu` and `platform` modules, each block with `// SAFETY:` (CL:32) |
| 2 | Window | A `WS_POPUP` window covering the monitor's rectangle from `kairo-display`, per-monitor-v2 DPI aware, refused unless the client area equals the monitor exactly. `--activate` reuses the testbed's foreground route (no input) and is on for bench runs |
| 3 | Swap chain | `FLIP_DISCARD`, `B8G8R8A8_UNORM`, 3 buffers, `DXGI_SCALING_NONE`, `ALPHA_MODE_IGNORE`, flags `ALLOW_TEARING \| FRAME_LATENCY_WAITABLE_OBJECT`, `SetMaximumFrameLatency(2)`, `MakeWindowAssociation(NO_ALT_ENTER)`. Every frame `Present(0, DXGI_PRESENT_ALLOW_TEARING)`, uncapped. No tearing support → exit 3, never a silent fallback |
| 4 | Present mode | No documented D3D11/DXGI call tells a plain RGB flip swap chain whether it is in independent flip. PresentMon's `PresentMode`, `AllowsTearing`, `SyncInterval` and `PresentFlags` columns are the truth; loadgen logs only cheap hints (§2.5) |
| 5 | Workloads | One full-screen triangle drawn in 16 scissored bands per frame [proposed]. **ALU**: a per-pixel loop of `--iters` steps. **Bandwidth**: `--reads` random texel loads per pixel from a 256 MiB `R32_UINT` 8192×8192 texture filled on the GPU. Both draw the same dim, slowly drifting picture |
| 6 | Comparability | `run` takes the count explicitly and never changes it; `calibrate` is a separate subcommand run before the measured runs; the log header records every parameter and the shader bytecode hash |
| 7 | Shaders | Runtime `D3DCompile` from the system's `d3dcompiler_47.dll` (raw-dylib, no import library), HLSL embedded with `include_str!`, `vs_5_0`/`ps_5_0`, optimisation level 3, strict, warnings as errors: the cookbook's choice for the engine (CB:377) |
| 8 | Own GPU time | A ring of 8 `{TIMESTAMP_DISJOINT, TIMESTAMP, TIMESTAMP}` query sets, read with `D3D11_ASYNC_GETDATA_DONOTFLUSH` through the vtable (R13:684–695, CB:410–419); never a blocking wait in a measured run |
| 9 | TDR | About 0.5 ms per draw at the 8 ms target; a start-up ramp from a tiny workload refuses anything predicted over 25 ms per draw or 100 ms per frame [proposed]; a run-time guard; never `D3D11_CREATE_DEVICE_DISABLE_GPU_TIMEOUT`; a lost device ends the run (exit 4) and is never recreated |
| 10 | CLI, files, exits | `run` and `calibrate`; `ready.json` after the ramp; a testbed-style text log; one JSON summary line on stdout; exit codes 0/2/3/4/5 with the testbed's meanings (tb/src/cli.rs:24–36) |
| 11 | Photosensitivity | sRGB codes 58–95 only, luminance span 5% of white, colour drift period 20 s, ±6-code noise scrolling at 120 px/s [proposed]; unit tests hold the picture to XAG 118's thresholds (§8) |
| 12 | House rules | No call reaches another process, input, capture, a driver, a service or elevation, and no identifier matches CI's scan (ci:65) |

---

## 1. Reuse: what loadgen takes from the testbed and `kairo-display`, and what differs

There is no `main.rs` in the testbed: the binary is `tb/src/bin/testbed.rs` (a 48-line wrapper) over a library whose root is `tb/src/lib.rs`.

### 1.1 Inventory

| Piece | Where | For loadgen |
|---|---|---|
| Crate shape: `#![deny(unsafe_code)]` at the root, `#[cfg(windows)] #[allow(unsafe_code)] mod gpu; mod platform;`, `windows_subsystem = "windows"`, a stub `main` off Windows | tb/src/lib.rs:15–28, tb/src/bin/testbed.rs:6, :39–48 | Same shape |
| Monitor by device path (case-insensitive), mirrored displays refused, refresh as an exact fraction, adapter LUID, HDR state; per-monitor-v2 DPI awareness first | disp/src/lib.rs:165–190, :237–246, :58–91; disp/src/platform.rs:64–70, :347–403; used at tb/src/run.rs:206–237 | Use as is |
| D3D11 device on the adapter with that LUID (`EnumAdapterByLuid`, FL 11.1/11.0, `D3D11_CREATE_DEVICE_BGRA_SUPPORT`), its DXGI index, the output that shows the HMONITOR | tb/src/gpu.rs:75–108, :211–224, :373–401 | Move to the shared crate (§1.3) |
| HDR read and HDR watch through a factory's `IsCurrent` | tb/src/gpu.rs:110–113, :226–282 | Move; loadgen refuses HDR and ends the run on a display change |
| Back buffer 0 and its RTV | tb/src/gpu.rs:403–415 | Move |
| Device-lost classification after `Present` (`DXGI_ERROR_DEVICE_REMOVED`, `_RESET`, `D3DDDIERR_DEVICEREMOVED`, `GetDeviceRemovedReason`) | tb/src/gpu.rs:471–481, :522–546 | Move, taking the present flags as a parameter (it hard-codes `DXGI_PRESENT(0)` at :526) |
| Wait on the frame-latency waitable object; close it in `Drop` | tb/src/gpu.rs:506–513, :647–650 | Move |
| Frame statistics (`GetFrameStatistics`, disjoint handling) | tb/src/gpu.rs:459–469, :554–568 | Move; loadgen doesn't need display times |
| The testbed's `Chain` (vsync, timer, exclusive fullscreen, `ResizeBuffers`), `Pacing`, `VblankClock`, `Renderer` (clears and copies) | tb/src/gpu.rs:287–364, :446–457, :484–735 | Stay in the testbed |
| QPC clock (`now_hns`, `qpc_to_hns`) | tb/src/platform.rs:42–64 | Move |
| EcoQoS opt-out (CB:57, P16) | tb/src/platform.rs:66–86 | Move |
| `KeepDisplayOn` (`ES_CONTINUOUS \| ES_DISPLAY_REQUIRED`) | tb/src/platform.rs:88–105, duplicated at hs/platform.rs:26–44 | Move; both callers use one copy |
| Window: popup or framed at a rectangle, black class brush, no cursor over the client, `WM_CLOSE` flag, message pump, `SW_SHOWNOACTIVATE`, `SetForegroundWindow` + `GetForegroundWindow` check, panic-safe window procedure | tb/src/platform.rs:185–388; class name `KairoTestbed` at :218, title `Kairo testbed` at :240 | Move; class name and title become parameters (`KairoLoadgen`, `Kairo loadgen`) |
| Idle time and foreground-lock time-out; the `--activate` wait (last input older than the time-out + 5 s) and 10 s retry | tb/src/platform.rs:155–183; tb/src/run.rs:410–444 | Move |
| `--stop-on-stdin-eof` watcher thread (devrun starts every job with a null stdin, devrun/run.rs:209) | tb/src/run.rs:396–408 | Move |
| Ready file written whole and only if new (`<path>.<pid>.partial`, then a hard link, so a reader never sees half and nothing is replaced) | tb/src/ready.rs:62–73 | Move as a function; loadgen has its own JSON |
| Log discipline: one `write_all` per line, header deferred until frame 0, setup events buffered, `e end reason=…`, an empty log removed | tb/src/run.rs:84–182; core/testbed_log.rs:1–30 | Copy the discipline; own format (§7.3) |
| `Stop(code, message)` with messages that name no path | tb/src/run.rs:50–59 | Copy |
| CLI idiom (`once`, `value`, `number`; usage text that lists the exit codes) | tb/src/cli.rs:10–36, :94–268 | Copy the idiom |
| High-resolution waitable timer | tb/src/platform.rs:107–153 | Not needed: loadgen never sleeps between frames |

### 1.2 What differs

| | Testbed | Loadgen |
|---|---|---|
| Purpose | Ground-truth pattern, paced to vblanks or a timer | Deterministic GPU load, uncapped |
| Window modes | Borderless, windowed, exclusive (R16:269–285) | Borderless only: `DXGI_PRESENT_ALLOW_TEARING` "can only be used in windowed mode" [MS-PRESENTFLAGS] |
| Swap chain | 2 buffers + waitable at latency 1 (vsync), 3 buffers (timer), no tearing flag (tb/src/gpu.rs:126–177) | 3 buffers, `ALLOW_TEARING \| FRAME_LATENCY_WAITABLE_OBJECT`, latency 2 |
| Present | `Present(k, 0)` or `Present(0, 0)` (tb/src/gpu.rs:524–526) | `Present(0, DXGI_PRESENT_ALLOW_TEARING)` |
| Frame numbering | Vblank group or timer slot (tb/src/pace.rs:1–48) | Present count; the picture follows QPC time, not frame numbers |
| Rendering | Clears and copies only, "No shader runs" (tb/src/gpu.rs:7–9; R16:304) | Vertex + pixel shaders from `D3DCompile`, a constant buffer, a 256 MiB SRV, scissored bands, queries. Loadgen is the first crate in the workspace with HLSL |
| Timing source | Frame statistics, for display times | GPU timestamps, for GPU ms per frame |
| Log | `kairo_core::testbed_log`, which `kairo-verify` parses | Own format; nothing in Rust parses it in M1 |
| Audio | 1 kHz beep through `wasapi` | None, so no `wasapi` dependency |
| HDR | FP16 scRGB on an HDR output | SDR `B8G8R8A8_UNORM` only; HDR refused in M1 (§2.4) |

### 1.3 A shared crate, and loadgen's own layout

- **Shared crate** (name is Claude's call at implementation, M1:50; `kairo-window` here) [proposed]: the "Move" rows of §1.1. Windows-only modules with `unsafe` (`platform`, `gpu`) behind `#[allow(unsafe_code)]`, pure types everywhere, stub-free (a library). The testbed keeps pacing, `VblankClock`, `Renderer`, its `Chain` and its log; the harness drops its `KeepDisplayOn` copy once its PRs merge.
  - **Why a crate, not a copy:** about 600 lines would otherwise exist twice, `KeepDisplayOn` already does (tb/src/platform.rs:88–105, hs/platform.rs:26–44), and M3's toast window (R08:697) and later probes want the same window and device code.
  - **Why after #31:** extraction touches every testbed file; doing it inside an open PR would mix a refactor into its review. It is a pure move with the WARP tests unchanged (tb/src/gpu.rs:835–948).
  - Fallback if #31's review drags: loadgen starts with a copy marked for the move, and the extraction PR deletes the copy. Not recommended.
- **`crates/kairo-loadgen`**:

```
Cargo.toml              [[bin]] kairo-loadgen; kairo-core (timeline), kairo-display, the shared crate, serde_json
src/bin/kairo-loadgen.rs  #![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]; stub off Windows
src/lib.rs              #![deny(unsafe_code)]
src/cli.rs              pure: run | calibrate | --version | --help
src/workload.rs         pure: band split, ramp schedule and guard maths, calibration fit
src/visual.rs           pure: the picture's CPU reference (tests, §8)
src/log.rs, summary.rs  pure: `# kairo-loadgen 1` lines, the stdout JSON, the calibration JSON
src/loadgen.hlsl        all shaders, include_str!
src/gpu.rs, gpu/shader.rs, gpu/queries.rs   #[allow(unsafe_code)], Windows only
src/run.rs              Windows only, no unsafe: ties it together
```

- **`windows` features** (workspace pin =0.62.2, Cargo:36):
  - `Win32_Foundation` (HWND, HANDLE, LUID, RECT, `S_OK`, `DXGI_STATUS_OCCLUDED`, which 0.62.2 puts here: win/Win32/Foundation/mod.rs:1114);
  - `Win32_Graphics_Direct3D` (`ID3DBlob`, `D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST`, driver types);
  - `Win32_Graphics_Direct3D_Fxc` (`D3DCompile`, `D3DCOMPILE_*`: win/Win32/Graphics/Direct3D/Fxc/mod.rs:2–11, :223–255; gated at win/Win32/Graphics/Direct3D/mod.rs:3–4);
  - `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Gdi`;
  - `Win32_System_LibraryLoader`, `Win32_System_Performance`, `Win32_System_Power`, `Win32_System_SystemInformation`, `Win32_System_Threading`;
  - `Win32_UI_Input_KeyboardAndMouse` (`GetLastInputInfo`, `--activate` only), `Win32_UI_WindowsAndMessaging`;
  - optional `Wdk_Graphics_Direct3D` for the preemption read-out (§6.4), as `wincheck` already compiles it (wincheck/Cargo.toml:31).
  - Not needed: `Win32_UI_HiDpi` (no framed window; DPI awareness is `kairo-display`'s), `Win32_Security` (no timer).

---

## 2. The swap chain and presents

### 2.1 Window

- `kairo_display::make_dpi_aware()` first, then `targets()` and `find_by_path(--monitor)`; refuse a mirrored display (tb/src/run.rs:206–227). Exit 3 if not found.
- `WS_POPUP` at `target.rect` (tb/src/run.rs:281–287), shown with `SW_SHOWNOACTIVATE`, or brought to the front with `--activate` (tb/src/run.rs:330–344). After showing, `client_rect()` must equal the monitor rectangle exactly, else exit 3 "the window doesn't cover the monitor": DirectFlip needs it (§2.4).
- The process holds `KeepDisplayOn` for its lifetime and opts out of EcoQoS (tb/src/run.rs:207, :325).
- No `WS_EX_LAYERED`, no `WS_EX_TOPMOST`, no `SetWindowDisplayAffinity`, and the swap chain never gets `DXGI_SWAP_CHAIN_FLAG_DISPLAY_ONLY`: that flag makes content "not accessible via remote accessing or through the desktop duplication APIs" [MS-SCFLAG], and Kairo's capture must see loadgen.

### 2.2 Creation

```rust
// gpu.rs, sketch for windows =0.62.2. Every call below sits in an unsafe block with // SAFETY:.
let f5: IDXGIFactory5 = factory.cast()?;                      // factory as in tb/src/gpu.rs:78, :101
let mut tearing = BOOL(0);
f5.CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING,     // win/Win32/Graphics/Dxgi/mod.rs:392, :3240
    (&raw mut tearing).cast(), size_of::<BOOL>() as u32)?;
if !tearing.as_bool() { return Err(Refuse::NoTearing) }        // exit 3: never fall back to sync interval 0 without it
let flags = (DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 | DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0) as u32;
let desc = DXGI_SWAP_CHAIN_DESC1 {
    Width: w, Height: h, Format: DXGI_FORMAT_B8G8R8A8_UNORM,
    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
    BufferCount: 3, Scaling: DXGI_SCALING_NONE, SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
    AlphaMode: DXGI_ALPHA_MODE_IGNORE, Flags: flags, ..Default::default() };
let swap = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;   // same file :2875
factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;  // as tb/src/gpu.rs:154–159
let swap2: IDXGISwapChain2 = swap.cast()?;
swap2.SetMaximumFrameLatency(2)?;                              // the waitable flag's default is 1 [MS-LAT2]
let waitable = swap2.GetFrameLatencyWaitableObject();          // same file :5687; CloseHandle in Drop
```

- The flags follow Microsoft's conditions [MS-PRESENTFLAGS]: "The swap chain must be created with the DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING flag", "The sync interval … must be 0", and full-screen Win32 apps "should present to a fullscreen borderless window and disable automatic ALT+ENTER fullscreen switching using IDXGIFactory::MakeWindowAssociation". Breaking them returns `DXGI_ERROR_INVALID_CALL`.
- `ALLOW_TEARING` "requires the use of a DXGI_SWAP_EFFECT_FLIP_* swap effect", and `ResizeBuffers` "can't be used to add or remove this flag" [MS-SCFLAG]; any `ResizeBuffers` must pass the same flag [MS-VRR]. Loadgen never resizes: a display change ends the run (§7.4).
- With the waitable flag, latency "must be set with the IDXGISwapChain2::SetMaximumFrameLatency API instead of IDXGIDevice1::SetMaximumFrameLatency", and the flag "isn't supported in full-screen mode, unless the render API is Direct3D 12" [MS-SCFLAG]. Loadgen is never in exclusive full screen.
- **[Unverified]** both flags together on a D3D11 flip swap chain. Nothing on Learn forbids it; PR L3's PC run checks it. Fallback: no waitable object, `IDXGIDevice1::SetMaximumFrameLatency(2)` (range 1–16, default 3 [MS-LAT1]) and a `Present` that blocks when the queue is full.

### 2.3 One frame

```
WaitForSingleObject(waitable, 1000)            timeout → check stop flags and GetDeviceRemovedReason, loop
t = (now_hns − anchor_hns) as f64 / 1e7        QPC time drives the picture, never the frame count (§3.4)
Map(cb, WRITE_DISCARD) ← FrameConstants; Unmap
ctx.Begin(disjoint[s]); ctx.End(ts_begin[s])   s = frame mod 8 (§5)
OMSetRenderTargets([rtv], None)                Present unbinds back buffer 0 every time [MS-PRESENT], [MS-FLIP]
RSSetViewports(full); RSSetState(scissor_on); IASetPrimitiveTopology(TRIANGLELIST); IASetInputLayout(None)
VSSetShader(vs_fullscreen); PSSetShader(ps_alu | ps_bw); PSSetConstantBuffers(0, cb); PSSetShaderResources(0, big)
for band in 0..16: RSSetScissorRects([band rect]); Draw(3, 0)
ctx.End(ts_end[s]); ctx.End(disjoint[s])
hr = swap.Present(0, DXGI_PRESENT_ALLOW_TEARING)      HRESULT, not Result, in 0.62.2 (win/Win32/Graphics/Dxgi/mod.rs:5234)
poll older query sets oldest first (DONOTFLUSH), write their `f` lines in order
```

- The RTV is made once from buffer 0 (tb/src/gpu.rs:403–415): in D3D11's flip model buffer 0 is always the current back buffer, but it has to be bound again after every `Present`.
- No vertex buffer: `vs_fullscreen` builds one covering triangle from `SV_VertexID` (§3.2); R08:554 draws its pass the same way, as a 4-vertex strip.
- CPU work per frame is a few dozen calls [Estimate: tens of µs]; the thread spends the rest of the frame in the wait.

### 2.4 What Windows needs for independent flip here

Microsoft's description [MS-FLIP]:
- "DirectFlip: Your swapchain buffers match the screen dimensions, and your window client region covers the screen."
- "Once your swapchain has been 'DirectFlipped,' then the DWM can go to sleep … This is 'Independent Flip' … If other desktop contents come on top, the DWM can either seamlessly transition back to composed mode, efficiently 'reverse compose' the contents on top of the application before flipping it, or leverage MPO to maintain the independent flip mode."
- "Check out the PresentMon tool to get insight into which of the above was used."

So loadgen has to provide, and the bench has to keep:

| Condition | How |
|---|---|
| Flip model | `FLIP_DISCARD` (§2.2) |
| Buffers = screen size, client = screen | Popup at the monitor rect in a DPI-aware process; swap chain at the client size; exact-match check, exit 3 otherwise (§2.1) |
| The device on the adapter that scans the monitor out | LUID from `kairo-display` (M1:66); a device on the other adapter would be composed across adapters [Unverified for this PC; `displays.json` shows which adapter drives the 180 Hz monitor] |
| Nothing on top of it on that monitor | Bench preconditions: no Adrenalin overlay or Game Bar (R13:823–828), no toast. A taskbar shown on the 180 Hz monitor is topmost and overlaps the window unless the shell hides it for a full-screen foreground window. **[Unverified]** whether a non-foreground covering window gets independent flip; hence `--activate` for bench runs, and PR L3 compares both |
| A format and colour space the output scans out directly | BGRA8 on an SDR desktop. On an HDR output an SDR swap chain is converted, so it is presumably composed [Unverified]; loadgen refuses HDR in M1 (exit 3, like the harness's capture runs, R16:411) |
| Not rotated | Log `DXGI_OUTPUT_DESC.Rotation`; a rotated output is a [Unverified] case the bench avoids |

- `ALLOW_TEARING` + sync interval 0 doesn't decide independent flip; it lets an independent-flip swap chain flip immediately (tearing) and lets a composed one present uncapped, the queue keeping only the newest frame: sync interval 0 means "Cancel the remaining time on the previously presented frame and discard this frame if a newer frame is queued" [MS-PRESENT].
- With the 180 Hz monitor's VRR (FreeSync) on and loadgen near 125 fps, the panel follows the presents and nothing tears; with VRR off, tear lines move over the screen. Either way, record VRR in `env.json` [Unverified: how PresentMon's `AllowsTearing` reads under VRR].
- Windows 11's "Optimizations for windowed games" only moves blt-model DX10/11 games to flip (R01:165); loadgen is flip already.
- Kairo's own capture may demote the window (R13:784, R08:430, R01:166, CB:1210). That is a real cost of Kairo, which is why the bench compares the mode shares of A and B (R13:731).

### 2.5 Can loadgen tell which present mode it got?

- **Not from a documented D3D11/DXGI call.** PresentMon decides from DxgKrnl ETW events: an MMIO flip turns `Composed_Flip` into `Hardware_Independent_Flip` and marks `SupportsTearing` on an immediate flip (PM/PresentData/PresentMonTraceConsumer.cpp:718–740; a non-MMIO `Flip_Info` with interval 0 also marks it, :914–934), `IndependentFlip_Info` sets independent flip (:936–957), and a multi-plane flip makes it `Hardware_Composed_Independent_Flip` (:1145–1165). The CSV strings are at PM/PresentMon/CsvOutput.cpp:25–34.
- **A self-trace is possible but rejected.** The owner's account is in Performance Log Users since M0 (M0:11, M0:114; R13:539), so an unelevated ETW session would work. It would duplicate PresentMon's consumer (MIT code to port), add tracing work inside the process being measured, and could disagree with the bench's own source. Elevation stays out of it (CL:10).
- **Hints loadgen logs instead** (`e present_hint …` at ready, then every 10 s) [proposed]:
  - `media=composed|overlay|none|failure|unavailable`: `IDXGISwapChainMedia::GetFrameStatisticsMedia().CompositionMode` (win/Win32/Graphics/Dxgi/mod.rs:396–419, :5933–5945). Learn documents it for decode (YUV) swap chains: "used to determine whether the app should continue to use the decode swap chain" [MS-MEDIA]. **[Unverified]** whether a plain RGB swap chain answers and what independent flip reads as; a hint, never a verdict.
  - `hwcomp=fullscreen,windowed,cursor_stretched|-`: `IDXGIOutput6::CheckHardwareCompositionSupport` (win/Win32/Graphics/Dxgi/mod.rs:4635, flags :470–472), once at start; "windowed" means "swapchain composition can be facilitated in a performant manner using hardware for windowed applications" [MS-HWCOMP], the condition for "Hardware Composed: Independent Flip".
  - `occluded=<n>`: presents that returned `DXGI_STATUS_OCCLUDED`.
- **The truth for the bench** is per frame in PresentMon's CSV (every row writer, PM/PresentMon/CsvOutput.cpp:684–692, :906–918, :1131–1145, prints these as decimal integers and strings): `SyncInterval` (0), `PresentFlags` (512 for `DXGI_PRESENT_ALLOW_TEARING` = 0x200, win/Win32/Graphics/Dxgi/mod.rs:1336), `AllowsTearing` ("1 if partial frames might be displayed on the screen", PM/README-ConsoleApplication.md:94) and `PresentMode` (:135–145). `pm-stats.ps1` already reports `present_modes` (M1:85); counting `AllowsTearing` and checking the flags is the bench part's call. Proposed rule: ≥99% of a run's rows in one mode, the same mode in every run of a session [proposed].
- PresentMon targets loadgen by PID: `--process_id id` "Only record the process with the specified process ID" (PM/README-ConsoleApplication.md:16), with the PID from `ready.json`, which is written after the start-up ramp (§6.2), so no ramp frame reaches a CSV.

### 2.6 Frame latency and queue depth when uncapped

- Without a waitable object, `Present` blocks once 3 frames are queued (the default latency) [MS-LAT1].
- With the waitable object at latency L, the thread waits before each frame:
  - **L = 1** (the swap chain's default): the CPU records frame N+1 only after frame N retires, so the GPU idles between frames for the wake-up and record time [Estimate: 0.05–0.5 ms per frame, up to about 6% at 8 ms], and that idle time moves with CPU scheduling, which Kairo's own threads affect. Noisy for a GPU-bound test.
  - **L = 2** (proposed default): one frame is always queued behind the one the GPU runs, so the GPU never waits for the CPU; the frame rate is 1 / GPU time, as for a GPU-bound game without a latency reducer.
- **3 buffers**: with L = 2, one buffer is on screen, one renders, one is queued. Microsoft notes the GPU stalls at the first write to a back buffer still in use [MS-FLIP]; a third buffer keeps that from happening in independent flip and leaves room for DWM's buffer when composed.
- `--latency 1..3` and `--buffers 2..4` exist for M2's sensitivity checks; both are logged. Driver-side queue control (Radeon Anti-Lag) would override L: the bench's off-list (R13:823–828) doesn't name it yet [Unverified effect; cross-note for the bench part].
- Expected PresentMon signature in A runs [Estimate]: `MsBetweenPresents` ≈ GPU frame time, `MsGPUBusy` ≈ `MsBetweenPresents`, `MsGPUWait` ≈ 0, `MsInPresentAPI` small; Kairo's GPU work then shows as higher `MsGPUWait`/`MsGPUTime` at the same busy time (R13:567).

### 2.7 Present results

| `Present` returns | Loadgen |
|---|---|
| `S_OK` | Count |
| `DXGI_STATUS_OCCLUDED` (success code) | Count `occluded`; keep rendering (the GPU load must not change) |
| `DXGI_ERROR_DEVICE_REMOVED`, `_RESET`, `D3DDDIERR_DEVICEREMOVED` (tb/src/gpu.rs:471–472, :531–541) | `e device_lost reason=0x…` from `GetDeviceRemovedReason`, exit 4; never recreate (§6.3) |
| `DXGI_ERROR_INVALID_CALL` | A tearing condition broke (exclusive full screen can't happen with `NO_ALT_ENTER`); `e present_failed hr=0x…`, exit 4 |

---

## 3. The two workloads

### 3.1 Common to both

- **Draws.** One triangle that covers the target, drawn 16 times per frame [proposed], each time scissored to one horizontal band of ⌈H/16⌉ rows (the last band takes the rest). Rasterizer state: `ScissorEnable = TRUE`, no culling. A pure `bands(h, n)` covers every row exactly once (unit test).
  - Why bands: per-draw time stays near 0.5 ms at the 8 ms target, which keeps any draw far from the TDR limit whatever the GPU's preemption granularity (§6), and lets DWM's composition of the other monitor and Kairo's own GPU work interleave every half millisecond, as they would with a game's many draws.
- **Constant buffer** (32 bytes, `D3D11_USAGE_DYNAMIC`, `Map(WRITE_DISCARD)` each frame):

```rust
#[repr(C)]
struct FrameConstants { drift: f32, scroll_px: f32, count: u32, seed: u32, inv_size: [f32; 2], tex_mask: [u32; 2] }
// drift = frac(t / 20 s) and scroll_px = (t · 120 px/s) mod 8192 are computed in f64 on the CPU,
// so precision never degrades over a long run and the picture wraps seamlessly.
```

- **Target**: about 8 ms of GPU per frame, as R13:707 asks for ALU; the same target for bandwidth [proposed], so both run near 125 fps, below the 180 Hz refresh.
- **Picture**: both modes draw `visual(px, t)` (§3.4) and add a ±0.5-code term computed from their work, which keeps the compiler from removing the work and stays invisible.

### 3.2 ALU mode

R13:713–724's shader is the model: a `[loop]` of `iters` steps per pixel. Two changes: the loop variable is per pixel, so no part of the work can become uniform (scalar) work, and the visible output comes from `visual()`, not from the loop, because R13's output cycles the whole screen's colour once per second (`cos(6.2831 * (… + t))`), which §8 rules out.

```hlsl
// loadgen.hlsl (sketch). Written for Kairo; no third-party shader code (CL:17).
cbuffer Frame : register(b0) { float drift; float scroll_px; uint count; uint seed; float2 inv_size; uint2 tex_mask; };
Texture2D<uint> big : register(t0);

float4 vs_fullscreen(uint id : SV_VertexID) : SV_Position {
    float2 p = float2((id << 1) & 2, id & 2);                 // (0,0) (2,0) (0,2)
    return float4(p.x * 2 - 1, 1 - p.y * 2, 0, 1);
}

float4 ps_alu(float4 pos : SV_Position) : SV_Target {
    float x = frac(dot(pos.xy, float2(0.6180339, 0.4142135)));   // per pixel
    float acc = 0;
    [loop] for (uint k = 0; k < count; ++k) {
        x = frac(x * 1.0001 + 0.5 * sin(x * 7.31 + k * 0.01)); // one transcendental + a few ALU ops
        acc += x;
    }
    return float4(visual(pos.xy) + (acc / max(count, 1) - 0.5) / 255.0, 1);
}
```

- Cost is linear in `count` and the same for every pixel and frame: no data-dependent branch, a uniform trip count, no divergence. Calibration (§3.5) finds `count` [Estimate: thousands at 1440p].

### 3.3 Bandwidth mode

- **Texture**: 256 MiB = `R32_UINT` 8192 × 8192 (`--texture-mib 1..1024`, powers of two, square power-of-two sides).
  - Limits: Texture2D sides up to 16,384, and a resource up to "min(max(128,0.25f \* (amount of dedicated VRAM)), 2048) MB"; point-sampled shader reads address "max(128,0.25f \* (amount of dedicated VRAM)) MB" [MS-LIMITS] (constants at win/Win32/Graphics/Direct3D11/mod.rs:3504–3512). 256 MiB is within both once VRAM ≥ 1 GiB; **WARP may stop at 128 MB** [Unverified], so CI uses a small texture (§10).
  - 256 MiB is larger than any RDNA 4 last-level cache (Infinity Cache, ≤64 MB on the series [Unverified per model]), so random reads go mostly to DRAM, which is the point: Kairo's copies and conversions are partly bandwidth (R13:708).
- **Fill**: once at start, on the GPU: the texture is also a render target, `ps_fill` writes `mix32(x ^ mix32(y ^ seed))` in 16 bands, then the RTV is unbound before the SRV is bound. No 256 MiB CPU buffer, deterministic from `--seed`.
- **Reads**: `count` independent random loads per pixel, addresses from an integer hash of (pixel, k, seed). Independent, not pointer-chasing, so many loads are in flight and the memory system, not latency, is the limit.

```hlsl
uint mix32(uint v) { /* two xorshift-multiply rounds; constants chosen at implementation, avalanche unit-tested */ }

uint ps_fill(float4 pos : SV_Position) : SV_Target { uint2 p = uint2(pos.xy); return mix32(p.x ^ mix32(p.y ^ seed)); }

float4 ps_bw(float4 pos : SV_Position) : SV_Target {
    uint2 p = uint2(pos.xy);
    uint h = mix32(p.x ^ mix32(p.y ^ seed)), acc = 0;
    [loop] for (uint k = 0; k < count; ++k) {
        h = mix32(h + k);
        acc += big.Load(int3(h & tex_mask.x, (h >> 16) & tex_mask.y, 0));
    }
    return float4(visual(pos.xy) + (float(acc & 255) / 255.0 - 0.5) / 255.0, 1);
}
```

- Most loads miss the caches and fetch one line each from DRAM, so traffic ≈ W × H × `reads` × line size [Estimate: at a 2560 × 1440 example and 64 B lines, `reads` = 8 moves about 1.9 GB per frame]; calibration finds `reads` [Estimate: single digits to tens].
- The seed is fixed per run (`--seed`, default `0x4B4C_4731` [proposed]), so every frame reads the same addresses. Re-reading doesn't help the caches: with 256 MiB against ≤64 MB of cache, the lines read first in a frame were evicted long before.
- A linear 256 MiB `ByteAddressBuffer` with 64-byte bursts would move more bytes per transaction; it stays an option for M2, since R13 asks for a texture and games read tiled textures.

### 3.4 The picture and the time base

- `visual(px, t)`: a muted colour field plus a soft, low-contrast noise layer [proposed]:

```hlsl
float3 visual(float2 px) {
    float2 uv = px * inv_size;
    float a = 6.2831853 * (drift + 0.5 * uv.x + 0.25 * uv.y);           // under one colour cycle across the screen
    float3 c = 0.30 + 0.05 * cos(a + float3(0, 2.0943951, 4.1887902)); // sRGB codes ≈ 64–89, desaturated
    return c + (value_noise(float2(px.x + scroll_px, px.y)) - 0.5) * (12.0 / 255.0);  // ±6 codes, grey
}
// value_noise: smoothstep-interpolated hash on a 32 px lattice that wraps every 256 cells (8192 px).
```

- **Time-based and deterministic.** The picture is a pure function of the pixel and t = (frame start − anchor) in QPC time, never of the frame count: two runs show the same picture at the same t whatever their frame rates, and the encoder sees comparable content in A and B.
- **It changes every frame.** WGC delivers only what DWM composes, so a static window would give Kairo nothing to capture (R08:387). The noise moves about one pixel per frame at 125 fps; the colour drift moves by a fraction of a code per frame.
- **Why the noise layer** (`--detail on`, default [proposed]): it gives Kairo's encoder real motion and texture, closer to a panning game than a smooth gradient that encodes as skip blocks, while staying far inside §8's limits. `--detail off` keeps only the drift. Either is logged.
- The two ±0.5-code work terms are static per pixel (they depend on the pixel, `count` and the seed), so they never flicker.

### 3.5 Making runs comparable

- **Explicit counts.** `run --workload alu --iters N` or `run --workload bw --reads N`: no default count, so a run can't quietly differ from the last. Nothing changes during a run; the ramp (§6.2) only refuses.
- **Everything in the header**: workload, count, texture size and seed, bands, size, format, buffers, latency, present flags and sync interval, detail on/off, and an FNV-1a 64 hash of each compiled bytecode (the corpus manifests' hash, M1:90), which shows a Windows update that changed `d3dcompiler_47.dll`'s output.
- **`calibrate`, never inside a measured run**:
  1. Same window, swap chain, bands and picture as `run`, after the bench's 2-minute warm-up (R13:830), so clocks and temperature match the runs.
  2. Doubling steps from `count` = 16 (ALU) or 1 (bandwidth), each ≥30 frames, median GPU ms from the timestamps; the TDR caps of §6.2 apply before every step.
  3. Stop when the median passes half the target; least-squares fit ms = a + b · count over the last three steps; refuse (exit 3) if R² < 0.99 [proposed].
  4. Verify the fitted count for 5 s; accept within ±3% of the target [proposed], else one correction step.
  5. Print one JSON line and, with `--out FILE`, write it create-new: `{"kairo_loadgen_calibration":1,"workload":"alu","target_ms":8.0,"count":…,"gpu_ms_p50":…,"fit":{"a_ms":…,"b_ms":…,"r2":…},"bands":16,"size":"WxH","texture_mib":256,"shader":"fnv64:…"}`. It is PC data: `C:\kairo\logs` and `.dogfood/` only (CL:18).
- **When to calibrate again** [proposed]: when the shader hash, the size, the driver version (`env.json`) or the adapter changes. A/B pairs in one session always share one count.
- **One process per run** is the recommendation for the bench [proposed]: the A/A spread then includes the launch-to-launch variation (present-mode negotiation, clocks) that separate A and B runs will also have.

---

## 4. Shader compilation without GPL code

| Route | `windows` features | Mac (`cargo xwin check`) | Windows CI (`windows-latest`) | PC builds | Verdict |
|---|---|---|---|---|---|
| **Runtime `D3DCompile`** from `d3dcompiler_47.dll` | `Win32_Graphics_Direct3D_Fxc` + `Win32_Graphics_Direct3D` | Type-checks; nothing links. `windows-link` imports through `#[link(name = …, kind = "raw-dylib", modifiers = "+verbatim")]` (wlink:17–26; `D3DCompile` at win/Win32/Graphics/Direct3D/Fxc/mod.rs:9), so no import library is ever needed (CB:377; wincheck compiled it, CB:11) | Builds and links; a `#[test]` compiles every entry point with no GPU (CB:1189) [Unverified: the DLL is in the image's System32] | Uses the inbox DLL | **Chosen** |
| `fxc.exe` in `build.rs` | `Win32_Graphics_Direct3D11` only (bytes in) | No `fxc.exe` on the Mac (CB:377), and `build.rs` runs on the host, so it would need a skip path | Windows SDK has it | VS Build Tools + SDK have it (CB:1101) | Two build paths for one benefit |
| Committed `.cso` bytecode | as above | Works | Works, plus a regeneration check with `fxc` | Works | Opaque binaries in a public repo, drift between HLSL and bytes |
| `dxc` | `Win32_Graphics_Direct3D_Dxc` | — | — | — | **Unusable**: dxc compiles "HLSL programs for shader model 6.0 or higher" to DXIL or SPIR-V [DXC]; D3D11's targets are `vs_5_0`, `ps_5_0`, `cs_5_0` … [MS-TARGETS], DXBC only |

- **The DLL.** Learn lists `d3dcompiler_47.dll` as `D3DCompile`'s DLL [MS-COMPILE]. Microsoft's Chuck Walbourn: version 47 "is now included with the Windows 8.1 / Windows 10 OS", and "For development purposes, it is often very convenient to use runtime compilation" while shipping titles compile at build time [WALBOURN]. Loadgen is a dev instrument; the engine made the same runtime choice (CB:377).
- **Licences.** The DLL is a Windows system component, loaded like `d3d11.dll` and `dxgi.dll`; the HLSL is written for Kairo, with no OBS or other GPL shader (CL:17; M1:202). No new crate, so no `deny.toml` change.
- **Call** (wincheck/src/shaders.rs:4–14 is the compiled pattern):

```rust
// gpu/shader.rs: SAFETY: the source slice outlives the call; both out-pointers are locals.
D3DCompile(SRC.as_ptr().cast(), SRC.len(), s!("loadgen.hlsl"), None, None::<&ID3DInclude>, entry, target,
           D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_WARNINGS_ARE_ERRORS, 0,
           &mut code, Some(&mut errors as *mut _))
// SRC = include_str!("loadgen.hlsl"); entries: vs_fullscreen/vs_5_0, ps_alu, ps_bw, ps_fill/ps_5_0.
```

- `pSourceName` is a bare name, used only "for strings that specify error messages" [MS-COMPILE], so compiler errors on stderr carry no path (CL:16). No `#include`, so `pInclude` stays null ("will cause a compile error if a shader contains a #include").
- A compile failure is exit 4 with the error blob's text.
- **DLL search.** The raw-dylib import loads at process start, application folder first. `C:\kairo\bin\<build-id>\` is admin-owned and `pc deploy` copies only checked files (M1:81), so no foreign `d3dcompiler_47.dll` lands there. If the DLL were missing, the loader would stop the process before `main` (no log, an NTSTATUS exit code); `LoadLibraryExW(…, LOAD_LIBRARY_SEARCH_SYSTEM32)` plus `GetProcAddress` would turn that into exit 4 at the cost of more `unsafe`. Not needed on Windows 10/11.

---

## 5. Loadgen's own GPU time per frame (R13 §5.4)

- **Queries.** A ring of 8 sets [proposed], each `{D3D11_QUERY_TIMESTAMP_DISJOINT, D3D11_QUERY_TIMESTAMP, D3D11_QUERY_TIMESTAMP}` (win/Win32/Graphics/Direct3D11/mod.rs:3345–3346), made with `CreateQuery` as wincheck/src/gpu_timer.rs:9–16 does. Per Learn, the disjoint query "should only be invoked once per frame or less", and a timestamp is "only useful if two timestamp queries are done in the middle of a D3D11_QUERY_TIMESTAMP_DISJOINT query" [MS-QUERY]. Frame f uses set f mod 8 as in §2.3.
- **Reading.** After each `Present`, for pending sets oldest first: `GetData` with `D3D11_ASYNC_GETDATA_DONOTFLUSH` (win/Win32/Graphics/Direct3D11/mod.rs:147) through the vtable, comparing with `S_OK`. The 0.62.2 wrapper ends in `.ok()` (same file :7548–7552), which turns `S_FALSE` ("the data is not yet available" [MS-GETDATA]) into success; the helper at R13:684–695 / CB:410–419 / wincheck/src/gpu_timer.rs:3–8 is the fix. Stop at the first set not ready, so `f` lines stay in order.
- **Value.** `gpu_us = (end − begin) × 10⁶ / Frequency` when `Disjoint` is FALSE; "The timestamp … is only reliable if Disjoint is FALSE" [MS-DISJOINT]. A disjoint frame logs `-` and counts in `gpu_disjoint`.
- **Never blocking.** If set f mod 8 is still pending when frame f+8 needs it, frame f gets `-` and counts in `gpu_unknown`. With latency 2 and 3 buffers, results arrive two or three frames later [Estimate], so 8 leaves room. Only `calibrate`, the ramp and the end of a run may poll with a flush, bounded at 2 s.
- **What it measures**: wall time on the GPU between the two marks, including time other processes' work (Kairo's shaders, DWM) takes from the frame. So loadgen's GPU ms rises with contention, like PresentMon's `MsGPUTime`; with `--qpc_time` (M1:74) PresentMon's `CPUStartQPC` can be matched to loadgen's `f` lines, which carry QPC times.
- **Summary**: p50, p99 and max GPU ms, plus the two counts, go to the stdout JSON (§7.4).

---

## 6. TDR: keeping every draw far below 2 s

### 6.1 The risk

- "The GPU scheduler … tries to preempt this particular task. The preempt operation has a 'wait' timeout, which is the actual TDR timeout. The default timeout period in Windows is two seconds. If the GPU can't complete or preempt the current task within the TDR timeout period, the OS diagnoses that the GPU is frozen" [MS-TDR].
- Repeats are worse: "the OS bug-checks the computer on the sixth (or more) GPU hang when it detects that five (5) or more GPU hangs … occur within one (1) minute" [MS-TDR]. A loader that recreated its device and resubmitted the same frame could blue-screen the owner's PC.
- How finely a long draw can be preempted depends on the driver's reported granularity, which is "the coarsest granularity among all engines"; at `PRIMITIVE_BOUNDARY` the driver "cannot stop currently running primitive buffers" [MS-PREEMPT]. One full-screen triangle is one primitive, so on such a GPU a draw's length is its preemption delay.
- At the 8 ms target nothing is near the limit. The danger is misconfiguration: a count calibrated elsewhere, a bigger screen, the device on the iGPU, WARP.

### 6.2 Guards

| Guard | Rule |
|---|---|
| Bands | 16 draws per frame: about 0.5 ms each at the target, 4,000× below 2 s [Estimate] |
| Parameter caps | `--iters` ≤ 65,536, `--reads` ≤ 4,096, `--bands` 1–64, texture ≤ 1,024 MiB [proposed] |
| **Start-up ramp** in `run` | Before the requested count is ever submitted: steps of ×4 from `count` = 16 (ALU) or 1 (bandwidth), each ≥10 frames, timed by the queries. A step runs only if 4 × the previous step's median predicts ≤ 25 ms per band and ≤ 100 ms per frame [proposed]; otherwise exit 3 with `e too_heavy predicted_ms=…`. The first step is tiny on any GPU, so even an absurd request is refused before a long frame exists. The ramp never changes the requested count: it reaches it or refuses. `ready.json` comes after the ramp, so PresentMon never records it |
| `calibrate` | The same caps before each doubling step |
| Run-time guard | Three frames in a row above 250 ms of GPU time end the run (`e too_heavy`, exit 3) [proposed]; it catches the device landing somewhere slow mid-run |
| Fill pass | 16 bands of one integer hash each: trivial |
| Never | `D3D11_CREATE_DEVICE_DISABLE_GPU_TIMEOUT` (win/Win32/Graphics/Direct3D11/mod.rs:855), which lets "such a long running packet … execute without resetting the GPU" [MS-DEVFLAG]; `TdrDelay`/`TdrLevel` registry changes (a PC change outside Kairo's folders, CL:43, and the wrong fix) |

### 6.3 When the device is lost anyway

- `Present`, `Map` or a query read reports a removed device: log `e device_lost reason=0x…` from `GetDeviceRemovedReason` and exit 4, as the testbed does (tb/src/run.rs:498–513). `DXGI_ERROR_DEVICE_HUNG` blames "badly formed commands sent by the application", `DXGI_ERROR_DEVICE_RESET` is "a run-time issue" [MS-ERR]; both stop the run. Never recreate the device or retry (§6.1).
- The bench treats a loadgen exit 4 after `ready` like `pc harness` treats a TDR: read System events 4101 and 41 since the job started, and any of them is P0, stop and report (R16:866, M1:214).

### 6.4 Optional read-out: preemption granularity

`D3DKMTOpenAdapterFromLuid` on loadgen's own LUID, then `D3DKMTQueryAdapterInfo(KMTQAITYPE_WDDM_1_2_CAPS)` → `D3DKMT_WDDM_1_2_CAPS.PreemptionCaps.GraphicsPreemptionGranularity` (win/Wdk/Graphics/Direct3D/mod.rs:421, :548, :4225–4229, :10303–10306, :13294; links `gdi32.dll`), then `D3DKMTCloseAdapter`. Logged as `preempt=none|dma|primitive|triangle|pixel|shader|-`. An adapter handle on our own adapter, read-only, no process involved. **[Unverified]** what the RX 9000-series driver reports.

---

## 7. CLI, ready file, log, summary and exit codes

### 7.1 Command line

```
kairo-loadgen run --monitor <monitorDevicePath> --workload alu --iters N | --workload bw --reads N
                  [--texture-mib 256] [--seed N] [--bands 16] [--buffers 3] [--latency 2] [--detail on|off]
                  [--seconds 60] [--activate] [--log FILE] [--ready FILE] [--stop-on-stdin-eof]
kairo-loadgen calibrate --monitor <monitorDevicePath> --workload alu|bw [--target-ms 8] [--texture-mib 256]
                  [--seed N] [--bands 16] [--buffers 3] [--latency 2] [--detail on|off] [--activate]
                  [--out FILE] [--log FILE] [--stop-on-stdin-eof]
kairo-loadgen --version | --help
```

- As in the testbed (R16:289, R16:402–403): `--monitor` is required and has no default screen; `--seconds` has no cap of its own (devrun's `max_seconds` bounds every job, M1:98); the run ends at `--seconds` after frame 0, at `WM_CLOSE`, or at stdin EOF only with `--stop-on-stdin-eof`, because devrun's stdin is null (devrun/run.rs:209). Each flag once; unknown arguments are refused.
- `--seconds` counts from the first full-workload frame. For a bench run it must cover PresentMon's `--delay` + `--timed` + margin [Estimate: 75 s for 5 + 60].
- **For M2, not built in M1**: `--oled`, which caps `--seconds` at 600 for SP1's ≤10-minute chunks (M1:40); an HDR format option if SP3 needs one. M1 has no OLED guard in loadgen itself: which screen is the OLED lives in `.dogfood/` (M1:38), so `pc bench` refuses the `oled` alias for loadgen in M1, as `pc harness` does for capture (R16:861).

### 7.2 `ready.json`

Written once the ramp has reached the full workload and that frame was presented, through the moved create-new writer (tb/src/ready.rs:62–73):

```json
{"kairo_loadgen_ready":1,"pid":4242,"hwnd":197794,"client":{"left":-2560,"top":0,"width":2560,"height":1440},
 "refresh_mhz":179960,"adapter_luid":"0x…","dxgi_adapter_index":1,"dxgi_output_index":0,"format":"bgra8",
 "workload":"alu","count":3072,"bands":16,"buffers":3,"latency":2,"tearing":true,"detail":true,
 "start_hns":123450000000,"seconds":75}
```

(Example values.) It names the window handle and the adapter of this run, so it stays local, like the testbed's (tb/src/ready.rs:1–5). Its `pid` feeds PresentMon's `--process_id`; `pid` and `hwnd` let devrun's future full-screen stop exempt the job's own window (M1:103, core/watch.rs:25–31: today `watch.json` reports loadgen's foreground window as busy).

### 7.3 The log

```
# kairo-loadgen 1
h workload=alu count=3072 texture_mib=- seed=- bands=16 size=2560x1440 refresh_mhz=179960 format=bgra8 buffers=3
  latency=2 present=512 sync=0 detail=1 shader=<fnv64 per entry> preempt=pixel hwcomp=fullscreen,windowed hdr=0
  start_hns=<QPC of frame 0>                                    (one line in the file)
f <n> <start_hns> <present_hns> <gpu_us|->
e <name> [key=value …]   ramp step=<k> count=<c> gpu_us=<p50> · activate result=ok|failed · ready · present_hint media=… occluded=<n>
                         · too_heavy predicted_ms=<x> · display_changed · device_lost reason=0x… · present_failed hr=0x…
                         · end reason=seconds|stdin|close|error
```

- Same rules as `kairo_core::testbed_log` (core/testbed_log.rs:1–30): UTF-8 text, times in QPC 100 ns (`kairo_core::timeline::hns_from_qpc`), one `write_all` per line, a torn last line dropped by readers, setup events held until the header.
- `f` lines are written when their GPU time resolves, in frame order: about 3.75 KB/s at 125 fps [Estimate].
- No device paths, monitor, adapter or PC names, window titles or wall-clock time (CL:16). The format lives in the loadgen crate; it moves to `kairo-core` only if a Rust reader appears (an M2 bench verb in `kairo-harness`, say).

### 7.4 Summary and exit codes

- At exit after `ready`, one JSON line on stdout (devrun keeps it as the job's `out.log`, devrun/run.rs:210):
  `{"kairo_loadgen":1,"workload":"alu","count":3072,"frames":N,"seconds":S,"fps":…,"gpu_ms_p50":…,"gpu_ms_p99":…,"gpu_ms_max":…,"gpu_unknown":k,"gpu_disjoint":d,"occluded":m,"end":"seconds"}`.
  Loadgen's own `fps` is informational; PresentMon's CSV is the measurement.
- A display change during a run (resolution, refresh, HDR toggle, through the HDR watch's factory `IsCurrent`, tb/src/gpu.rs:248–282) ends it with `e display_changed`, exit 3: the run is no longer comparable.

| Exit | Meaning (testbed numbering, tb/src/cli.rs:24–36) |
|---|---|
| 0 | Ran to `--seconds`, or stopped by `WM_CLOSE` or stdin EOF; `calibrate`: result written |
| 2 | Bad command line; log, ready or `--out` file exists or can't be made |
| 3 | Refused: monitor not found or mirrored, HDR output, no tearing support, client ≠ monitor, the ramp's or run-time TDR guard, a display change, a calibration that can't reach its target within the caps |
| 4 | Device, swap chain, shader compile, or device lost mid-run |
| 5 | `--activate` didn't get the foreground |

devrun's own stop gives the job exit code 1 (core/job.rs:17) with the end reason in its result, so loadgen leaves 1 unused; 6 stays the `pc` tools' safety-stop code (M1:98).

### 7.5 A bench job (`.dogfood/jobs/`, since it names a monitor, M1:76)

```json
{"id":"bench-<stamp>-alu-1","cmd":["C:\\kairo\\bin\\current\\kairo-loadgen.exe","run","--monitor","<180 Hz device path>",
 "--workload","alu","--iters","<from calibrate>","--seconds","75","--activate","--log","bench-<stamp>\\alu-1.log",
 "--ready","bench-<stamp>\\alu-1.ready.json"],"cwd":"C:\\kairo\\logs","max_seconds":120,"abort_on_input":true}
```

`window` and `priority` take their defaults (hidden console, normal priority: core/job.rs:44–67), as timing jobs should.

---

## 8. Photosensitivity: many minutes, uncapped, on the 180 Hz monitor

**Thresholds** (Xbox Accessibility Guideline 118 [MS-XAG118]):
- Luminance flash: "a 10% change in luminance (where 100% is the maximum luminance of a white screen)", darker value "below 0.8"; failing when flashes come "approximately more than three per second" over "approximately 20 percent or more" of the screen. Also: "Lower intensity flashing can also cause a failure if it's continued for an extended period of time."
- Red flash: either extreme saturated red, "R/(R + G + B) >= 0.8".
- Spatial pattern: "Alternating bands that have high contrast", failing when "The difference in contrast is greater than 10 percent" over about 20% of the screen.

**The picture against them** (computed with `research17/scripts/visual_check.py` and `visual_check2.py` in the Mac scratch folder: the drift on a 65 × 65 grid over a full 20 s cycle, then drift plus the scrolling noise at a 2560 × 1440 example size, sampled every 37 px over 20 s):

| Property | Value | Limit |
|---|---|---|
| sRGB codes used | 58–95 | — |
| Relative luminance | 0.051–0.103 of white: the whole picture spans 5.2% | Flash and spatial: 10% |
| Largest per-pixel change in 1/3 s | 0.0016 of white from the drift; 0.024 with the noise passing | 10% |
| Largest screen-average change in 1/3 s | 0.0009 of white | 10% over ≥20% of the screen |
| Noise layer | ±6 codes ≈ ±1.2% of white, random (no bands); ~3.75 cells/s pass a pixel | 10%; no regular stripes |
| Red ratio R/(R+G+B), maximum | 0.394 | 0.8 |
| Screen-average luminance over the 20 s cycle | 0.066–0.083 | No flash: 0.05 Hz, smooth |
| Change between consecutive frames | under one code from the drift; about 1 px of noise movement | Tear lines invisible |

R13:713–724's output isn't fit for many minutes on screen: `0.5 + 0.5 * cos(6.2831 * (uv.xyx + … + t))` takes every pixel through full-range, saturated hues once per second, a moving rainbow at full contrast. Loadgen keeps that shader's work and replaces its output (§3.2).

**More rules** [proposed]:
- No flashes anywhere by construction: nothing in `visual()` is a step in time. The ramp, the run and the end draw the same picture; the window's black background and the desktop around a run are single transitions, one per run (≥75 s apart).
- No high-contrast element ever, so tearing (VRR off) shows at most a one-pixel offset in low-contrast noise.
- **Tests make it a contract** (`visual.rs`, Mac and Linux CI), over a grid and a full cycle [proposed limits, half of XAG's or tighter]: luminance span < 0.08; every 1/3 s change < 0.05 per pixel and < 0.005 for the screen average; red ratio < 0.5; no luminance difference > 0.08 inside any window of 20% of the screen. The WARP test (§10) checks that the GPU draws what `visual.rs` describes.
- The OLED isn't involved in M1 (M1:40, M1:99). The picture is moving content at SDR level, so M2's ≤10-minute OLED chunks with the cover between them (CL:26) need no other picture.

---

## 9. House rules and CI's scan, call by call

| Calls | Rule check |
|---|---|
| `CreateDXGIFactory2`, `EnumAdapterByLuid`, `EnumAdapters1`, `EnumOutputs`, `D3D11CreateDevice`, `CreateSwapChainForHwnd`, `MakeWindowAssociation`, `CheckFeatureSupport`, `SetMaximumFrameLatency`, `GetFrameLatencyWaitableObject`, `Present`, `GetDeviceRemovedReason`, `GetFrameStatisticsMedia`, `CheckHardwareCompositionSupport`, `GetDesc1` | Our own device and swap chain: no other process (CL:7, CL:9) |
| `CreateVertexShader`, `CreatePixelShader`, `CreateBuffer`, `CreateTexture2D`, `CreateShaderResourceView`, `CreateRenderTargetView`, `CreateRasterizerState`, `CreateQuery`, `Map`/`Unmap`, `Begin`/`End`/`GetData`, `OMSetRenderTargets`, `RSSetScissorRects`, `RSSetViewports`, `Draw`, `ClearState`, `Flush` | Own device only |
| `D3DCompile` (d3dcompiler_47.dll) | Code in our own process; no hook |
| `D3DKMTOpenAdapterFromLuid`, `D3DKMTQueryAdapterInfo`, `D3DKMTCloseAdapter` (optional) | An adapter handle on our own adapter; read-only; no process handle (CL:9) |
| `CreateWindowExW`, `RegisterClassExW`, `ShowWindow`, `SetForegroundWindow`, `GetForegroundWindow`, `PeekMessageW`, `DispatchMessageW`, `SetCursor`, `GetClientRect`, `ClientToScreen`, `DestroyWindow`, `SystemParametersInfoW(SPI_GETFOREGROUNDLOCKTIMEOUT)`, `GetLastInputInfo` | Our own window. Taking the foreground sends no input (CL:10, M1:101); `GetLastInputInfo` reads the session's idle time only, as the testbed does (R16:279–281) |
| `QueryPerformanceCounter`/`Frequency`, `SetThreadExecutionState`, `SetProcessInformation(ProcessPowerThrottling)` on `GetCurrentProcess()`, `WaitForSingleObject`, `CloseHandle` | Own process and thread only |
| Not used | Any capture (CL:8), any other process's handle (CL:9), synthetic input, drivers, services, registry writes, elevation (CL:10–11); `SetGPUThreadPriority` (an R13 experiment for Kairo, not loadgen) |

- **CI's scan** (ci:58–76): the forbidden regex (ci:65) and the sockets regex (ci:71) match none of these identifiers. Near misses checked: `PROCESS_POWER_THROTTLING_*` is not `PROCESS_VM_*`, `PROCESS_ALL_ACCESS`, `PROCESS_CREATE_THREAD` or `PROCESS_DUP_HANDLE`; `D3D11_CREATE_DEVICE_DEBUG` doesn't contain `DEBUG_PROCESS`.
- The scan reads every line of every `*.rs` file under `crates/`, comments included (ci:66): loadgen's comments must not name a forbidden API (write "never synthesises input", as tb/src/platform.rs:4–5 does). The HLSL file isn't scanned (M1:102).
- **Privacy**: no clips, audio, keys, window titles or paths in any output (CL:16); ready file, log and calibration are raw results kept in `C:\kairo\logs` and `.dogfood/` (M1:75, M1:78); docs get aggregates (CL:18).
- **Engineering**: `windows` =0.62.2 (CL:30); `unsafe` only in `gpu` and `platform` with `// SAFETY:` on every block, enforced by `undocumented_unsafe_blocks = "deny"` (Cargo:48); no `unwrap`/`expect` outside tests (Cargo:49–50).
- **Clock**: loadgen and the shared crate are instrument work, outside the exit-rule clock (M1:7, M1:110).

---

## 10. Tests

| Tier | Where | What |
|---|---|---|
| **U** | Mac and Linux CI (`cargo test -p kairo-loadgen`; ci:50 gains `-p kairo-loadgen` in PR L2) | CLI parse and refusals; band split covers each row once at odd heights; ramp schedule and the guard's predictions; calibration fit (linear, noisy, non-linear → refused); log lines and the summary JSON; the hash's avalanche; §8's photosensitivity contract on `visual.rs` |
| **X** | Mac | `cargo xwin check` and `clippy --target x86_64-pc-windows-msvc -p kairo-loadgen`. No `FFMPEG_DIR` needed, since loadgen doesn't depend on the FFmpeg crates |
| **W** | Windows CI (ci:150–157) | (1) Every entry point compiles through `D3DCompile` with the production flags, and twice gives the same bytes (CB:1189). (2) WARP: a 320 × 180 offscreen target, ALU at `count` 4 and bandwidth at `count` 2 with a 1 MiB texture, 4 bands: the read-back matches `visual.rs` within ±2 codes (transcendentals differ between WARP and the CPU), and the timestamp ring resolves [Unverified on WARP; the test reports rather than fails if WARP gives no timestamps]. No swap chain on CI: like the testbed's WARP tests (tb/src/gpu.rs:787–817) |
| **P** | PC, PR L3 | §11's acceptance runs |

---

## 11. PR plan

Branches `m1/<slug>`, Conventional Commits with DCO, squash after CI (CL:44). Sizes are lines of code plus tests [Estimate].

| # | Branch | Scope | Size | Tested on | PC and owner |
|---|---|---|---|---|---|
| L1 | `m1/window-lib` | After PR #31 merges: move §1.1's "Move" rows into the shared crate, class name and title as parameters; testbed unchanged in behaviour (WARP tests pass as they are); the harness's `KeepDisplayOn` copy goes if #34–35 have merged; M1 contract's Libraries line (M1:64) | M ~650 moved | Mac (xwin), Linux/Windows CI | none |
| L2 | `m1/loadgen` | The crate: CLI, swap chain with tearing, shaders, workloads, ramp and guards, queries, log, ready, summary, `calibrate`; U/X/W tests; ci:50 adds the crate; M1 contract rows (program, log, ready and calibration file names, formats, exit codes); M1:13 and PLAN:352 say "borderless on the 180 Hz monitor in M1; the OLED only in M2's SP1, in chunks of ≤10 minutes" (step 6's answer, M1:40); CHANGELOG | L ~1,600 | Mac, Linux CI, Windows CI | none |
| L3 | (PC run, no branch unless fixes) | Attended, owner home and hands-off (as R16:208 for gate 2), on the 180 Hz monitor: `calibrate` both workloads; 3 × 60 s runs each with PresentMon by PID: `Hardware: Independent Flip` share, `AllowsTearing`, `SyncInterval` 0, `PresentFlags` 512, loadgen p50 GPU ms within ±1% across runs [proposed]; one pair with and without `--activate`; one run without the waitable object if §2.2's combination fails; System events 4101/41 none | — | PC | **"go" in chat, ~15 min** [Estimate]; no AMD setting touched |

- **Dependencies.** L1 needs PR #31. L2 needs L1. L3 needs L2 and a deploy between the owner's sessions (CL:24).
- Gate 4's A/A (M1:29) then needs `pc bench` and the scripts (the other parts of research 17).
- Runs while the owner is away wait for devrun's safety stops and agreed test days (M1:35, M1:103), which also need the stop to exempt loadgen's window (§7.2).

---

## 12. Owner questions

None new; the monitor was answered (M1:40). Two things to know, no decision needed:
- During bench runs the 180 Hz screen shows a dim, slowly drifting colour field with faint moving grain, and loadgen takes the foreground on that screen, so its taskbar hides. Any touch of keyboard or mouse stops the job.
- AMD Software may list `kairo-loadgen.exe` as a game with its own profile [Unverified]. Claude never changes AMD's settings (R13:811); the diff script and `env.json` record what's there.

---

## 13. Risks and [Unverified] items

| # | Item | Settled by | Fallback |
|---|---|---|---|
| 1 | Independent flip for a covering window that isn't in the foreground (taskbar on that screen) [Unverified] | L3's pair with and without `--activate` | `--activate` always; with MPO, "Hardware Composed: Independent Flip" is acceptable if it is the same in every run |
| 2 | `ALLOW_TEARING` + `FRAME_LATENCY_WAITABLE_OBJECT` on D3D11 [Unverified] | L3, the first PC run | Blocking `Present` with `IDXGIDevice1::SetMaximumFrameLatency(2)` |
| 3 | `GetFrameStatisticsMedia` on an RGB swap chain [Unverified] | L3 logs it next to PresentMon | `media=unavailable`; hints are never verdicts |
| 4 | HDR output → composed [Unverified] | — | HDR refused in M1 |
| 5 | WARP: 256 MiB textures (128 MB rule) and timestamp queries [Unverified] | L2's Windows CI | Small texture; timing part reported, not failed |
| 6 | `d3dcompiler_47.dll` on `windows-latest` [Unverified] | L2's Windows CI | `LoadLibraryExW(SYSTEM32)` route (§4) |
| 7 | A Windows update changes the compiler's output | Bytecode hash in every header | Recalibrate; treat like a driver update (M1:217) |
| 8 | Preemption granularity on the RX 9000-series [Unverified] | §6.4's read-out | Bands keep draws ~0.5 ms either way |
| 9 | The 180 Hz monitor driven by the iGPU instead of the card [Unverified] | `.dogfood/pc/displays.json` | Bench refuses; a cable change is a physical action (CL:43), so Claude asks |
| 10 | Radeon Anti-Lag or Enhanced Sync change the queue or tearing [Unverified] | The diff script, `env.json` | Add both to the bench's off-list (R13:823–828) |
| 11 | VRR changes tearing and maybe `AllowsTearing` [Unverified] | L3 with VRR as the owner has it | Record it; compare like with like |
| 12 | A/A CV ≤1% for a synthetic GPU-bound load [Estimate] | Gate 4 | More runs (M1:29), longer warm-up |
| 13 | Adrenalin per-app profile for `kairo-loadgen.exe` [Unverified] | Diff script | Global settings off, as R13:823–828 |
| 14 | TDR or device loss mid-bench | Exit 4 + events 4101/41 | P0: stop and report (M1:214) |

---

## Sources

Microsoft Learn, read 2026-11-14:
- [MS-VRR] Variable refresh rate displays: https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/variable-refresh-rate-displays
- [MS-PRESENTFLAGS] DXGI_PRESENT: https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-present
- [MS-FLIP] For best performance, use DXGI flip model: https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model
- [MS-PRESENT] IDXGISwapChain::Present: https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-present
- [MS-SCFLAG] DXGI_SWAP_CHAIN_FLAG: https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ne-dxgi-dxgi_swap_chain_flag
- [MS-LAT2] IDXGISwapChain2::SetMaximumFrameLatency: https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-setmaximumframelatency
- [MS-LAT1] IDXGIDevice1::SetMaximumFrameLatency: https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgidevice1-setmaximumframelatency
- [MS-MEDIA] IDXGISwapChainMedia::GetFrameStatisticsMedia, DXGI_FRAME_STATISTICS_MEDIA, DXGI_FRAME_PRESENTATION_MODE: https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchainmedia-getframestatisticsmedia, https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/ns-dxgi1_3-dxgi_frame_statistics_media, https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/ne-dxgi1_3-dxgi_frame_presentation_mode
- [MS-HWCOMP] DXGI_HARDWARE_COMPOSITION_SUPPORT_FLAGS: https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_6/ne-dxgi1_6-dxgi_hardware_composition_support_flags
- [MS-ERR] DXGI_ERROR: https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-error
- [MS-QUERY] D3D11_QUERY: https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_query
- [MS-DISJOINT] D3D11_QUERY_DATA_TIMESTAMP_DISJOINT: https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_query_data_timestamp_disjoint
- [MS-GETDATA] ID3D11DeviceContext::GetData: https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-getdata
- [MS-TDR] WDDM support for timeout detection and recovery: https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery
- [MS-DEVFLAG] D3D11_CREATE_DEVICE_FLAG: https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_create_device_flag
- [MS-PREEMPT] D3DKMDT_GRAPHICS_PREEMPTION_GRANULARITY: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmdt/ne-d3dkmdt-_d3dkmdt_graphics_preemption_granularity
- [MS-COMPILE] D3DCompile: https://learn.microsoft.com/en-us/windows/win32/api/d3dcompiler/nf-d3dcompiler-d3dcompile
- [MS-TARGETS] Specifying compiler targets: https://learn.microsoft.com/en-us/windows/win32/direct3dhlsl/specifying-compiler-targets
- [MS-LIMITS] Resource limits (Direct3D 11): https://learn.microsoft.com/en-us/windows/win32/direct3d11/overviews-direct3d-11-resources-limits
- [MS-XAG118] Xbox Accessibility Guideline 118 (photosensitivity): https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/118

Other Microsoft-authored pages:
- [WALBOURN] Chuck Walbourn, "HLSL, FXC, and D3DCompile": https://walbourn.github.io/hlsl-fxc-and-d3dcompile
- [DXC] DirectX Shader Compiler README: https://github.com/microsoft/DirectXShaderCompiler

Source read locally: `windows` 0.62.2 and `windows-link` 0.2.1 (Mac cargo registry); PresentMon v2.6.0 files (`research17/pm-src`, from GitHub tag v2.6.0); the testbed, harness and `kairo-display` crates and the repo files in the citation keys.
