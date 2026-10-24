# Requirements digest: testbed, voice-tone process, kairo-verify (M1)

*2026-10-24 · Design input only; no repo code was written or changed. Repo `main` moved from 3775e7c to **21f0770** (#16, docs only: the owner answered six M1 questions) during this session, so all citations use 21f0770's line numbers. PLAN lines up to 656 are the same in both; M1.md lines from 153 on moved by +3.*

## How to read this

**Citation keys.** Each key is a file and a line number.

| Key | File |
|---|---|
| `M1:n` | `docs/milestones/M1.md` |
| `M0:n` | `docs/milestones/M0.md` |
| `PLAN:n` | `docs/PLAN.md` |
| `CL:n` | `CLAUDE.md` |
| `CB:n` | `docs/research/15-implementation-cookbook.md` |
| `R00`, `R05`, `R06`, `R08`, `R09`, `R10`, `R12`, `R13` `:n` | `docs/research/NN-*.md` |
| `ADR1:n` | `docs/adr/0001-architecture-and-stack.md` |
| `core/<file>:n` | `crates/kairo-core/src/<file>` |
| `ci:n`, `deny:n`, `Cargo:n` | `.github/workflows/ci.yml`, `deny.toml`, the workspace `Cargo.toml` |
| `boxes.py:n` | `docs/research/prototypes/scripts/mp4boxes.py` |
| `FF9:<file>:n` | FFmpeg n9.0.2 source, read-only from GitHub tag `n9.0.2` (the PC runs BtbN's `n9.0.2-17`); copies are in `verify-design/dl-ffdocs/` |
| `EXP-A` to `EXP-E` | Mac experiments run for this digest (§12). Native `aac` and `h264_videotoolbox` only; no GPL filter; nothing was linked |

**Tags.**
- **[Rec]** is my recommendation for a question the sources leave open; it comes with its reasons.
- **[Conflict]** marks sources that disagree.
- **[Gap]** marks something no source specifies.
- **[Unverified]** has not been checked; on the PC unless the item says otherwise.
- **[Mac exp]** was measured on the Mac with stand-in encoders. AMF behaviour may differ.

---

## 0. Summary

- **Three instruments, all outside the exit-rule clock.** M1:7 and M1:104 put `testbed.exe` (crate `kairo-testbed`, M1:54), the voice-tone process (name: Claude's call, M1:55) and `kairo-verify` (M1:56) outside the clock. They serve gate 1 (M1:26), gate 2 (M1:27) and gate 3 (M1:28). They are reused by every later media gate: M3's timed gate, M5's harness 500/500, M7's export checks, M8, M11 Win10, M13 and "Test my setup" (§8).
- **`kairo-verify`'s check list** (M1:12, PLAN:351):
  - drops and duplicates ≤0.5% [proposed];
  - content cadence; CFR;
  - A/V offset per track ≤40 ms;
  - IDR at 0; closed 1 s GOP; pts = dts;
  - moov first; no edit list; `colr`;
  - decoded bars ≥40 dB PSNR;
  - routing (voice tone only on T4; Kairo's cue absent);
  - tone SNR; curve levels; export profiles;
  - nothing GPL linked.

  Its own analysis must work without `ffprobe.exe` (M1:56).
- **Five thresholds conflict** (§4.3):
  - drops and duplicates: 0.5% vs 1% vs a strict single-frame rule for gate 1;
  - A/V: 40 ms vs <10 ms with a ≤21.3 ms gate vs a <22 ms track-start check;
  - judder: SP2's "p99 ≤ one refresh period" sits exactly on the 90/s-at-180 Hz judder pattern;
  - tone SNR: research 10's −50 dB alarm was set for a −20 dBFS PCM tone, but AAC caps a −30 dBFS tone at ≈49 dB [Mac exp];
  - routing: "only on T4" vs voice in the mix, plan B and Win10.

  [Rec] Set thresholds per profile (§4.5), as M1:189 recommends.
- **Gate 1** lists 9 defects (M1:26). [Rec] Make every gate-1 clip synthetic: render the testbed's own pattern, encode it on the Mac with `h264_videotoolbox` (the stand-in for AMF), and inject one defect per copy (M1:188). Build the judder clip by replaying an ideal 90/s-at-180 Hz (and 72/s-at-360 Hz) delivery through `kairo-core`'s pacer. This keeps gate 1 Mac-only, as M1:119 lists it.
- **Gate 2** is one unattended `\Kairo\Run` job: testbed on the 180 Hz monitor, FFmpeg CLI capture into `h264_amf`, then `kairo-verify`, start to report in under 5 minutes. The report is the check, not a pass (M1:27). My budget estimate is ≈1–1.5 min (§6.5). [Rec]
  - Gate 2 is video-only. The CLI cannot record playback without "Stereo Mix" (M1:187), so don't switch on a Windows sound setting.
  - Capture with `gfxcapture` in window mode, pinned to the monitor's adapter.
  - Run `kairo-verify` inside the job, in the owner's session, not in the elevated SSH session.
- **Key findings from the Mac experiments** (§12):
  - A 56-bit block barcode with 16–32 px blocks at 1440p read 100% through 1080p, 720p, 540p and a 9:16 crop, down to 1.2 Mbps.
  - FFmpeg's CLI writes `colr` only when the frames carry colour (`setparams`). `-color_*` output options alone wrote none.
  - The default MP4 mux writes an `elst` unless `-use_editlist 0` is set.
  - Kairo's P8 layout makes the first video sample longer. Decoding to rawvideo without `-fps_mode passthrough` invents 3 duplicate frames.
  - PSNR ≥40 dB missed every simulated gridline corruption; a row/column-mean error check caught the strong ones.
- **[Rec] Decode backend for M1.** `kairo-verify` decodes through the pinned LGPL `ffmpeg.exe` as a subprocess, which ships for exports (PLAN:239, PLAN:555), and parses MP4 boxes itself. It links no FFmpeg in M1: no WTFPL exception, no FFmpeg in CI, no `unsafe`. M3+ adds an in-process decoder behind the same trait for "Test my setup". As a result, M1:181 (`cargo xwin check` with `ffmpeg-sys-next`) stays open until M3.

---

## 1. Rules that bind all three instruments

| Rule | Requirement | Source |
|---|---|---|
| Clock | Testbed and `kairo-verify` days don't count toward the exit rule. The voice-tone process isn't named there; [Rec] treat it as testbed code | M1:7, M1:104 |
| Crates and names | `testbed.exe` in `kairo-testbed`; `kairo-verify` in `kairo-verify`; voice-tone name and crate are Claude's call; a new name is added to the M1 contract first | M1:50–56, PLAN:296–297 |
| Launch | GUI, capture and audio programs start only through `\Kairo\Run`: in the owner's session, priority 4, no console window. `kairo-devrun` refuses jobs from the SSH session | M1:91, CL:24, M0:63–67, CB:1163 |
| Job file | One program per job: `{"id","cmd":[exe,args…],"cwd","max_seconds","abort_on_input","window"}`. Result goes to `C:\kairo\logs\<id>.result.json`, output to `<id>.out.log` and `<id>.err.log` | M0:72–73, core/job.rs:30–53 |
| Limits | Every job has `max_seconds`; `abort_on_input` stays on; one job at a time; exit 6 = safety stop; nothing overnight unless the owner says so | M1:92, CL:25 |
| Busy | `pc status` before each long step; BUSY or UNKNOWN → work on the Mac. Capture needs the owner signed in and unlocked, displays on, no RDP | M1:90, CL:23, CB:1175 |
| Unattended | No job runs while the owner is away until `kairo-devrun` can stop within 2 s of a game start or another fullscreen window (the job's own testbed excepted), restore displays and audio, keep the PC awake and cover the OLED. Until then, only short jobs while the owner is home and has said the PC is free | M1:97, M1:129, M1:203 |
| Monitor | Picked by `monitorDevicePath`, never "primary" or an index | M1:54, M1:38, R08:770, CB:156 |
| Adapter | A D3D11 device on the adapter that drives its monitor, chosen by LUID (the PC has an iGPU and an RX 9000-series card); the LUID is logged | M1:63, CB:185–195, R13:87 |
| Screens | Patterns, harness runs and soaks on the 180 Hz monitor. The OLED gets moving content only, ≤10 min per run, black capture-excluded cover when unattended. The owner allowed testbed moving patterns on the OLED, ≤10 min, on agreed days | CL:26, M1:93, M1:39, PLAN:334 |
| Sound | Tones at −30 dBFS, the soak tone included (research 10 used −20) | CL:26, M1:93, PLAN:334 |
| Mic | Nothing in M1 opens the owner's mic | M1:15 |
| Encoding | AMF gets NV12 only, every option set and read back, H.264 peak ≤50 Mbps. One H.264 session at a time, ≥1 s apart. Capture only via WGC (`gfxcapture`) or DDA (`ddagrab`), never `vsrc_amf` | CL:31, M1:94, PLAN:166, CL:8 |
| No synthetic input | No `SendInput` or hooks; CI's scan bans them in `crates/**/*.rs` | CL:10, M1:95, ci:59 |
| Privacy | Never log clips, audio, typed keys, non-game window titles or raw paths. Never commit logs, clips or PC details; committing even a synthetic clip needs the owner's OK. Job files that name a monitor live in `.dogfood/`; outputs go to `C:\kairo\logs` | CL:16, CL:18, M1:96, M1:119, M1:72–74 |
| Licences | No GPL dependencies or copied GPL code; FFmpeg only as LGPL DLLs; no OBS shader code | CL:17, PLAN:314, ADR1:16 |
| GPL-only FFmpeg filters | Must never be used, even in experiments. FF9's `configure` marks them `deps="gpl"`: `blackframe`, `boxblur`, `colormatrix`, `cover_rect`, `cropdetect`, `delogo`, `eq`, `find_rect`, `fspp`, `histeq`, `hqdn3d`, `interlace`, `kerndeint`, `mcdeint`, `mpdecimate`, `mptestsrc`, `nnedi`, `owdenoise`, `perspective`, `phase`, `pp7`, `pullup`, `repeatfields`, `sab`, `signature`, `smartblur`, `spp`, `stereo3d`, `super2xsai`, `tinterlace`, `uspp`, `vaguedenoiser` | FF9:configure:4152–4293, CL:17 |
| Colour formulas | HDR patches and curve references come from the published BT.2100 and BT.2390 formulas; the plan's converter uses "no OBS code". The HLSL sketches in the cookbook and research 09 carry a "derived from OBS" header, so the testbed and `kairo-verify` must not reuse them | M1:192, PLAN:163, CB:356, R09:390 |
| Code | Exact pins (`windows` =0.62.2, `wasapi` =0.26.0, `ffmpeg-next` =9.0.0, `ffmpeg-sys-next` =9.0.1). `unsafe` only in capture, gpu, encode, audio, platform or input modules, each with `// SAFETY:`. No `unwrap` or `expect` on runtime paths (lint-enforced) | CL:30–32, Cargo:43–44 |
| CI | Linux runs clippy over the whole workspace (ci:47) but tests only `kairo-core` and the mock (ci:50). Windows builds and tests the whole workspace (ci:98–104). New pure-Rust tests need `-p kairo-verify` added to ci:50 | ci:47–50, ci:98–104 |

---

## 2. testbed.exe (crate `kairo-testbed`)

### 2.1 Required features

| # | Feature | Requirement as written | Source |
|---|---|---|---|
| T1 | Modes | Windowed, borderless or exclusive fullscreen | M1:11, PLAN:350 |
| T2 | Frame rates | 60–360 fps | M1:11, PLAN:350 |
| T3 | Monitor | Picked by `monitorDevicePath`, never "primary" | M1:54 |
| T4 | Adapter | D3D11 on the adapter driving that monitor, by LUID; LUID logged | M1:63 |
| T5 | Barcode | "A block barcode that survives scaling and H.264"; fallback if lost: bigger blocks | M1:11, M1:200 |
| T6 | Moving bars | "moving bars"; `kairo-verify` checks the decoded bars at ≥40 dB PSNR | M1:11–12 |
| T7 | Flash + game beep | "A per-second flash with a 1 kHz 'game' beep". Research 10: flash white and beep at the same QPC instant every second; the brief: frame counter and timed beeps | M1:11, R10:808, R00:441, R05:316 |
| T8 | HDR patches | "HDR patches", from published BT.2100/BT.2390 formulas; recommendation: the patches move (OLED) | M1:11, M1:192–193 |
| T9 | Ground truth | "A QPC log as ground truth" | M1:11, PLAN:350 |
| T10 | Voice beep | Played by a separate process (§3) | M1:11 |
| T11 | Tone level | −30 dBFS | CL:26, M1:93 |
| T12 | Gate 3 | Runs as moving content beside `wgc-probe` on both monitors; on the OLED ≤10 min | M1:28, M1:39 |
| T13 | PC acceptance | "The testbed on the 180 Hz monitor: its modes and frame rates, the QPC log, a barcode that survives H.264" | M1:131 |
| T14 | Fullscreen test target | The watch agent's fullscreen test runs against the testbed in borderless and exclusive fullscreen; the runner's future fullscreen stop must exempt the job's own testbed | M1:159, M1:97 |
| T15 | Later uses | SP2 at 360 fps on both monitors (PLAN:428); SP3 patches (PLAN:430); M3 task 4, 360 fps for 10 min (CB:1188); M3 task 8, game audio by include-tree on the testbed's PID (CB:1192); M8 crash dump byte-searched for barcode bytes and tone samples (PLAN:503) | as listed |

### 2.2 Constraints these imply (derived)

- **The 1 kHz beep plays in the testbed's own process** (CB:1192: include-tree on the testbed PID). The voice beep must therefore not come from the testbed's process tree (§3.1).
- **Per-monitor-v2 DPI** so the swap chain maps 1:1 onto physical pixels. Otherwise DWM scaling blurs the barcode and the bars [Gap; derived from T5/T6]. Research 08 requires PMv2 for DDA (R08:767).
- **Frames are whole.** Exclusive fullscreen above refresh without vsync tears, and a torn barcode fails its CRC. [Rec] exclusive fullscreen only at fps ≤ refresh, vsynced.
- **Privacy.** No window titles or other apps' names; the log names no monitor or adapter; file names only (CL:16).
- **Keeps no static high-contrast element on the OLED** (CL:26, M1:39).
- **Never generates input** (CL:10). It is identifiable by image name so the runner can exempt it (M1:97).
- **Unsafe** only in its gpu, audio and platform modules (CL:32). WASAPI render uses `wasapi` =0.26.0 (CL:30).

### 2.3 Not specified by the sources [Gap]

- Barcode payload, bit count, error check, block size, colours and placement.
- Bar layout: which bars, sizes, speed.
- Flash size, region, level and duration; beep length, envelope, channels and schedule.
- How "same QPC instant" is achieved and recorded.
- HDR patch levels, sizes, motion and swap-chain format.
- The QPC log: format, fields, file name.
- The testbed's command line, exit codes and run length.
- Behaviour when fps > refresh, and on HDR toggles or display changes.
- Whether exclusive mode picks a display mode.
- What the OLED pattern looks like.
- Whether "Test my setup" ships the testbed's pattern (§4.8).

### 2.4 Recommendations [Rec]

1. **Pattern as a pure function.** The pattern is a pure function `frame N → list of solid axis-aligned rectangles`, in a platform-independent `kairo-core` module.
   - The testbed draws those rectangles with D3D11.
   - A CPU raster of the same list renders synthetic clips and gives `kairo-verify` its expected geometry and colours.
   - Reasons: one source of truth for testbed, generator and verifier; exact equality because the rectangles are integer and solid; testable on the Mac and Linux CI.
   - A WARP test on CI's Windows runner proves that the GPU draw equals the CPU raster, in the spirit of PLAN:342.
   - It is instrument code, outside the clock even inside `kairo-core` (M1:104 counts only the engine maths).
2. **Barcode.**
   - Content: a 24-bit frame counter, 9-bit testbed fps, 7-bit run tag and CRC-16, so 56 bits.
   - Layout: 14 × 4 data blocks plus a calibration pair (one white, one black) per row.
   - Size: blocks of H/45 (32 px at 1440p).
   - Calibration: the pair swaps sides every frame, so no pixel stays white on an OLED; the reader takes the threshold as the midpoint of the pair.
   - Placement: inside the centre 9:16 band, x within W/2 ± 9H/32, so vertical exports (M7) keep it.
   - Evidence: a 40-bit variant read 100% in every tested scaling and bitrate, with a minimum bit margin of 0.44 at 32 px [Mac exp, EXP-B].
   - Fps is in the code so cadence works without a log. CRC-16 because a CRC-8 lets 1 in 256 random misreads through over 1,800 frames.
3. **Bars.**
   - Seven 75% bars: 191 in sRGB 8-bit, expected BT.709 limited codes from R09:372–382.
   - Band height H/6; motion 1 bar width per second, so a 60 fps clip shows steady motion. Position is a function of N.
   - At least two full bars always inside the 9:16 band.
4. **Flash and beeps.**
   - A flash patch inside the 9:16 band, not full screen (less glare; it survives crops). It goes from black to 100% white (Y 235) for 100 ms each second.
   - The beep: 1 kHz, 100 ms, 2 ms raised-cosine ramps, −30 dBFS peak on both channels.
   - Both are scheduled on whole seconds of QPC, with the frame clock anchored to a whole QPC second so flash frames are exact multiples of fps. The voice process then aligns to the same QPC grid without IPC (§3.3).
   - Evidence: the 100 ms beep gives 80 ms of steady tone for SNR, and matched-filter onsets are sample-exact through AAC [Mac exp, EXP-A].
5. **HDR mode** (monitor in HDR; FP16 scRGB swap chain, 1.0 = 80 nits, R09:404).
   - Grey patches at 80, 203, SDR-white, 400 and 1000 nits.
   - Barcode and bars at SDR-white-referenced values.
   - Patches drift so nothing stays static (M1:193).
   - 203, 240 and 400 nits separate "Natural" from "Match SDR apps" by 5.6–20 codes, more than the ±2-code rule (EXP-E).
   - Log MaxLuminance and SDR white, which `kairo-verify` needs for the expected curve values (CB:383).
6. **Pattern `motion` for OLED and gate 3.** Full-frame moving gradient and bars; no barcode, no static element. Gate 3 needs only delivery timestamps (M1:28).
7. **Timing.**
   - Frame N's content time is `T0 + N/fps` (time-scheduled, like a game).
   - Present with vsync when fps divides the refresh. Otherwise use flip-discard with sync interval 0 and no tearing flag in windowed or borderless mode, where DWM shows the newest frame each vblank.
   - Log every miss, so `kairo-verify` can tell testbed misses from capture drops.
8. **QPC log** (`<job-id>.testbed.jsonl` in `C:\kairo\logs`).
   - Format: JSON Lines, one write per line, so a stopped run leaves whole lines (the same rule as traces, M1:83, core/trace.rs:21–23).
   - Header: schema, version, mode, fps, size, measured refresh, HDR state, SDR white, MaxLuminance, adapter LUID, DXGI adapter and output index (for the capture command), pattern parameters, run tag, T0.
   - Per frame: N, QPC before and after Present, and DXGI frame statistics (PresentCount, SyncRefreshCount, SyncQPCTime) when available, which give display time and the real refresh [Unverified in windowed flip model].
   - Per beep: k, frequency, scheduled QPC, and render QPC from `IAudioClock::GetPosition` plus the position-to-frame mapping (the method of R10:812).
   - Per flash: onset frame.
   - Events: ready (first frame displayed), HDR or display change, device loss, audio device change, exit reason.
   - Times in 100 ns via `core/timeline.rs:88` `hns_from_qpc`. No device paths, names, titles or wall-clock time.
9. **Command line.**
   ```
   testbed.exe --monitor <monitorDevicePath> --mode windowed|borderless|exclusive
               --fps N --seconds S [--pattern full|motion] [--audio on|off]
               [--log <file name>] [--tag N]
   ```
   - It refuses to start without `--monitor` or when the path isn't found. It caps itself at 600 s with `--pattern motion`; `kairo-devrun`'s `max_seconds` stays the backstop.
   - Exit codes: 0 ok; 2 bad arguments; 3 monitor not found; 4 adapter or device failure; 5 audio failure.
10. **PC acceptance (M1:131).**
    - Each mode × {60, 180, 360} fps for 20 s on the 180 Hz monitor; the log shows no missed vblank at fps ≤ refresh in borderless or exclusive.
    - Gate 2's capture reads 100% of barcodes.
    - Exclusive mode at 180 only.
11. **M8 hook.** A deterministic, seeded high-entropy strip and the fixed tone sample table, so the M8 dump byte-search has unique needles (PLAN:503). Flat white and black runs would match everywhere.

---

## 3. Voice-tone process

### 3.1 Requirements

| # | Requirement | Source |
|---|---|---|
| V1 | A separate process plays a 2 kHz "voice" beep | M1:11, PLAN:350 |
| V2 | Its own process ID for the 2 kHz tone (so include-tree can target it as the voice-chat stand-in) | M1:55, PLAN:187 (T4 = include-tree on `Discord.exe` + list) |
| V3 | Name and crate: Claude decides at implementation time, then adds them to the M1 contract | M1:55, M1:50 |
| V4 | −30 dBFS; launched only via `\Kairo\Run`; renders only, never opens the mic | CL:26, M1:91, M1:15 |
| V5 | Feeds the routing check "voice tone only on T4" and, later, "no voice tone unless ticked" (M7), "Discord voices absent while voice chat is off" (M5 G5) and "no voice tone … in T1/T5 before consent" (M11) | M1:12, PLAN:496, PLAN:476, PLAN:523 |
| V6 | **Derived:** never a descendant of the testbed. Include-tree on the testbed PID (CB:1192) would also capture the voice tone and break routing. The engine's game-root rule walks up within the install folder (PLAN:185, CB:772–773), so a harness in the same folder as `testbed.exe` is reachable too: tests should hand the engine the testbed PID directly | CB:1192, PLAN:185, CB:772–773 |

### 3.2 Not specified [Gap]

Beep length, schedule and phase relative to the game beep; channels; its log; how it finds its stop time; whether it plays continuously or in beeps; how it relates to `audio-soak`'s tone process (research 10's `soak-tone.exe` is a child of `audio_soak`, R10:812).

### 3.3 Recommendations [Rec]

1. **Name and home.** `voicetone.exe`, a second binary in `kairo-testbed`. It shares the testbed's WASAPI render and QPC-log module (one audio module with `// SAFETY:`), and it stays outside the clock as testbed code. `audio-soak`'s tone process may reuse the module, but it belongs to `audio-soak` (M1:59).
2. **Signal.** 2 kHz, 100 ms, 2 ms ramps, −30 dBFS peak, every second at QPC whole second + 500 ms. That is the game beep's grid shifted half a second, so even 2nd-harmonic distortion of the 1 kHz beep can't pass for the voice tone: it would come at game-beep times. The Mac experiment measured the 2nd harmonic at −87 to −107 dB (EXP-A).
3. **Log.** `<job-id>.voice.jsonl` with a header and one line per beep: index, scheduled QPC, render QPC, the testbed's log format (§2.4 item 8).
4. **Command line.** `voicetone.exe --seconds S [--log <file name>]`. It exits by itself. The harness starts it as a sibling of the testbed, never as its child.

---

## 4. kairo-verify

### 4.1 Requirements on the tool

| # | Requirement | Source |
|---|---|---|
| K1 | "ffprobe plus its own analysis"; the own analysis works without `ffprobe.exe` because "Test my setup" ships it | M1:12, M1:56, PLAN:271 |
| K2 | "Nothing GPL linked"; finds the flash and the beep itself (`blackframe` is GPL-only); research 13's libvmaf test stays outside | M1:12, M1:192, R13:741 |
| K3 | `ffprobe.exe` is for tests only and never ships | M1:70, R09:848 |
| K4 | Report layout: Claude decides | M1:84 |
| K5 | Gate 1 is judged per clip: each bad clip fails on its defect's check; each good clip passes | M1:26 |
| K6 | Privacy: no clip content or raw paths in output | CL:16, M1:96 |
| K7 | Survives a switch of the engine to C#: drives nothing engine-internal | ADR1:53, ADR1:67, PLAN:452 |

### 4.2 The checks

Each row gives what the check means, how to measure it, and the threshold(s) in the sources. Profile thresholds are in §4.5.

| Check | Meaning and measurement | Threshold(s) in the sources |
|---|---|---|
| **Drops and duplicates** | Read the barcode in every decoded frame (one per sample). Content time = N × 10⁷ / testbed fps. Run `core/cadence.rs:60` `cadence()`: `repeats` = steps of 0, `skipped` = n − 1 per step of n ≥ 2 ticks. Plan reading: (repeats + skipped) / steps (core/cadence.rs:11–13). List every event by output frame and content frame number (M1:188). With a testbed log, label each event source-side (frame never presented or missed vblank) or capture-side | ≤0.5% [proposed] (M1:12, PLAN:351; SP2 PLAN:428); 1% (R13:732: a duplicate-tick target); strict: any single one fails a gate-1 clip (M1:188) [Conflict] |
| **Content cadence / judder** | p99 nearest-rank of \|step − 1 tick\|, longest hold and largest step from `cadence()`. M3 task 4's "no gap >2 ticks" reads as `longest_hold ≤ 2` and `max_step_ticks ≤ 2` (core/cadence.rs:13–14) | SP2: p99 ≤ one refresh period (PLAN:428). The 90/s-at-180 Hz pattern lands 0.78 × 100 ns above it (core/trace.rs:410, M1:189). Tolerance and nominal-vs-measured refresh are open; `kairo-verify` needs its own rule (M1:189) |
| **CFR** | Container: every video sample lasts 1 tick (`stts`), and the frame count matches the duration × fps. **Kairo clips are an exception:** P8 makes the first video sample longer by the audio pre-roll (CB:678, core/cut.rs:9–12, 50–51). Allow exactly that: Δ ∈ [21.3, 42.7] ms, or [0, 21.3] at a track's start, and Δ must equal the audio lead. Use real sample pts for all timing, never index / fps | CFR 60 (PLAN:446); ≤42.7 ms hold (PLAN:179, CB:49); reproduced with the CLI as `stts` [1×1045, 179×256] (EXP-C) |
| **A/V offset per track** | Per track carrying a known beep: matched-filter onset against the beep's reference (sample-exact through AAC, EXP-A) vs the flash. **Perceived:** the first output frame whose barcode ≥ the flash onset frame. **Pipeline:** the barcode/QPC-log map from clip time to content QPC, plus the testbed's logged beep render QPC. Report per event and the median/max | ≤40 ms (M1:12, PLAN:351, PLAN:446, R09:993, R00:450); <10 ms target with a ≤21.3 ms gate (CB:1193); "\|offset\| < 10 ms" unconfirmed (CB:678, R10:808); stable ±10 ms over 8 h (R10:839) [Conflict] |
| **Track-start offset** | \|audio start − video start\| | < 0.022 s (R09:979); a separate check (M1:189) |
| **IDR at 0** | The first video sample is a sync sample (`stss` holds 1) and holds NAL type 5. Parse length-prefixed NALs with `avcC`'s length size; the Mac clips show [6, 5] (EXP-C). Cross-check: ffprobe's first row `0,0,K__` | R09:969, R09:971–972, CB:675, PLAN:179, PLAN:446 |
| **Closed 1 s GOP** | Sync samples at exactly 0, fps, 2·fps…, each holding an IDR; no other sync samples; the last GOP may be short. GOP = fps, so 30, 60 or 120 (core/timeline.rs:21–28) | PLAN:166, CB:685 ("K every 60"), R09:969 |
| **pts = dts** | No `ctts` box, or all offsets zero; every packet pts = dts | PLAN:446, R09:45–46, CB:685; `bf=2` makes `ctts` and pts ≠ dts on 67/130 packets (EXP-C) |
| **moov first** | Top level ftyp, moov, [free], mdat; exactly one moov | PLAN:179, R09:976, CB:686, boxes.py:70–71 |
| **No edit list** | No `edts`/`elst` in any track | PLAN:179, R09:590, CB:686; the CLI writes one by default (EXP-C) |
| **colr** | `colr` `nclx` in the `avc1` sample entry with primaries 1, transfer 1, matrix 1, full_range 0. It must agree with the SPS VUI when the VUI has colour (`h264_mf` writes none: warn, don't fail) | M1:12, R09:681–687, R09:893, R09:897, CB:684; `boxes.py` doesn't look inside `stsd`, the new parser must (EXP-C) |
| **Bars ≥40 dB PSNR** | Per fully visible bar per frame (bar positions from N), PSNR of interior Y/Cb/Cr against the expected 75% codes (R09:372–382, ±1 code on the shader path: CB:351) | ≥40 dB; "catches silent encoder corruption" (M1:12) |
| **Routing** | Per-track presence of the 1 kHz beep, the 2 kHz beep and the cue, gated to each schedule's times | "voice tone only on T4; Kairo's cue absent" (M1:12) [Conflict, §4.4] |
| **Tone SNR** | Sine fit at the known frequency over each beep's steady 80 ms; SNR = tone power / residual power (the research 10 noise-detector method, R10:821) | Soak: "tone SNR, drift" (PLAN:345). The −50 dB THD+N alarm is for −20 dBFS PCM and must be rechecked at −30 (R10:821, M1:190) [Conflict] |
| **Curve levels** | HDR-source clips: the Y code of each HDR patch against expected values from BT.2100 PQ, BT.2390 EETF, sRGB and BT.709 for the chosen curve, with Lw = MaxLuminance (400–4000) and Lmax = SDR white | ±2 codes; Natural ≈ Y 215, Match SDR apps Y 235 at 240-nit white (PLAN:430, CB:383–385); EXP-E confirms 215.0 / 235.0 |
| **Export profiles** | Per-preset rules (§4.5) | PLAN:241–249, R06:15–16, R06:132, R06:156, PLAN:496 |
| **Nothing GPL linked** | §4.4 | M1:12 |

**Derived checks** I recommend adding, each traceable to a source:

- **Duration** 30 s ± 1 GOP for a 30 s save (PLAN:446, CB:1193, R09:994).
- **Track layout and metadata.**
  - Titles "Mix", "Game", "Mic", "Voice chat"; handler names = titles; `und`.
  - Default disposition only on Mix; Mix first (CB:598, CB:684, P9 CB:50, PLAN:179).
  - AAC-LC 48 kHz; Mic mono (PLAN:191, CB:556).
  - The `description` JSON holds no paths or names (PLAN:179).
- **Audio pre-roll.** Lead ∈ [1024, 2047] samples, or [0, 1023] at a track's start (core/cut.rs:50–51, CB:665).
- **Peak bitrate.** Over any 1 s window ≤ the preset's peak and ≤50 Mbps (CL:31, PLAN:173–177).
- **Encoder read-back from the stream.** Profile High, CABAC, no B slices, SPS/PPS spacing. This is the CLI path's only "read back" evidence (CL:31, PLAN:166).
- **Clip window.** The last frame's content QPC ≈ the save QPC, and the first is the keyframe ≤ T − length. "IPC latency never moves the clip" (CB:669).
- **Unreadable barcode frames.** Counted and reported.

### 4.3 Threshold conflicts, with my resolution [Rec]

1. **Drops and duplicates: 0.5% vs 1% vs strict.** The plan's 0.5% [proposed] (PLAN:351) matches SP2 (PLAN:428). Research 13's 1% (R13:732) is a MinUpdateInterval duplicate-tick target at game fps ≥90, not a clip rule.
   - Use the **strict** rule (0 events) for gate 1, as M1:188 recommends.
   - Use **≤0.5%** for real recordings until SP2's ADR, then follow the ADR.
2. **A/V: 40 ms vs 10/21.3 ms vs 22 ms.** These measure different things:
   - At 60 fps, a newest-frame-≤-tick pacer shows a flash 0–16.7 ms after its true time, so per-event perceived offsets spread over about a tick, with a mean near 8 ms.
   - "Under 10 ms (≤21.3 ms gate)" (CB:1193) therefore only makes sense as **mean <10 ms and every event ≤21.3 ms**.
   - The plan's **≤40 ms** (PLAN:446) is the gate: every event, per track.
   - The 22 ms track-start check (R09:979) is separate (M1:189).
   - Run all three as named checks in the `replay` profile. Only ≤40 ms decides M3's timed gate; the task-9 pair decides M3 task 9.
   - Also report the barcode-corrected "pipeline offset" (median, max), which removes the pacing quantisation and shows real sync errors.
3. **Judder.** Rule: **fail when p99 \|step − tick\| exceeds the refresh-limited ideal by more than half a measured refresh period.**
   - The ideal is the p99 of an every-vblank delivery at the measured refresh, replayed through `core/trace.rs:259` `replay` over a phase sweep.
   - The measured refresh comes from the testbed log; the nominal value is the fallback.
   - Results:
     - The 90/s-at-180 Hz pattern fails by 2.78 ms of margin, with no dependence on 100 ns rounding.
     - 72/s at 360 Hz fails.
     - The inherent judder of 170/175 Hz panels passes (ideal = observed, core/trace.rs:411–412), which "Test my setup" on friends' PCs needs.
   - Adopt SP2's ADR when it lands (M1:189).
4. **Tone SNR.** Native AAC caps a −30 dBFS tone at **49.3–49.5 dB** (stereo 160k and mono 128k), with beeps at 43.3–51.6 dB [Mac exp, EXP-A]. Research 10's −50 dB alarm can't apply to AAC tracks.
   - Pass at **≥35 dB** [proposed] for beeps in clips, then recalibrate on the PC's first good AMF clips.
   - Keep −50 dB THD+N for `audio-soak`'s PCM captures, rechecked at −30 dBFS (M1:190).
5. **Bars at 40 dB.** Fine for capture bitrates: min 48.7 dB at 1080p 25 Mbps and 57 dB at 1440p.
   - Not for low-bitrate exports: 40.2 dB at 1080p 4.64 Mbps, 35.9 dB at 540p 1.2 Mbps, worst-case moving-noise background [Mac exp, EXP-B]. So the threshold is per profile.
   - PSNR missed every simulated gridline corruption (40.9–49.0 dB). A **row/column-mean error check ≤3 codes** inside bar interiors flagged the two stronger patterns: good 1.0, gridlines 14.3 and 6.8 (EXP-D).
   - Add it beside PSNR. The plan's PSNR rule stays.
6. **Edit lists: research 10 vs plan.** Research 10 proposed an `elst` for the audio pre-roll (R10:36, R10:680). The plan and cookbook chose negative pts with no edit list (PLAN:179, CB:49, CB:77). `kairo-verify` follows the plan.

### 4.4 What the named terms mean in the sources

- **Routing** (M1:12, PLAN:351). Sound lands on the right tracks: the 2 kHz voice-tone process stands in for voice chat. T4 is the "Voice chat" stem, include-tree on `Discord.exe` plus a list (PLAN:187). T2 is Game, include-tree on the game's root (PLAN:185); T1 is Mix (PLAN:189).
  - [Conflict] "Only on T4" collides with the consent text: voices are saved "on their own track **and in the main mix**" (PLAN:193).
  - It also collides with plan B's T5, "labelled 'includes voice chat'", and Win10 endpoint loopback, which "hears everything" (PLAN:188).
  - [Rec] An expectation matrix per configuration, with tracks found by title, never by index (T4 is absent when voice chat is off):

    | Signal | Present in | Absent from |
    |---|---|---|
    | 1 kHz game beep | Mix, Game | Voice chat |
    | 2 kHz voice beep | Voice chat; Mix when voice is mixed (the default when on); T5 under plan B or Win10 | Game, and T5 under plan A |
    | Cue | nowhere | every track |
  - **Mic is report-only.** The room mic re-captures speaker audio (R10:762), so absence can't be a pass rule.
- **Kairo's cue absent** (M1:12). The save cue is "<300 ms … (never recorded)" (PLAN:213). Research 10's include/exclude test: "exclude-tree on the engine: Kairo's cue absent" (R10:792). Win10 plays no cue while recording (PLAN:188); M11 checks "no cue in T1/T5 before consent" (PLAN:523).
  - [Gap] The cue sound isn't designed yet (M4).
  - [Rec] `kairo-verify` takes the cue's reference WAV and runs a matched filter over every track. M1 tests it with a synthetic stand-in cue whose energy sits away from 1 and 2 kHz. M4's cue should keep a distinctive signature.
- **Tone SNR** (M1:12; PLAN:345 soak "tone SNR, drift"). How cleanly a known test tone survives capture and encoding: fit and subtract the known sine, then compare tone power with the rest. It is research 10's #8064 noise detector (R10:821) applied to a track. It catches crackle or buzz from the Windows process-loopback bug and gross encoder damage.
- **Curve levels** (M1:12). After HDR→SDR, known HDR patches must land on the codes the chosen curve predicts: Natural (BT.2390 EETF), Match SDR apps (clip) or Reinhard (PLAN:164, CB:381–385). The tolerance is ±2 codes (PLAN:430).
  - The references come from the published formulas (M1:192). EXP-E computed them for Lw 1000 and Lmax 240:

    | Patch (nits) | Natural (Y code) | Match SDR apps (Y code) |
    |---|---|---|
    | 80 | 150.1 | 150.1 |
    | 120 | 177.0 | 177.0 |
    | 203 | 207.8 | 219.4 |
    | 240 | 215.0 | 235.0 |
    | 400 | 229.4 | 235.0 |
    | 1000 | 235.0 | 235.0 |
  - The knee is at 114.2 nits, matching R09:469.
  - [Gap] Reinhard's exact variant is SP3's choice.
- **Export profiles** (M1:12). The per-preset rules of PLAN:241–249 and research 06, checked as `kairo-verify --profile <name>` (PLAN:496 names `--profile reels`). Details in §4.5.
- **Nothing GPL linked** (M1:12). [Rec] Read it as a constraint on the tool, plus a cheap guard:
  1. `kairo-verify` links no GPL code and drives FFmpeg only through LGPL components with no GPL-only filter (CL:17, FF9:configure:4152–4293).
  2. It reports the build flags of the FFmpeg it drives and fails on the PC if they include `--enable-gpl` or `--enable-nonfree`.
  3. It flags clips whose H.264 carries x264's SEI or a libx264/libx265 encoder tag. This matters on the Mac: Homebrew's FFmpeg is a GPL build whose `mp4` muxer defaults to H.264, and libx264 is its first H.264 encoder (checked by listing, `ffmpeg -h muxer=mp4` and `-encoders`, without running x264). A fixture made without an explicit `-c:v h264_videotoolbox` would therefore be x264-encoded.

### 4.5 Profiles and thresholds [Rec]

| Check | `strict` (gate 1) | `capture-cli` (gate 2) | `replay` (M3+ clips) | `selftest` (Test my setup, 5 s) | Exports (`discord`, `whatsapp`, `reels`, `share`, `original`, `precise-trim`) |
|---|---|---|---|---|---|
| Drops + dups | 0 | report | ≤0.5% [proposed] | ≤0.5% (≤1 event in 299 steps) | 0 added versus the source |
| Judder (§4.3 item 3) | fail rule | report | fail rule | fail rule | inherits the source |
| Unreadable barcodes | 0 | report | ≤0.1% [proposed] | 0 | report |
| A/V per event, per track | ≤40 ms | skipped (no audio) | ≤40 ms (gate); mean <10 and max ≤21.3 ms (task 9) | ≤40 ms | ≤40 ms |
| Track-start offset | <22 ms | report | <22 ms | <22 ms | <22 ms |
| IDR at 0, GOP | IDR, GOP = fps | IDR, GOP = fps | IDR, GOP = fps | IDR, GOP = fps | IDR at 0, closed GOP (R06:156) |
| pts = dts, moov first, no `elst`, `colr` 1/1/1/0 | required | required | required | required | required; `elst` allowed only for Quick trim inside Kairo (PLAN:237) |
| CFR | uniform, P8 first-sample rule | uniform | P8 rule | P8 rule | uniform |
| Bars | ≥40 dB and line ≤3 | ≥40 dB and line ≤3 | ≥40 dB and line ≤3 | ≥40 dB | report below 8 Mbps |
| Tone SNR | ≥35 dB | skipped | ≥35 dB | ≥35 dB | ≥30 dB [proposed] |
| Routing | matrix | skipped | matrix | game beep present | mix only; voice tone absent unless ticked (PLAN:249, PLAN:496) |
| Curve levels | ±2 when the clip is HDR-sourced | skipped | ±2 | — | ±2 |
| Peak bitrate | ≤50 Mbps | ≤50 Mbps | ≤ preset peak | ≤ preset peak | Reels ≤25 Mbps (PLAN:245) |
| Size, format | — | — | — | — | Discord ≤19,000,000 B, H.264 + AAC, faststart, mix only, metadata title only (PLAN:243, PLAN:249). WhatsApp ≤1080p30, AAC 128k (PLAN:244). Reels 1080×1920, 23–60 fps, ≤25 Mbps, closed GOP, no `elst`, ≤15 min, ≤300 MB, AAC ≤48 kHz (PLAN:245, R06:156, R06:132). Share ≤1080p60 ~6–8 Mbps (PLAN:246). Precise trim ±1 frame by barcode (PLAN:496) |
| Duration | as generated | 30 s ± 1 frame | requested ± 1 GOP | 5 s ± 1 GOP | requested ± 1 frame |

### 4.6 Report layout (M1:84 leaves it to Claude) [Rec]

`<job-id>.verify.json`, plus one summary line on stdout. Exit codes: 0 pass, 1 a check failed, 2 tool or input error.

- **Header.** schema, tool version, profile, clip file name only, size, duration, tracks, FFmpeg version line with its build flags.
- **Checks array.** Each entry: `{id, status: pass|fail|warn|skip, measured, limit, unit, note, source}`, where `source` holds a citation key such as `PLAN:446`.
- **Events.** Every drop, duplicate and unreadable frame by output and content frame number, with testbed-vs-capture attribution. A/V per event per track.
- **ffprobe.** An optional `ffprobe` section with any disagreement with its own parser.
- **Never in it.** Samples, pixels, paths or PC names (CL:16). Reports stay in `C:\kairo\logs` and `.dogfood/`; the gate report gets one line (M1:209).

### 4.7 Not specified [Gap]

- Inputs and command line.
- Statistics: mean, median or max; per event or per clip.
- Denominators.
- Where PSNR is measured: which frames and regions; per frame or pooled.
- A/V reference points: display vs present vs content time.
- How the expected track layout and routing are given.
- What runs on clips with no barcode (real game clips, M4).
- HEVC and AV1 rules (SP7).
- Screenshot verification (M5 G7, PLAN:478).
- Long-file performance (M13's 60-minute recordings, PLAN:544).
- Whether `kairo-verify` checks the clip window against the save QPC.
- What "Test my setup" records: a testbed window, the desktop with a tone (PLAN:95), or something else.

### 4.8 Recommendations on structure [Rec]

1. **Library plus CLI.** `#![forbid(unsafe_code)]`; no dependency on engine crates (K7).
   - Own MP4 box parser: ftyp/moov/mdat order, every `trak` (`tkhd`, `mdhd`, `hdlr`, `udta` name), `edts`/`elst`, `stsd`/`avc1`/`avcC`/`colr`, `mp4a`/`esds`, `stts`, `ctts`, `stss`, `stsz`, `stco`/`co64`.
   - SPS/PPS parsing for profile, entropy coder and VUI colour.
   - Bounds-checked with size limits, since clips from friends' PCs reach it later (K6, CL:32).
2. **Decode.** A `FrameSource` / `PcmSource` trait. M1 implements it with a subprocess of the pinned LGPL `ffmpeg.exe` (M1:70); on the Mac, Homebrew's binary for development only, never linked.
   - Explicit native decoders (`-c:v h264`, `-c:a aac`).
   - Raw `gray`/`yuv420p` and `f32le` pipes.
   - **`-fps_mode passthrough`**: without it the P8 first frame comes out 4 times (183 vs 180 frames, EXP-C).
   - Only `crop`, `format` and `vstack`-style LGPL filters, to send regions of interest for long files.
   - In-process decoding (`ffmpeg-next` =9.0.0 safe API) arrives with the engine (M3) for "Test my setup".
   - Reasons: no WTFPL `cargo-deny` exception in M1 (deny:33); no FFmpeg dev files on CI (ci:47, ci:98); Mac runs; no `unsafe`.
   - Consequence: M1:181 settles in M3.
3. **Shared DSP.** Matched-filter onsets, sine-fit SNR and gated tone presence go into `kairo-core` next to the pattern spec and log format (§2.4 item 1), so `audio-soak`'s noise detector (R10:821) uses the same code.
   - CI tests use synthetic frames, PCM and hand-built box trees. No clip and no encoder in CI (M1:119; distribution FFmpeg packages on Linux runners are GPL builds [Unverified]).
4. **ffprobe** (`--ffprobe <path>`) is an optional cross-check used by the harness, never required (K1). R09:960–985 and CB:683–686 show the expected output to compare.
5. **Synthetic clips come from `testclip`.** A cross-platform third binary in `kairo-testbed`, never shipped.
   - It renders frames from the pattern spec and writes a matching "virtual" testbed log, so `kairo-verify` follows one code path.
   - It encodes with `h264_videotoolbox` on the Mac (dev only) or `h264_amf` on the PC with the full option set and NV12, ≥1 s between sessions (M1:94).
   - It sets colour with `setparams` (EXP-C).

---

## 5. Gate 1: bad clips and good clips

**The gate.** `kairo-verify` flags 100% of synthetic bad clips and passes good ones. The defects (M1:26, PLAN:417):

- dropped or duplicated frame;
- judder;
- 50 ms A/V offset;
- no faststart;
- edit list;
- B-frames;
- wrong `colr`;
- swapped tracks;
- corrupted bars.

**How it's checked** (M1:26):
- one bad clip per defect;
- each bad clip fails on its defect's check;
- each good clip passes.
- How the bad clips are made, and whether one dropped frame fails a clip, are open (M1:188).

**M1's recommendation** (M1:188): one defect per copy of a good testbed clip; judder by capturing at 72/s or 90/s; a strict single-frame rule for gate-1 clips. One drop in a 30 s clip is 0.06%, so the 0.5% rule alone would miss it.

**Where:** M1:119 lists gate 1's clips under "Mac only". The clips stay in `.dogfood/`; committing one needs the owner's OK (M1:119).

### 5.1 Good clips [Rec]

At least three, each 30 s with the full Kairo layout: Mix (default, both beeps), Game (1 kHz), Mic (silence), Voice chat (2 kHz). Each has the P8 audio pre-roll, `-use_editlist 0`, `+faststart`, and `setparams` BT.709 tv.

1. 1080p60 from a virtual 180 fps source on a 180 Hz panel, `h264_videotoolbox` 25/40 Mbps.
2. 1440p60 from 360 fps on a 360 Hz panel.
3. When the PC is free: clip 1 re-encoded by `h264_amf` with the plan's options.

Gate 2's real capture is reported but is not a gate-1 clip.

### 5.2 Bad clips [Rec]

Each is a copy of good clip 1, with collateral failures expected and listed.

| Defect | Recipe | Must fail | Collateral |
|---|---|---|---|
| Dropped frame | Content sequence skips one frame (one step of 2 ticks), re-encode | drops/dups (skipped = 1) | duration −1 tick (inside ±1 GOP) |
| Duplicated frame | Content repeats one frame, re-encode | drops/dups (repeats = 1) | — |
| Judder | Content times from `core/trace.rs` `replay` of an ideal 90/s-at-180 Hz trace (p99 5.556 ms, core/trace.rs:410); second clip 72/s at 360 Hz (p99 11.1 ms, 119 skips per 599 steps, core/trace.rs:408) | judder rule (§4.3 item 3) | 72/s clip also fails drops |
| 50 ms A/V offset | Audio shifted +50 ms against video (remux with `-itsoffset`, or PCM regenerated) | A/V per track (50 > 40) | track-start offset |
| No faststart | Remux without `+faststart` (moov last) | moov first | — |
| Edit list | Remux without `-use_editlist 0`; the default writes `elst` (EXP-C) | no edit list | — |
| B-frames | Re-encode with `-bf 2` (EXP-C: `ctts`, pts ≠ dts) | pts = dts | CFR (irregular `stts`, EXP-C), GOP positions |
| Wrong `colr` | `setparams=color_primaries=bt2020` (and a `range=pc` variant) | `colr` | — |
| Swapped tracks | Remux with the Game and Voice chat streams swapped, titles left in place; a variant with the default disposition on Game | track layout, routing | — |
| Corrupted bars | DCC-like 64 × 16 tiles of wrong values in the bars band of 10 frames (AMF #574's failure, PLAN:163); a second variant with gridlines | bars PSNR (tiles); line check (gridlines) | — |

Why synthetic judder rather than a real 72/s or 90/s capture: it is exact, reproducible and Mac-only (M1:119), and it uses the same pacer logic a real capture would. SP2 captures the real thing (PLAN:428).

### 5.3 Extra fixtures for checks outside gate 1's list [Rec]

M1:119 asks for "the plan's full list of checks", so give every check one failing fixture:

- unreadable/garbled barcode frames;
- tone SNR (crackle or noise added to Game);
- routing (voice tone leaked into Game);
- cue present (synthetic cue in T5);
- curve levels (a clip rendered with the wrong curve);
- CFR (one doubled sample without B-frames);
- IDR at 0 (a stream copy starting on a P-frame);
- GOP 120 at 60 fps;
- export profiles: Reels with `elst`, Discord over 19,000,000 B, vertical 61 fps;
- the x264 guard (§4.4): tested on a hand-built byte buffer, since x264 itself can't be used (CL:17).

---

## 6. Gate 2: the unattended flow and its 5-minute budget

### 6.1 The gate

"Unattended testbed + FFmpeg CLI capture → a verify report in <5 min" (PLAN:417, M1:27). With no one at the keyboard, `pc harness` starts one `\Kairo\Run` job (only one runs at a time) that:
1. opens the testbed on the 180 Hz monitor;
2. records it with the FFmpeg CLI: `gfxcapture` or `ddagrab` into `h264_amf`, NV12, every option set and read back, peak ≤50 Mbps;
3. runs `kairo-verify`.

Claude times it from start to report. **The check is the report, not a pass**, because `gfxcapture` doesn't hold a stable frame rate (M1:27, R08:386, FF9:filters.texi:29982–29984).

### 6.2 Preconditions

- **Free PC.** `pc status` is IDLE (M1:90, CL:23); `pc doctor` passes that session (M1:90).
- **Owner state.** Signed in, unlocked, displays on, no RDP (M1:90, CB:1175).
- **Unattended rule.** Until the runner's safety stops land (M1:97, M1:129), "no one at the keyboard" means the owner is home, has said the PC is free, and doesn't touch it. `abort_on_input` stays on. After that, the agreed test days (M1:35, still open).
- **Displays stay on.** Whether a screen Windows turned off can come back without synthetic input is open (M1:196). Start while the displays are on.
- **Software in place.**
  - The re-pinned FFmpeg is on the PC: today it still holds `n9.0.2-29` (M1:156, PLAN:309).
  - A first AMF session from the CLI (M1:130) and the short recording test with AMD Software back (M1:155) come first.
  - AMD Software is installed again (M1:42), so Instant Replay may share the VCN (R13:37–38). Record its state in the report; never change AMD settings (CB:1066).
- **Build.** A deployed build with `testbed.exe`, the harness runner and `kairo-verify` (M1:69). Builds now run as `\Kairo\Run` jobs too (M1:44, M1:154), so they compete for the one-job slot.

### 6.3 Flow [Rec]

1. **Mac.** `tools/mac/pc harness` (new verb; PowerShell on stdin, M1:86) refuses when BUSY or UNKNOWN (like `pc run`), reads the monitor's device path from `.dogfood/`, and writes `C:\kairo\jobs\next.json`:
   - `id`: `harness-<n>`
   - `cmd`: the harness runner with a scenario
   - `cwd`: `C:\kairo\logs`
   - `max_seconds`: 240 [proposed]
   - `abort_on_input`: true
   - `window`: hidden

   Then it starts `\Kairo\Run` (M0:72, M1:73, M1:92).
2. **Runner.** `kairo-harness.exe`, crate `kairo-testbed` [Rec], in the owner's session:
   - Start `testbed.exe --monitor … --mode borderless --fps 180 --seconds 45` and wait for its `ready` log line.
   - Start the FFmpeg capture with `-t 30`, keeping its verbose log in `<id>.ffmpeg.log`.
   - Wait for FFmpeg to exit, then stop the testbed.
   - Run `kairo-verify --profile capture-cli` on `<id>.capture.mp4` with `<id>.testbed.jsonl`, writing `<id>.verify.json`.
   - Exit 0 if a report exists, whatever the checks say.

   Why a separate runner: a job holds one program (M0:72), and the voice process must be the testbed's sibling, not its child (V6). `kairo-verify` runs here, in the owner's limited session, rather than in the elevated SSH session, so file analysis never runs elevated (M0:86).
3. **Mac.** Polls `<id>.result.json` like `pc run`, copies the report into `.dogfood/`, prints the summary and the wall time from step 1 to report.

### 6.4 Capture recipe (FFmpeg CLI) [Rec, Unverified on the PC]

```
ffmpeg -hide_banner -v verbose -init_hw_device d3d11va=kd:<ADAPTER_INDEX> -filter_hw_device kd ^
  -filter_complex "gfxcapture=window_exe='^testbed\.exe$':capture_cursor=0:max_framerate=180,fps=60,
     hwdownload,format=bgra,scale=1920:1080:flags=bilinear:out_color_matrix=bt709:out_range=tv,
     format=nv12,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv" ^
  -t 30 -c:v h264_amf -usage transcoding -quality balanced -rc vbr_peak -b:v 25M -maxrate 40M -bufsize 40M ^
  -g 60 -vbaq 1 -preanalysis 0 -high_motion_quality_boost_enable 0 -preencode 0 -bf 0 -max_b_frames 0 ^
  -frame_skipping 0 -filler_data 0 -coder cabac -profile:v high -header_spacing 60 -async_depth 4 ^
  -use_editlist 0 -movflags +faststart C:\kairo\logs\<id>.capture.mp4
```

| Choice | Why | Source |
|---|---|---|
| `gfxcapture`, window mode | The engine's API (WGC); window capture keeps notifications and other windows out of the clip. `ddagrab` is the fallback | PLAN:149, PLAN:154, FF9:filters.texi:29972–29999 |
| `-init_hw_device … -filter_hw_device` | Without them, both `gfxcapture` and `ddagrab` create their D3D11 device on the default adapter, and `h264_amf` with system frames creates its own. That matters with an iGPU (M1:186). The adapter index comes from the testbed log (its LUID) | FF9:vsrc_gfxcapture_winrt.cpp:1207–1229, FF9:vsrc_ddagrab.c:865–885, FF9:amfenc.c:302–327 |
| `ddagrab` fallback | `output_idx` indexes the device's adapter's outputs (FF9:vsrc_ddagrab.c:195–203), so it needs the same adapter index and the output index from the testbed log | FF9:filters.texi:29878–29883 |
| NV12 before AMF, BT.709 via `scale` | AMF NV12 only (CL:31). swscale's matrix must be set to BT.709 limited or the bars codes shift. Not `colormatrix` (GPL) | CL:31, FF9:configure:4163 |
| `setparams` | The FFmpeg 8.0.1 CLI wrote no `colr` from `-color_*` output options, but did from `setparams` [Mac exp, EXP-C; 9.0.2 Unverified] | EXP-C |
| `-use_editlist 0 +faststart -bf 0 -g 60` | The default mux writes `elst` (EXP-C); faststart and B-frames per plan | PLAN:166, PLAN:179 |
| Options listed in full, 25/40 Mbps | Every option set; Recommended preset; peak ≤50 Mbps | PLAN:166, PLAN:173, CL:31 |
| "Read back" | The CLI can't call `av_opt_get` (CB:534). Evidence instead: (a) treat "option … not used" warnings in the verbose log as failure [Unverified for 9.0.2]; (b) the AMF version and adapter lines in the log; (c) `kairo-verify` reads profile, CABAC, no B slices, GOP and peak bitrate from the stream (§4.2) | CB:534, M1:94 |
| Video only | No loopback device without changing a Windows sound setting (M1:187); nothing opens the mic (M1:15) | M1:187 |

### 6.5 Budget [Estimate]

| Step | Time |
|---|---|
| `pc harness`: SSH, status, job file, Start-ScheduledTask | 3–8 s |
| Runner start, testbed first displayed frame | 1–3 s |
| AMF open, 30 s capture, close, faststart rewrite | ≈32–33 s |
| Testbed stop | <1 s |
| `kairo-verify`: decode 1,800 frames at 1080p via pipe, analysis | ≈10–25 s |
| Result polling, copy to `.dogfood/` | 2–5 s |
| **Total** | **≈50–75 s**, against 300 s |

`max_seconds` 240 kills a hung AMF session inside the budget (M1:204). A killed run has no report, so that run fails the gate. A driver timeout is P0 (M1:204).

### 6.6 What the report must hold [Rec]

- The `capture-cli` verify report (§4.6).
- Wall time from start to report.
- FFmpeg version and build flags.
- The adapter AMF and `gfxcapture` used, against the testbed's LUID. This answers M1:186.
- AMF version.
- Option warnings.
- The testbed's presented and missed counts.
- Instant Replay state if known.

The gate report gets one line (M1:209).

---

## 7. Gate 3's use of the testbed

- **Per monitor:** one `\Kairo\Run` job starts the testbed as moving content and `wgc-probe` together (M1:28). That takes the same runner (§6.3), because a job holds one program (M0:72).
- **On the OLED:** moving content only, ≤10 min, on agreed days (M1:28, M1:39 answered). [Rec] Use `--pattern motion` (§2.4 item 6), 360 fps, with job `max_seconds` ≤600 s.
- **With the owner present.** The first run may show a Windows permission prompt (M1:28, M1:39), and that also settles the cover question for that run: no cover is needed. M1:193 leaves open whether an occluded testbed under the capture-excluded cover is throttled [Unverified]. M1's recommendation: "the job is the run, and the HDR patches move".
- **What `wgc-probe` takes and writes:** the monitor, length, output fps and `MinUpdateInterval` (M1:58). Its trace has no names, refresh rate or wall clock (M1:83).
- **Testbed fps:** 360 on the 360 Hz OLED; 180 (or 360 uncapped) on the 180 Hz panel. SP2 later uses 360 fps on both (PLAN:428).

---

## 8. Reuse after M1

| When | Use | Source |
|---|---|---|
| Every media PR | Green harness smoke; until `pc harness` lands, each media PR carries its own short PC smoke | CL:34, M1:194, PLAN:344 |
| After a Windows, GPU-driver or Adrenalin update | Harness smoke at the next idle window | PLAN:333, PLAN:557, M1:207 |
| M2 SP2 | Testbed at 360 fps on both monitors; the same cadence rules (≥99.5% fresh ticks, p99 ≤ one refresh period, ≤0.5% repeated or skipped) | PLAN:428 |
| M2 SP3 | Testbed HDR patches; curve levels ±2 codes | PLAN:430 |
| M2 SP8 | 4-track MP4 and an edit-list variant in players (`kairo-verify` labels which is which) | PLAN:434 |
| M3 tasks | Task 4: testbed 360 fps, 10 min. Task 7: ffprobe set. Task 8: include-tree on the testbed PID. Task 9: flash vs beep <10 ms (≤21.3 ms gate), mix default only, titles, durations. Task 10: 20/20 | CB:1188–1194 |
| M3 timed gate | 20/20 harness saves via `kairo-cli` pass `kairo-verify` (30 s ± 1 GOP, CFR 60, IDR at 0, pts = dts, moov first, no edit list, `colr`, mix + game + mic, A/V ≤40 ms); save→file p95 <1 s; no capture stall | PLAN:446, ADR1:72 |
| M3 untimed | Stall recovery with the ring intact (verify the clips across it) | PLAN:447 |
| M4 | "Valid files" on real game clips: container checks only, no barcode | PLAN:461 |
| M5 | G2 harness 500/500 incl. overlapping, 2-minute and HDR-toggle saves; G5 voices absent while voice chat is off; G6 forced DDA passes the harness; G4/G7 HDR and screenshots | PLAN:473–478 |
| M6 | "Test my setup": 5 s self-test on friends' PCs, without ffprobe; the version-watch 5 s self-test | PLAN:271, PLAN:488, PLAN:140, M1:56 |
| M7 | Discord copies ≤19,000,000 B; no voice tone unless ticked; vertical passes `--profile reels`; Precise trim ±1 frame | PLAN:496 |
| M8 | 8 h soak 48/48 saves valid; dump byte-search for testbed barcode bytes and tone samples | PLAN:503 |
| M11 | Win10: no voice tone and no cue in T1/T5 before consent; harness 100/100 | PLAN:523, PLAN:529 |
| M13 | 10 manual recordings of 5–60 min pass `kairo-verify` (streaming, region-of-interest decode) | PLAN:544 |
| C# switch | UI, contract, fixtures, mock and harness stay | PLAN:452, ADR1:67 |
| Soaks | Tone SNR and drift share `kairo-verify`'s detectors | PLAN:345, R10:821–822 |

---

## 9. Decisions M1 leaves to Claude, with my answers [Rec]

| # | Decision | Answer | Source |
|---|---|---|---|
| D1 | Voice-tone name and crate | `voicetone.exe`, second binary in `kairo-testbed` | M1:55 |
| D2 | Names where the sources give none | `kairo-harness.exe` (runner) and `testclip` (generator), both in `kairo-testbed`. Logs: `<id>.testbed.jsonl`, `<id>.voice.jsonl`, `<id>.capture.mp4`, `<id>.ffmpeg.log`, `<id>.verify.json`. Added to the M1 contract as they land | M1:50, M1:72 |
| D3 | Report layout | §4.6 | M1:84 |
| D4 | How bad clips are made | Synthetic, one defect per copy; judder by pacer replay (§5.2) | M1:188 |
| D5 | Does one dropped frame fail? | Yes in `strict` (gate 1); ≤0.5% for real recordings | M1:26, M1:188 |
| D6 | Judder rule | Excess over the refresh-limited ideal > ½ measured refresh period | M1:189 |
| D7 | Thresholds per profile | §4.5 | M1:189 |
| D8 | Audio in the CLI capture | None in gate 2; never ask for Stereo Mix; audio joins the harness with the engine's process loopback | M1:187 |
| D9 | `gfxcapture` or `ddagrab` | `gfxcapture` window mode, adapter pinned; `ddagrab` as fallback | M1:27 |
| D10 | Do HDR patches move? | Yes, plus the `motion` pattern for the OLED | M1:193 |
| D11 | Smoke runs before `pc harness` exists | Each media PR carries its own short PC smoke (as M1 recommends) | M1:194 |
| D12 | Decode backend | ffmpeg CLI subprocess in M1; in-process with the engine | M1:56, M1:181 |
| D13 | Where shared pattern, log and DSP code lives | `kairo-core` modules, counted as instrument code | M1:104 |
| D14 | QPC log format | §2.4 item 8 | M1:11 |
| D15 | Barcode, bars, flash, beep design | §2.4 items 2–4 | M1:11, M1:200 |

---

## 10. What the sources do not specify (consolidated) [Gap]

**Testbed**
- Barcode format and size; bars, flash and HDR-patch geometry, levels and motion.
- Beep and flash timing method; the QPC log format.
- Command line and exit codes; DPI awareness; behaviour above refresh.
- Exclusive-mode display mode; the OLED-safe layout.
- M8's byte-search needle.

**Voice process**
- Schedule, length, log, command line, process ancestry.

**`kairo-verify`**
- Inputs, profiles and statistics: mean, max, per event.
- Drop/duplicate denominator.
- A/V reference points and per-track scope.
- PSNR region and pooling; judder tolerance (deferred to SP2); tone-SNR value at −30 dBFS.
- The routing matrix under plan A/B, Win10 and voice off; Mic expectations.
- The cue's sound; Reinhard's variant; the export profile list as CLI names.
- Behaviour on clips without a barcode.
- HEVC, AV1 and screenshots; long-file handling; report layout.
- What "nothing GPL linked" checks.

**Gates**
- How many good clips gate 1 needs; whether real captures count as good clips.
- Gate 2's capture length, resolution and bitrate; the exact start and end of "5 min".
- What gate 2's report must contain; how AMF options are "read back" through the CLI.

**Later**
- What "Test my setup" records and whether the testbed pattern ships.
- How the M3 harness gets the testbed treated as the game (an explicit PID).
- Whether harness runs with the engine may open the owner's mic (needed for M3's "mix + game + mic", PLAN:446).

---

## 11. Owner questions that touch these instruments

**From Your part** (M1:35–44), as of 21f0770:

| Step | State | Effect on these instruments |
|---|---|---|
| 1 Test days | **Open** | Gate 2 runs either with the owner home and saying the PC is free (M1:97), or on agreed days once the runner's stops exist. Recommendation: a few fixed weekday windows (M1:35) |
| 2 Updates and sleep | Answered: sleep Never; Claude checks Windows Update read-only (M1:36, M1:153) | After updates, a harness smoke (M1:207) |
| 3 Overnight | Default holds: no overnight jobs (M1:37) | No overnight harness |
| 4 Monitors | Answered (M1:38) | Testbed and capture by device path; the mapping in `.dogfood/` |
| 5 OLED | Answered: moving patterns, ≤10 min, on agreed days (M1:39) | `motion` pattern for gate 3; the first `wgc-probe` run with the owner present for the border prompt (recommended answer: Allow) |
| 6 Load generator | Answered: 180 Hz (M1:40) | — |
| 7 Audio devices | **Open** (M1:41) | Not needed: the testbed and voice process render to the current default device and never switch it |
| 8 AMD Software | Answered: installed again (M1:42) | AMF should open; Instant Replay may share the VCN in harness runs; recorded, never changed |
| 9 Exit rule | Answered: decision 37 (M1:43, PLAN:657) | Testbed, voice and `kairo-verify` stay outside the clock |
| 10 Builds | Answered: builds move to `\Kairo\Run` (M1:44) | Builds and harness share the one-job slot |

**From Unverified items, needing the owner's OK:**
- **Stereo Mix** (M1:187). Recommendation: don't ask. Gate 2 stays video-only; A/V, routing and tone rest on gate 1's clips.
- **Committing a synthetic clip** (M1:119). Recommendation: none needed. Commit generators and test data built in code; clips stay in `.dogfood/`.
- **The OLED cover and "a run"** (M1:193). Recommendation, as M1's: the job is the run; patches move. Run gate 3's OLED job with the owner present.
- **Screens for unattended jobs** (M1:196). The first such job runs while the owner is home.

**New, for later (not M1):** may M3's harness clips include the owner's mic track (M3 gate "mix + game + mic", PLAN:446)? They stay in `.dogfood/`. Recommendation: yes, on agreed days; Claude asks in M3, since M1 opens no mic (M1:15).

---

## 12. Mac experiments behind the [Mac exp] claims

Scripts are in the Mac's scratch folder (`exp/`, not in the repo) and outputs in `exp-out/` beside it. They used FFmpeg 8.0.1 (Homebrew GPL build) as a separate program only: native `aac`, `h264_videotoolbox`, the `scale`, `crop`, `format`, `setparams`, `testsrc2` and `sine` filters, none of them GPL-only (FF9:configure:4152–4293). Nothing was linked. The clips are synthetic and stay in the scratchpad.

- **EXP-A, audio** (`audio_exp.py`). −30 dBFS tones through native AAC.
  - Continuous 1 kHz and 2 kHz at 160k stereo, 1 kHz at 128k mono: SNR 49.3–49.5 dB.
  - 100 ms beeps: steady SNR 43.3–51.6 dB.
  - Onset error: 0.00 ms (matched filter), ≤0.06 ms (50% envelope).
  - 2nd harmonic: −87 to −107 dB.
  - Decoded vs input lag: 0 samples.
  - Plus −80 dBFS white noise: 34.6–42.2 dB.
- **EXP-B, barcode and bars** (`video_exp.py`).
  - Setup: 90 frames per configuration, a worst-case random-block background changing every frame, a 40-bit code in 10 × 4 blocks plus calibration.
  - Barcode: 100% reads for 16, 24 and 32 px blocks at 1440p, at 1440p 25M, 1080p 25M, 1080p 4.64M, 720p 2M, 540p 1.2M and a 9:16 crop scaled to 1080×1920 at 8M. Minimum bit margin 0.32 (16 px at 540p) to 0.50.
  - Bars PSNR minimum: 57.1 (1440p 25M), 48.7 (1080p 25M), 40.2 (1080p 4.64M), 40.1 (720p 2M), 35.9 (540p 1.2M).
- **EXP-C, container and encoder** (`boxes_exp.py` plus FFmpeg).
  - `h264_videotoolbox` by default: pts = dts, K every 60 with `-g 60`, High. With `-bf 2`: `ctts`, pts ≠ dts on 67/130 packets, irregular `stts`, keys at 59/118.
  - The default mux writes a 1-entry `elst`; `-use_editlist 0` writes none.
  - `colr` appears only via `setparams`.
  - P8-like mux (audio −30 ms plus AAC priming): video `stts` [1×1045, 179×256] at 15360, so the first frame is 51.3 ms longer, and both start at 0.
  - Rawvideo decode: 183 frames in the default or CFR mode, 180 with passthrough or VFR.
  - First sample NAL types [6, 5].
- **EXP-D, corruption** (`bars_exp.py`). Gridlines in the encoder's input at 1080p 25M:

  | Pattern | PSNR min | Line error |
  |---|---|---|
  | +24 every 64 px | 40.9 | 14.3 |
  | +8 every 128 px | 48.1 | 6.8 |
  | +6 every 32 px | 47.9 | 3.6 |
  | +4 every 256 px | 49.0 | 3.0 |
  | good, 25M | — | 1.0 |
  | good, 4.64M | — | 3.0 |

- **EXP-E, curves** (inline Python). BT.2100 PQ and BT.2390 EETF with Lw 1000 and Lmax 240: the knee is at 114.2 nits; values in §4.4.

**Caveats.** VideoToolbox is not AMF and Homebrew's FFmpeg 8.0.1 is not 9.0.2, so every PC-side claim stays [Unverified] until gate 2's first run. The background in EXP-B is harsher than any real game, so its low-bitrate figures are pessimistic.
