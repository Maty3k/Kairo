# Kairo planning brief

*A synthesis of research reports 01–07 and the fact-check pass, written so decisions can be made from it. Prepared 2026-10-21. Nothing is built yet.*

> **Kairo:** "Catch the perfect moment. Kairo clips your best plays from games and anything else on your screen." It is a Windows-only desktop app. v1 is an instant replay (an always-on rolling buffer where a hotkey saves the last N seconds) plus screenshots.

**How to read this**
- Each section opens with a recommendation, followed by the facts behind it. The full detail is in the reports: [01 video](01-video-capture.md), [02 audio, console, cloud](02-audio-console-cloud.md), [03 triggers, safety, detection](03-triggers-safety-detection.md), [04 competitors](04-competitors.md), [05 stack, workflow](05-stack-workflow.md), [06 after the clip](06-post-clip-sharing.md), [07 shipping, legal](07-shipping-legal.md).
- Tags:
  - **[Corrected]**: the fact-check refuted or changed a research claim. This brief uses the corrected version.
  - **[Uncertain]**: rests on a single source, a secondary source, absence of evidence, or something nobody has tested.
  - **[Estimate]**: arithmetic or judgement. Measure it.
  - Untagged statements were checked against a primary source or source code on or before 2026-10-21.
- None of this is legal advice.

**Already decided**
- Windows only.
- v1 is instant replay plus screenshots. Manual recording and auto-highlights come later.
- AMD AMF and Intel Quick Sync are first-class. NVIDIA NVENC must also work.
- Staged path: a private beta built to product quality, then a free public release, and monetization only if there is traction.
- Local-first. Cloud sharing comes later.

**Decisions this brief proposes (each needs your yes or no)**

| # | Decision | Recommendation | See |
|---|---|---|---|
| D1 | How to capture | Windows.Graphics.Capture (WGC) on Windows 11. DXGI Desktop Duplication (DDA) on Windows 10, and as the Win11 fallback. **Never inject into games.** | §2, §5 |
| D2 | Windows 10 | Support it through DDA by the public release. Certify Win10 audio only after soak tests. The beta runs on whatever OS the testers have. | §2, §10 |
| D3 | Default clip format | 1080p60, or native resolution if that is lower. H.264 High, VBR 25 Mbps target / 40 Mbps peak, 1 s closed GOP, AAC 48 kHz, MP4 with the index at the front. | §2 |
| D4 | Buffer | RAM ring of encoded packets. 30 s by default (≈95 MB) and at most 120 s in v1 (≈381 MB). A disk-backed long buffer comes later. | §2 |
| D5 | Audio | Separate tracks in one MP4, with the mixed track first, then game and your own mic. **Friends' voice chat is off by default.** | §2, §8 |
| D6 | Stack | Tauri 2 UI plus a standalone Rust engine, connected by a named pipe. If the Rust engine spike runs past about 7 working days, switch the engine to C#/.NET 10. | §3 |
| D7 | Hotkeys | Alt+F8 saves a clip and Alt+F6 takes a screenshot, via `RegisterHotKey`. Validate in the spike. | §5 |
| D8 | Console and cloud | v1: cloud games are captured like any other window, plus a generic file import. Capture cards in v1.x, and only if you own one. | §6 |
| D9 | Sharing | No backend until links are needed: export presets, copy to clipboard, drag-out. Later, links on Cloudflare R2 with a Laravel backend. | §7 |
| D10 | Distribution | Beta: a signed direct download (per-user NSIS installer plus updater). Public release: add a Microsoft Store MSIX. | §8 |
| D11 | Telemetry | Opt-in only, as EU law requires. Sentry in its EU region. No analytics during the beta. | §8 |
| D12 | Licensing | No GPL dependencies. FFmpeg ships as LGPL DLLs. Choose Kairo's own licence before accepting any outside code. | §8 |

---

## 1. Headline findings

