# kairo-verify, the testbed and the voice-tone process: architecture, tooling and process

*Design for M1 (docs/milestones/M1.md), 2026-10-24. Nothing in the repo was changed. Repo snapshot: `main` at `21f0770`. The brief named `3775e7c`; PR #16 landed while this was written. It only recorded the owner's answers to six "Your part" questions and added PLAN decision 37, and §4.1 uses those answers. Line numbers below are from `21f0770`.*

**How to read the references.** `CLAUDE` is CLAUDE.md. `PLAN` is docs/PLAN.md. `M0` and `M1` are docs/milestones/M0.md and M1.md. `R01`, `R08`, `R09`, `R10`, `R12` and `R13` are docs/research/NN-*.md, and `CB` is docs/research/15-implementation-cookbook.md. `core/…` means crates/kairo-core/src/…, `devrun/…` means crates/kairo-devrun/src/…, `pc` is tools/mac/pc, `remote/…` is tools/mac/remote/… and `ci.yml` is .github/workflows/ci.yml. `ff:` is the FFmpeg **n9.0.2** source (github.com/FFmpeg/FFmpeg/blob/n9.0.2/…), read for this design. `EXP-n` is an experiment run on the Mac for this design (Homebrew FFmpeg 8.0.1, using only `h264_videotoolbox`, native `aac`, demuxers and decoders); see Appendix A. A label in brackets, such as **[Unverified]**, **[Estimate]**, **[Verified: EXP-n]** or **[Verified: ff source]**, says how sure a claim is.

---

## 0. Decisions at a glance