1. **Capture through the compositor, never from inside the game.**
   - WGC (Windows 11) and DDA (Windows 10) both deliver GPU textures without touching the game process.
   - Every recent anti-cheat breakage involved injection:
     - A Vanguard update blacked out OBS Game Capture in VALORANT and LoL on 2026-07-21 ([win.gg](https://win.gg/obs-game-capture-not-working-valorant-league-of-legends-fix/)).
     - Riot and FACEIT did not accept the new certificate on OBS 31's game-capture hook.
     - AMD Anti-Lag+ led to CS2 VAC bans in Oct 2023 ([GGRecon](https://ggrecon.com/articles/counter-strike-2-bans-for-amd-anti-lag-users-set-to-be-reversed)).
   - Since 2026-10-06 Riot blocks game-memory access by unknown third-party apps. Approved tools have until April 2027 ([Riot](https://support.riotgames.com/en-us/riot/performance/game-memory-access-removed-for-third-party-apps)).
   - No ban report was found for out-of-process WGC or DDA capture. **[Uncertain]** That is absence of evidence, not proof.
2. **Windows 10 needs a second capture path.**
   - WGC draws a yellow border around whatever it captures. Removing it needs `GraphicsCaptureSession.IsBorderRequired`, called after `GraphicsCaptureAccess.RequestAccessAsync(Borderless)`. Both start at build 20348, which on consumer PCs means Windows 11 ([MS Learn](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired)).
   - Win10 22H2 is build 19045, so an always-on WGC buffer there would show a permanent yellow frame. DDA draws no border.
   - Windows 10 is still 23.61% of Steam users, against 71.34% on Windows 11 (Sep 2026, [Steam survey](https://store.steampowered.com/hwsurvey)). Consumer ESU runs through **2027-10-12** ([Microsoft](https://www.microsoft.com/en-us/windows/extended-security-updates)).
3. **[Corrected] The OS assumption in the project brief is out of date.**
   - Windows 11 **26H2** (build 26300) has been GA since 2026-09-29.
   - **24H2** Home/Pro reached end of updates on 2026-10-13, and **23H2** Home/Pro is also out of updates.
   - 25H2 (26200) is serviced until 2027-10-12 ([release health](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information)).
   - So plan for Windows 10 22H2 plus Windows 11 24H2, 25H2 and 26H2. (26H1 ships only on new devices.) Those Win11 builds all have the 24H2 WGC APIs (`MinUpdateInterval`, `DirtyRegionMode`, `IncludeSecondaryWindows`), but keep a runtime check.
4. **WGC does not deliver frames at a constant rate.**
   - Without `MinUpdateInterval` (build 26100+), these all treat WGC as capped at about 60 fps:
     - OBS ([PR #12759](https://github.com/obsproject/obs-studio/pull/12759), approved 2026-07-14, still unmerged);
     - Apollo ([PR #785](https://github.com/ClassicOldSong/Apollo/pull/785), merged 2025-06-07);
     - FFmpeg's `gfxcapture`, whose `max_framerate` defaults to 60 and whose docs say the source "does NOT hold a stable FPS".
   - Even with it, delivery is irregular. On a 60 Hz panel one user measured about 45 fps at 1/60 s and about 60 fps at 1/90 s ([windows-capture #190](https://github.com/NiiightmareXD/windows-capture/issues/190)).
   - Kairo must therefore pace frames on its own high-resolution clock (QPC, Windows' QueryPerformanceCounter timer) and write constant-frame-rate video.
5. **Games in true exclusive fullscreen are a blind spot for both APIs.** **[Uncertain]** The main evidence is one report.
   - [OBS #14021](https://github.com/obsproject/obs-studio/issues/14021) (build 26300, opened 2026-10-11, closed the next day) covered two exclusive-fullscreen OpenGL games, Minecraft Java and osu! stable:
     - DDA looped on ACCESS_LOST, re-creating about 15 times a second and getting 0–1 frames each time;
     - WGC froze after the first frame.
   - Maintainers called it a Microsoft API issue. Microsoft's [swap-chain docs](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ne-dxgi-dxgi_swap_chain_flag) back the concern: full-screen swap chains created with `IDXGIFactory::CreateSwapChain` behave as DISPLAY_ONLY, which hides them from DDA.
   - Most modern DX11/DX12 games use flip model or fullscreen optimizations and can be captured. Only injection, as in OBS Game Capture, handles the rest.
   - Kairo should detect a frozen capture and suggest borderless mode.
6. **Hardware encoders are fragmented, so H.264 is the only safe default.**
   - Every in-scope GPU family has hardware H.264 and HEVC encoding.
   - Hardware AV1 exists only on:
     - NVIDIA RTX 40/50;
     - AMD VCN 4.0/5.0: RX 7000/9000, Ryzen 7040/8000 "Phoenix" and Strix-class APUs;
     - Intel Arc and Core Ultra.
   - RX 6300/6400/6500 XT ("decode only") and GeForce MX450 have **no encoder at all**.
   - AMD's HEVC API exposes only IDR/I/P frames, so no B-frames. RDNA3 AV1 codes 1080p as 1082 lines.
   - Every GeForce that has NVENC allows 12 sessions ([NVIDIA matrix](https://developer.nvidia.com/video-encode-decode-support-matrix), [AMF wiki](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/GPU-and-APU-HW-Features-and-Support)).
   - Kairo therefore needs a capability probe at startup, and a decision about PCs with no encoder.
7. **FFmpeg covers the prototype, the production encoder and the muxer, but not audio capture.**
   - Two capture filters: `ddagrab` (DDA, since 6.0) and `gfxcapture` (WGC, since **8.1**, 2026-03-16).
   - Both output D3D11 GPU frames that go zero-copy into `h264/hevc/av1_amf` and `*_nvenc`, and into `*_qsv` via `hwmap`. `scale_d3d11` (8.0+) converts to NV12/P010 on the GPU.
   - Current stable is 9.0.2, released 2026-09-18 ([download](https://ffmpeg.org/download.html), [Changelog](https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/Changelog)).
   - **No FFmpeg version has a WASAPI input device**, so audio capture is native code.
   - Ship an LGPL build as DLLs.
8. **The replay buffer is a solved design: copy OBS.**
   - OBS keeps reference-counted *encoded* packets (video and every audio track) in a RAM queue, capped by seconds and MB. The UI defaults are 20 s / 512 MB.
   - It trims one GOP (group of pictures, the frames from one keyframe to the next) at a time, so the buffer always starts on a keyframe and never holds fewer than 2 keyframes.
   - On save, it snapshots the packets and muxes them on a background thread ([obs-ffmpeg-mux.c](https://github.com/obsproject/obs-studio/blob/master/plugins/obs-ffmpeg/obs-ffmpeg-mux.c)).
   - RAM ≈ bitrate × seconds / 8. At 25 Mbps: **30 s ≈ 95 MB, 2 min ≈ 380 MB, 5 min ≈ 950 MB**.
9. **A target of ≤3% FPS cost is realistic, but the AMD path must be measured early.**
   - LTT Labs, Aug 2026: Ryzen 7 7800X3D, RTX 5060 / RX 9060 XT, H.264 at 6 Mbps, in Cyberpunk 2077, F1 24 and RDR2 ([GIGAZINE write-up](https://gigazine.net/gsc_news/en/20260826-pc-screen-recorder/)).
   - Average FPS loss:

     | Tool | RTX 5060 | RX 9060 XT |
     |---|---|---|
     | Game Bar | −1 | −1 |
     | NVIDIA's recorder | −3 | n/a |
     | AMD's recorder | n/a | −5 |
     | OBS | −5 | **−16** |
     | Streamlabs | −12 | −22 |

   - **[Uncertain]** These numbers come from GIGAZINE's report. LTT's own page could not be fetched, and the resolution and averaging method are not stated.
10. **Per-app audio tracks work on every target OS, and no free tool offers them.**
    - Per-process loopback (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) is documented as requiring "Windows 10 Build 20348" ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-audioclient_activation_type)). OBS enables it from build 19041 anyway, with the comment "MS says 20348, but process filtering seems to work earlier" ([OBS source](https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/plugin-main.cpp)). So Win10 22H2 works in practice.
    - Competitors: AMD writes the mic to a separate file, Steam records one mixed track, and no separate-track option was found for Game Bar.
    - The risk: on long sessions the audio can turn noisy, crackly or out of sync. Reported onset ranges from about 2 minutes to about 2 hours.
    - OBS maintainers say Microsoft fixed this only on Windows 11 (24H2+). Users confirmed the fix on 25H2 26200.6899 ([OBS #8064](https://github.com/obsproject/obs-studio/issues/8064)).
    - Kairo needs a watchdog and multi-hour soak tests.
11. **AMD and Intel owners are under-served. That gap is Kairo's way in.**
    - Intel retired its recording and instant-replay "Studio" page and sends users to OBS, Steam and Game Bar (article last reviewed 2026-07-07, [Intel](https://www.intel.com/content/www/us/en/support/articles/000100252.html)).
    - Discord Clips' minimum-GPU list names only NVIDIA and AMD.
    - AMD Adrenalin's Instant Replay records at 30 or 60 fps only and has no per-app audio ([AMD DH3-023](https://www.amd.com/en/resources/support-articles/faqs/DH3-023.html)).
    - The NVIDIA app is strong on NVIDIA hardware (free, login optional, AV1, 240 fps on RTX 40/50), but it needs an RTX 20-series or newer.
12. **Nobody owns the "private by default" position.**
    - Medal leads with 17M+ monthly active users and about 3bn clips a year.
    - Its free tier has ads, a watermark on downloads that can't be removed, and links that expire after 14 days.
    - Its sister company General Intuition trains AI on Medal clips: a $133.7M seed in Oct 2025, $320M at $2.3bn in Jan 2026, and $220M at $6.2bn in Sep 2026. Medal offers an opt-out toggle.
    - Insights pays users for recordings that "may" train AI.
    - "No account to record, no ads, never used for AI training" is credible and unclaimed.
13. **Record and share H.264, because of playback.**
    - In WebView2 (which Tauri's UI uses) and in Media Foundation players, HEVC plays only with a valid HEVC Video Extension ([Edge docs](https://learn.microsoft.com/en-us/troubleshoot/microsoft-edge/development/video-playback-issues)). Retail costs $0.99. A free "from Device Manufacturer" version exists only on some OEM PCs.
    - AV1 plays without any extension.
    - Every share copy should be H.264 + AAC in an MP4 with the index at the front.
    - Discord's free upload limit has been 20 MB since 2026-08-14, so a "Copy for Discord" preset aims for ≤19,000,000 bytes.
14. **Stack: a Tauri 2 UI plus a separate, always-on Rust engine.**
    - On Windows, WebView2 *is* Chromium. In [one empty-app benchmark](https://github.com/Elanis/web-to-desktop-framework-comparison), Tauri used about 307 MB RAM and Electron about 279 MB.
    - So the footprint win comes from an engine process with no webview, and a UI that closes completely while you play.
    - Capture and hardware encoding can only be built and tested on the Windows PC:
      - Parallels on the M2 offers DirectX 11.1 and no AMD, Intel or NVIDIA GPU.
      - GitHub's Windows GPU runner ($0.102/min) can test NVENC only.
15. **Signing rules and EU law shape the shipping plan.**
    - EV certificates stopped bypassing SmartScreen in 2024. Reputation now builds per file and per certificate, over "several weeks and hundreds of clean installs" ([MS Learn](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)).
    - Microsoft Artifact Signing ($9.99/month) issues certificates to individuals only in the US and Canada. Organisations qualify in 12 regions. For an EU individual, the cheapest OV certificate is Certum, from €209.
    - Microsoft Store registration is free, and Microsoft re-signs Store MSIX packages, so they show no SmartScreen warning. But an Individual account can't later become a Company account.
    - In the EU, crash reports and analytics need opt-in consent (EDPB Guidelines 2/2023, para 33).
    - Recording friends' voice chat can be a crime in some EU states (e.g. §201 StGB in Germany), hence voice chat off by default.

---

## 2. Recommended v1 capture pipeline

### 2.1 Shape

In plain terms: the engine copies each new frame that Windows' compositor produces and converts it on the GPU. The GPU's built-in video encoder compresses it. The engine keeps the last ~30 s of compressed video and audio in memory. The hotkey writes that memory to an MP4 in the background, and nothing ever runs inside the game.

```
Win11: WGC (borderless; MinUpdateInterval on 26100+) ─┐
Win10 + fallback: DXGI Desktop Duplication ───────────┴─► "latest frame" texture (BGRA8 or FP16) + QPC time
                                                                 │                         │
                                   screenshot hotkey ───────────┘                         ▼
                                   (staging copy → WIC PNG; HDR: JXR + PNG)    QPC pacer @ 60 fps (CFR)
                                                                                           ▼
                                                  HLSL shader: scale + BT.709 limited → NV12 (tone-map if HDR)
                                                                                           ▼
                           libavcodec h264_amf | h264_qsv | h264_nvenc (D3D11 frames) → h264_mf → CPU (TBD)
                                                                                           ▼ H.264 packets
 WASAPI: game (process loopback) + own mic [+ voice chat, opt-in] → AAC-LC 48 kHz ──► ReplayRing (RAM; s + MB caps)
                                                                                           ▼ save hotkey
                         SaveJob(pre, post) → snapshot packet refs → mux thread (libavformat MP4 +faststart)
                         → temp file → fsync → rename into library → poster/sprite → SQLite → sound cue
```

### 2.2 Video capture

| Backend | When | Key API details |
|---|---|---|
| **WGC** | Windows 11 default | <ul><li>Frame pool: `Direct3D11CaptureFramePool.CreateFreeThreaded`, 2 buffers. `CopyResource` each frame out immediately.</li><li>Border: `RequestAccessAsync(Borderless)`, then `IsBorderRequired=false`. **[Uncertain]** Whether an unpackaged app sees a consent prompt on 25H2/26H2 is untested.</li><li>Pacing: `MinUpdateInterval` ≈ 1/(1.5 × target fps), e.g. 1/90 s for 60 fps, on 26100+.</li><li>Cursor: `IsCursorCaptureEnabled` (19041+).</li><li>Timestamps: `SystemRelativeTime` is QPC in 100 ns units.</li><li>Recreate the pool on `ContentSize` changes. Set `SetMultithreadProtected(TRUE)` on the shared device.</li></ul> |
| **DDA** | Windows 10 default; Win11 fallback | <ul><li>Create the device on the adapter that owns the monitor.</li><li>`DuplicateOutput1` for FP16/10-bit, on a per-monitor-aware-v2 thread.</li><li>Re-create on `DXGI_ERROR_ACCESS_LOST` (mode switches, UAC, fullscreen changes).</li><li>Draw the cursor yourself from `GetFramePointerShape`.</li><li>At most 4 processes can duplicate per session.</li><li>On hybrid laptops it fails with `DXGI_ERROR_UNSUPPORTED` on the dGPU, so run it on the iGPU.</li></ul> |

- **What to capture by default.** Research default: the monitor showing the game. Alternative on Win11: `CreateForWindow` on the game window. That is cleaner and more private because notifications, Discord pop-ups and other windows never land in clips. This is product question Q8. Window capture on Win10 would show the yellow border, so Win10 is monitor-only.
- **Freeze watchdog.** If no new frame timestamps arrive while the game is in the foreground, mark the clip "possibly frozen" and suggest borderless mode. Detect true exclusive fullscreen with `SHQueryUserNotificationState() == QUNS_RUNNING_D3D_FULL_SCREEN`.
- **Kairo's own windows** get `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` (Win10 2004+), so the toast never appears in clips.
- **Black-frame check.** Detect all-black frames, which come from DRM or capture-protected windows, and tell the user.
- **Not in v1: hook/injection capture.** It means injecting into D3D8–12, OpenGL and Vulkan games, with 32/64-bit helpers. Revisit only if the spike shows many of your games run true exclusive fullscreen, and probably never because of anti-cheat.

### 2.3 Frame pacing, colour, HDR

**Pacer**
- Tick at the target fps on QPC.
- On each tick, encode the newest frame captured at or before the tick. If nothing new arrived, encode the previous frame again; duplicate frames cost almost no bits.
- The tick time is the presentation timestamp, so video shares a clock with audio.

**Conversion**
- Convert in an **HLSL shader**, as OBS does, to control the colour matrix, range and tone-mapping exactly.
- `ID3D11VideoProcessor` (FFmpeg's `scale_d3d11`) also works, but its colour handling varies by driver **[Uncertain]**.

**SDR clips**
- BT.709 primaries, transfer and matrix; limited (TV) range; NV12.
- Signal this in the stream (VUI) and in the MP4 `colr` atom.

**HDR desktop**
- Capture FP16 scRGB (`R16G16B16A16_FLOAT`).
- Tone-map to SDR with the BT.2390 EETF on maxRGB, which OBS implements in `color.effect`.
- Read the user's SDR white level from `DISPLAYCONFIG_SDR_WHITE_LEVEL` (nits = value / 1000 × 80).
- HDR10 output (HEVC Main10 or AV1 10-bit from P010, BT.2020 PQ plus metadata) is an opt-in for later.

### 2.4 Encoders

**Startup probe**
- Per adapter, record: vendor, codecs, 10-bit support, B-frame capability, AV1 alignment.
- Keep the resulting capability report. It is how beta bugs get triaged.

**Fallback order**
1. libavcodec `h264_amf`, `h264_qsv` or `h264_nvenc`, fed D3D11 hardware frames.
2. `h264_mf` (the Media Foundation encoder).
3. CPU encoding, if you decide to support it (Q11). Options: Microsoft's built-in software H.264 encoder (no extra licence, heavy CPU at 1080p60) or OpenH264. Avoid x264, because it is GPL.

**Per-vendor settings**

| Vendor | Settings |
|---|---|
| **AMD (AMF)** | `usage=transcoding`, `quality=balanced\|quality`, `rc=vbr_peak`, VBAQ. QVBR and HQ-VBR need Pre-Analysis and work only at 8-bit. B-frames on H.264 only when `CAP_BFRAMES` is reported; **none on HEVC**. Runtime ships in the driver. SDK 1.5.3 (2026-09-29) is MIT. |
| **Intel (QSV)** | VBR, `preset=medium`, B-frames on; `low_power` is the default on Gen12+. The GPU path is `scale_d3d11=format=nv12,hwmap=derive_device=qsv`. **[Uncertain]** That is the least-proven line in the whole pipeline; validate it on day 1 if your GPU is Intel. `vpl-gpu-rt` covers Tiger Lake and newer; older iGPUs use the legacy Media SDK runtime. |
| **NVIDIA (NVENC)** | Preset P4–P5 with `tune=hq`, `rc=vbr`, spatial AQ, B-frames as reference (middle), `forced-idr`. FFmpeg 9 dropped pre-11.1 NVENC SDKs, and nv-codec-headers n13.1.15.0 needs **driver ≥ 610**. Choose an FFmpeg build whose headers match the oldest NVIDIA driver you will support. |

**Other notes**
- On hybrid laptops, create the capture device on the encoding adapter so the frame is copied only once.
- Media Foundation encoders behave differently per vendor and driver. An AV1 encoder may be missing from Media Foundation even when `av1_qsv` works. Always probe.
- D3D12 and Vulkan video encoding are not for v1: driver maturity is unproven.

### 2.5 Replay buffer and saving

**Ring**
- Payloads go into an arena allocator, with an index of keyframes.
- Caps on seconds *and* MB, with a floor of 2 GOPs. "Pinning" keeps packets an in-progress save still needs.
- Set the internal cap to clip length + 1 GOP. Otherwise a saved clip can be up to one GOP short, as in OBS.

**Save flow**
1. Hotkey at time T creates `SaveJob{start = keyframe ≤ T−pre, end = T+post}`, and the GOP holding `start` is pinned.
2. Once `end` is reached, copy the packet references under a lock. That takes microseconds.
3. A worker thread then:
   - muxes with libavformat to a temp MP4 with `+faststart`;
   - fsyncs and renames the file into the library;
   - writes a poster frame (from the latest texture or the first keyframe) and the metadata to SQLite;
   - plays the "saved" cue.
- Overlapping saves queue rather than drop. **Capture never stalls during a save.**

**Post-roll**
- OBS has none: its clips end at the hotkey. Kairo can offer 0–10 s of post-roll, with a "saving in N s" cue.

**Disk-backed buffer (v1.x)**
- An optional mode for long buffers: 2–4 s keyframe-aligned fMP4/TS segments on a drive the user picks.
- It writes 11–36 GB per hour. At 25 Mbps, 3 h/day of play is ≈12.5 TB a year of SSD wear.
- A RAM ring is lost on a crash. That is acceptable for "the last 30 s".

### 2.6 Audio

**Sources (v1)**

| Source | How | Default |
|---|---|---|
| **Game** | Process loopback, include-tree, on the game's *root* process. Start from the foreground window's process and walk up the parents while the image name stays the same. Users can pin an exe and add helper processes, e.g. a launcher's voice process. | On |
| **Your mic** | WASAPI shared mode, event-driven. Call `IAudioSessionControl2::SetDuckingPreference(TRUE)`; otherwise Windows can turn other audio down by 80% while a communications mic is open. Reconnect on `AUDCLNT_E_DEVICE_INVALIDATED`. Follow the default device via `IMMNotificationClient`. | On |
| **Voice chat** | Process loopback, include-tree, on the root `Discord.exe` plus a user-editable list. Requires a one-time consent notice (§8.5). | **Off** |
| **System audio** | For desktop / "anything on screen" clips, or when no game process is found. Either process loopback that excludes Kairo's own process tree, or endpoint loopback that re-opens when the default device changes and keeps a silent render stream alive. Pick one after the soak tests. It includes voice chat if voice chat is playing, so say so in the UI. | Desktop mode |

**Process-loopback quirks**
- `GetMixFormat` and `IsFormatSupported` return E_NOTIMPL, so pass a fixed format with `AUTOCONVERTPCM`.
- `GetDevicePeriod` and `GetCurrentPadding` don't work, and there is no `IAudioClock`.
- Each activation targets one process tree.
- Microsoft says it isn't tied to an endpoint. **[Uncertain]** Whether it survives a headphones-to-speakers switch is undocumented; test it.
- Don't copy the known bugs in Microsoft's own sample: a flag passed in the wrong parameter, and a non-zero buffer duration.
- In wasapi-rs, `include_tree=false` means **exclude**, whatever its doc comment says.

**Track layout**

| Track | Content | Flag |
|---|---|---|
| 1 | Mixed: what the clip should sound like | **default (enabled)** |
| 2 | Game | |
| 3 | Mic (mono is fine) | |
| 4 | Voice chat, only if enabled | |

- FFmpeg's MP4 muxer marks the first track default and groups the rest as alternates.
- Chromium (so Discord), Windows Media Player, File Explorer and Instagram play only the first or default track. Premiere 26.3 shifts every track after the first by one frame. DaVinci Resolve imports the stems correctly.
- Offer a **"Share copy" export that contains only the mixed track**.

**Encoding**
- AAC-LC at 48 kHz: stereo at 160 kbps (range 128–192), mono mic at 96–128 kbps.
- Encoder options:
  - FFmpeg's native `aac`, which is LGPL. **[Uncertain]** AAC patent licensing for shipping it was not researched.
  - The in-box Media Foundation AAC encoder (`aac_mf`). It accepts 16-bit PCM only; stereo bitrates are 96, 128, 160 or 192 kbps. It is missing on Windows N editions unless the Media Feature Pack is installed.
- No Opus: Premiere doesn't support it.

**Sync**
- Stamp every packet with the QPC time from `GetBuffer`.
- Resample every source onto one 48 kHz timeline, with drift compensation driven by those timestamps (libswresample `async`). Device clocks drift ±50–100 ppm, which is 180–360 ms per hour.
- Insert silence on `DATA_DISCONTINUITY`, and whenever loopback delivers nothing.
- On save, drop audio before the first video keyframe. The leftover offset is at most one AAC frame (21.3 ms), with no edit lists.

**Watchdog**
- Re-create a process-loopback client on `TIMESTAMP_ERROR`, when the target process exits, or when packets stop while the target is alive.
- Consider re-creating it every 20–30 min at quiet moments, to work around the Win10 noise bug.
- Show level meters per source, so a silent game track is obvious.
- Show a clear diagnostic when an exclusive-mode app holds the device.

**Kairo's own sound cue** never reaches clips: the game stem doesn't include it, and system audio excludes Kairo's process tree.

### 2.7 Screenshots

- **Source:** on the hotkey, take the latest captured texture at native resolution, before any downscale. `CopyResource` it to a staging texture and `Map` it.
- **Encoding:** a worker thread encodes with WIC (built into Windows). No extra capture session is needed.
- **Formats:**
  - **PNG** by default: ≈2–6 MB at 1080p, ≈10–25 MB at 4K **[Estimate]**.
  - JPEG at quality 90–95 as a "small" option.
  - **HDR displays:** a 16-bit `.jxr` plus a tone-mapped `.png`, as Game Bar does.
  - WebP and AVIF would need bundled libraries; JPEG XL is not for v1.
- **Performance:** keep encoding off the critical path. A 4K PNG can take 100+ ms of CPU **[Estimate]**.

### 2.8 v1 defaults

| Setting | Default | Options |
|---|---|---|
| Capture | Win11: WGC, borderless, on the game's monitor. Win10: DDA. | Game-window mode (Win11), a specific monitor |
| Resolution | min(native, 1080p), aspect preserved | 720p / 1080p / 1440p / native |
| Frame rate | 60, constant | 30 / 60 / 120. 120 needs Win11 26100+ or DDA; confirm in the spike. |
| Codec | H.264 High 8-bit | HEVC or AV1 when the probe allows (opt-in) |
| Bitrate (VBR target/peak) | 1080p60: 25 / 40 Mbps | 1440p60: 40 / 60. HEVC 1080p60: 16 / 25. AV1 1080p60: 12 / 20. 2160p60 HEVC: 50 / 75. 120 fps ≈ 1.5× the 60 fps figures. |
| GOP | 1 s, closed, every keyframe an IDR, parameter sets repeated | 2 s |
| B-frames | 2 where supported (none on AMD HEVC) | – |
| Look-ahead | Off | "Max quality" preset |
| Colour | BT.709 limited range, NV12. HDR desktop → tone-mapped SDR. | HDR10 (later) |
| Buffer | 30 s in RAM; v1 cap 120 s | Disk mode (v1.x) |
| Post-roll | 0 s | 0–10 s |
| Audio | AAC-LC 48 kHz. T1 mix 160 kbps, T2 game 160 kbps, T3 mic mono 128 kbps. Voice chat off. | Voice-chat track; system-audio mode |
| Container | MP4 with the index first (`+faststart`) and a `colr` atom | – |
| Screenshot | PNG at native resolution; HDR: JXR + PNG | JPEG |
| Library | `Videos\Kairo`. Warn if OneDrive has redirected the Videos folder. | Any local NTFS drive |

### 2.9 Expected cost at the defaults

| Resource | Expected | Basis |
|---|---|---|
| Game FPS | Target ≤3% average loss. Vendor tools lose 1–5 fps in LTT's test; baseline fps was not given. | Measured anchors (secondary source) |
| RAM, engine only (UI closed) | ≈95 MB buffer + engine. Engine estimates disagree: 15–40 MB (report 05, Rust) vs 50–150 MB (report 01, including FFmpeg/D3D state). Proposed budget: ≤60 MB + buffer. | Arithmetic + **[Estimate]**. Measure in S2. |
| RAM, UI open | +100–300 MB for WebView2 | Benchmark |
| VRAM | ≈100–200 MB at 1080p; ≈200–400 MB at 4K/FP16 | **[Estimate]** |
| CPU | ≈1–3% of an 8-core. Rises sharply if frames ever travel through system memory. | **[Estimate]** |
| GPU 3D time | <1 ms per frame for copy + convert at 1080p | **[Estimate]** |
| Encoder block | Separate fixed-function hardware (VCN, Quick Sync, NVENC); moderate load at 1080p60 | Measure |
| Save time | <1 s for a 30 s clip (spike exit criterion). Mux ≈0.1–0.5 s for 100–400 MB on NVMe. | **[Estimate]** |
| Disk | Nothing while buffering. ≈95 MB per 30 s clip. | Arithmetic |

**RAM by buffer length:**

| Profile | 30 s | 60 s | 2 min | 5 min |
|---|---|---|---|---|
| 1080p60 H.264 at 25 Mbps | 95 MB | 190 MB | 381 MB | 952 MB |
| 1440p60 H.264 at 40 Mbps | 151 MB | 303 MB | 606 MB | 1.51 GB |
| 2160p60 HEVC at 50 Mbps | 189 MB | 378 MB | 756 MB | 1.89 GB |

For comparison, Medal claims 100–400 MB of RAM for its buffer.

### 2.10 Deliberately not in v1

- Hook capture.
- HDR10 output.
- Disk-backed buffer.
- Manual recording.
- Auto-highlights.
- D3D12/Vulkan encoding.
- Opus audio.
- Webcam capture.

---

## 3. Stack options (ranked for you) and the dev workflow

### 3.1 Ranking

| Rank | Stack | Verdict |
|---|---|---|
| **#1** | **Tauri 2.12 UI (TypeScript + Vite) + a standalone Rust engine (`kairo-engine.exe`).** Engine components:<ul><li>Capture: `windows-capture` 2.0.1 (WGC + DDA).</li><li>Encode and mux: `ffmpeg-next` 9.0.0 over LGPL FFmpeg 9 DLLs (AMF/QSV/NVENC + MP4).</li><li>Audio: `wasapi` 0.26.0 (app loopback, system loopback, mic).</li><li>IPC: `interprocess` 2.4.6 (named pipe) with JSON-RPC 2.0; `ts-rs` 12 generates the TypeScript types.</li><li>Shell: `tray-icon`, `global-hotkey` (RegisterHotKey) and `auto-launch`.</li></ul> | **Recommended.** Lightest footprint while gaming: no webview runs during play. Rust is the only native language. You can compile-check from the Mac. A macOS port stays possible. |
| **#2** | **The same Tauri UI + a C#/.NET 10 LTS NativeAOT engine.** CsWinRT 2.3.1, CsWin32, Vortice 3.8.3 (D3D11/MF), NAudio 3.3.0 (`WasapiRecorder` with per-process loopback), FFmpeg.AutoGen 9.0.1.1 and StreamJsonRpc 2.26. | **Fallback.** Much easier for a PHP/Laravel developer to read and own, with mature, current libraries. Switch if the Rust engine spike runs past ~7 working days. The UI and the IPC contract stay unchanged. |
| **#3** | **A libobs engine** (OBS 32.2.2, GPL-2.0-or-later) in its own process behind the same UI | **Only if you choose GPL / open source.** The fastest route to battle-tested multi-vendor capture, encoding and replay; Streamlabs and Overwolf's recorders are built on it. |
| Avoid | <ul><li>NativePHP Desktop 2.3.1: Electron-only, plus a PHP runtime and local HTTP listeners.</li><li>Electron for anything always-on.</li><li>C++/Qt.</li><li>WPF or WinUI as the main UI: can't be developed on the Mac.</li></ul> | See report 05 §2. |

### 3.2 Trade-offs

| | #1 Tauri + Rust engine | #2 Tauri + C# engine | #3 libobs engine |
|---|---|---|---|
| Always-on RAM (UI closed, excluding buffer) | ~15–40 MB **[Estimate]** | ~25–60 MB **[Estimate]** | ~100–250 MB **[Estimate]** |
| With the UI open | +100–300 MB (WebView2) | +100–300 MB | Depends on the UI |
| Engine learning curve for PHP/JS | **Hard**: ownership rules, `unsafe` COM interop | Moderate: classes and DI feel like Laravel | Hard: C and CMake |
| Fit for AI-written code | Good: the compiler guards memory and threads. Watch the windows-rs churn. | Very good | Medium |
| Library maturity | Medium: official bindings are mid-migration, community crates active | Medium–high | Very high |
| Licensing | MIT/Apache crates + LGPL FFmpeg | MIT packages + LGPL FFmpeg | **GPL** |
| macOS later | Good: the UI and shell port as-is; only capture/encode is rewritten | UI only | Good |

### 3.3 Why #1, and when to switch

**For #1**
- Nothing Chromium-based runs during play: the engine owns the tray and hotkeys, and the UI can exit completely.
- One native toolchain covers the Tauri shell, the engine and a mock engine.
- `cargo xwin check --target x86_64-pc-windows-msvc` lets the agent catch Windows compile errors on the Mac.
- Tauri ships the per-user NSIS installer, updater and single-instance pieces.

**Costs you accept with #1**
- **Rust's learning curve.** Mitigation: keep `unsafe` COM code in one small module, and have the agent explain what it writes.
- **windows-rs churn.** `windows` is still 0.62.2 on crates.io (2025-10-06). The new 0.100 family (windows-core 0.100.0, 2026-09-03) changes signatures and module layout, with MSRV 1.95 (windows-sys 1.88). Pin 0.62 and budget a migration.
- **`ffmpeg-next`** describes itself as being in "maintenance mode".
- **Tauri 3** is in alpha (3.0.0-alpha.5; a separate CEF runtime crate is at alpha.6). Expect a migration during 2027; stay on 2.x.

**When to switch**
- If the engine skeleton (spike S2) runs past ~7 working days because Rust/COM friction keeps sending the agent in loops, switch the engine to C#. Nothing else changes.

### 3.4 Architecture

```
login ─► HKCU\…\Run ─► kairo-engine.exe   (per-user, in the interactive session, no webview)
                        ├─ tray + hotkeys (RegisterHotKey, MOD_NOREPEAT)
                        ├─ capture → GPU convert → HW encode → packet ring (+ WASAPI audio)
                        ├─ save → libavformat MP4 (in-process, or a helper in a kill-on-close Job Object)
                        └─ \\.\pipe\kairo-<user-SID>  JSON-RPC 2.0 (explicit DACL, PIPE_REJECT_REMOTE_CLIENTS)
user opens library ─► Kairo.exe (Tauri) ── connects to the pipe ── exits completely when closed
```

- **Not a Windows service.** Services run in Session 0, which can't capture the desktop or receive hotkeys.
- **Run unelevated.** If an elevated helper is ever needed, it is a separate process.
- **Crash isolation:**
  - a watchdog restarts the engine;
  - minidumps go to Sentry;
  - "safe mode" after N crashes at startup switches the encoder or capture API.
- **Contract-first IPC.** Define the RPC types in Rust and generate TypeScript with `ts-rs`. Notifications: `bufferStatus`, `clipSaved`, `error`.
- **Pipe security.** The default pipe DACL lets Everyone read, so set an explicit one.
- **Alternative B:** the Tauri app itself becomes the tray process and runs the engine as a sidecar. Choose it only if spike S3 shows all `msedgewebview2.exe` processes exit once the window closes.

### 3.5 Dev workflow across the Mac and the Windows PC

| Where | What happens there |
|---|---|
| **Mac (M2, 8 GB)** | <ul><li>UI work in Tauri/TypeScript, which runs natively on macOS.</li><li>A **mock engine** speaking the same JSON-RPC over a Unix socket (the same `interprocess` code), with fake buffer status and sample clips. `@tauri-apps/api/mocks` `mockIPC` covers Vitest.</li><li>`cargo xwin check` for Windows compile checks. Install: rustup, `cargo-xwin` 0.23.1, `brew install llvm`; it downloads the MSVC CRT and SDK under Microsoft's licence.</li><li>Specs, docs, and later the Laravel backend.</li><li>**No Windows VM:** 8 GB RAM is too little, and a VM has no vendor GPU and only DirectX 11.1.</li></ul> |
| **Windows gaming PC** | <ul><li>All capture, encode and audio work.</li><li>Install: Git for Windows; VS Build Tools (MSVC + Windows SDK); rustup `x86_64-pc-windows-msvc`; Node 22 + pnpm; the BtbN FFmpeg 9 **LGPL shared** build; PresentMon; GPU-Z.</li><li>Claude Code for Windows (`irm https://claude.ai/install.ps1 \| iex` or WinGet; needs Windows 10 1809+ and 4 GB+ RAM).</li></ul> |
| **Bridge** | <ul><li>OpenSSH Server on the PC; VS Code Remote-SSH from the Mac.</li><li>**Gotcha:** anything sshd starts runs non-interactively, so it can't capture or get hotkeys. Launch the engine into the logged-in session: create a task once with `schtasks /create … /it`, then trigger it with `schtasks /run`.</li><li>Or run Claude Code directly on the PC and watch through Parsec, Moonlight/Sunshine or Steam Remote Play.</li><li>**Avoid RDP for capture tests:** it swaps in a virtual display adapter.</li></ul> |
| **Automated check** | A test harness: <ol><li>Open a D3D11 test-pattern window that shows a frame counter and plays timed beeps.</li><li>Trigger a save over the pipe.</li><li>`ffprobe` checks duration, frame count, a keyframe at t=0 and the A/V offset.</li></ol> The agent can then verify capture without a human watching. |
| **CI** | <ul><li>GitHub Actions on `windows-latest` (Server 2025): build, unit tests, publish the updater JSON.</li><li>Windows 2-core costs $0.010/min beyond the included minutes (2,000 on Free, 3,000 on Pro). Standard runners have no GPU. The GPU runner costs $0.102/min and covers **NVENC only**.</li><li>AMF and QSV are tested only on your PC and testers' machines.</li><li>A self-hosted runner on the gaming PC is acceptable only for a private repo with trusted workflows.</li></ul> |

### 3.6 First two weeks: spikes on the gaming PC

| Spike | What to do | Done when |
|---|---|---|
| **S0** (day 1) | Toolchain, SSH, the `schtasks /it` launcher | The agent can build and launch into your desktop session |
| **S1** (days 1–3, no code) | <ul><li>FFmpeg 9.0.2 `gfxcapture`/`ddagrab` into `h264_amf`/`hevc_amf`, or the QSV path.</li><li>In 3+ games (one DX11/12 with fullscreen optimizations, one borderless, one Vulkan/OpenGL exclusive-fullscreen), record with PresentMon: average FPS, 1% lows, `PresentMode` (does capture knock the game off "Independent Flip"?), CPU and encoder load.</li><li>WGC rate with and without `MinUpdateInterval`, at 60 and 144 Hz.</li><li>Border and consent prompt; HDR on; a segment-ring prototype.</li></ul> | Your GPU's H.264/HEVC (and AV1) paths work, and overhead numbers are recorded |
| **S2** (≤7 working days) | Rust engine skeleton: WGC → NV12 shader → libavcodec hardware encoder → packet ring → hotkey → MP4. Plus WASAPI system and process loopback and mic → AAC. | <ul><li>A 30 s clip saves in <1 s.</li><li>A/V offset <40 ms.</li><li>Engine ≤ ~60 MB + buffer; <3% CPU at 1080p60 (you set the budgets).</li><li>**Over 7 days → switch to C#.**</li></ul> |
| **S3** | Total `msedgewebview2` RAM with the Tauri UI open and closed | Choose between the separate engine and alternative B |
| **S4** | <ul><li>Alt+F8 vs F8 in a normal game, an elevated window, and an SDL 3.4 app using `RIDEV_NOHOTKEYS`.</li><li>Clashes with vendor overlays.</li><li>GameInput vs XInput background reads.</li><li>Passive read of a Bluetooth DualSense with Steam running.</li></ul> | Trigger compatibility matrix filled in |
| **S5** | <ul><li>4–8 h audio soak test on Win11, and on Win10 if available: include-game, include-Discord, exclude-Kairo, endpoint loopback.</li><li>Headset ↔ speakers switching; an exclusive-mode app; ducking with and without `SetDuckingPreference`.</li></ul> | Mixed-track source chosen, watchdog thresholds set |
| **S6** | <ul><li>Multi-track playback in WMP, Movies & TV, Explorer, Chrome/Edge, Discord, Premiere, Resolve, CapCut and VLC.</li><li>In WebView2, `MediaSource.isTypeSupported` for HEVC.</li></ul> | Track layout and the H.264 default confirmed |

### 3.7 Build order after the spikes

| Milestone | Contents |
|---|---|
| **M1 Engine core** | <ul><li>Capture interface with WGC and DDA backends, pacer, encoder probe and capability report.</li><li>Ring and save, screenshots, audio stems and mix.</li><li>Tray, hotkeys, sound cue, in-app capture self-test.</li></ul> |
| **M2 UI and library** | <ul><li>Library grid, player, Quick/Precise trim, export presets, copy/drag-out, settings.</li><li>Onboarding with explicit consent for autostart, telemetry and voice chat.</li></ul> |
| **M3 Beta hardening** | Signed installer and updater, opt-in Sentry, privacy policy, soak tests, anti-cheat compatibility matrix, the Win10 path |
| **M4 Private beta** | You and friends |
| **M5 Free public release** | Store MSIX, name clearance, docs |
| **Then v1.x** | Controllers, Raw Input fallback, elevated mode, disk buffer, capture card, Switch import |
| **Later** | Share links (stage C) |

### 3.8 House rules for the coding agent (put these in CLAUDE.md)

1. **No code ever runs inside a game process.** No DLL injection, no graphics hooks, no in-context hooks (`WH_KEYBOARD`, `WH_GETMESSAGE`, `WINEVENT_INCONTEXT`).
2. Capture only through WGC or DDA.
3. Open game processes with `PROCESS_QUERY_LIMITED_INFORMATION` only. No `ReadProcessMemory`, no module enumeration, no handle duplication.
4. No `SendInput`, no virtual HID or controller devices, no kernel drivers.
5. Game events come only from official local APIs.
6. Overlays are separate top-level windows only.
7. Kairo runs unelevated. No packers or obfuscators. Releases are signed.
8. No GPL dependencies (x264, x265, libobs). FFmpeg is LGPL, linked dynamically.
9. Never write to PlayStation controllers.
10. Telemetry is opt-in. Never attach clips or screenshots to crash reports, and never store minidumps as attachments.

---

## 4. Competitors and where Kairo can win

### 4.1 Condensed matrix (late 2026)

| Tool | Replay buffer | Max fps | GPUs / codecs | Separate audio | Account, ads, watermark | Free sharing | Weakness Kairo can exploit |
|---|---|---|---|---|---|---|---|
| **NVIDIA app** | Yes; 30 s default (max not verified) | **240** on RTX 40/50; 4K120 / 8K30 HDR recording | NVENC; **RTX 20+ only**; H.264, HEVC, AV1 | Game + mic (user reports) | Login optional; no ads | None (local) | NVIDIA-only. 11.0.9.565 added a local MCP server for AI agents; press reports it as a beta that was on by default **[Uncertain]** |
| **AMD Adrenalin** | Up to 20 min in 15 s steps | **30 / 60** | AMD only; AVC, HEVC, AV1 | Mic saved as a **separate file**; no per-app audio | None | None | 60 fps ceiling; tied to the driver; stutter reports |
| **Intel Graphics Software** | **None** (Studio retired) | – | – | – | – | – | Nothing for Intel users |
| **Xbox Game Bar** | "Record that": 30 s default, up to 10 min **[Uncertain: third-party guides]** | 30 / 60 | Windows' own encoder | None found | None | None | One app window, no desktop, no trimming, no bitrate control |
| **Steam Game Recording** | Disk buffer; Instant Clip 30 s **[Uncertain: beta notes]** | n/v | NVIDIA/AMD hardware, CPU otherwise; HEVC with H.264 fallback | **One mixed track** | Steam account | Links expire after **2 days** | Only games launched through Steam |
| **Discord Clips** | 30 s / 60 s / 2 min, **only while streaming** | n/v | NVIDIA GTX 980+, AMD RX 500+; **no Intel** | Game and voice toggles | Nitro early access | Inside Discord | Requires streaming and Nitro |
| **Medal** | Yes, plus full sessions and auto-clips | Free uploads ≤60 fps | NVIDIA, AMD, Intel | Game / mic / Discord (claims per-app tracks) | Account to share; **ads; non-removable watermark** on free downloads | Links expire after **14 days**; ≤20 min at 1080p | Electron app, "100–400 MB RAM"; AI training via General Intuition; reliability and uninstall complaints. Premium FAQ (24 h / 4K / 144 fps) conflicts with the "unlimited" blog post |
| **Outplayed (Overwolf)** | Yes | n/v | n/v | n/v | **Ads**; needs the Overwolf client | Hosted 3 months; 400 MB cap | Overwolf dependency; no standalone version planned |
| **Insights Capture** | Records whole matches | n/v | "GPU-agnostic" (vendor) | n/v | Sign-in required | 5 GB storage | Aimed at teams and coaching; Shard AI-data programme |
| **SteelSeries Moments** | Yes; AI clips in ~27 titles | n/v | NVIDIA, AMD, Intel | Multi-track editing | n/v | Direct upload | Inside the GG suite; memory bug fixed 2026-10-20 |
| **OBS Studio** | RAM buffer, 20 s / 512 MB default | Configurable | NVENC, AMF, QSV | Up to 6 tracks; app audio from 19041 | None | None | Complex setup; no library or editor; GPL |
| **ShareX 22.0** | **No replay** | 120 | QSV through FFmpeg | System + mic | None | Many upload targets | Screenshots and recording only |
| **Allstar** | Clips rendered server-side from demos | 30/60/120 by tier | Cloud | n/a | Account; watermark | 5 clips/month at 720p30 | A handful of games |
| **Clippy** (May 2026) | 30 s in RAM | n/v | Mentions NVENC only | Game / desktop / mic | No account needed to record | clippy.rip | Tiny traction; NVIDIA-focused |

**[Corrected] AMD driver status.** Report 04 said AMD's notes "still list" an Instant Replay timeout. In fact, 26.9.2 lists it under *Fixed Issues*: a War Thunder timeout in DX12 with Instant Replay and Borderless Region Capture on RX 6000. 26.10.1 (2026-10-20) has no capture-related fixes or known issues ([26.9.2](https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-26-9-2.html), [26.10.1](https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-26-10-1.html)).

### 4.2 Where Kairo can win (ranked)

1. **A vendor-neutral replay that is first-class on AMD and Intel.**
   - Evidence: Intel has nothing; AMD caps at 60 fps with no per-app audio; Discord excludes Intel.
   - The bar to clear:
     - "never misses the clip" (Medal's top complaint);
     - saves in about 1–2 s;
     - 60–144 fps;
     - HEVC/AV1 where the hardware allows.
2. **Real multi-track audio in one file** (game, voice, mic, other apps), with a mixed default track and stems you can mute at export.
3. **Private by default.**
   - No account to record, no ads, opt-in telemetry.
   - A public "never used for AI training" promise.
   - A clean uninstall: some Medal users say it keeps reinstalling or relaunching itself.
4. **A light native footprint.** No Electron, no Overwolf, no injection, and a UI that exits while you play.
5. **Sharing without accounts and with fair retention.** Discord-sized presets now. Links later that keep clips longer than Steam's 2 days or Medal's free 14 days.
6. A fast editor with vertical 9:16 export.
7. The same buffer for any window, cloud gaming and capture cards.
8. HDR-correct clips and screenshots. Vendors keep shipping HDR colour fixes, which shows the pain is real.
9. Later:
   - smart highlights from official game APIs only;
   - optionally open source, which builds trust, as OBS and ShareX show.

### 4.3 Pricing anchors (for later)

- The free baseline is high: NVIDIA, AMD, Game Bar, Steam, OBS and ShareX cost nothing. A paid tier must therefore sell cloud, sharing or editing extras, never the replay itself.
- Medal Premium: $9.99/month, or $7.99/month billed yearly.
- Allstar: $0.99, $4.99 and $11.99/month.
- Insights team plans: $39–229/month.
- Eklipse: $19.99/month.

### 4.4 The name "Kairo"

**Collisions**
- **KAIRO** at kairoclips.com: a sports highlight-reel SaaS, domain registered 2026-05-24, with Free/Pro/Ultra Pro plans priced in GBP. It uses the same "clips" vocabulary.
- Kairosoft's "Kairo" game brand (Kairo Land, Kairo Club).
- A 2013 Steam game called *Kairo* (app 233230).

**Domains**

| Domain | Status |
|---|---|
| kairo.gg | Taken, registered 2025-10-08 |
| kairo.app | Taken since 2018, parked |
| kairo.io | Registered at GoDaddy since 2023-02-03. **[Uncertain]** It may be for sale; the Afternic listing wasn't checked. |
| getkairo.app | In redemption, may drop |
| kairoclip.com | **Unregistered** |

**Action:** the name is fine for a private beta. Run a trademark search (USPTO, EUIPO, UK IPO, WIPO; Classes 9, 41 and 42) before spending on branding, and plan for a domain with a modifier.

---

## 5. Triggers, anti-cheat safety, in-game feedback, game detection

### 5.1 Keyboard hotkeys (v1)

**Mechanism.** `RegisterHotKey` with `MOD_NOREPEAT`. Tauri's `global-hotkey` and Electron's `globalShortcut` both use it underneath. If registration fails, the combination is already taken, and the UI should say so.

**Proposed defaults** (validate in S4): **Alt+F8** to clip, **Alt+F6** for a screenshot.

**Keys to avoid**

| Owner | Keys |
|---|---|
| NVIDIA | Alt+F10 / Alt+Shift+F10 (sources conflict on which saves), Alt+Z, Alt+F9, Alt+F1 |
| AMD | Ctrl+Shift+S and friends |
| Game Bar | Win+Alt+G |
| Medal | F8 |
| Steam | F12, Ctrl+F11 |
| Windows | F12 (reserved for debuggers), Win+ combinations, PrintScreen (opens the Snipping Tool) |
| Keyboard layouts | Ctrl+Alt+letter, because AltGr equals Ctrl+Alt on many European layouts and would fire while typing in game chat |

**Elevated (admin) games**
- Without UIAccess, a normal process can't read input aimed at an elevated window through low-level hooks, Raw Input or `GetAsyncKeyState` ([Microsoft](https://learn.microsoft.com/en-us/windows/security/threat-protection/security-policy-settings/user-account-control-only-elevate-uiaccess-applications-that-are-installed-in-secure-locations)).
- LibreAutomate says "most single-key and Shift+key hotkeys don't work" when the active window is elevated ([LibreAutomate](https://libreautomate.com/api/Au.More.RegisteredHotkey.Register.html)).
- **[Corrected]** Report 03 said Ctrl/Alt combinations "reportedly work". The source doesn't say that. It's an inference, consistent with AMD requiring Ctrl and/or Alt, and must be tested in S4.
- Medal tells users to run it as administrator to fix hotkey problems.

**Games that disable other apps' hotkeys**
- Games that register Raw Input with `RIDEV_NOHOTKEYS` (an opt-in hint since SDL 3.4.0; The Witness used it) disable `RegisterHotKey` while they have focus.
- v1.1 fallback: a Raw Input keyboard sink (`RIDEV_INPUTSINK`), with duplicates within ~100 ms dropped.

**v1 handling for elevated games**
- Detect them and explain.
- v1.1: an optional "start Kairo elevated" setting, via a scheduled task.
- UIAccess only later: it needs a trusted signature and an install under Program Files.

### 5.2 Mouse, controller and voice triggers

**Mouse side buttons** (v1, opt-in)
- Poll `GetAsyncKeyState(VK_XBUTTON1/2)` every ~8–10 ms, only while a mouse button is bound.
- Don't use `WH_MOUSE_LL`: it puts Kairo in the path of every mouse movement in the game.
- The click still reaches the game, so users should pick an unbound button.

**Controller** (v1.1)
- Use **GameInput 4.0.302** (NuGet, 2026-10-19; Windows 10 19H1+):
  - `SetFocusPolicy(EnableBackgroundInput | EnableBackgroundShareButton | EnableBackgroundGuideButton)`;
  - `RegisterSystemButtonCallback` for the Share button;
  - hold-to-trigger chords of 500–800 ms.
- GameInput needs its runtime (MSI or winget). **[Uncertain]** Details of side-by-side deployment are unverified.
- A focused game can claim the Share button exclusively. Game Bar may also capture on Share, giving double captures.
- Windows.Gaming.Input gives no background input.
- XInput is documented as focus-gated, but practitioners report otherwise. **[Uncertain]** Test it.
- **PlayStation controllers: read only.** Any output report, or even reading feature reports, switches a Bluetooth DualSense to "enhanced" mode, which breaks DirectInput for other apps until it is power-cycled.

**Voice: "clip that"** (v2)
- Use **sherpa-onnx** keyword spotting (Apache-2.0; any phrase without retraining) or **Vosk** (Apache-2.0; 40 MB English model).
- Not Picovoice: it has no free or personal plans, and free keys reportedly stopped working on 2026-06-30 **[Uncertain: quoted emails]**.
- Not openWakeWord's pre-trained models: they are CC BY-NC-SA, so non-commercial only.
- Processing stays local and opt-in, listening only while a game is detected, with voice-activity detection and a cooldown.

### 5.3 Anti-cheat safety

- The hard rules are in §3.8. In short: Kairo never touches the game process, and uses only out-of-process capture.

**Compatibility matrix.** Keep one, and re-test after anti-cheat updates:

| Games | Anti-cheat |
|---|---|
| VALORANT, LoL | Vanguard |
| Fortnite, Apex | EAC |
| R6, PUBG | BattlEye |
| CS2 | FACEIT |
| CoD | Ricochet |
| BF6 | Javelin (TPM enforcement coming; blocks reWASD and virtual buses) |

**Gaps:** no official EAC, FACEIT or Ricochet statement on recorders could be retrieved.

**Future auto-highlights** may only use official local APIs, e.g. League's Live Client Data API at `https://127.0.0.1:2999`, or CS2 Game State Integration. Riot's memory block makes this mandatory.

### 5.4 "Clip saved" feedback

- **Primary feedback: a short sound cue** (<300 ms, adjustable volume). It never reaches clips (§2.6).
- **Optional small toast for ≤1.5–2 s:**
  - a separate top-level window with `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`;
  - shown with `SW_SHOWNOACTIVATE`, and never `SetForegroundWindow`;
  - set to `WDA_EXCLUDEFROMCAPTURE`;
  - skipped when the game is in true exclusive fullscreen (`QUNS_RUNNING_D3D_FULL_SCREEN`).
- **No Windows toast notifications during play.** Windows 11 can turn on Do Not Disturb "when playing a game" or "in full-screen". Use toasts only for after-session summaries.
- **No rumble in v1.**

### 5.5 Game detection and metadata

**Runtime signal** (no injection)
- `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, …, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)`.
- Then `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW` → match against the database.
- A Toolhelp process poll every 2–5 s catches game start and exit.

**Kairo's own game database**, built from:
- Steam: `libraryfolders.vdf` + `appmanifest_*.acf`;
- Epic: the `*.item` manifests;
- Ubisoft and GOG: registry keys;
- Battle.net: Windows Uninstall entries;
- Xbox / Game Pass: packages that contain `MicrosoftGame.config`;
- a curated exe list;
- a **"Mark as game"** button.

**Heuristics:** fullscreen or borderless, plus `QUNS_*` state, plus GPU load. **Don't** enumerate a game's loaded modules: that needs `PROCESS_VM_READ`, which anti-cheats watch.

**Don't depend on:**
- Discord's `/api/v9/applications/detectable` (24,780 apps, no auth). It is undocumented, and Discord's Terms forbid scraping without written consent.
- Windows' `GameList` API, which requires a restricted capability.

**Metadata**
- v1: the exe icon plus launcher names.
- Later: IGDB through a small proxy or a bundled snapshot. It is free for non-commercial use at 4 requests/s; commercial use needs a free partnership with user-facing attribution; its client secret can't ship in the app.
- SteamGridDB only with the user's own key.

---

## 6. Console and cloud gaming

### 6.1 Cloud gaming (v1)

**Approach**
- Capture the client window or monitor through the normal pipeline, plus audio from the client process.
- Ship presets for:
  - GeForce NOW (app or browser);
  - Xbox Cloud Gaming (browser or Xbox app);
  - PS Remote Play.

**What can blank a capture**
- `SetWindowDisplayAffinity`.
- DXGI DISPLAY_ONLY swap chains, including legacy full-screen ones.
- Hardware-protected DRM video.

No DRM is documented on any cloud-gaming stream. xCloud/xHome run over WebRTC, according to an open-source client ([xbox-xcloud-player](https://github.com/unknownskl/xbox-xcloud-player)).

**Per service**

| Service | Notes |
|---|---|
| GeForce NOW | The terms (v. 2025-08-18) don't mention recording. They do bar copying or distributing "any portion of GFN" and commercial use. |
| PS Remote Play | Sony blocks content at the source: video services are unavailable, and "some images may not be displayed" depending on the game. |
| Browser clients | One process tree for the whole browser, so every tab's audio is mixed together. |
| Boosteroid, Luna | Unknown; test. |

**Product behaviour**
- Check display affinity where possible. `GetWindowDisplayAffinity` only works on layered windows.
- Detect black frames and tell the user.
- Re-test whenever a service updates.

### 6.2 Capture cards (v1.x, only if you own one)

**Devices and audio**
- Elgato HD60 X, 4K X, 4K S and Neo are driver-free UVC devices.
- Audio arrives as a separate device: stereo PCM up to 24-bit/48 kHz, with no Dolby, DTS or 5.1. Set the Switch to stereo, or centre-channel dialogue is lost.

**Constraints**
- Elgato says these models have **no multi-app support**, so Kairo would compete with OBS or Elgato Studio for the device. `MediaCaptureSharingMode.SharedReadOnly` might coexist. **[Uncertain]** Untested; Elgato's own Multi App list names only older devices and is Windows 10 only.
- **PS5 has HDCP on by default.** Turn it off in Settings → System → HDMI. Xbox and Switch enable HDCP only inside media apps.
- Windows camera privacy settings can block the card.

**Implementation**
- Pair the video and audio devices by container ID.
- Give each card its own A/V offset setting.
- Open the card in NV12/YUY2, or P010 for HDR, rather than MJPEG.
- 4K60 NV12 needs USB 3.2 Gen 2.

**Prices**

| Card | Price |
|---|---|
| Elgato 4K S | $159.99 (Jul 2025) |
| Elgato HD60 X | $179.99 |
| Elgato 4K X | $229.99 MSRP at the Feb 2024 launch; **now $249.99** US retail |
| AVerMedia GC553G2 | $209.99 on sale, $299.99 regular |

**Competition:** Elgato's own 4K Capture Utility already has "Flashback Recording". Kairo's value here is one library for PC and console clips under one hotkey.

### 6.3 Importing console-native clips

None of the consoles offers a public import API.

| Console | Official paths | Retention and limits |
|---|---|---|
| **PS5** | USB copy; auto-upload to the mobile PS App only | Clips under 3 min, ≤1920×1080, kept 14 days |
| **Switch / Switch 2** | MTP over USB; Nintendo Switch App | App keeps uploads 30 days, up to 100 files |
| **Xbox** | USB; official OneDrive auto-upload (Sept 2023) | Xbox network deletes captures after 90 days. **[Uncertain]** A 10 GB cap appears only in third-party guides. |

The unofficial OpenXBL API costs $0 for 150 requests/hour, or $5, $15 and $35/month for 500, 2,500 and 5,000.

### 6.4 Staging

| Stage | Console and cloud work |
|---|---|
| **v1** | <ul><li>Cloud presets, black-frame and display-affinity warning.</li><li>Generic import: drag-drop or "Import folder", recursive, MP4/WebM/MOV/JPG/PNG, deduplicated by hash.</li></ul> |
| **v1.x** | <ul><li>UVC capture-card source at 1080p60 NV12 first, with paired audio, per-card offset, HDCP and privacy onboarding.</li><li>Switch import over MTP (Windows Portable Devices).</li></ul> |
| **v2** | <ul><li>Xbox via the OneDrive folder or Microsoft Graph `driveItem: delta` (`Files.Read`).</li><li>4K/HDR capture-card replay.</li></ul> |
| **Never** | <ul><li>Reverse-engineered PSN downloads (PSN ToS §10.7).</li><li>Anything that bypasses HDCP or DRM.</li><li>OpenXBL only if users demand it.</li></ul> |

Note: under Nintendo's 2024 guidelines, raw, uncommented footage counts only if it was captured with Nintendo's own system features. Capture-card footage needs creative input.

---

## 7. After the clip: library, editing, sharing roadmap with costs

### 7.1 Library

**Layout**
- Clips are ordinary files, e.g. `Videos\Kairo\<Game>\2026-10-21_21-14-03_<Game>.mp4`, with screenshots beside them and an `_Exports\` folder.
- The index is SQLite in WAL mode at `%LOCALAPPDATA%\Kairo\library.db`, **never on a network or cloud folder**.
- Posters and sprites live in a cache that can be rebuilt.
- Metadata is mirrored into MP4 tags or a sidecar file, so a rescan restores the index.

**Watch out for**
- Windows "Known Folder Move" can put `Videos` inside OneDrive. Clips then eat quota and become online-only placeholders. Detect this on first run and suggest a local path.
- Watch the folder (`ReadDirectoryChangesW` / the `notify` crate) so renames and deletes made in Explorer reach the index.

**Schema essentials**
- Clips: ULID id; kind; relative path; size, duration, resolution, fps, codecs and audio-track layout; game; session; capture time; non-destructive trim in/out; favourite and locked flags; poster and sprite; content hash.
- Also: tables for exports and shares, and FTS5 for search.

**Thumbnails and hover-scrub**
- With a 1 s GOP, a keyframe-only sprite gives one thumbnail per second for about 0.5 s of work per 30 s clip (measured on the M2; Windows to be confirmed). A poster frame takes 0.17 s.
- Run this work at idle priority, and pause it while a game is in the foreground.

**Cleanup**
- Three limits:
  - maximum library size (e.g. 100 GB);
  - minimum free space (e.g. 15 GB or 5%);
  - maximum age for unstarred clips (off by default).
- Delete oldest first, skipping favourite, locked, edited, exported and shared clips.
- Show a dry run first.
- Automatic cleanup deletes permanently; manual deletes go to the Recycle Bin.

**Serving video to the UI**
- Tauri's asset protocol may read whole files without a Range header ([tauri#4133](https://github.com/tauri-apps/tauri/issues/4133)); 2.12 improved async/range handling. Test seeking on 500 MB+ files.
- Microsoft says media through WebView2's virtual host mapping "can be slow to load". The fallback is a loopback HTTP server with Range support.

### 7.2 Editing (v1 = a single-clip trimmer)

**Two trim modes**
- **Quick (lossless):** snaps to a keyframe. With a 1 s GOP the error is ≤1 s. Edit lists are fine *inside* Kairo.
- **Precise:** re-encodes on the GPU. This is the **default for exports**, because Instagram Reels requires "no edit lists, moov atom at the front" and a closed GOP.
- Skip "smart cut": LosslessCut still calls it experimental and says it fails on many files.

**Other editing**
- **Merge:** clips with the same settings join losslessly and almost instantly with the concat demuxer; anything else is re-encoded.
- **Vertical 9:16 presets** (the filter graphs were tested): facecam on top with gameplay below, centre crop, or the full frame over a blurred background. The facecam rectangle is drawn by the user and remembered per game.
- **Captions** come later: FFmpeg 8's Whisper filter needs a build with whisper.cpp, or run whisper.cpp alongside.
- **Multi-track preview:** in Chromium, `audioTracks` sits behind a flag. Preview stems as `.m4a` sidecars mixed with Web Audio, or mix down.

**Components:** Video.js 10.1, Vidstack or Media Chrome for the player; wavesurfer.js for waveforms; Mediabunny (MPL-2.0) for in-browser processing. A multi-clip timeline comes later.

### 7.3 Playback constraint

| Player path | HEVC | AV1 |
|---|---|---|
| WebView2 / Edge (Tauri) | Only with the HEVC Video Extension: $0.99 retail, or the free OEM listing on some PCs | Yes |
| Electron / Chrome | Yes, but only on GPUs with hardware HEVC decoding | Yes |
| Media Foundation (Movies & TV, WinUI) | Needs the extension | Needs the free AV1 extension |
| libVLC / FFmpeg-based players | Yes | Yes |

Hence H.264 by default. HEVC is opt-in, with an in-app `isTypeSupported` check and a link to the Store.

### 7.4 Sharing with no backend (private beta)

**Copy to clipboard**
- Write `CF_HDROP` plus `Preferred DropEffect = DROPEFFECT_COPY`.
- Chromium reads CF_HDROP first, so Discord should accept the paste as a file. Test Discord and WhatsApp desktop.
- In Tauri this is about 40 lines of Rust with the `windows` crate; the community clipboard plugin hasn't been updated since 2024-10.
- "Copy for Discord" first creates a ≤19 MB export.

**Other routes**
- Drag-out with `tauri-plugin-drag`. Keep the UI unelevated, because Windows blocks drag-and-drop from lower- to higher-privilege apps.
- "Show in folder" with `revealItemInDir`.
- Optional "post to my Discord server" through a webhook URL the user pastes. Store the URL with DPAPI. The limit is 20 MiB.

**Discord upload limits** ([API changelog](https://github.com/discord/discord-api-docs/blob/main/developers/change-log.mdx))

| Who | Limit |
|---|---|
| Free | **20 MB.** User-facing since 2026-08-14; the API changelog dates the 20 MiB limit to 2026-09-03. |
| Nitro Basic | 50 MB |
| Nitro | "Up to 1 GB" per the Help Center (edited 2026-09-30). Press said 500 MB, and Discord says it is "experimenting", so **use 500 MB as the safe preset**. |
| Server Boost Level 2 / 3 | 50 / 100 MB. A 250 MB "Larger File Uploads" experiment costs 5 boosts. |

**Fitting a size target**
- video kbps = (0.95 × limit bytes × 8 / seconds) / 1000 − audio kbps.
- Examples for 20 MB with 160 kbps audio: 15 s → 9,973 kbps; 30 s → 4,907 kbps; 60 s → 2,373 kbps.
- Measure the output and retry at 0.9× if it comes out over. **[Uncertain]** How accurately the hardware encoders hit a size target is untested.

**Resolution ladder for H.264**

| Video bitrate | Resolution |
|---|---|
| ≥8 Mbps | 1080p60 |
| 4–8 Mbps | 1080p30 or 720p60 |
| 2–4 Mbps | 720p |
| <2 Mbps | 540p or 480p |

### 7.5 Sharing roadmap with costs

| Stage | What ships | Infra / month | Legal and ops load |
|---|---|---|---|
| **A. Private beta** (local only) | <ul><li>Library, Quick/Precise trim, merge, 9:16 presets.</li><li>Export presets: Discord 20 / 50 / 500 MB, Shorts/TikTok/Reels.</li><li>Copy file, drag-out, show in folder, optional webhook.</li></ul> | **$0** | None |
| **B. Free public release** | "Upload to my YouTube": OAuth into the user's own channel, resumable upload | **$0.** The default quota is 100 uploads/day per Cloud project (all users combined); raising it needs a compliance audit. | Google OAuth consent and verification; privacy policy |
| **C. Share links** | <ul><li>Laravel 13 API with Socialite (Discord, Google, Steam OpenID 2.0).</li><li>Clips on **Cloudflare R2**, served from a custom domain.</li><li>A server-rendered share page with `og:video` tags.</li><li>Unlisted links, 30-day retention.</li></ul> | Storage + delivery **≈$22 (R2)** to ≈$49 (S3 + CloudFront flat-rate Pro) at 1,000 users × 50 clips. App hosting ≈$15–60 (Forge $12 + VPS, or Laravel Cloud $5–20 + usage). **Total ≈$40–110.** | Hosting duties: <ul><li>EU DSA Arts. 11–18: contact points, notice-and-action, statements of reasons; an EU legal representative if Kairo isn't established in the EU.</li><li>A DMCA agent: $6 per 3 years.</li><li>A runbook for reporting CSAM to NCMEC.</li><li>ToS and privacy policy.</li></ul> |
| **D. Social** (profiles, feeds, comments) | Public profiles, feeds, comments | Grows with views; R2 egress stays $0 | <ul><li>Online-platform duties; most of DSA Arts. 20–28 are waived for micro/small enterprises (except Art. 24(3), and for 12 months after outgrowing that status).</li><li>A moderation queue and CSAM hash matching.</li><li>Age gating: 13+, and 16+ in the EU for public features.</li><li>UK Online Safety Act risk assessments.</li></ul> |
| **E. Direct posting** | <ul><li>TikTok upload to the user's inbox (Direct Post only after an audit).</li><li>Instagram Reels (App Review + Business Verification; 100 posts/24 h).</li><li>X.</li></ul> | X: $0.015 per post, $0.20 if it contains a URL | Platform audits and UX rules |

**Discord link previews** (official docs, 2026-10-06)
- Discord makes one GET and runs no JavaScript, so the tags must be in server-rendered HTML.
- The whole fetch must finish within 10 s, including measuring images and video.
- `og:video` must point directly at an HTTPS .mp4/.webm/.mov file.
- `og:type` must be `video.*`, or `twitter:card` must be `player`.
- `og:image` is required.
- Previews are cached for about 30 min.
- So share files must have the index at the front (faststart) and come from a CDN.

### 7.6 Hosting-cost comparison

Worked example: 1,000 users × 50 clips × 30 MB a month, kept 30 days, 20 views per clip. That is 1.5 TB stored, 30 TB of egress, 25k minutes stored and 500k minutes delivered.

| Option | Monthly cost | Notes |
|---|---|---|
| **Cloudflare R2** + custom domain | **≈$22** | $0.015/GB-month, egress free. Cloudflare allows R2-hosted video on its CDN. `r2.dev` URLs are for development only. |
| S3 + CloudFront flat-rate Pro | ≈$49 | $15 plan with a 50 TB *soft* allowance; sustained excess can degrade delivery |
| B2 + Bunny CDN | ≈$160 on Bunny's Volume network; **≈$310 on the Standard network** | **[Corrected/qualified]** Report 06 gave only the Volume figure |
| Bunny Stream | ≈$165–324 | Not re-derived by the fact-check |
| Mux (basic) | ≈$380–475 | Not re-derived by the fact-check |
| Cloudflare Stream | ≈$625 | $5 per 1k minutes stored + $1 per 1k minutes delivered |
| S3 + CloudFront pay-as-you-go | ≈$2,400 | |

At 5× virality (150 TB of egress), R2 is still ≈$22. Buy Stream, Mux or Bunny only if you need adaptive streaming, analytics or server-side transcoding. Kairo can encode the share copy on the user's own GPU: H.264, ≤1080p60, ~6–8 Mbps, faststart.

---

## 8. Shipping: signing, installer, updates, crash reporting, legal and licensing

### 8.1 Code signing

**Why it matters for Kairo**
- SmartScreen warns on new browser downloads.
- **Smart App Control** on Windows 11 blocks unsigned executables without reputation, and it checks *every* executable. Since the April 2026 update it can be re-enabled without reinstalling Windows. **[Uncertain]** That last point is from press reports; it was first previewed in KB5074105.
- Antivirus heuristics distrust an app that combines a tray icon, global hotkeys, screen capture and a microphone.

**SmartScreen facts**
- EV certificates no longer bypass SmartScreen (removed in 2024).
- Reputation comes from the file hash plus the publisher certificate, and can take "several weeks and hundreds of clean installs".
- One indie publisher reports prompts on every new release after 10 years on the same certificate.
- Tell beta testers to expect a "Run anyway".

**Options** (prices read 2026-10-21)

| Option | Cost | Who can use it | Notes |
|---|---|---|---|
| **Certum Standard (OV) Code Signing in the Cloud** | **From €209** | **Individuals OK** | The cheapest credible OV for an EU individual. Signing through SimplySign is interactive (mobile one-time code). Cheaper card-based variants: a €169 card set, or a €139 electronic code if you already own a Certum card. |
| Certum Open Source | €25 (code) / €49 (cloud, out of stock) / €69 (card set) | Individuals with an OSS project | The certificate reads "Open Source Developer", and it is revoked if used for commercially distributed software |
| SSL.com OV/IV | $129/yr + eSigner $20/month for 20 signings (≈$309/yr), or a $379 YubiKey | Individuals OK | 20 signings a month is tight when every PE file counts |
| Sectigo (direct) | From $536.25/yr on a 5-year plan | Unclear | |
| **Microsoft Artifact Signing** | Basic **$9.99/month** (5,000 signatures), Premium $99.99/month, $0.005 per extra signature | **Organisations** in 12 regions (US, Canada, EU, UK, AU, NZ, JP, KR, SG, CH, NO, IL); **individuals only in the US or Canada** | <ul><li>Needs a paid Azure subscription; no EV; 72-hour certificates renewed daily.</li><li>An older Microsoft page (2026-08-29) lists only US, Canada, EU and UK for organisations; the quickstart (2026-10-08) lists 12.</li><li>**[Uncertain]** Whether it qualifies for the Store EXE route: its CAs are being removed from the CCADB but stay on the Windows Platform Trust List.</li></ul> |
| SignPath Foundation | Free | Fully OSI-licensed projects | The Foundation appears as publisher; it needs prior reputation and verifiable CI builds |

**Rules**
- Certificate validity is at most 460 days since 2026-03-01 (Certum issues 459 days). Keys must live on an HSM or hardware token.
- Sign **every PE file** with SHA-256 and an RFC 3161 timestamp: both exes, the installer, the uninstaller and the FFmpeg DLLs.
- Keep the certificate subject identical on renewal.
- An individual certificate shows **your legal name** as the publisher.

### 8.2 Store and direct download

**Store account**
- Registration is now free for Individual and Company accounts.
- **Converting Individual to Company isn't supported.** You would need a new account. This matters for "free now, monetize later".

**Store MSIX**
- Microsoft re-signs the package, hosts it on its CDN and auto-updates it, and users never see SmartScreen.
- Build it with Microsoft's **winapp CLI**, in public preview, which supports Tauri/Rust.
- Under MSIX, writes to `HKCU` are redirected into a private per-app location, so autostart needs `windows.startupTask`. That activates only after the first launch, and the Tauri autostart plugin won't work.
- Declare `runFullTrust`, `graphicsCaptureProgrammatic` and `graphicsCaptureWithoutBorder` (both general-use capabilities), plus the microphone.

**Store EXE/MSI**
- You sign it with a certificate chaining to a Trusted Root Program CA.
- It must install silently and offline. That means bundling the offline WebView2 installer, about +127 MB.
- It must sit at an immutable, versioned URL, and the Store won't update it.

**Plan:** use a signed direct download for the fast-moving beta. Add the Store MSIX for the public release, then keep two build flavours.

### 8.3 Installer and updater

**Default: Tauri NSIS + `tauri-plugin-updater` 2.14.0**
- Installs per-user into `%LOCALAPPDATA%` with no admin rights.
- Requires its own **update-signing key, which can't be disabled**. Back it up offline: losing it strands every installed copy.
- Downloads full packages; there are no delta updates.
- The app exits during an install. `NSIS_HOOK_PREINSTALL`/`POSTINSTALL` hooks stop and restart the engine.

**Alternative: Velopack 1.2.161 (MIT)**
- Per-user install.
- zstd per-file delta updates, on by default.
- Worth it if large FFmpeg DLLs make full updates painful.

**Hosting**
- GitHub Releases has no bandwidth cap, but each file must be under 2 GiB.
- A private repo can't serve updates anonymously, so publish from a public releases repo or a bucket.

**Rule:** never update while a game is running or a clip is being saved. Install on exit or when idle.

### 8.4 Crash reporting and telemetry

**Sentry**
- Plans: Developer is free (5k errors/month, 1 user); Team is $26/month billed annually. An EU (Frankfurt) region is available.
- Rust SDKs: `sentry` 0.49.3, plus `sentry-rust-minidump` for native crashes.
- Sentry drops minidumps after processing by default, and **doesn't scrub attachments**. So keep minidump storage off, scrub file paths in `before_send`, and never attach clips or screenshots.
- Upload PDBs from CI; never ship them.

**Consent (EU)**
- Under EDPB Guidelines 2/2023 (v2.0, para 33), software that "proactively call[s] an API endpoint" falls under ePrivacy Art. 5(3).
- So you need an **opt-in toggle in onboarding plus a "Send report?" prompt** for each crash.
- Update checks: keep the payload minimal (version, OS build), disclose it, and let users turn it off.
- No analytics in the beta. Later, opt-in Aptabase (free up to 20k events/month).

### 8.5 Legal and privacy checklist

**Privacy policy** (Store policy 10.5.1 requires one for every Win32 app; GDPR Art. 13)
- Clips, screenshots and audio stay on the device.
- List what *does* leave the device (update checks, opt-in crash reports), with the legal basis, processors and retention for each, and users' rights.
- Some countries also require a legal notice (Impressum).

**Voice chat**
- By default, record only the user's own mic.
- Voice-chat apps go on a separate track that is **off by default**.
- Show a one-time notice: "Recording others may require their consent".
- At share time, offer "remove voice-chat track".
- Why:
  - §201 StGB (Germany) makes recording others' non-public speech a crime.
  - Publishing a clip takes it out of GDPR's household exemption (*Buivids*).
  - California requires all-party consent.

**Publisher rules for game footage**
- Users are responsible for what they post; say so in the ToS and in a first-share dialog.
- **Kairo's own marketing** (Store listing, trailers, website, ads) uses only your own UI shots or footage you have written permission for.
- Microsoft's Game Content Usage Rules:
  - forbid game content "in an app that you sell in an app store";
  - require permission to promote a commercial venture, and nobody has been granted it yet;
  - bar free apps from earning ad revenue from game content;
  - require a disclaimer and a link to the Rules.
- Riot "prohibit[s] the use of our IP in games and apps".
- **Never build paywalled clip viewing.** Microsoft, Riot, Valve and EA forbid charging viewers. **[Uncertain]** The Valve and EA texts weren't re-read.
- Nintendo requires creative input unless the footage was captured with Nintendo's system features.

**Music and DMCA**
- Separate tracks let users mute music.
- Add a reminder at share time.

**Minors**
- A local v1 is mainly a Store age-rating (IARC) question. Keep telemetry off by default.
- Cloud accounts: 13+, given the amended COPPA rule (compliance by 2026-04-22), and 16+ for public social features in the EU.

**EU product law**
- **Cyber Resilience Act:**
  - Reporting to ENISA's Single Reporting Platform has applied since 2026-09-11: 24 h early warning, 72 h notification, final report within 14 days.
  - Full obligations apply from 2027-12-11.
  - Monetizing, charging for support, or requiring personal data beyond security or compatibility counts as commercial activity. Non-monetized FOSS is out of scope.
- **Product Liability Directive:** software counts as a product when placed on the market after 2026-12-09.

**Also**
- A security contact (`security.txt`) and a written process for handling vulnerabilities.
- Clear the name (§4.4).

### 8.6 Licensing constraints

**FFmpeg (LGPL route)**
- Build without `--enable-gpl` or `--enable-nonfree`, and link the DLLs dynamically.
- Ship the FFmpeg source and the configure line, and credit FFmpeg.
- The EULA must **not forbid reverse engineering**. Don't obfuscate the DLL names.
- No x264 or x265.
- BtbN's LGPL builds support Windows 10 22H2 and later.

**libobs** is GPL-2.0-or-later. Using it makes Kairo GPL; isolating it in a separate process is only *arguably* safe under the FSF's aggregation guidance.

**Codec patents**
- AMD's AMF licence grants no codec licence and says "you will pay any royalties due".
- Via LA's H.264 pool: the first 100,000 units a year are royalty-free, then $0.20/unit **[Uncertain]**. How free software counts is unclear.
- Get a lawyer's view before monetizing or reaching scale.

**Other dependencies**
- Intel VPL and NVIDIA's nv-codec-headers: MIT.
- Tauri, Velopack and sentry-native: permissive.
- Avoid WiX (an OSMF fee above $10k revenue) and Inno Setup (commercial licence requested above $5k).

**Kairo's own licence options**

| Licence | Pros | Cons |
|---|---|---|
| MIT/Apache | Most trust; qualifies for SignPath | Anyone can fork it commercially |
| GPL | Required if you ever use libobs | Some contributors and companies avoid GPL |
| FSL-1.1-ALv2 (fair source) | Source visible; each version becomes Apache after 2 years; blocks competing forks | Not OSI |
| Closed | Full control | Less trust; no OSS perks |

Decide before accepting outside code, and use a CLA or DCO if you might relicense.

### 8.7 Year-1 cash

| Situation | Signing | Year-1 cash |
|---|---|---|
| Store-only | Microsoft re-signs | **$0**. But slow certification per beta build, and no direct download. |
| **EU individual (recommended assumption)** | Certum OV | **≈€209 + domain** |
| EU company | Artifact Signing | ≈$120 + company costs (possible business-history requirement **[Uncertain]**) |
| US or Canadian individual | Artifact Signing | ≈$120 |

Sentry, GitHub Releases and the Store account are free at beta scale.

---

## 9. Risks and unknowns

### 9.1 Top risks

| # | Risk | Why it matters | Mitigation |
|---|---|---|---|
| R1 | Exclusive-fullscreen games freeze or go black (WGC/DDA, 24H2+) | Missed clips in some titles; only injection fixes it | <ul><li>Freeze detection and borderless guidance.</li><li>List your affected games in S1.</li><li>Never inject.</li></ul> |
| R2 | The Windows 10 split | A second capture backend; per-app audio undocumented there; the long-session noise bug is unfixed on Win10 | <ul><li>DDA backend, which doubles as the Win11 fallback.</li><li>Watchdog and soak tests.</li><li>Label Win10 audio "best effort".</li></ul> |
| R3 | Narrow hardware coverage | One PC (AMD *or* Intel). Hosted CI tests NVENC only. | <ul><li>Pick beta testers for GPU and OS spread.</li><li>Attach the capability report to every bug.</li><li>In-app capture self-test.</li></ul> |
| R4 | AMD overhead | In LTT's test OBS lost 16 fps on the RX 9060 XT vs 5 on the RTX 5060; cause unknown | Measure the AMF path with PresentMon in S1/S2 before committing |
| R5 | Encoder fragmentation | No encoder on some GPUs; AV1 limited; no AMD HEVC B-frames; RDNA3 1082-line AV1; QSV `hwmap` unproven; NVENC headers need driver ≥610 | <ul><li>Probe and default to H.264.</li><li>Choose the FFmpeg build's NVENC headers deliberately.</li><li>Decide on CPU fallback (Q11).</li></ul> |
| R6 | Engine-language churn | windows-rs 0.62 → 0.100; `ffmpeg-next` in maintenance mode; the agent looping on borrow-checker errors | <ul><li>Pin `windows` 0.62.</li><li>Isolate `unsafe` code.</li><li>Switch to C# after 7 days if needed.</li></ul> |
| R7 | Hotkeys fail silently | Elevated games (UIPI), `RIDEV_NOHOTKEYS` games, vendor-tool clashes | <ul><li>Modifier defaults and a conflict UI.</li><li>An elevated-game notice.</li><li>Raw Input fallback in v1.1.</li></ul> |
| R8 | Silent or wrong audio | Wrong process (launchers, windowless voice processes), ducking, exclusive mode, drift | <ul><li>Process-tree resolution plus manual override.</li><li>Level meters; `SetDuckingPreference`.</li><li>Drift compensation; watchdog.</li></ul> |
| R9 | Legal exposure | Voice-chat consent, EU opt-in, publishers' footage in marketing, hosting duties, CRA once monetized, codec patents | §8.5 defaults; a lawyer before monetizing or hosting |
| R10 | Trust friction | SmartScreen, Smart App Control and AV flags on a hotkey + capture + mic app; an EU individual can't use Artifact Signing | <ul><li>Sign every PE file with a stable identity.</li><li>Store MSIX for the public release.</li><li>Submit false positives to Defender.</li><li>Test with Smart App Control on.</li></ul> |
| R11 | Name collision | KAIRO highlights SaaS, Kairosoft, the 2013 Steam game | Trademark clearance before the public launch; a domain with a modifier |
| R12 | OS updates change capture behaviour | A 24H2 DDA/MPO change (later fixed by KB5046617) | In-app self-test; re-test after every Windows feature update |

### 9.2 Fact-check changes applied

| Research claim | Status | What this brief uses |
|---|---|---|
| AMD driver notes "still list" an Instant Replay timeout (report 04, TL;DR and tables) | **Refuted** | 26.9.2 lists it under *Fixed Issues* (War Thunder DX12, Instant Replay + Borderless Region Capture, RX 6000). 26.10.1 has no capture fixes or known issues. The rest of AMD's profile (20 min in 15 s steps, AV1/HEVC/AVC, 30/60 fps, separate mic file) is confirmed. |
| Win10 consumer ESU "ran to 2026-10-13" (report 04 note) | **Corrected** | ESU runs through 2027-10-12 |
| Ctrl/Alt `RegisterHotKey` combos "reportedly work" against elevated windows, citing LibreAutomate (report 03) | **Corrected** | LibreAutomate only says single-key and Shift+key hotkeys mostly fail. Ctrl/Alt working is an inference to test. |
| The brief's target Win11 versions: 23H2, 24H2, 25H2 | **Corrected** | 24H2, 25H2 and 26H2. 23H2 and 24H2 Home/Pro are out of updates. |
| Process-loopback noise starts after "15 min to several hours" (report 02) | Qualified | Reported onset ~2 min to ~2 h. Maintainers said Microsoft fixed it in Win11 24H2+ only; some 24H2 users still saw it in Jul–Aug 2025; #8064 closed after reports on 25H2 26200.6899; #8086 closed as a likely duplicate. |
| Exclusive-fullscreen freeze on 24H2+ | Qualified | One report, two OpenGL games, closed in a day; supported by Microsoft's swap-chain docs |
| LTT Labs FPS figures | Qualified | Via GIGAZINE; primary page not fetched; resolution and averaging unknown |
| Process loopback survives default-device switches | Uncertain | Microsoft says it isn't tied to an endpoint; switching behaviour is undocumented, so test |
| Xbox 10 GB capture cap | Uncertain | Third-party guides only. The 90-day deletion is confirmed. |
| NVIDIA MCP server "beta, on by default" | Qualified | NVIDIA's notes say neither; the press does |
| Medal Premium "unlimited" uploads | Conflict | Blog (2026-06-17): unlimited. FAQ still lists 24 h / 4K / 144 fps. GI also raised $320M at $2.3bn in Jan 2026. |
| Steam: 30 s Instant Clip, HEVC default | Qualified | From late-2024 beta notes, not the FAQ |
| Game Bar: 30 s default, up to 10 min, 30/60 fps | Qualified | Dell and third-party guides, not Microsoft |
| kairo.io "for sale" | Uncertain | Registered at GoDaddy since 2023-02-03; Afternic not checked |
| B2 + Bunny ≈$160/month | Qualified | Bunny Volume network; ≈$310 on Standard. Bunny Stream and Mux figures not re-derived. |
| HEVC extension "$0.99" | Qualified | Retail $0.99. Edge's docs link the free "Device Manufacturer" listing, which only some OEM PCs have. |
| Artifact Signing organisation regions | Qualified | 12 per the quickstart (2026-10-08); an older page lists 4; signature quotas not re-read |
| Smart App Control re-enable (KB5083769) | Qualified | Press reports; previewed in KB5074105 |
| Store EXE/MSI "non-gaming only" | Not found | Doesn't affect Kairo |
| FFmpeg edit-list trim experiment | Not reproduced | The Instagram and LosslessCut statements are confirmed; Reels also needs a closed GOP |
| Elgato 4K X price | Updated | $229.99 MSRP at launch; $249.99 US retail now |
| Picovoice shutdown | Qualified | The FAQ shows no personal plans; the 2026-06-30 date comes from emails quoted on a forum |
| GeForce NOW terms | Qualified | Silent on recording; bar copying or distributing "any portion of GFN" and commercial use |
| GitHub GPU runner | Qualified | $0.102/min; the billing page doesn't name the GPU |
| Electron clipboard needs a native addon | Qualified | Electron could probably write `FileNameW` for single-file pastes into Chromium apps (untested); Explorer still needs CF_HDROP |
| DSA Art. 19 | Qualified | Exempts micro/small enterprises from Arts. 20–28 except 24(3), and for 12 months after outgrowing that status. Ofcom's hash-matching expectation not re-checked. |
| Certum prices | Qualified | Also a €169 card set and a €139 code; 459-day maximum from 2026-02-27; the OSS revocation term not re-read |
| Velopack signing flag, electron-updater blockmap default, `ms-appinstaller` default | Not re-read | Treat as likely but unverified |
| No anti-cheat bans for WGC/DDA capture | Absence of evidence | Can't be positively confirmed |

### 9.3 Still to settle

**On the gaming PC (the spikes)**
- Capture:
  - Whether an unpackaged app sees a consent prompt for a borderless capture.
  - Whether capture knocks games off Independent Flip.
  - WGC delivery at 60 and 144 Hz.
  - The QSV `hwmap` path.
  - HDR tone-mapping.
- Audio:
  - Process loopback on Win10.
  - Device switching.
  - Ducking.
- Input:
  - XInput background reads.
  - GameInput deployment, and its effect on DualSense mode.
  - `RegisterHotKey` against elevated and NOHOTKEYS games.
- UI and playback:
  - Whether WebView2 processes exit.
  - WebView2 HEVC playback.
  - Tauri range seeking.
- Sharing and capture cards:
  - CF_HDROP paste into Discord and WhatsApp.
  - Size accuracy of the hardware encoders.
  - `SharedReadOnly` coexistence with OBS.

**With vendor docs or support**
- Signing and the Store:
  - Whether Artifact Signing qualifies for the Store EXE route.
  - Whether Partner Center can transfer an app from an Individual to a Company account.
- Cloud APIs:
  - Whether R2 accepts presigned UploadPart URLs.
  - Discord Nitro: 500 MB or 1 GB.
  - YouTube's private lock on unverified projects, and Google OAuth verification for `youtube.upload`.

**With a lawyer**
- Codec patents for a free vs paid app (the AMF disclaimer, Via LA).
- AAC encoder licensing.
- Voice-chat recording law where you live.
- The GPL aggregation question, if libobs is ever considered.
- CRA scope once Kairo is monetized.
- DSA and UK OSA duties once Kairo hosts clips.

---

## 10. Product questions you still need to decide

These are product decisions, not technical trivia. Recommended defaults are in brackets.

**A. Ambition, people, hardware**
1. Do you confirm the staged path (private beta → free public release → monetize only with traction)? Rough dates: when does the beta start, how long does it run, and how many friends join? What would count as a successful beta? [e.g. no missed clips in a week of play, ≤3% FPS cost]
2. What exactly is in your PC: GPU model, CPU, RAM, Windows edition and build, monitor resolution and refresh rate, HDR on or off? Desktop or laptop? This decides AV1, HEVC and 120 fps, and which encoder path gets tested first.
3. What do your beta testers run? Is anyone on NVIDIA, Intel, Windows 10, or a laptop? Will they click "Run anyway" on a SmartScreen warning? Do they have Microsoft accounts?
4. Which games do you and your friends play most, and with which anti-cheats? Do any run as administrator or in exclusive fullscreen? Do you confirm "never inject into games" as a hard rule? [yes]

**B. Capture behaviour**
5. Should Windows 10 be in scope for v1? It costs a second capture backend, for ~23.6% of Steam users. Or Windows 11 only until the public release? [support it by the public release]
6. What default and maximum clip length? 30 s ≈ 95 MB of RAM, 2 min ≈ 381 MB. Should Kairo keep recording a few seconds after the hotkey? [30 s default, 120 s max, 0 s post-roll]
7. 1080p60 for everyone, or native resolution? Offer 120/144 fps? AMD's own tool stops at 60. [1080p60; 120 fps as an option]
8. Capture the whole monitor or only the game window? The whole monitor matches "anything else on your screen" but catches notifications, DMs and pop-ups. Game window is cleaner and private, Win11 only.
9. Should the buffer run only while a game is detected, or always, including the desktop? Should Kairo start with Windows by default after you agree at onboarding?
10. Is a manual start/stop recording needed in the beta, or strictly later? [later]
11. PCs with no hardware encoder (RX 6400/6500 XT, MX450): CPU fallback at 720p30–60, or unsupported?
12. HDR: is a tone-mapped SDR clip enough, or will you ever want HDR10 export? [SDR by default]

**C. Audio and privacy**
13. Which audio should be separate tracks (game, your mic, voice chat, other apps), and what goes in the default mix? Should friends' voice chat be recorded by default? [off]
14. Which voice apps do you and your friends use (Discord, in-game voice, TeamSpeak, Steam chat)? Push-to-talk or open mic?
15. Which public promises will Kairo make: no account ever for local use, no ads, no data sale, never used for AI training? [all four]
16. Telemetry: opt-in crash reports only, or opt-in analytics too? Is Sentry (a US company, EU data region) acceptable? [crash reports only in the beta]

**D. Triggers and feedback**
17. Are Alt+F8 (clip) and Alt+F6 (screenshot) good defaults? Which keyboard layout do you use (AltGr)? Should Kairo stop the game from also seeing the key?
18. Mouse side buttons, controller buttons (which controllers; through Steam Input or DS4Windows?) or a voice command ("clip that"): v1, v1.1, or never? [mouse in v1, controller in v1.1, voice in v2]
19. Should the sound cue be on by default? The small toast? Should Kairo ever put a watermark on clips? [cue on, toast on, no watermark]

**E. Console and cloud**
20. Which consoles (PS5, Xbox, Switch 2) and cloud services (GeForce NOW, Xbox Cloud, PS Remote Play) do you use? Do you own or plan to buy a capture card, and which one? Is capture-card replay a v1.x must?
21. For console clips, is a generic "import files or folder" enough at first? [yes]

**F. After the clip**
22. Where do your clips mostly go: Discord DMs or servers, WhatsApp, TikTok/Shorts/Reels, YouTube? This decides the presets and when hosted links become urgent.
23. What should the beta's editor do: trim only, merge, vertical 9:16 with a facecam layout, captions? Do you use a webcam? [trim + 9:16 presets]
24. Where should the library live (Videos\Kairo or another drive), how should it be organised (by game?), and what should automatic cleanup do (maximum size, minimum free space)?
25. When share links come, should they be unlisted only, or public profiles, feeds and comments? How long should clips be kept, and what is the minimum user age? [unlisted, 30 days, 13+]
26. Screenshots: capture only, or also annotation and upload, ShareX-style? [capture only in v1]

**G. Business, identity, distribution**
27. Where are you legally resident, and do you have or plan a registered company? This decides the signing route (Certum ~€209 vs Artifact Signing $9.99/month), the Store account type (which can't be converted later), and exposure to the Cyber Resilience Act.
28. Open source or closed? If open, which licence: MIT/Apache, GPL, or fair source (FSL)? Would you ever use libobs, which forces GPL?
29. Which public channel: the Microsoft Store, a signed direct download, or both? What is your year-1 budget ceiling (€0 / ~€50 / ~€209 / ~$120 + company)? [both; ~€209]
30. If Kairo is ever monetized, what would you sell (cloud storage, editing extras, team features)? Note that paywalling clip *viewing* breaks publisher rules.
31. Keep the name "Kairo" despite the KAIRO highlights SaaS, Kairosoft and the 2013 Steam game? What domain approach (kairoclip.com is unregistered; kairo.gg and kairo.app are taken)?
32. How much of the engine do you want to be able to read yourself? Learn to read Rust (option #1), own a C# engine (option #2), or delegate it to the agent?
33. Later auto-highlights: which games matter to you? Only official APIs are allowed, e.g. League Live Client Data or CS2 Game State Integration.