| # | Question | Decision |
|---|---|---|
| 1 | Where the code goes | `kairo-core` gets three pure additions: `testsig` (the testbed's signal contract: layout, barcode, bars, flash, tone schedule and references), `testbed_log` (the QPC log format) and `cadence::steps`. New crates: **`kairo-verify`** (a library plus the CLI; runs on any OS; `#![forbid(unsafe_code)]`); **`kairo-testbed`** (`testbed.exe` and the voice-tone process **`kairo-tone.exe`**); **`kairo-display`** (one Windows library for "monitor by device path, adapter by LUID", shared by the testbed, the harness, loadgen and `wgc-probe`); **`kairo-harness`** (`kairo-harness.exe`, the Windows orchestrator behind `pc harness`) |
| 2 | Decoding | **Both, behind one trait, with fixed roles.** Every structural check (boxes, sample tables, edit lists, `colr`, SPS/VUI, slice types, AAC config) uses Kairo's own pure-Rust MP4, H.264 and AAC parser, so neither ffprobe nor any FFmpeg is involved. Pixels and samples come from **linked LGPL FFmpeg** through `ffmpeg-next` =9.0.0 / `ffmpeg-sys-next` =9.0.1. It is Windows-only and is the production and gate path, which "Test my setup" reuses later. A **dev-only CLI-pipe backend** spawns an ffmpeg executable named on the command line and reads rawvideo and f32 PCM from its stdout, so the Mac can analyse real H.264. A **synthetic backend** feeds the tests. Timing always comes from Kairo's parser, never from a decoder |
| 3 | Good clips for gate 1 | Two kinds. **Golden clips:** the testbed pattern is rendered by `kairo-verify synth` and encoded on the PC by the pinned LGPL `ffmpeg.exe` with `h264_amf` (NV12, the full option list), native `aac` and the engine's mux settings. They are known-good by construction and carry four audio tracks. **Real captures:** the testbed recorded by the gate-2 harness (`gfxcapture` → `h264_amf`, video only) |
| 4 | Bad clips | Each has one defect, injected at the earliest point where it can be exact. Content defects (drop, duplicate, judder, 50 ms A/V, corrupted bars) are made in the synthetic source. B-frames come from an encoder setting. Container defects (no faststart, edit list, wrong `colr`, swapped tracks) are box-level edits (`kairo-verify mutate`) of the encoded golden clip, never FFmpeg remuxes, because a remux silently drops other metadata **[Verified: EXP-5]**. A manifest lists `must_fail` and `may_fail` for each clip, and `kairo-verify corpus check` turns it into gate evidence |
| 5 | Stereo Mix | Don't switch it on. M1's captures are video-only. The A/V, routing and tone checks rest on the golden clips (M1:187) |
| 6 | Gate 2 | `pc harness capture` writes one `\Kairo\Run` job that runs `kairo-harness.exe run …`. The harness starts the testbed on the 180 Hz monitor, waits 3 s, records 30 s with `gfxcapture` → `fps=60` → `hwdownload` → swscale BT.709 limited → NV12 → `h264_amf`, then runs `kairo-verify --profile recording`. It takes about 70 s **[Estimate]** against the 300 s budget. Outputs go to `C:\kairo\logs\harness-<stamp>\` |
| 7 | Profiles | `test` (strict), `recording` (lenient), `engine` (M3 saves), and one per export: `discord`, `whatsapp`, `reels`, `share` and `original` |
| 8 | Judder rule | A *judder step* rounds to exactly one tick but is more than ¼ tick away from it. The check fails when more than 1% of steps are judder steps. A 90/s-at-180 Hz cadence gives 100%. The beat between a 60.000 Hz pacer and a ~180 Hz panel gives ≪1%. The rule is relative to the tick, not the refresh rate, which settles M1:189 |
| 9 | Report | A versioned JSON report (`"kairo_verify": 1`) and a short text summary. It carries file names only and nothing private. Exit codes: 0 pass, 1 fail, 2 usage, 3 cannot evaluate, 4 internal |
| 10 | Clips | Never committed. On the PC they live in `C:\kairo\logs\{corpus,harness}-<stamp>\`, on the Mac in `.dogfood/`. CI generates lossless I_PCM H.264 clips at test time instead of fixtures, so the owner's OK for a committed clip (M1:119) is never needed |
| 11 | PR plan | Nine PRs (§6). PRs 1–5 are Mac/CI-only. PRs 6–7 and 9 need short attended PC runs. PR 8, the devrun safety stops, gates any unattended run |

---

## 1. Crates and modules

### 1.1 Crate map

| Crate | Kind | Builds on | `unsafe` | New dependencies | Holds |
|---|---|---|---|---|---|
| `kairo-core` (exists) | lib | all | forbidden (core/lib.rs:15) | none | + `testsig`, `testbed_log`, `cadence::steps` (§1.2) |
| **`kairo-verify`** | lib + bin `kairo-verify` | all; the linked decoder only on Windows | **forbidden** (the `ffmpeg-next` safe API is enough, §2.3) | Windows only: `ffmpeg-next` =9.0.0 (`default-features=false`, `codec`, `format`) and `ffmpeg-sys-next` =9.0.1 (`avcodec`, `avformat`) | parser, checks, profiles, report, decoders, analysis, `synth`, `mutate`, `corpus`, test generator |
| **`kairo-testbed`** | bins `testbed`, `kairo-tone` | Windows; stub `main` elsewhere, like devrun/main.rs:37-41 | only in `platform`, `gpu` and `audio`, with `// SAFETY:` (CLAUDE:32) | `windows` =0.62.2, `wasapi` =0.26.0 (which itself needs `windows` ^0.62.2, R10:45) | window, swapchain, pacing, renderer, WASAPI beeps, QPC log |
| **`kairo-display`** | lib | Windows; pure types everywhere | only in `platform` | `windows` =0.62.2 | QueryDisplayConfig, device path → GDI name → HMONITOR → rect and refresh, adapter LUID → DXGI index and vendor:device, HDR state (R08:614-673) |
| **`kairo-harness`** | bin `kairo-harness` | Windows; stub elsewhere | none (uses `kairo-display`) | none | `run` (gate 2), `corpus` (gate 1 on the PC), `displays` |

These names are Claude's to choose (M1:50) and get added to the M1 contract's Programs table as each PR lands: `kairo-tone.exe`, `kairo-harness.exe` and the `kairo-display` library. PLAN's crate list (PLAN:296-297) already names `kairo-verify` and `kairo-testbed`.

### 1.2 `kairo-core` additions (pure, Mac and Linux CI)

**`testsig.rs`: the signal contract, "pattern v1".** It is shared by the producer (the testbed renders it, `kairo-tone` plays its tones, `synth` writes it) and the consumer (`kairo-verify` reads it back), so the two can never drift apart.

- **Layout.** The unit is `u = round_even(H/27)`: 40 px at 1080p, 54 at 1440p, 20 at 540p. Two barcode copies of 8×6 blocks of u×u sit centred horizontally, one at the top (rows u..7u) and one at the bottom (H−7u..H−u). Centring keeps them inside a 9:16 centre crop, which the vertical exports need (PLAN:245). A band of 75% bars runs at y 9u..18u: 8 bars of 2u each, scrolling `u/4` px per 1/60 s of content time, quantized to 2 px so that chroma edges stay on even pixels. The flash patch is a 4u×4u square at the lower right (x W−6u..W−2u, y 19u..23u): black, and white for content time [k s, k s + 50 ms). The HDR or grey patches are a row of 2u×2u squares at the lower left (y 19u..21u) that drift slowly within the left third, so nothing stays static on the OLED (M1:193). The background is RGB 64.
  - **Nothing overlaps.** At 16:9 the width is 48u. The centred barcodes take x 20u..28u, while the patches stay left of 14u and the flash right of 42u.
  - **Vertical crops.** A 9:16 centre crop keeps both barcodes and the middle of the bars band but cuts off the flash patch. `av.offset` then takes the flash time from the barcode's content numbers.
- **Barcode payload, 40 bits.** Row 0 is a calibration row, W B W B W B W B, which gives the threshold and the orientation. The other 40 blocks carry the 24-bit content frame number (13 h at 360 fps), a 4-bit content-rate code (30, 60, 72, 90, 120, 144, 165, 180, 240, 300, 360), a 4-bit run tag and a CRC-8. Bits 8–39 are XOR-whitened with a PRBS seeded by the raw low byte, so no block holds a level for more than 256 frames, which is OLED care (M1:93). A frame decodes only when its CRC matches. When the two copies decode but disagree, the frame is *torn*, a useful diagnostic for SP2's tearing modes. Because the blocks are white or black, decoding works on luma alone.
  - **[Verified: EXP-9]** This geometry survived `h264_videotoolbox` at 1080p, at 720p/4.6 Mbps, at 540p/2 Mbps, and through a 9:16 centre crop scaled to 1080×1920: 0 bit errors in 300 frames with random payloads, and the worst block was still 0.49 of the black–white span from the threshold. The M1 fallback, "barcode lost in compression → bigger blocks" (M1:200), stays available through the unit.
- **References.** `BARS_75_RGB` and `bt709_limited(rgb) → (Y, Cb, Cr)` follow the published BT.709 matrix: 75% white is (180, 128, 128), yellow (168, 44, 136), and so on. Unit tests compare them with an independent Python model, as the other kairo-core tests do (core/cadence.rs:117). PQ (BT.2100) and the BT.2390 EETF are implemented for the curve-level check, from the published formulas and not from OBS code (M1:192, CLAUDE:17).
- **Schedules.** `content_hns(n, rate)` turns a content frame number into content time, with the same exact-integer rounding as core/timeline.rs:86-148. `flash_on(n, rate)` gives the flash state. The tones, with onsets in content seconds k: the game beep is 1 kHz at k+0 (played by the testbed); the voice beep is 2 kHz at k+0.5 s (played by `kairo-tone`); a 3 kHz mic marker at k+0.25 s exists in synthetic clips only; and the cue placeholder (880+1320 Hz at k+0.75 s) appears only in a defect clip until M4 defines the real cue. Every tone is a 50 ms burst at a **−30 dBFS peak** (CLAUDE:26) with 2 ms raised-cosine ramps. `burst(tone, 48000)` returns the exact samples. The renderer plays them, and `kairo-verify` uses the same samples as its matched-filter template.

**`testbed_log.rs`: the QPC log format.** It follows trace.rs's conventions (core/trace.rs:1-23; M1:83): JSON Lines, a version header `{"kairo_testbed_log":1,…}`, every line written in one `write_all`, a parser that drops a torn last line, times in 100 ns counted from an epoch the harness passes in, and no monitor, adapter or PC names, refresh rates or wall-clock times. Line kinds:

- `frame`: n, render start, Present return, present count;
- `stats`: present count → refresh count and sync time, from `GetFrameStatistics`;
- `beep`: index, Hz, target time, device position, mapped time;
- `miss`: a vblank the testbed missed;
- `end`.

**`cadence::steps`.** This is a per-step iterator, `Step { index, ticks, deviation_hns }`, with exactly the rounding of core/cadence.rs:59-111. `cadence()` is re-expressed on top of it. That gives `kairo-verify` frame-numbered events (which step skipped or repeated) without a second rounding rule. The existing tests stay unchanged.

**Reused as they are.** `timeline::FrameRate` and its constants are the clip's output rate (30, 60 or 120 fps; core/timeline.rs:19-57). `cadence` provides drops and dups as `(repeats + skipped)/steps` and the M3 limits `longest_hold ≤ 2` and `max_step_ticks ≤ 2` (core/cadence.rs:11-15). `cut.rs` documents the allowed first-video-sample hold of 1,024 to 2,047 samples (core/cut.rs:7-12, 49-52). `trace::replay` turns synthetic vblank traces, and later the reduced SP2 trace fixture (M1:195), into content sequences for kairo-verify's tests, so the judder and drop rules are checked against the pacer's own replay. `layout::Layout` supplies `C:\kairo\logs` to the harness (core/layout.rs:67-70).

### 1.3 `kairo-verify` (library and CLI)

```
kairo-verify/src/
  lib.rs            pub fn check(path, &Profile, &Expect, &mut dyn Decoder) -> Report
  mp4/              boxes.rs (tree, read and serialize) · tables.rs (stts, ctts, stss, stsc, stsz/stz2, stco/co64 → per-sample
                    offset, size, dts, cts, sync) · timeline.rs (presentation times, single-entry elst applied the way players do)
                    · meta.rs (tkhd flags = disposition, alternate_group, hdlr name, udta/name, ilst tags)
  h264.rs           length-prefixed NAL iteration, emulation prevention, SPS (profile, chroma, bit depth, frame_mbs_only,
                    cropping, VUI colour, max_num_reorder_frames), PPS (entropy_coding_mode), slice header (nal type, slice_type)
  aac.rs            esds → AudioSpecificConfig (object type, rate index, channels)
  decode/           mod.rs (trait) · linked.rs (cfg(windows), ffmpeg-next) · cli.rs (spawn ffmpeg) · synthetic.rs
  analyze/          barcode.rs · flash.rs · bars.rs (PSNR) · tones.rs (matched filter, onset, SNR) · cadence.rs · av.rs · hdr.rs
  checks/           structure.rs · content.rs · audio.rs · export.rs, one fn per check id (§5.2)
  profile.rs        thresholds per profile, each row with its source
  report.rs         JSON v1 (serde) and the text summary
  synth.rs          pattern v1 → NV12 frames (stdout) and one WAV per track, with --defect
  mutate.rs         box-level defects (§3.4)
  corpus.rs         build: synth → ffmpeg → mutate → manifest; check: manifest vs reports
  testgen.rs        Annex B H.264 writer with I_PCM macroblocks (lossless, CAVLC, IDR period n), for CI
  main.rs, cli.rs   hand-written argument parsing in the style of core/args.rs, so no clap on the 8 GB Mac
```

**Dependencies.** Only `serde`, `serde_json` and `kairo-core` from the workspace on every OS, plus FFmpeg on Windows. CRC-8, the PRBS, the WAV writer and the matched filter are a few dozen lines each and need no crates.

### 1.4 `kairo-testbed`: `testbed.exe` and `kairo-tone.exe`

```
testbed.exe   --monitor-path <monitorDevicePath> [--mode borderless|windowed|exclusive] [--size WxH]
              [--fps 60..360] [--pace vsync|timer] [--seconds N] [--run-tag 0..15] [--epoch-qpc HNS]
              [--log FILE] [--ready FILE] [--beep on|off] [--hdr auto|on|off] [--oled-safe]
kairo-tone.exe beeps --hz 2000 --phase-ms 500 [--period-ms 1000] [--burst-ms 50] [--dbfs -30]
              --epoch-qpc HNS --seconds N --log FILE          (M1: the voice tone)
kairo-tone.exe soak  …                                          (added by the audio-soak PR; see below)
```

- **Monitor and adapter.** `kairo-display` resolves the device path to the GDI name, then the HMONITOR, rect and refresh, then the adapter LUID, then the `IDXGIAdapter1` with that LUID. The D3D11 device is created on that adapter and the LUID is logged (M1:38, M1:63). It never uses "primary" (M1:54).
- **Window modes.**
  - *Borderless:* `WS_POPUP` covering the monitor rect, per-monitor-v2 DPI aware.
  - *Windowed:* a framed window of `--size`.
  - *Exclusive:* `SetFullscreenState(TRUE, output)` on the `IDXGIOutput` whose `Monitor` is the HMONITOR. This mode is for M0's carried-over fullscreen test (M1:159) and for SP6.
- **Swapchain.** Flip model (`FLIP_DISCARD`) with a frame-latency waitable object and `SetMaximumFrameLatency(1)`. It is BGRA8, or FP16 scRGB when the monitor is in HDR mode and `--hdr` is auto or on. `ALLOW_TEARING` is used only with `--pace timer`.
- **Pacing (60–360 fps).**
  - `vsync` presents with sync interval `k = refresh/fps` and refuses rates that are not within 0.5% of refresh/k. On the 180 Hz monitor that gives 60, 90 and 180 fps.
  - `timer` uses a `CreateWaitableTimerExW(HIGH_RESOLUTION)` on absolute QPC deadlines, from the same tick maths as core/timeline.rs, and presents with tearing: 72, 144, 240 and 360 fps on any monitor.
  - The process opts out of EcoQoS, as the cookbook does for media (CB:57, P16).
  - The content number is **time-based**: n is the index of the vblank or timer slot the frame targets. A late frame therefore shows up as a skipped n and a `miss` line, just as a game's simulation clock keeps going, and kairo-verify can tell the testbed's own misses from capture drops.
- **Renderer.** Every element of pattern v1 is a solid rectangle, so drawing is `ID3D11DeviceContext1::ClearView` with a list of rects. There is no HLSL and nothing from OBS (CLAUDE:17). **[Unverified]** ClearView-with-rects on the RX 9000 driver; the fallback is a 20-line quad shader through runtime `D3DCompile`.
- **Audio.** The 1 kHz game beep comes from the testbed's own process, so a process-loopback include-tree on the testbed's PID captures it as "Game" in M3. It uses `wasapi` =0.26.0 in shared, event-driven mode at 48 kHz f32. `AudioClient::get_audioclock()` and `AudioClock::get_position() → (pos, qpc)` (wasapi 0.26.0 `src/api.rs:1409, 2000`, **[Verified: crate source]**) map each beep's target QPC to a stream sample.
  - *Aiming.* Beep k is aimed at the predicted display time of the first frame whose content time is ≥ k s, the flash's first frame. The prediction comes from the swapchain's vblank timing, and audio is scheduled at least 100 ms ahead.
  - *Logging.* The predicted time, the time mapped from the device position and the flash frame's actual display time (from `GetFrameStatistics`) are all logged, so the testbed's own flash-versus-beep offset is known for every beep.
  - *The epoch* passed in only makes the log's times relative. The beep is never louder than −30 dBFS: `--dbfs` above −30 is refused (CLAUDE:26).
- **Ready file and exit.** After the first present the testbed writes `ready.json`: pid, hwnd, rect, refresh in mHz, LUID, swapchain format, mode, fps and the first present time. That file is the job's record and stays local. The testbed exits at `--seconds`, at EOF on stdin (the harness closes the pipe, which needs no unsafe call), or at WM_CLOSE. Each log line is written whole.
- **`kairo-tone.exe` is the voice-tone process.** It is a separate exe so that it has its own PID and image name (M1:55), and the harness starts it as a *sibling* of the testbed, never as a child: an include-tree on the testbed's PID must not hear the voice tone. It shares the testbed crate's `audio` module. Recommendation for the audio-soak PR: add the `soak` mode (a continuous sine with 50 ms markers and `markers.jsonl`, R10:812, at −30 dBFS per M1:93) to this same binary instead of writing `soak-tone.exe`. Days spent on that mode count toward the exit-rule clock as audio-soak work (M1:104).

### 1.5 `kairo-display` (shared Windows library)

M1 has four programs that must pick a monitor by device path and an adapter by LUID: the testbed, the harness, `kairo-loadgen` and `wgc-probe` (M1:54-63). The engine's `capture/monitors.rs` follows later (CB:83-147). One small crate avoids four copies of R08 §7.2-7.3's QueryDisplayConfig code.

- **API.** `targets() -> Vec<DisplayTarget{device_path, gdi_name, hmonitor, rect, refresh: Ratio, adapter_luid, hdr: HdrState}>` and `adapter_for_luid(luid) -> AdapterInfo{dxgi_index, vendor_id, device_id}`.
- **The index is per process.** The DXGI index is valid only inside the calling process, because DXGI may order adapters differently per process **[Unverified]**. The harness therefore also checks FFmpeg's own log line to confirm the adapter (§4.4).
- **No names in shared output.** It reports friendly names only into the local `displays.json` (`.dogfood/`), never into logs that might be shared.

### 1.6 `kairo-harness` (Windows orchestrator)

There is one binary with three verbs: `run` (gate 2 capture), `corpus` (gate 1 on the PC: it resolves the adapter and then drives `kairo-verify corpus build/check`) and `displays` (writes `displays.json`). It is a console program started by devrun with `CREATE_NO_WINDOW` (devrun/run.rs:152-155). It starts every console child (`ffmpeg.exe`, `kairo-verify.exe`) with `CREATE_NO_WINDOW` too, so no console flashes. Every child stays inside devrun's kill-on-close job object (devrun/spawn.rs:47-72), so owner input or `max_seconds` stops the whole tree (devrun/run.rs:187-203). Details in §4.

### 1.7 What later code reuses

- **"Test my setup"** (PLAN:271) links `kairo-verify`'s library and calls `check()` with its linked decoder against Kairo's own LGPL DLLs. Kairo's own minimal build keeps the needed parts: `--enable-decoder=h264,…,aac` and `--enable-demuxer=mov` (R09:826-848).
- **The M3 post-save verify** (CB:667-678, step 7; PLAN:179) can run the structure checks from `kairo_verify::mp4` in milliseconds, without `avformat_find_stream_info`. M3 decides.
- **The M3 harness.** `kairo-harness` gains an `engine` capture mode (`kairo-cli` `clip.save`). The `engine` profile already exists from M1.

---

## 2. Decoding

### 2.1 Decision

- **Structure** is parsed by Kairo's own pure-Rust MP4, H.264 and AAC code on every OS. Nothing in kairo-verify runs ffprobe.
- **Pixels and samples** come from **linked FFmpeg** (`ffmpeg-next` =9.0.0 / `ffmpeg-sys-next` =9.0.1, CLAUDE:30) against the PC's pinned BtbN LGPL build `n9.0.2-17-g2a571b6068` (PLAN:309). The library is linked on Windows only, through `FFMPEG_DIR`, with `avcodec-63.dll`, `avformat-63.dll` and `avutil-61.dll` copied next to the exe. PR 5 confirms that list with `dumpbin /dependents` and copies any further FFmpeg DLL they import.
- **Two more backends implement the same trait.** The **CLI pipe** (dev and test only) spawns an ffmpeg given with `--ffmpeg <path>`. The **synthetic** backend serves tests.
- **The linked backend refuses a GPL FFmpeg.** It checks `ffmpeg_next::codec::license()` at start-up (ffmpeg-next 9.0.0 `src/codec/mod.rs:81`, **[Verified: crate source]**) and refuses to run unless the licence string contains "LGPL". It records the version and licence in the report. This guards "nothing GPL linked" (PLAN:351; CLAUDE:17) even if someone points `FFMPEG_DIR` at a GPL build.

### 2.2 Why (the factors the brief asked about)

| Factor | Linked only | Pipe only | Chosen: own parser + linked + dev pipe |
|---|---|---|---|
| Nothing GPL linked | LGPL DLLs, allowed (CLAUDE:17; ADR-0001 item 3) | nothing linked at all | Linked on Windows against the LGPL build, with the licence guard. The pipe never ships, and on the Mac it only spawns Homebrew's binary with GPL-free codecs |
| "Test my setup" in the shipped app | reuses the engine's DLLs; the engine already verifies in-process (CB:667-678) | needs the export `ffmpeg.exe` plus frame-copy pipes, and CLI behaviour changes between versions (`-vsync` → `-fps_mode`) | linked path, in-process, with no ffprobe (M1:56) |
| M1:181, `cargo xwin check` with `ffmpeg-sys-next` | settled by the first linking crate | never settled | settled by PR 5 (CB:600-610 recipe) |
| No LGPL FFmpeg on the Mac | can't run on the Mac: Homebrew's build is GPL, and building LGPL from source costs 15–20 min and disk on 8 GB | runs everywhere | The linked code is type-checked on the Mac (xwin) and run on Windows CI and the PC; the Mac runs the pipe backend on real `h264_videotoolbox` clips |
| CI | the Windows runner needs the BtbN import libraries: a 77 MB pinned download, SHA-256 checked (§2.4) | no change | the same download, which also gives CI an LGPL `ffmpeg.exe` for a lossless end-to-end corpus (§2.5) |
| Build time on the 8 GB Mac | the xwin check adds bindgen over the FFmpeg headers; M0 gate 5 took 76 s for `ffcheck` including the SDK download (M0:167) | none | native Mac builds stay FFmpeg-free (`[target.'cfg(windows)'.dependencies]`, as R09:93-106 does for the engine) |
| Timing exactness | the decoder's pts for cross-checks | the CLI re-times frames: a default rawvideo decode emitted **361 frames for 360 samples**, because it duplicated the held first frame **[Verified: EXP-2]** | timing comes from Kairo's own sample tables in every backend. Decoders only return frames in presentation order, and `decode.clean` checks the counts |

**Why not ffprobe.** Three findings show that parsing the boxes directly is the more reliable check, not merely the cheaper one:

1. ffprobe reports colour from the SPS VUI even when no `colr` box exists **[Verified: EXP-6]**, so research 09's ffprobe recipe (R09:960-985) would pass a clip that has no `colr`.
2. A `-c copy` remux drops the `udta` track titles **[Verified: EXP-5]**.
3. An MP4 with no edit list and AAC priming has a lengthened first video sample (`stts` (1, 584), (359, 256) at 1/15360) **[Verified: EXP-1]**, as the cookbook predicted (CB:678). The CFR check has to understand that hold.

### 2.3 The decoder trait and its backends

```rust
pub trait Decoder {
    /// Video track `t`, frames in presentation order; the callback borrows the planes (no copies).
    fn video(&mut self, t: TrackId, f: &mut dyn FnMut(usize, &Planes<'_>) -> Result<(), Stop>) -> Result<DecodeStats, DecodeError>;
    /// Audio track `t`, mono f32 at the track's rate, every decoded sample (priming included).
    fn audio(&mut self, t: TrackId, f: &mut dyn FnMut(&[f32]) -> Result<(), Stop>) -> Result<DecodeStats, DecodeError>;
}
```

- **`linked`** (Windows). It opens the file with `format::input_with_dictionary` and `ignore_editlist=1` (ffmpeg-next `src/format/mod.rs:176`), sets frame threading with `Context::set_threading` (`src/codec/context.rs:114`), and lowers the FFmpeg log level. It reads `frame::Video::data(i)` and `stride(i)` in place. The safe API covers all of this, so the crate stays `forbid(unsafe_code)`. If a gap ever appears, the FFI goes into a `platform` module with `// SAFETY:` (CLAUDE:32).
- **`cli`** (all OSes, dev only). For video it runs `ffmpeg -v error -nostdin -ignore_editlist 1 -i F -map 0:v:0 -fps_mode passthrough -f rawvideo -pix_fmt yuv420p -`. For audio it runs `… -map 0:a:N -f f32le -ac 1 -`, one process per track, one after another. Both commands' frame and sample counts were verified to equal the sample tables **[Verified: EXP-2, EXP-3]**. It refuses any `-vf` or `-af`: it decodes only.
- **`synthetic`** (tests). It renders frames straight from `testsig` with optional distortions (block noise, ±k code quantization, bilinear scaling, a shift by a fraction of a pixel) and makes PCM from `burst()` with optional noise and delay.

### 2.4 Build, deploy and CI

**PC.**
- The bootstrap already sets `FFMPEG_DIR=C:\kairo\tools\ffmpeg` and `LIBCLANG_PATH` machine-wide (tools/pc/bootstrap.ps1:1416-1419). They also apply when builds move into the owner's session (M1:44, M1:154).
- `pc deploy` copies only `.exe`, `.pdb` and `.dll` from the build output (remote/deploy.ps1:37). It must **also copy the three DLLs from `C:\kairo\tools\ffmpeg\bin`, which is admin-only, and never from the build output**. Before copying, it checks `KAIRO-PIN.json`'s SHA-256 against the pin in bootstrap.ps1:159-161, so the DLLs always match the headers the bindings were generated from. Never rely on PATH (R09:800).
- **Prerequisite:** the PC still holds `n9.0.2-29` (M1:156). Re-pin it first, between the owner's sessions.

**Mac.**
- Run `FFMPEG_DIR=~/kairo-deps/ffmpeg-9.0-lgpl-shared cargo xwin check --target x86_64-pc-windows-msvc -p kairo-verify` (CB:600-610), followed by `cargo xwin clippy`.
- `~/kairo-deps` still holds the old pin (`FFMPEG_VERSION "n9.0.2-29-g6dfaba896f-20261020"`, read for this design). Refresh `include/` and `lib/` from the pinned zip, after checking its SHA-256, before the first check.

**CI (`ci.yml`).**
- **Windows job** (ci.yml:84-104). Before `Build`, a step reads `$FfmpegUrl` and `$FfmpegSha256` out of tools/pc/bootstrap.ps1, so CI and the PC have a single source for the pin. It downloads the month-end zip, which BtbN keeps until about 2028-09 (M1:162), checks the SHA-256, extracts it, and caches it with `actions/cache` keyed by the SHA-256. It then sets `FFMPEG_DIR`, sets `LIBCLANG_PATH` (the runner's LLVM, **[Unverified]** that `C:\Program Files\LLVM\bin\libclang.dll` exists on `windows-latest`; otherwise install a pinned LLVM), and adds `%FFMPEG_DIR%\bin` to PATH for the test step only.
- **Licence policy.** CI uploads no DLLs and no clips; PRs upload test reports only (PLAN:370).
- **Linux job.** Unchanged. `kairo-verify`'s pure parts build and test there; the testbed, display and harness crates build as stubs.
- **`cargo-deny`.** Add the per-crate exceptions deny.toml:33-34 already anticipates: WTFPL for `ffmpeg-next` and `ffmpeg-sys-next`, recorded in **ADR-0002**.

**ADR-0002** (decided by Claude Code within the plan; ADR-0001 item 3 already chose LGPL FFmpeg through these crates) records three things: the WTFPL exceptions, "own parser plus linked decode, no ffprobe", and the dev-only CLI backend. No owner approval is needed: it adds no network host, changes no house rule and adds no elevated component (docs/adr/README.md).

### 2.5 How the Mac and Linux CI test the analysis

1. **Pure units, on Mac and Linux.**
   - The `testsig` round-trips: barcode encode/decode with whitening and CRC, BT.709 bars, PQ and EETF values from a Python model.
   - The parser, on MP4s **built in the test**: a small box builder makes every variant needed (moov before or after mdat, elst, colr values, ctts, an irregular stts, titles, tkhd flags).
   - The H.264 parser, on SPS, PPS and slice headers written by an in-test bit writer (with emulation prevention).
   - Analysis on synthetic frames and PCM. The barcode must survive noise, scaling and quantization. Drop, dup and judder sequences come from `trace::replay` of vblank traces (the same table as core/trace.rs:401-438). Each A/V offset is measured within 0.1 ms. The routing matrix and the SNR are tested with added noise.
2. **End to end on Windows CI, with lossless clips generated at test time.**
   - `testgen` writes Annex B H.264 with I_PCM macroblocks from the same synthetic frames. The BtbN `ffmpeg.exe` from the CI download muxes them with WAV tracks (`-c:v copy -c:a aac -use_editlist 0 -movflags +faststart`). That is the real FFmpeg 9.0.2 muxer and native AAC, with no H.264 encoder needed and nothing GPL.
   - Each clip is 320×180, about 6 s, and about 33 MB **[Estimate]**. The linked decoder then must reproduce the pixels exactly (I_PCM is lossless). It also confirms the first-frame hold on 9.0.2 itself.
   - `kairo-verify corpus build --encoder ipcm` followed by `corpus check` runs gate 1's logic on every PR: every defect except B-frames, which needs a real encoder. CI never uploads the clips.
3. **The Mac, outside CI.** Opt-in tests behind `#[ignore]` run with `KAIRO_FFMPEG=/opt/homebrew/bin/ffmpeg cargo test -p kairo-verify -- --ignored`. They run the same lossless corpus through the CLI backend, plus the rehearsal corpus encoded with `h264_videotoolbox` (§3.3), so barcode, PSNR and tone robustness are tried against a real lossy encoder before the PC is free. R12:1156-1165 already uses VideoToolbox for Mac-side fixtures that never ship.

### 2.6 Facts this rests on

Read in the FFmpeg n9.0.2 source and Microsoft's docs for this design:

- **`scale_d3d11` sets no colour space.** The file has no `VideoProcessorSet*ColorSpace` call (ff:libavfilter/vf_scale_d3d11.c), and D3D11's default `YCbCr_Matrix` is 0, **BT.601** (Microsoft, `D3D11_VIDEO_PROCESSOR_COLOR_SPACE`, "The default state value is 0 (BT.601)"). A GPU NV12 path through `scale_d3d11` would therefore be BT.601-coded but tagged BT.709. The harness uses swscale with an explicit BT.709 matrix instead (§4.4). The bars check would catch the mistake if anyone switched to `scale_d3d11`.
- **`gfxcapture`.**
  - `max_framerate` defaults to 60 and sets `MinUpdateInterval = 1/max_framerate` (ff:libavfilter/vsrc_gfxcapture.c:44; vsrc_gfxcapture_winrt.cpp:377-378).
  - `hwnd` and `hmonitor` options exist (vsrc_gfxcapture.c:45-46).
  - It uses the filter graph's D3D11VA device if one is given, and otherwise creates one on the **default adapter** (winrt.cpp:1207-1229).
  - pts are `SystemRelativeTime` minus the first frame's (winrt.cpp:1417-1457), so the QPC epoch isn't in the file.
  - Output frames come from its own pool of 8 (winrt.cpp:1085).
  - It never calls `RequestAccessAsync`, so it raises no permission prompt.
- **`d3d11va` device selection.** The device string is a **DXGI adapter index** (`atoi`, ff:libavutil/hwcontext_d3d11va.c:644). The `vendor_id` option cannot tell the iGPU from the dGPU, because both are AMD (:646-648). FFmpeg logs `Selecting d3d11va adapter N` (:660) and `Using device vvvv:dddd (…)` (:673).
- **The CLI and h264_amf.** The CLI hands an encoder the single device of the type it needs, and logs `Using device kd (type d3d11va) with h264_amf encoder.` (ff:fftools/ffmpeg_enc.c:133-180). It finds none when two devices of that type exist (ffmpeg_hw.c:28-40). `h264_amf` derives AMF from that device (ff:libavcodec/amfenc.c:302-325) and logs `AMF via D3D11.` (ff:libavutil/hwcontext_amf.c:760). Without a device it falls back to the default adapter (amfenc.c:325). All of R13's `h264_amf` options exist in 9.0.2 (ff:amfenc_h264.c:35-126). With `bf>0` on a GPU without H.264 B-frames, it only warns and disables them (amfenc_h264.c:486-523).

---

## 3. Gate 1: good and bad clips

### 3.1 Two kinds of good clip

Gate 1 needs kairo-verify to flag every bad clip and pass the good ones (M1:26; PLAN:417). A captured clip cannot be certified good independently: `gfxcapture` "does NOT hold a stable FPS" (R08:386; M1:27), and a genuine capture hiccup must *fail* the strict profile. So:

- **Golden clips** are the reference. They are the testbed's pattern v1, encoded by the PC's pinned `ffmpeg.exe` with `h264_amf` exactly as the engine will mux (PLAN:179; CB:564-598). Their content cadence is exact by construction, their A/V offset is 0, and they carry four known audio tracks. They must pass **`test`**. These are the clips M1:187 means when it says the A/V, routing and tone checks rest on gate 1's clips.
- **Real testbed captures** come from the gate-2 harness (§4). They must pass **`recording`**, and their `test` result is reported for information. They prove the barcode, bars and cadence checks on real WGC → swscale → AMF frames ("a barcode that survives H.264", M1:131).
- **Gate-1 evidence** is one line per clip in the corpus report: 2 golden clips pass `test`; each bad clip fails its `must_fail` checks and nothing outside `must_fail ∪ may_fail`; and at least 3 harness captures pass `recording`.

### 3.2 The synthetic source (`kairo-verify synth`)

- **Content.** 30 s of **180 fps** content sampled at **60 fps**: output frame k shows content n = 3k, the same arrangement as the harness's 180 Hz testbed (§4.3). Barcode rate code 180, run tag 0. The flash is on for content [k s, k s + 50 ms), which is 3 output frames.
- **Pixels.** NV12 computed directly with the BT.709 limited matrix. Every rect edge falls on an even pixel, so 4:2:0 chroma is exact and the encoder is the only source of error.
- **Two presets.** 1920×1080 at 25/40 Mbps and 2560×1440 at 36/50 Mbps, the plan's Recommended and High (PLAN:173-174; R13:126-127). The peak never exceeds 50 Mbps (CLAUDE:31).
- **Audio tracks** (PLAN:183-189; CB:598 titles). Each is a 48 kHz WAV made from `testsig::burst`:

  | Track | Title | Content | AAC |
  |---|---|---|---|
  | a:0 | Mix (default) | game + voice + mic, summed, peak about −24 dBFS | 160 kbps stereo |
  | a:1 | Game | 1 kHz at k s | 160 kbps stereo |
  | a:2 | Mic | 3 kHz at k+0.25 s | 128 kbps mono |
  | a:3 | Voice chat | 2 kHz at k+0.5 s | 160 kbps stereo |

- **`expect.json`.** synth writes duration, rate, run tag, schedule, tracks and titles, size and fps next to the clip, for `--expect`.

### 3.3 Encoding the golden clips

**On the PC** (inside the corpus job, §3.5), with `kairo-harness corpus` supplying the adapter index from the LUID that drives the 180 Hz monitor:

```
kairo-verify synth --preset 1080p60 --seconds 30 --wav-dir . --expect good-1080p60.expect.json |
ffmpeg.exe -hide_banner -nostdin -loglevel verbose -y -init_hw_device d3d11va=kd:<index>
  -f rawvideo -pix_fmt nv12 -video_size 1920x1080 -framerate 60 -i pipe:0
  -i mix.wav -i game.wav -i mic.wav -i voice.wav -map 0:v -map 1:a -map 2:a -map 3:a -map 4:a
  -c:v h264_amf -usage transcoding -quality balanced -rc vbr_peak -b:v 25M -maxrate 40M -bufsize 40M
     -vbaq 1 -preanalysis 0 -high_motion_quality_boost_enable 0 -preencode 0 -bf 0 -max_b_frames 0
     -frame_skipping 0 -filler_data 0 -coder cabac -profile:v high -header_spacing 60 -async_depth 4 -g 60
     -color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv
  -c:a aac -b:a:0 160k -b:a:1 160k -b:a:2 128k -b:a:3 160k
  -metadata:s:a:0 title=Mix -metadata:s:a:0 handler_name=Mix  … (Game, Mic, Voice chat likewise)
  -disposition:a:0 default -disposition:a:1 0 -disposition:a:2 0 -disposition:a:3 0
  -movflags +faststart -use_editlist 0 good-1080p60.mp4
```

- **What goes in.** AMF receives system-memory NV12 (CLAUDE:31; R13:802 options, R13:805 NV12). The output names files relative to the job's working folder, so no raw paths reach the logs (CLAUDE:16).
- **Spacing.** The builder leaves ≥1.5 s between encodes and never runs two at once (M1:94).
- **Read-back.**
  - *Through the log.* The CLI cannot call `av_opt_get`, so the builder scans the verbose log. It fails on `has not been used for any stream` (an unused option only *warns*, with exit 0, while an unknown option fails with exit 8, **[Verified: EXP-8]** on 8.0.1; **[Unverified]** that 9.0.2 is the same). It fails on any B-frame capability warning. It requires `Using device kd (type d3d11va) with h264_amf encoder.`, `AMF via D3D11.`, and a `Using device vvvv:dddd` that matches the vendor and device of the adapter with that LUID.
  - *Through the bitstream.* kairo-verify itself checks what the options control: High profile, CABAC (the PPS `entropy_coding_mode_flag`), no B slices, an IDR every 60 frames, peak ≤50 Mbps over 1 s windows, and `colr` and VUI BT.709 limited.
  - The engine's real read-back through `av_opt_get` comes in M3 (CB:1190, task 6).
- **Colour tags.** **[Unverified]** whether `h264_amf` through the CLI makes movenc write `colr` (it needs primaries, transfer and matrix all set, R09:683). If it doesn't, the clip fails `video.colr`, the build stops, and Claude decides whether to finish the golden clips with `mutate set-colr 1 1 1 tv` and records that in the manifest. `h264_videotoolbox` on the Mac does lose primaries and transfer, so the Mac rehearsal always applies that finishing step **[Verified: EXP-6]**.

**On the Mac (rehearsal):** the same synth with `-c:v h264_videotoolbox -b:v 25M -maxrate 40M -g 60 -bf 0 -profile:v high`, no AMF options, and the `set-colr` finishing step. The builder accepts only the encoders `h264_amf`, `h264_mf`, `h264_videotoolbox`, `copy` (I_PCM) and `aac`, and passes no filters, so `libx264`, `libx265` and GPL-only filters can never be used, even by mistake.

### 3.4 Bad clips, one defect each

Made from `good-1080p60`, 30 s, 1,800 frames. "Synth" means the same command as the good clip with one `--defect`.

| Clip | Defect | How it's made (exact) | `must_fail` | `may_fail` | Note |
|---|---|---|---|---|---|
| `drop1` | 1 dropped frame | synth `--defect drop@870`: n_k = 3k for k < 870, then 3(k+1) | `cadence.drops_dups` | — | Content after 14.5 s runs one tick early. A/V for the later flashes becomes −16.7 ms, inside the 40 ms limit, so the A/V check still passes (single defect) |
| `dup1` | 1 duplicated frame | synth `--defect dup@870`: n_k = 3k for k < 870, then 3(k−1) | `cadence.drops_dups` | — | Mirror of `drop1`, +16.7 ms. Gate 1 names both kinds (M1:188) |
| `judder` | 90/s delivery under a 60 fps pacer on a 180 Hz panel | synth `--defect judder`: n_k = 3k − (k mod 2) → 0, 2, 6, 8, 12…, steps of 11.1 and 22.2 ms | `cadence.judder` | — | No repeats and no skips: every step rounds to 1 tick, which is exactly the case M1:189 describes |
| `av50` | audio 50 ms late | synth `--defect av-offset=+50ms`: every tone onset in every track is 2,400 samples later | `av.offset` | — | Timestamps and structure are identical to the good clip |
| `bars` | corrupted bars | synth `--defect corrupt-bars`: 1 px luma lines (Y ±48) every 16 px, both directions, **inside the bars band only**, every frame (looks like the AMF #574 DCC gridlines, R13:113) | `content.bars` | — | Barcode, flash and audio are untouched |
| `bframes` | B-frames | same synth; encoder `-bf 2 -max_b_frames 2`, mux `-movflags +faststart+negative_cts_offsets -use_editlist 0` | `video.no_reorder` | set from the first run, per encoder: e.g. `video.cfr` and `video.gop` | B-frames are an encoder defect, so they need a B-capable encoder. **[Unverified]** whether the RX 9000's AMF does H.264 B-frames. If AMF warns and disables them, the build fails loudly and tries `h264_mf -bf 2` **[Unverified]**. VideoToolbox on the Mac produced B slices, but also "Invalid DTS", an irregular `stts` and 57-frame GOPs **[Verified: EXP-7]**, hence `may_fail` |
| `nofaststart` | moov after mdat | `kairo-verify mutate undo-faststart`: rewrite as ftyp, mdat, moov, with every `stco`/`co64` entry shifted | `container.faststart` | — | Byte-exact media; only the box order and chunk offsets change |
| `editlist` | edit list present | `mutate add-elst`: an identity `edts/elst` in every trak (segment = track duration, media_time 0), with offsets fixed | `container.no_editlist` | — | Presentation is unchanged, so only the presence check fires |
| `colr` | wrong `colr` | `mutate set-colr 6 6 6 tv` (BT.601/SMPTE 170M), patched in place in the same box size; the SPS VUI is left alone | `video.colr` | — | `video.vui_matches_colr` warns, which is correct and is not a fail |
| `swapped` | Game ↔ Voice chat | `mutate swap-track-names Game "Voice chat"`: swaps the `hdlr` names and `udta/name` titles (sizes change, offsets fixed) | `audio.routing` | — | The track titled Game now holds the 2 kHz voice tone. `av.offset` still measures Mix and every track where the game beep *is* found, so it doesn't fail as collateral |

**Extras** (not gate items; they cover the remaining checks): `cue` (synth puts the cue placeholder into Mix → `audio.routing`), `noisy` (white noise at −45 dBFS on Game → `audio.tone_snr`), `nocolr` (`mutate remove-colr` → `video.colr`), `vfr` (`mutate perturb-stts` → `video.cfr`) and `gop2s` (encode `-g 120` → `video.gop`).

**Never** an over-50 Mbps AMF encode: `video.peak_bitrate` is exercised with the lossless I_PCM clips in CI, whose bitrate is huge anyway.

**Why not FFmpeg remuxes or bitstream filters for the container defects.** A plain `-c copy` remux already changed the titles **[Verified: EXP-5]**. Bitstream filters such as `noise=drop` with `setts` retiming can drop packets, but a dropped P-frame corrupts every frame until the next IDR. Making the defect at the source keeps each clip to exactly one change.

### 3.5 Manifest, `corpus check` and the gate evidence

```json
{"kairo_corpus":1,"pattern":1,"encoder":"h264_amf","ffmpeg":"n9.0.2-17-g2a571b6068","finishing":[],
 "clips":[{"file":"good-1080p60.mp4","expect":"pass","profile":"test"},
          {"file":"drop1.mp4","defect":"drop@870","must_fail":["cadence.drops_dups"],"may_fail":[]},
          {"file":"bframes.mp4","defect":"bf=2","must_fail":["video.no_reorder"],"may_fail":["video.cfr","video.gop"],
           "why":"filled in from the first PC run"}]}
```

- `kairo-verify corpus check <dir>` runs `check --profile test --expect <clip>.expect.json` on every clip and writes `corpus-report.json` plus a one-screen summary.
- It exits 0 only when every good clip passes, every bad clip fails all of its `must_fail` checks, and no clip fails a check outside `must_fail ∪ may_fail`.
- The optional `--ffprobe C:\kairo\tools\ffmpeg\bin\ffprobe.exe` compares the parser's per-packet pts, dts and key flags and the stream fields with ffprobe's JSON. Differences are warnings. This is a PC-only cross-check of the parser against FFmpeg's demuxer, and ffprobe never ships (M1:70).

### 3.6 Where clips live

- **PC:** `C:\kairo\logs\corpus-<stamp>\` and `C:\kairo\logs\harness-<stamp>\`, where everything a job writes goes (M1:72). The harness keeps the newest 2 corpus and 5 harness folders and deletes older ones that match its own pattern.
- **Mac:** `.dogfood/corpus-mac/<stamp>/` (rehearsal clips), `.dogfood/corpus-pc/<stamp>/corpus-report.json` and `.dogfood/harness/<stamp>/` (reports; a clip only with `--fetch-clip`). `.dogfood/` is gitignored (.gitignore).
- **Never committed** (CLAUDE:18; M1:119). CI needs no committed clip because it generates I_PCM clips at test time (§2.5). The one committed fixture M1 might ask about is the reduced SP2 trace (M1:195), and that is wgc-probe's question, not this design's.

### 3.7 Audio in M1: Stereo Mix

**Recommendation: don't ask to switch Stereo Mix on.** M1's captures are video-only, as M1:187 allows. The reasons:

1. It changes a Windows sound setting, which needs the owner's OK (M1:187; CLAUDE:43), and many PCs lack the device (R01:279).
2. FFmpeg's dshow audio would not share a clock with `gfxcapture`'s video. The video's pts are relative to the first frame (winrt.cpp:1457), so any A/V number would describe the CLI pipeline, not anything Kairo ships.
3. The golden clips already exercise A/V, routing, tone SNR and the cue check with exactly known answers.
4. M3's engine records process loopback with QPC timestamps, which is the real A/V test (CB:1193).

In the harness the testbed still plays its −30 dBFS beep and logs the planned and mapped times, which validates its scheduling before M3. `kairo-tone` is started only with `--voice`, from M3 on. Nothing in M1 opens the microphone (M1:15).

---

## 4. Gate 2: the unattended flow

### 4.1 Prerequisites

1. **Merged:** PRs 5 (linked decoder and DLL deploy), 6 (`kairo-display`), 7 (testbed), **8 (devrun safety stops, M1:97 and M1:129)** and 9 (harness). Without PR 8 no job may run while the owner is away. Until then, harness runs are short attended ones after the owner says in chat that the PC is free.
2. **The owner's test days**, which are still open (M1:35, M1:147). Sleep is already Never (M1:36). The PC must be signed in and unlocked, with the displays on and no RDP (M1:90).
3. **The PC's FFmpeg re-pinned** to `n9.0.2-17` (M1:156), so the DLLs, `ffmpeg.exe` and the bindings all match.
4. **A first attended AMF CLI session** (M1:130). This is also the "short recording test with AMD Software back" (M1:155). AMD Software is installed again (M1:42), so AMF opening without it (M1:186) is now moot. Instant Replay may hold its own VCN session alongside ours (R09:907, gotcha 22), so the harness records whether one was running (via `encoder-usage.ps1` once it exists, M1:80). Any TDR is a P0 (M1:204).

### 4.2 The job

`pc harness capture` writes it into `.dogfood/jobs/`, because it names a monitor (M1:73). The schema is M0's (M0:72; core/job.rs:35-53):

```json
{"id":"harness-20261025-101500",
 "cmd":["C:\\kairo\\bin\\current\\kairo-harness.exe","run",
        "--monitor-path","<180 Hz monitorDevicePath from .dogfood/pc/monitors.json>",
        "--testbed-fps","180","--mode","borderless","--capture","gfxcapture","--capture-fps","180",
        "--output","1920x1080@60","--seconds","30","--profile","recording",
        "--ffmpeg","C:\\kairo\\tools\\ffmpeg\\bin\\ffmpeg.exe","--out","harness-20261025-101500"],
 "cwd":"C:\\kairo\\logs","max_seconds":300,"abort_on_input":true,"window":"hidden"}
```

- `max_seconds` is gate 2's 5 minutes, so overrunning it is a gate failure as well as a stop.
- `abort_on_input` stays on (M1:92). Owner input stops the whole tree (devrun/run.rs:190-203), and `pc` reports exit 6.

### 4.3 What `kairo-harness run` does, with timeouts

| Phase | What | Limit | Typical **[Estimate]** |
|---|---|---|---|
| 0 setup | Prune old run folders. Require ≥2 GB free. Refuse if another `ffmpeg.exe` from `C:\kairo\tools` is running (one H.264 session at a time, M1:94). `kairo-display`: device path → rect, refresh, HMONITOR, LUID → DXGI index and vendor:device. Read the QPC epoch | 5 s | <1 s |
| 1 testbed | Start `testbed.exe --monitor-path … --mode borderless --fps 180 --pace vsync --seconds 60 --run-tag N --epoch-qpc E --log testbed.jsonl --ready ready.json --beep on` with stdin piped, then wait for `ready.json` | 15 s | 1–2 s |
| 2 warm-up | Let DWM and flip promotion settle. The testbed log must show steady presents | 3 s | 3 s |
| 3 capture | `ffmpeg.exe` (§4.4), `-t 30`. Stop it if it runs 30 s past its length | 60 s | 32 s |
| 4 close | Close the testbed's stdin and wait for its `end` line. Kill it after 5 s | 5 s | <1 s |
| 5 verify | `kairo-verify check capture.mp4 --profile recording --expect expect.json --testbed-log testbed.jsonl --json verify.json --summary verify.txt` | 120 s | 10–15 s |
| 6 report | Write `harness.json`: phases and durations, exit codes, LUID, the FFmpeg version line, the adapter, the AMF log read-back result and verify's verdict. Print a summary to stdout, which becomes the job's `out.log` | — | <1 s |

The total is about 50–60 s on the PC, plus the job start and the Mac's polling: comfortably under 300 s. The harness exits 0 whenever a report exists, whatever its verdict, because gate 2's check is the report, not a pass (M1:27). Its other exit codes are in §5.5.

### 4.4 The capture command

```
ffmpeg.exe -hide_banner -nostdin -loglevel verbose -y
  -init_hw_device d3d11va=kd:<index from the LUID> -filter_hw_device kd
  -filter_complex "gfxcapture=hwnd=<testbed hwnd>:max_framerate=180:capture_cursor=0:capture_border=0:display_border=0,
                   fps=60,hwdownload,format=bgra,
                   scale=w=1920:h=1080:flags=bicubic+accurate_rnd+full_chroma_int:in_range=pc:out_range=tv:out_color_matrix=bt709,
                   format=nv12[v]"
  -map "[v]" -t 30
  -c:v h264_amf <the §3.3 option list> -b:v 25M -maxrate 40M -bufsize 40M -g 60
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv
  -movflags +faststart -use_editlist 0 capture.mp4
```

- **Window capture by `hwnd`**, not by title regex, so FFmpeg enumerates no other windows' titles (CLAUDE:16), and not by monitor, so no notification or other window can ever end up in a clip. `ddagrab=output_idx=<output of that monitor on that adapter>:framerate=60:draw_mouse=0` (R01:244-246) is only a fallback, because it records the whole monitor.
- **`max_framerate=180`** gives `MinUpdateInterval` 5.56 ms, so every vblank's frame is eligible (winrt.cpp:377). `fps=60` picks frames on the GPU *before* `hwdownload`, so only 60 frames/s are read back. `gfxcapture`'s pool of 8 means the fps filter never starves WGC (winrt.cpp:1085).
- **swscale with an explicit BT.709 limited matrix**, not `scale_d3d11` (§2.6). AMF therefore gets correctly coded NV12 (CLAUDE:31), and the clip's `colr` and VUI tell the truth.
- **One D3D11VA device, created on the LUID-matched index**, serves both the filter (`-filter_hw_device`) and, through ffmpeg_enc.c's lookup, the encoder. A second d3d11va device would make that lookup return nothing (ffmpeg_hw.c:28-40). The harness fails with exit 4 unless the log shows:
  - `Selecting d3d11va adapter <index>`;
  - `Using device vvvv:dddd` equal to the vendor:device of the adapter with that LUID;
  - `Using device kd (type d3d11va) with h264_amf encoder.`;
  - `AMF via D3D11.`.

  This answers "which adapter `gfxcapture` and `h264_amf` pick" (M1:186) on every run. `Failed setting border drawing mode` (winrt.cpp:369-371) is on the harmless-warnings allowlist; an unused-option warning is fatal (§3.3).
- **Expected result.** Content steps of 3 testbed frames. A ±1-frame blip appears where the 60.000 Hz output grid crosses the panel's real rate, about one per 25–80 s **[Estimate]**: well under 1% of steps, so the clip passes `recording`. Where `kairo-verify` sees a content gap that the testbed log shows was never displayed, it counts a *source* miss, not a capture drop (`source.stability`).
- **Optional extra run** (not a gate item): `--capture-fps 90` makes the judder M1:188 describes with real WGC, and it must fail `cadence.judder`.

### 4.5 Outputs

`C:\kairo\logs\harness-<stamp>\` holds `harness.json`, `ready.json`, `testbed.jsonl`, `expect.json`, `capture.mp4`, `ffmpeg.log`, `verify.json` and `verify.txt`. devrun's own `<id>.result.json`, `<id>.out.log` and `<id>.err.log` sit next to the folder (M0:73).

- `ffmpeg.log` names the GPU model, so it is a PC detail: it stays on the PC and in `.dogfood/`.
- `harness.json` and the verify report carry hardware IDs or nothing. The gate report uses the hardware class only (M1:209-211).
- These file names go into M1's Files table as they land (M1:72).

### 4.6 `tools/mac/pc harness`

The verb is new (M1:86). Like the other verbs, it sends its PowerShell on stdin.

```
pc harness capture [--monitor 180hz] [--seconds 30] [--testbed-fps 180] [--mode borderless|windowed|exclusive]
                   [--capture gfxcapture|ddagrab] [--capture-fps 180] [--output 1080p60|1440p60]
                   [--smoke] [--fetch-clip] [--no-wait] [--force]
pc harness corpus  [--presets 1080p60,1440p60] [--seconds 30] [--force]
pc harness displays
```

1. **Aliases.** It maps the monitor alias to a device path from `.dogfood/pc/monitors.json`, which `displays` fills from `displays.json` (M1:38). In M1 it refuses `--monitor oled` for `capture` and `corpus`: patterns and harness runs belong on the 180 Hz monitor (M1:93; CLAUDE:26).
2. **The job.** It writes the job file into `.dogfood/jobs/` and reuses `cmd_run`'s path (pc:641; remote/run.ps1), including the busy and unknown refusals and the wait of `max_seconds` + 30 s.
3. **Fetching.** It runs a new read-only `remote/harness-fetch.ps1`, which prints `harness.json`, `verify.json` and `verify.txt` (or `corpus-report.json`) as Base64 between markers, into `.dogfood/harness/<stamp>/`. That avoids scp's unverified `C:/` paths (M1:159). The clip is fetched only with `--fetch-clip`.
4. **TDRs.** It reads System events 4101 (and 41) since the job's `started_utc`, which is read-only. If any appear, it prints a P0 and exits 7 (M1:204).
5. **Output.** It prints the verdict, the failing checks and **start → report in seconds**, from `harness.json`, which is gate 2's timing.

- **Exit codes:** the same as `pc run` (pc:317-319), that is 0 ok, 1 failed, 2 usage, 3 or 4 refused, 5 still running and 6 stopped, plus **7 for a TDR seen during the run**.
- **`--smoke`** is a 10 s capture. It is the "green harness smoke" media PRs need (CLAUDE:34; M1:194).

### 4.7 Failures and P0s

| Failure | Result |
|---|---|
| Owner input or a game start | devrun stops the tree. The result's reason is `input`, or PR 8's new reason; exit 6. Partial outputs stay; an MP4 without a moov is simply invalid |
| AMF hang (no output) | Phase 3's 60 s limit kills `ffmpeg.exe`, and the job's `max_seconds` backs it up. Exit 1. The TDR check runs, and a TDR is a P0: stop and report (M1:204) |
| Adapter mismatch or an unused-option warning | Exit 4. No clip is verified |
| Testbed misses more than 0.1% of vblanks | `source.stability` fails. The report says to rerun, and the clip isn't blamed |
| AMF won't open | Exit 1, then the plan's fallback, `h264_mf`, which "takes the first hardware encoder with no adapter filter, so Claude checks which one" (M1:205) |

---

## 5. kairo-verify: CLI, checks, profiles, report, exit codes, performance

### 5.1 CLI

```
kairo-verify check <clip.mp4> [--profile test|recording|engine|discord|whatsapp|reels|share|original]
        [--expect FILE] [--testbed-log FILE] [--voice-log FILE] [--voice-included]
        [--decoder linked|cli|none] [--ffmpeg PATH] [--threads N]
        [--json FILE|-] [--summary FILE|-] [--quiet]
kairo-verify synth  --preset 1080p60|1440p60|WxH@fps [--seconds N] [--defect …] --wav-dir DIR --expect FILE   (NV12 on stdout)
kairo-verify ipcm   …same as synth, but writes Annex B H.264 (I_PCM) for the lossless path
kairo-verify mutate undo-faststart|add-elst|set-colr P T M R|remove-colr|swap-track-names A B|perturb-stts IN OUT
kairo-verify corpus build --ffmpeg PATH --encoder h264_amf|h264_mf|h264_videotoolbox|ipcm [--hw-device-args …]
        [--presets …] [--seconds N] --out DIR
kairo-verify corpus check DIR [--ffprobe PATH]
kairo-verify boxes <clip.mp4>          (box tree with file name only, a port of prototypes/scripts/mp4boxes.py)
```

- **Default decoders.** `linked` on Windows. On other OSes, `cli` when `--ffmpeg` is given, otherwise `none`.
- **`--decoder none`** runs the structure checks only, in milliseconds. That is the M3 post-save mode. Content checks then report `skip (no decoder)`, which is a verdict `error` for a profile that requires them.
- **`--expect` is required for `test`.** For `engine` the profile fixes the expectations (30 s ± 1 GOP; Mix, Game and Mic; PLAN:446), and flags override them.

### 5.2 Checks

The list follows PLAN:351, M1:12 and M3's timed gate (PLAN:446), plus CB:680-689. "Required" means the profile needs it for a pass.

| Id | What it checks | Source |
|---|---|---|
| `container.faststart` | ftyp, then the moov before the first mdat | PLAN:179; R09:650-657 |
| `container.no_editlist` | no `edts` or `elst` in any trak | PLAN:351; R09:586-598 |
| `video.codec` | H.264 High, 8-bit 4:2:0, frame_mbs_only, CABAC (PPS), the expected size | R13:801-802; CB:684 |
| `video.colr` | a `colr` box of type nclx with the expected primaries, transfer, matrix and range (SDR: 1/1/1, limited) | PLAN:351; R09:681-687 |
| `video.vui_matches_colr` | SPS VUI = `colr` (warning) | EXP-6 |
| `video.cfr` | every sample lasts one tick at 30, 60 or 120 fps, except the first, which may run up to 2,047 samples (42.7 ms) long, and the last, which may be shorter. Reports `first_frame_hold_ms` | PLAN:179; core/cut.rs:7-12; EXP-1 |
| `video.idr_start` | sample 0 is a sync sample, NAL type 5, presented at 0 | PLAN:351 |
| `video.gop` | every sync sample is an IDR, one every `fps` frames (the last GOP may be shorter); closed | PLAN:351; R13:804 |
| `video.no_reorder` | no B slices; no `ctts`, or only zero offsets; SPS `max_num_reorder_frames` = 0 when present | PLAN:351; R09:586-598 |
| `video.peak_bitrate` | the largest 1 s sliding-window bitrate from `stsz` ≤ the cap | CLAUDE:31; R13:139 |
| `audio.format` | AAC-LC, 48 kHz, the channel count each role expects, frames of 1,024 samples | PLAN:191; R09:702-714 |
| `tracks.layout` | track count, roles and titles (`udta/name`, `hdlr`), only Mix enabled (default), alternate group | PLAN:179 (P9); CB:50, CB:598 |
| `duration` | per profile (below) | PLAN:446 |
| `decode.clean` | no decode errors; frames = samples; audio samples consistent with `stts` | R09:981 |
| `content.barcode` | share of frames whose CRC is valid and whose two copies agree; torn and unreadable frames listed; run tag matches the testbed log | M1:11 |
| `cadence.drops_dups` | repeats and skips from `cadence::steps`, **each listed with its frame number and content numbers** | M1:188; core/cadence.rs:11-13 |
| `cadence.judder` | share of judder steps, plus p99 deviation for SP2's metric (§0 item 8) | M1:189 |
| `content.bars` | PSNR of Y and CbCr over the interiors of the bars, at the position given by each frame's content number (margins u/4 to the sides, u/2 top and bottom) | PLAN:351; M1:185 |
| `av.offset` | per track that carries the game beep (always Mix): flash onset time from the container timeline, corrected by the barcode's content time (δ = (n_f − n_b)/rate), minus the beep onset from the matched filter. Median and max. With `--testbed-log`, the testbed's own per-beep offset is subtracted | PLAN:351; M1:189; R10:808 |
| `av.track_start` | \|audio track start − video track start\| < 22 ms | R09:979; M1:189 |
| `audio.routing` | the tone-presence matrix: present if SNR ≥ 20 dB, absent if ≤ 6 dB, inconclusive in between, which fails | PLAN:351; PLAN:496; PLAN:523 |
| `audio.tone_snr` | tone power in its burst window against the residual | PLAN:351 |
| `hdr.curve_levels` | decoded Y of each HDR patch against the chosen curve's expected code ±2 | PLAN:430 (SP3); M1:192 |
| `source.stability` | the testbed's own missed vblanks, from its log | §4.4 |
| `export.size`, `export.metadata` | size caps; title as the only tag in share copies | PLAN:243-249 |

**Timing model (shared by all checks).**
- *Video:* each sample's presentation time comes from `stts` and `ctts`, plus a single-entry `elst` when present.
- *Audio:* sample s of the decoded track, priming included, plays at `(first_dts + s)/48 kHz`, minus the `elst` media_time if there is one. FFmpeg keeps the 1,024 priming samples without an `elst` and drops them with one **[Verified: EXP-3]**.
- With these rules the flash-to-beep offset of a correct no-elst clip measures **0.08 ms** **[Verified: EXP-4]**.

### 5.3 Profiles and thresholds

| Check | `test` (gate 1, synthetic) | `recording` (captures, SP2) | `engine` (M3 saves) | exports: `discord` · `whatsapp` · `reels` · `share` · `original` |
|---|---|---|---|---|
| faststart, no_editlist | required | required | required | required (`original`: warn, since quick trim may use an elst inside Kairo, PLAN:237) |
| codec, colr, idr_start, gop, no_reorder | required | required | required | required; `reels`: closed GOP (PLAN:245) |
| cfr | expected fps; hold ≤ 42.7 ms | same | 60 fps; hold ≤ 42.7 ms | ≤60 fps (`reels`), ≤30 fps (`whatsapp`) |
| peak_bitrate | ≤ 50 Mbps | ≤ 50 Mbps | ≤ 50 Mbps | `reels` ≤ 25 Mbps; `discord` judged by size |
| tracks.layout | from `--expect` | video-only allowed (M1) | Mix (default only), Game, Mic [+ Voice chat] | one audio track, Mix (except `original`) |
| duration | expect ± 1 frame | requested ± 1 s | 30 s ± 1 GOP (PLAN:446) | source ± 1 frame |
| content.barcode | 100% | ≥ 99% | ≥ 99% | ≥ 99% when in frame |
| drops_dups | **0 and 0** (M1:188) | ≤ 0.5% (PLAN:351) | ≤ 0.5%, longest hold ≤ 2, max step ≤ 2 (core/cadence.rs:13-14) | ≤ 0.5% |
| judder | ≤ 1% judder steps | ≤ 1% | ≤ 1% | ≤ 1% |
| bars PSNR | ≥ 40 dB on every frame | ≥ 40 dB on ≥ 99% of frames | ≥ 40 dB on every frame | ≥ 30 dB **[proposed]** |
| av.offset | median and max ≤ 40 ms (PLAN:351) | ≤ 40 ms; skip without audio | median ≤ 10 ms (warn above), **max ≤ 21.3 ms** (CB:1193) | ≤ 40 ms |
| av.track_start | < 22 ms | < 22 ms | < 22 ms | < 22 ms |
| routing | exact matrix from `--expect` | skip without audio | game on Mix and Game, none on Mic, voice per settings, no cue | voice absent unless `--voice-included` (PLAN:496); no cue |
| tone_snr | ≥ 30 dB **[proposed]** | skip | ≥ 30 dB **[proposed]** | ≥ 25 dB **[proposed]** |
| export.size | — | — | — | `discord` ≤ 19,000,000 bytes (PLAN:243); `reels` ≤ 300 MB, ≤ 15 min (PLAN:245) |

- **Sources.** Every threshold lives in `profile.rs` with its source string, which the report repeats. **[proposed]** values are marked in the report and settle in SP2, M3 or M7.
- **Judder.** ¼ tick is 4.17 ms at 60 fps. It flags 90/s on 180 Hz (100% of steps), 180/s at 360 Hz under 120 fps, and 175 Hz panels (about 8% of steps). It passes 180/s at 180 Hz and 120/s at 360 Hz. These cases match core/trace.rs:401-451.

### 5.4 Report

```json
{"kairo_verify":1,"tool":"0.1.0","pattern":1,
 "decoder":{"kind":"linked","ffmpeg":"n9.0.2-17-g2a571b6068","license":"LGPL version 3 or later"},
 "file":{"name":"capture.mp4","bytes":94211873,"sha256":"…"},
 "profile":"recording","verdict":"pass",
 "summary":{"video":"h264 High 1920x1080 60 fps CFR (first frame +0.0 ms), 1800 frames, peak 38.4 Mbps",
            "audio":"none","content":"1800/1800 barcodes, 0 drops, 0 dups, judder 0.11%, bars min 44.2 dB"},
 "checks":[
  {"id":"container.faststart","status":"pass","value":{"order":["ftyp","moov","free","mdat"]}},
  {"id":"cadence.drops_dups","status":"pass","value":{"dropped":0,"duplicated":0,"share":0.0},
   "limit":{"max_share":0.005,"source":"PLAN §3.2 [proposed]"},"events":[]},
  {"id":"cadence.judder","status":"pass","value":{"judder_steps":2,"share":0.0011,"p99_dev_ms":0.0},
   "limit":{"max_share":0.01,"source":"kairo-verify rule (M1 unverified item, judder)"},
   "events":[{"frame":1203,"content_from":3606,"content_to":3608}]},
  {"id":"av.offset","status":"skip","reason":"no audio track"}],
 "timings_ms":{"parse":9,"decode":6100,"analysis":900}}
```

The text summary is one screen:

```
kairo-verify 0.1.0 · capture.mp4 · recording · PASS (22 pass, 0 fail, 1 warn, 4 skip)
  container  ftyp/moov/mdat · no elst · colr nclx 1/1/1 tv
  video      h264 High 1920x1080 · CFR 60 · IDR at 0 · GOP 60 closed · pts=dts · CABAC · peak 38.4 Mbps
  content    1800/1800 barcodes · 0 drops · 0 dups · judder 0.11% (limit 1%) · bars ≥44.2 dB
  audio      none (video-only capture)
```

Statuses are `pass`, `fail`, `warn`, `skip` and `error`. The verdict is `fail` if any required check fails, `error` if a required check couldn't run, and `pass` otherwise.

### 5.5 Exit codes

| Program | Codes |
|---|---|
| `kairo-verify check` | **0** pass (warnings allowed) · **1** fail · **2** usage · **3** cannot evaluate (not an MP4, fragmented, no H.264 track, unreadable, decoder missing for a required check) · **4** internal error |
| `kairo-verify corpus check` | 0 when every clip behaves as its manifest says, 1 otherwise, 2–4 as above |
| `kairo-harness` | 0 a report was written (any verdict) · 1 a component failed or timed out · 2 usage · 3 the environment refused (monitor not found, OLED requested, disk < 2 GB, another ffmpeg running) · 4 encoder read-back failed (adapter mismatch, unused option, B-frame warning) · 5 internal |
| `pc harness` | the `pc run` set (pc:317-319) plus 7 for a TDR during the run |

### 5.6 Privacy

- **Names.** The report holds the file *name* only, never a directory. Errors are written without paths: `io::Error`'s Display adds none, and kairo-verify adds none.
- **No PC details.** No monitor, adapter or user names, no device paths, no wall-clock time, no refresh rate.
- **No media content.** No pixels, thumbnails, waveforms or samples; statistics only (CLAUDE:16; M1:96).
- **The harness.** It runs `ffmpeg` with relative file names in the run folder, so FFmpeg's logs hold no paths either. The device path appears only in the job file and in devrun.log, which logs the job's cmd (devrun/run.rs:96-100). Both are local and never committed.

### 5.7 Performance

- **Budget.** A 30 s 1440p60 clip should take **≤ 15 s** on the PC **[Estimate]**, well inside gate 2's 300 s.
  - Parsing takes milliseconds: it reads the sample tables and only the first 32 bytes of each video sample for the NAL and slice header.
  - H.264 decoding with frame threading (`threads = min(cores, 16)`) runs at about 300–600 fps at 1440p, so 3–6 s.
  - Analysis takes about 0.3 ms per frame: 48 block means for each barcode copy, one flash mean, and bars PSNR on interior pixels sampled every second row and column.
  - AAC decoding for 4 tracks is under 1 s.
- **Memory.** Streaming throughout: each frame is analysed in place in the decoder's buffer (no copy) and dropped. Per-frame results are about 64 bytes. Memory stays flat even for M13's 60-minute recordings (PLAN:544).
- **Exactness.** The loop filter is never skipped, because PSNR needs exact decoded pixels.
- **The Mac (CLI pipe, dev only).** A 1080p 30 s clip pipes about 5.6 GB of yuv420p. That is acceptable for development **[Estimate: 30–60 s on the M2]**.

---

## 6. PR plan

The PRs are in order, each testable on its own. Sizes are lines of code plus tests **[Estimate]**. Branches follow `m1/<slug>` (CLAUDE:44). Every PR updates M1's contract tables when it introduces a name (M1:50, M1:72, M1:84) and adds a CHANGELOG entry.

| # | Branch | Scope | Size | Tested on | PC and owner |
|---|---|---|---|---|---|
| 1 | `m1/testsig` | kairo-core: `testsig` (pattern v1, barcode codec with whitening and CRC, BT.709 bars, schedules, `burst`, PQ and BT.2390), `testbed_log`, `cadence::steps`. Python-model expected values | S (~900) | Mac, Linux CI | none |
| 2 | `m1/verify-structure` | `kairo-verify` crate: CLI, MP4 parser (boxes, tables, timeline, meta), H.264 SPS/PPS/slice parser, AAC config, every structure check, profiles, JSON and text report, exit codes, `boxes`. Tests on MP4s built in the test and on bit-writer fixtures | L (~2,200) | Mac, Linux CI | none |
| 3 | `m1/verify-mutate` | Box serializer with chunk-offset fixing; `mutate` operations; tests that each mutation flips exactly its own check | M (~700) | Mac, Linux CI | none |
| 4 | `m1/verify-content` | Decoder trait; the synthetic and CLI-pipe backends; barcode, flash, bars, tones, cadence, A/V, routing, SNR and HDR analysis with their checks; `synth` with every defect; I_PCM `ipcm` testgen; `corpus build/check`. **A Mac rehearsal of gate 1** with `h264_videotoolbox` (clips in `.dogfood/corpus-mac/`) | L (~2,600) | Mac, Linux CI; Mac `--ignored` with Homebrew ffmpeg | none |
| 5 | `m1/verify-ffmpeg` | The linked backend (Windows) with its licence guard; **ADR-0002** and deny.toml's WTFPL exceptions; CI's Windows job gets the pinned BtbN libraries and runs the lossless corpus on every PR; the Mac's `cargo xwin check` (**settles M1:181**); `pc deploy` copies 3 DLLs from the admin-only tools folder after a pin check; `corpus check --ffprobe` | M (~900, plus CI and ps1) | Mac (xwin), Windows CI; PC smoke: a hidden `\Kairo\Run` job of the lossless corpus | Claude re-pins the PC's FFmpeg first (M1:156). The smoke runs when the owner has said the PC is free (M1:97) |
| 6 | `m1/display` | `kairo-display`; `kairo-harness displays`; `pc harness displays` → `.dogfood/pc/displays.json` and `monitors.json` aliases | M (~700) | Mac (pure parts), Windows CI; PC: one hidden job | owner home, PC free (M1:97) |
| 7 | `m1/testbed` | `testbed.exe` (modes, vsync and timer pacing, ClearView renderer, HDR and grey patches, QPC log, ready file, 1 kHz beep) and `kairo-tone.exe beeps` | L (~2,200) | Mac (pure pacing and log), Windows CI build; PC attended | Short visible runs on the 180 Hz monitor: modes, 60/90/180 vsync and 240/360 timer, log sanity. Beeps at −30 dBFS, so tell the owner. Settles M1:131 together with PR 9 |
| 8 | `m1/devrun-safety` | M1's own deliverable (M1:97, M1:129): stop within 2 s on a game start or another fullscreen window, with the job's own processes exempt (foreground PID in the job object); keep the PC awake with the displays on (`SetThreadExecutionState`, PLAN:334); the OLED cover (capture-excluded, by device path); restore displays and audio | M | Windows CI; PC attended | Owner present to see the cover and the stops. Covers M1:159's fullscreen cases with the testbed |
| 9 | `m1/harness` | `kairo-harness run/corpus`, the AMF log read-back, pruning; `pc harness` with `harness-fetch.ps1` and the TDR check; the `--smoke` preset | M (~1,500) | Windows CI build; PC | **First attended AMF CLI session** (M1:130, M1:155; risk M1:204). Then **gate 1** on the PC (`pc harness corpus`) and attended captures. **Gate 2** unattended only after PR 8 and the owner's test days (M1:35) |

**Running the gates** (no new code):
- **Gate 1.** `pc harness corpus` produces `corpus-report.json`: the 2 golden clips pass `test`, and each of the 10 bad clips fails its `must_fail` checks with no unexpected failure. Add ≥3 attended harness captures that pass `recording`.
- **Gate 2.** On an agreed test day, with no one at the keyboard, `pc harness capture` produces a report, timed start → report.

**Timing against the exit-rule clock.** PRs 1–9 are testbed, verify and harness work, outside the clock (M1:7, M1:104). `wgc-probe` reuses PRs 6 and 7. Starting it after them keeps counted days from being spent on testbed plumbing; its first commit starts the clock (M1:103).

---

## 7. Open items, [Unverified] and owner questions

**[Unverified] items, each settled by the first run named:**
1. AMF H.264 B-frames on the RX 9000 card, needed for `bframes`, and whether `h264_mf -bf 2` produces B-frames. Settled by the first PC corpus.
2. Whether a CLI `h264_amf` encode makes movenc write `colr`. Settled by the first PC corpus; if not, the documented `set-colr` finishing step (§3.3).
3. Whether 9.0.2's CLI still only warns about unused encoder options. The harness treats the warning as fatal either way.
4. LLVM and `libclang` on `windows-latest`. Settled by PR 5's CI run, with a pinned install as the fallback.
5. Whether DXGI orders adapters differently per process (per-app GPU preference). The harness's `Using device` check catches it on every run.
6. `ClearView` with rects on the AMD driver (fallback: a quad shader), and `GetFrameStatistics` for a flip-model borderless window. Settled by PR 7's attended run.
7. WGC delivering about 180/s at `max_framerate=180` on the 180 Hz monitor, and how often the beat blips. Settled by PR 9's attended captures; the `recording` thresholds already allow them.
8. The PSNR AMF reaches on the bars at 25 Mbps 1080p **[Estimate: >45 dB on flat interiors]**. The first PC corpus measures it; if it fell below 40 dB, Claude would revisit the margins, not the threshold.
9. The testbed's beep scheduling through `IAudioClock` **[Estimate: ±1 ms]**. Logged on every run, and it matters from M3.

**Owner questions** (each with a recommendation):
- **Test days** (M1:35). Gate 2's unattended run needs them. Recommendation, as M1 already says: a few fixed weekday windows.
- **Stereo Mix** (M1:187). Recommendation: leave it off, so nothing changes in Windows' sound settings (§3.7). Nothing is needed from the owner unless they disagree.
- **First AMF CLI session** (M1:130). Recommendation: an attended 5-minute window: the corpus's 8 short encodes, 1.5 s apart.
- **Committed clips.** None needed: CI generates its clips (§2.5). M1:119's question can be closed as "not needed".

---

## Appendix A. Experiments run for this design

All ran on the Mac with Homebrew FFmpeg 8.0.1, using only `h264_videotoolbox`, native `aac`, the mov muxer and demuxer and the native decoders. No GPL-only encoder or filter was used. Scripts are in the Mac's scratch folder (`exp-scripts/`, not in the repo) (`gen.py`, `boxes.py`, `av.py`, `barcode_exp.py`); outputs are in `…/verify-design/exp-out/`. They are scratch files, not product code. The raw NV12 and grey dumps (about 1.1 GB) were deleted afterwards to free disk on the Mac; the scripts recreate them. The encoded MP4s are kept.

| # | What | Result |
|---|---|---|
| EXP-1 | 6 s, 640×360, 60 fps NV12 plus a WAV, encoded with videotoolbox and AAC using `-movflags +faststart -use_editlist 0` | no `elst`; video `stts` = (1 × 584), (359 × 256) at 1/15360, so the first sample is lengthened by 328 ticks = 21.3 ms (the AAC priming), as CB:678 predicted; audio starts at 0 |
| EXP-2 | Decoding that clip to rawvideo through the CLI | the default gave **361** frames (the held first frame was duplicated); `-ignore_editlist 1 -fps_mode passthrough` gave 360 = samples |
| EXP-3 | Decoding AAC to f32 through the CLI | without an elst, 289,792 samples = 283 × 1,024, priming included; with an elst (media_time 1,024), 288,768, priming dropped |
| EXP-4 | Flash vs beep with times from the container (EXP-1 rules) | offset +0.08 ms on every flash; the barcode decoded monotonically across all 360 frames |
| EXP-5 | A plain `-c copy -movflags +faststart -use_editlist 0` remux | the `udta/name` track titles were lost (`hdlr` names kept) |
| EXP-6 | `colr` with videotoolbox and the `-color_*` options | no `colr` written, because primaries and transfer were "unknown"; after an `h264_metadata` remux ffprobe reported bt709 for all three (from the VUI) while `colr` was **still absent** |
| EXP-7 | videotoolbox with `-bf 2` and `+negative_cts_offsets` | B slices (264 B / 89 P / 7 I), `ctts` v1 with negative entries, but "Invalid DTS … replacing by guess", 265 `stts` entries and a GOP of 57 |
| EXP-8 | CLI option handling | an option that belongs to another encoder (`-usage` on videotoolbox) only warns ("has not been used for any stream"), exit 0; an unknown option fails, exit 8 |
| EXP-9 | Barcode, 8×6 blocks of H/27 with random payloads, plus moving bars, 300 frames | 0 bit errors at 1080p/25M, 720p/4.6M, 540p/2M and a 9:16 crop scaled to 1080×1920 at 8M; the worst block was 0.494 of the span from the threshold (decode needs ≥0.25) |

**Source read** (downloaded, read-only, into `…/verify-design/dl-ffsrc/` and `…/dl-crates/`):
- FFmpeg n9.0.2: `vf_scale_d3d11.c`, `vsrc_gfxcapture.c`, `vsrc_gfxcapture_winrt.cpp`, `hwcontext_d3d11va.c`, `hwcontext_amf.c`, `amfenc.c`, `amfenc_h264.c`, `fftools/ffmpeg_enc.c`, `fftools/ffmpeg_hw.c`, `movenc.c`.
- Microsoft's `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` and `VideoProcessorSet*ColorSpace` pages (MicrosoftDocs/sdk-api).
- The crates `ffmpeg-next` 9.0.0 and `wasapi` 0.26.0.
