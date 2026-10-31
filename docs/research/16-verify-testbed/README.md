# kairo-verify, the testbed and the voice-tone process: design (M1), final

> **About this folder.** M1's instruments are built from this design (docs/milestones/M1.md). It was written on 2026-10-24 by four research notes, a synthesis and three critiques; the notes are kept beside it (`requirements.md`, `container.md`, `signals.md`, `architecture.md`). The experiments behind the **[Mac]** labels, their clips and the critiques' scripts ran in the Mac's scratch folder and are **not** in the repo: paths such as `signals/…`, `container/…`, `exp-…/`, `design-scripts/…` or `critique-…/` name those files. Line numbers cite the repo as it was at 21f0770.

*2026-10-24 · Written against `main` at 21f0770. Nothing in the repo was changed. This merges four research notes in this folder (`requirements.md`, `container.md`, `signals.md`, `architecture.md`), settles where they disagree, and applies three critiques (feasibility, completeness, rules). "Changes after critique" at the end lists each issue and what changed. Claude implements from it; the owner can skim §1, §2 and §11.*

**Citation keys.**

| Key | File |
|---|---|
| `CL:n` | CLAUDE.md |
| `PLAN:n` | docs/PLAN.md |
| `M0:n`, `M1:n` | docs/milestones/M0.md, M1.md |
| `CB:n` | docs/research/15-implementation-cookbook.md |
| `R06`, `R08`, `R09`, `R10`, `R12`, `R13` `:n` | docs/research/NN-*.md |
| `core/x.rs:n` | crates/kairo-core/src/x.rs |
| `devrun/x.rs:n` | crates/kairo-devrun/src/x.rs |
| `ci:n`, `deny:n`, `Cargo:n` | .github/workflows/ci.yml, deny.toml, the workspace Cargo.toml |
| `ff:` | FFmpeg n9.0.2 source (the PC runs BtbN n9.0.2-17), read at github.com/FFmpeg/FFmpeg tag n9.0.2 |
| `MS` | Microsoft Learn: `SetForegroundWindow`, `IDXGISwapChain::SetFullscreenState` and `::Present` (read 2026-10-24) |
| `crit` | the critiques' scripts and clips (Mac scratch, not in the repo) |

**Evidence labels.**
- **[Mac]**: measured on the Mac with Homebrew FFmpeg 8.0.1, using only `h264_videotoolbox`, native `aac` and LGPL filters. These are stand-ins for AMF; details are in the four research notes and `crit`.
- **[Unverified]**: not yet checked. §12 says how each one gets settled.
- **[proposed]**: a threshold Claude chose, to confirm with PC data.
- **[Estimate]**: a planning number, not a measurement.

---

## 1. Goal and scope

**M1 needs these now.**
- **`kairo-verify`** with the plan's full check list (M1:12, PLAN:351). It does all its own analysis without `ffprobe.exe` (M1:56).
- **`testbed.exe`** and the **voice-tone process** (M1:11).
- **Gate 1's good and bad clips** (M1:26).
- **The `pc harness` flow behind gate 2**: testbed, FFmpeg CLI capture, then `kairo-verify`, with a report in under 5 minutes (M1:18, M1:27).
- The testbed is also gate 3's moving content, started together with `wgc-probe` by `kairo-harness probe` (M1:28), and the target of M0's carried-over fullscreen test (M1:159).
- All of this is instrument work, outside the exit-rule clock (M1:7, M1:104). §10 lists the two edges the owner can veto.

**Later milestones reuse it unchanged.**

| When | Reuse |
|---|---|
| Every media PR | The harness smoke (CL:34, M1:194; §8.7) |
| M2 | SP2 cadence at 360 fps on both monitors (PLAN:428): barcodes on the 180 Hz monitor; on the OLED only the motion pattern runs, so its cadence comes from `wgc-probe` traces and pacer stats. SP3 curve levels ±2 codes (PLAN:430); SP8 labels its edit-list clip (PLAN:434) |
| M3 | Task 4 (testbed at 360 fps, 10 min; on the OLED, motion pattern and pacer stats), task 8 (include-tree on the testbed's PID), task 9 (flash vs beep), and the timed gate's "20/20 pass `kairo-verify`" through `--gate m3` (CB:1188–1194, PLAN:446) |
| M4 | "Valid files" checks on real game clips, structure only (PLAN:461) |
| M5 | Harness 500/500, voices absent while voice chat is off, forced DDA (PLAN:473–477) |
| M6 | "Test my setup", which links the library on friends' PCs (PLAN:271); its cadence rules hold at any refresh rate (§2 items 11–12) |
| M7 | `--profile reels`, no voice tone unless ticked, Discord size, "Precise trim ±1 frame" through `export.trim` (PLAN:496) |
| M8 | Soak saves; the crash-dump byte search for barcode bytes and tone samples (PLAN:503) |
| M11 | Win10: no voice tone or cue in T1/T5 before consent (PLAN:523) |
| M13 | Recordings of 5–60 min (PLAN:544) |

**Out of scope here.**
- HEVC and AV1 rules (SP7): `hvc1` and `av01` exit 3 ("cannot evaluate") until then.
- Screenshot checks (M5 G7).
- The real cue sound (M4). A placeholder tests the detector until then.
- The Reinhard variant (SP3).
- What "Test my setup" records (M6).
- `kairo-devrun`'s safety stops (their own M1 item, M1:97, M1:129; a dependency, §10).
- `wgc-probe`, which is on the clock.

---

## 2. Decisions

Each item gives the decision, then the reason and its sources. **Conflict** marks a place where the research notes disagree; it says which one this design follows and why.

### Code and tooling

1. **Six code homes.**
   - **`kairo-core`** gains three pure modules. `testsig` is "pattern v1": layout, barcode codec, bars, ramp, flash, patches, load texture, tone schedule, burst samples, BT.709, PQ and BT.2390. `testbed_log` is the log format. `cadence::steps` is a per-step iterator with `cadence()`'s exact integer rounding (core/cadence.rs:60–111). It takes content in any integer unit per second (100 ns, or testbed frames at the nominal source rate) and any integer output rate, because `FrameRate` has only 30, 60 and 120 (core/timeline.rs:21–28) while exports run at 23–60 fps. A non-integer rate reports `skip`.
     - Why: one source of truth for the testbed, the clip generator and the verifier.
     - Clock: `testsig` and `testbed_log` are instrument code. `cadence.rs` arrived with skeleton PR #10 (343d799), and M1:104 counts "all of the `kairo-core` skeleton's maths". So `steps` lands in PR 1, before the first `wgc-probe` commit, and later edits to it count (§10).
   - **`kairo-verify`**: a library plus a CLI, `#![forbid(unsafe_code)]`.
   - **`kairo-testbed`**: `testbed.exe` and `kairo-tone.exe`.
   - **`kairo-display`**: a Windows library for "monitor by device path, adapter by LUID", shared by the testbed, the harness, loadgen and `wgc-probe` (M1:54, M1:63).
   - **`kairo-harness`**: `kairo-harness.exe`, the PC-side orchestrator (verbs `run`, `corpus`, `probe`, `displays`).
   - **Conflict:** requirements put the harness inside `kairo-testbed`. It gets its own crate because it grows an engine mode in M3 and must not depend on testbed internals.
   - Each new name goes into M1's contract tables, and into PLAN:296–297, in the PR that adds it (M1:50).
2. **The voice-tone process is `kairo-tone.exe`, a second binary of `kairo-testbed`.** It shares the testbed's audio module.
   - **Conflict:** the notes propose `voicetone.exe`, `voice-tone.exe` and `kairo-tone.exe`. The `kairo-` prefix matches the other tools.
   - A mode argument (`beeps`) lets `audio-soak` add `soak` instead of writing `soak-tone.exe`. That is a recommendation only; days on that mode count (M1:59, M1:104).
3. **Decoding.** Every structural check uses Kairo's own pure-Rust MP4, H.264-header and AAC-config parser. Pixels and samples come from linked LGPL FFmpeg (`ffmpeg-next` =9.0.0 and `ffmpeg-sys-next` =9.0.1, safe API only) on Windows. There is also a dev-only CLI-pipe backend (the Mac) and a synthetic backend (tests). Timing always comes from Kairo's own sample tables.
   - The CLI backend and the corpus builder's `ffmpeg` spawner build only with the default `dev-tools` feature, which M6's shipping build turns off, so the shipped tool can't start an arbitrary `ffmpeg`.
   - Audio is decoded at its native channel count, and shared Rust code downmixes (L+R)/2. Both backends then read a −30 dBFS stereo tone as −30; swresample's `-ac 1` read it as −27 [crit].
   - **Conflict:** requirements wanted an `ffmpeg.exe` subprocess only, so nothing is linked in M1. Linking wins for four reasons:
     - CB:1181 lists "Mac `cargo xwin check` of the workspace with `ffmpeg-sys-next` passes" as an M3 prerequisite *outside* the clock, and M1:181 settles "when a workspace crate first links FFmpeg". Linking in an instrument crate keeps that plumbing off M3's 7 counted days.
     - "Test my setup" runs in-process against the app's own DLLs (PLAN:271, PLAN:555).
     - M13's 60-minute files need zero-copy streaming.
     - A pipe re-times frames: a default rawvideo decode duplicates the held first frame [Mac].
   - Why not ffprobe:
     - It reports colour from the SPS VUI even when no `colr` box exists [Mac].
     - It can't see box order, edit lists as written, `alternate_group`, or IDR vs non-IDR I (container.md §8).
     - Its `start_time` is 0 for good, edit-list and misaligned clips alike [Mac].
4. **Own MP4 parser, no crate.** `mp4` 0.14.0 and `re_mp4` lack `colr`. `mp4-atom` 0.16.2 drops duplicate or unknown boxes and had four breaking releases in three months. `mp4parse` is MPL-2.0, which deny:18–28 doesn't allow. A verifier must see the file as written (container.md §3). The parser is about 600–800 lines with zero dependencies.
5. **Three rules apply to every FFmpeg CLI encode** (golden clips, the gate-2 capture). Colour is tagged on the *frames* with `setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv`. AMF gets `-pix_fmt nv12` explicitly. The mux uses `-movflags +faststart -use_editlist 0`.
   - FFmpeg 9.0.2's CLI copies the encoder's colour from each frame (ff: fftools/ffmpeg_enc.c:265–270, `enc_ctx->color_primaries = frame->color_primaries`). `-color_primaries` and `-color_trc` as output options therefore never reach the encoder; on the Mac they wrote VUI 2/2/1 and no `colr` [Mac].
   - `h264_amf` also accepts BGR0, RGB0, BGRA, ARGB and RGBA and converts them inside AMF (ff: amfenc.c:55–72): the DCC-corruption path the NV12-only rule exists to avoid (CB:1054, CL:31).
   - Without `use_editlist 0` the muxer writes an `elst` in every track [Mac; movenc.c].
   - **Conflict:** the architecture note's commands used `-color_*` options and would have failed `video.colr`.

### Signals

6. **Pattern v1 geometry and signals follow signals.md**, the most-measured design. The unit is `u` = client height / 1080.
   - Barcode: two 16×4 copies of 24 u cells (cells of 12–32 px).
   - Moving bars with soft edges, a ramp, a scrolling load band, a centred flash and a drifting patch row (§4.5).
   - **Conflict:** the other two layouts were 14×4 at H/45 (requirements) and 8×6 at H/27 (architecture). Signals' layout read 0 unreadable frames and accepted 0 false CRCs in 11,100 frames (540p–1440p, 0.7–35 Mbps). Cells of 6 px or more never failed; cells of 4.5 px or less failed only under starvation [Mac].
7. **Barcode payload: 48 bits.** 24-bit frame number, 1-bit HDR flag, 7-bit run tag, CRC-16/CCITT-FALSE.
   - Two OLED-care changes to the measured layout: the reference row's phase flips every frame, and 40 of the 48 bits are XOR-whitened by the frame number's low byte (§4.6). No cell keeps a level for more than 128 frames (CL:26, M1:39). Cell size and the black/white levels are unchanged, so the robustness measurement still applies.
   - **Conflict:** CRC-8 (architecture) lets about 1 in 256 random misreads through; CRC-16 doesn't.
   - The nominal fps isn't encoded (requirements wanted 9 bits): it comes from the log header, or log-free from the barcode line's slope.
8. **Flash and tone timing.** The flash is lit while `N mod F < round(0.1·F)`, where F is the nominal fps. The 1 kHz game beep is aimed at the flash frame's predicted display time. The 2 kHz voice beep plays on the absolute QPC grid at whole seconds + 0.5 s.
   - Both are anchored because frame 0 is the first vblank at or after a whole QPC second.
   - With F = 180 on a real 179.96 Hz panel the grids drift 0.22 ms/s, so the beeps first overlap after about 30 min. The detectors tolerate an overlap (signals.md §3.1), and voice windows come from detections or the voice log, never from the flash schedule (§6.3).
   - Log-free A/V then knows each flash's exact frame number (N_f = F·m); a QPC-grid flash wouldn't.
   - Beeps last 100 ms (80 ms of steady tone for SNR).
   - **Conflict:** the architecture note used 50 ms bursts and a corner flash that a 9:16 crop removes.
9. **The QPC log is UTF-8 text in `trace.rs`'s style**: a version line, then one record per line, each written in one piece (core/trace.rs:1–23, M1:83). Times are absolute QPC in 100 ns (core/timeline.rs:88), so the testbed and `kairo-tone` share a clock without IPC.
   - **Conflict:** requirements and architecture proposed JSON Lines. Text is about 2.5× smaller at 360 fps and reuses the trace parser pattern.
   - The log is optional: every check runs without it.

### Checks

10. **Profiles.** Thresholds live per profile (M1:189): `test` (gate 1's synthetic clips), `capture` (CLI captures of the testbed: gate 2, later SP2), `smoke` (`capture` with cadence as warnings: the media-PR smoke, §8.7), `engine` (M3+ saves; structure only when a clip has no testbed pattern), and the exports `share`, `discord`, `whatsapp`, `reels`, `original`.
    - `selftest` (M6) is `engine` with a 5 s length; M6 completes it.
    - `--gate m3` narrows `engine`'s verdict to PLAN:446's list (§6.5).
    - **Conflict:** the four notes used four naming schemes; these names say what the clip is.
11. **Drops and duplicates.** `test`: 0 repeats, 0 skips, 0 backward steps, every event listed by frame (M1:188). `capture` and `engine`: (repeats + skipped)/steps, minus the same share in item 12's ideal delivery, ≤0.5% [proposed] (PLAN:351, PLAN:428), and no backward step.
    - The ideal is 0 at 60, 120, 180 and 360 Hz, but 25% at 75 Hz and 16.7% at 50 Hz (`design-scripts/judder_rule.py`), so a friend's panel isn't blamed for its refresh rate.
    - One drop in a 30 s clip is 0.06% (M1:188). Research 13's 1% (R13:732) is a duplicate-tick target for capture tuning, not a clip rule.
12. **Judder rule.** Steps are counted in testbed frames: ΔN between consecutive readable output frames, against a tick of F_s/fps frames (F_s = the nominal source rate). A *judder step* rounds to exactly one tick but is more than ¼ tick away: |4·(ΔN·fps − F_s)| > F_s, in exact integers. A step exactly on the boundary (1.25 ticks at 144 Hz) can't then flip with timestamp jitter. The *excess* is the clip's judder-step share minus the share of an ideal every-vblank delivery, replayed through `kairo-core`'s pacer (core/trace.rs:259) over 7 phases (worst case) at the refresh rate measured from the barcode line, and classified the same way. It fails when the excess is above 0 (`test`) or above 5 percentage points [proposed] (`capture`, `engine`). p99 \|step − tick\| is reported for SP2's own metric (PLAN:428).
    - Computed results (`design-scripts/judder_rule.py`, the method of core/trace.rs:406–413, exact fractions):

      | Delivery | Judder-step share | Ideal | Result |
      |---|---|---|---|
      | 90/s at 180 Hz | 100% | 0% | fail |
      | 87.5/s at 175 Hz | 100% | 8.3% | fail |
      | 82.5/s at 165 Hz | 100% | 25% | fail |
      | Every vblank at 175 / 170 / 165 Hz | 8.3% / 16.7% / 25% | the same | pass |
      | Every vblank at 144 Hz | 0% (ΔN 3 is exactly ¼ tick off, not more) | 0% | pass |
      | Every vblank at 75 / 50 Hz | 0%; drops + dups 25.1% / 16.7% | the same | pass (item 11) |
      | 72/s at 360 Hz | 0% | — | the drop check fails it (19.9% skips) |
    - **Conflict:**
      - SP2's "p99 ≤ one refresh period" sits exactly on the 90/s case (M1:189).
      - Requirements' p99 excess rule misses 87.5/s at 175 Hz.
      - Architecture's flat "≤1% judder steps" fails every 165–175 Hz panel, so "Test my setup" would false-alarm on friends' PCs.
      - Comparing logged display times against a nominal tick read +6 to +25 pp of false judder at 144 Hz [crit: judder_144_log.py]; whole-frame steps remove that.
13. **A/V offset.**
    - **Method: the barcode line** (signals.md §3.4). Fit clip time against barcode frame number; for each flash, take the local freshest-frame line α_m; then offset = audio onset − (α_m + β·N_f).
    - Why: the plan's "beep vs first lit frame" method under-reads by up to one output frame. On a clip that was really +41.6 ms late it read +27 to +31 ms, so it would pass a +50 ms bad clip under 40 ms [Mac]. The barcode line was accurate to 0.2–0.4 ms.
    - **Thresholds:**
      - Every event on every track: ≤40 ms (PLAN:351, PLAN:446) in `test`, `engine` and the exports. `capture` has no audio in M1, so it skips this check.
      - `engine` adds `av.offset_tight` for M3 task 9: median ≤10 ms and maximum ≤21.3 ms (CB:1193). It is a warning by default and required with `--av-tight`.
      - The verifier's own precision is asserted by tests instead: golden clips must measure \|offset\| ≤1 ms, and `av50` +50 ± 1 ms.
    - **Conflict:** signals proposed 21.3 ms for gate 1. 40 ms keeps a one-tick shift from a drop/duplicate bad clip from failing A/V as collateral.
    - Research 09's "<22 ms" `start_time` test (R09:978–979; M1:189) is blind [Mac]. `av.start_alignment` replaces it (§6.2).
14. **Bars.** The analytic reference is regenerated from the frame number at the clip's pixel centres. Three rules apply:
    - **PSNR:** min(Y, Cb, Cr) ≥40 dB on every frame (PLAN:351), unreadable frames included (§6.3). Clean frames measured ≥46.0 dB (Light) and ≥49.3 dB (Recommended, High); corrupted ones 6.9–32.6 dB [Mac].
    - **Relative colour test:** fail if a BT.601, full-range or Cb↔Cr-swapped reference fits ≥6 dB [proposed] better. It works at any bitrate.
    - **Row/column line error** on Y, for DCC-style gridlines (PLAN:163, R13:113): in `test`, fail above 4 codes [proposed] on the Mac and CI corpora, while the PC corpus sets its own limit, max(4, 2 × its clean AMF value) (§7.4). Elsewhere it is a warning until AMF data calibrates it.
      - Why it's needed: a +24-code gridline every 64 px scored 40.9 dB and passed PSNR, but its line error was 14.3 codes [Mac].
      - **Conflict:** requirements wanted a hard gate of ≤3 codes. Signals saw starved encodes reach 9–22 codes, so it gates only where the bitrate is known.
    - Exports: PSNR ≥30 dB [proposed], plus the relative test. The ramp's chroma (128 ± 2) only warns there: at export bitrates its first chroma rows next to the coloured bars stray 2.5–5 codes from grey [Mac, PR 4b's review].
    - HDR-sourced `engine` clips: the bars rules are warnings until SP3's ADR fixes how the curve treats colour (signals.md:354).
15. **Tone SNR.** ≥35 dB [proposed], frequency ±1 Hz [proposed], on every beep of single-source tracks. Report-only on Mix when a mic feeds it.
    - Clean AAC measured 41.0–52.2 dB; crackle measured 33.6 dB [Mac].
    - **Conflict:** architecture proposed 30 dB. Research 10's −50 dB alarm judges raw PCM at −20 dBFS (R10:821, M1:190) and stays with `audio-soak`.
16. **Routing is a per-configuration expectation matrix**, read from `--expect` or a profile. Each tone's narrowband level is measured per track in 80 ms windows:
    - present: ≥−60 dBFS [proposed] in ≥90% of windows;
    - absent: ≤−70 dBFS [proposed] in every window;
    - leak: anything in between.
    - Mic is report-only in real clips: a room mic hears the speakers (R10:762).
    - **Conflict:** the plan's "voice tone only on T4" (M1:12) collides with voices in the mix (PLAN:193), plan B's T5 and Win10 (PLAN:188); the matrix holds all of them. Architecture's SNR-based presence thresholds were not measured; these were (present −30 dBFS, absent ≤−93 dBFS) [Mac].
17. **Container checks are container.md's C0–C14**, renamed to dotted IDs (§6.2).
    - `colr` ≠ VUI **fails**: players read one or the other.
    - A VUI with no colour description fails in `test` and `capture` and warns elsewhere: `h264_mf` writes none (CB:478), and "Test my setup" must not fail on fallback PCs.
    - **Conflict:** architecture made `colr` ≠ VUI a warning.
18. **Nothing GPL linked** (M1:12).
    - The linked backend requires the licence strings of avutil, avcodec and avformat (ffmpeg-next 9.0.0's safe `util::license()`, `codec::license()`, `format::license()`) to contain "LGPL", else exit 3 before analysis.
    - The CLI backend is spawned, never linked. It records its licence as information (Homebrew's build is GPL v3, which is why it is only ever spawned), and `build.lgpl` judges linked code only.
    - The corpus builder accepts only `h264_amf`, `h264_mf`, `h264_videotoolbox`, I_PCM copy and `aac`, always named explicitly (Homebrew's `mp4` muxer defaults to libx264), and passes only LGPL filters and bitstream filters.
    - `test` fails a clip carrying x264 or x265 SEI.

### Gates

19. **Gate 1's clips are synthetic.** `kairo-verify synth` renders pattern v1 (NV12 plus one WAV per track); an encoder then encodes it. There are three corpora:
    - **Mac:** `h264_videotoolbox`. This is the formal gate-1 run, since M1:119 lists gate 1 under "Mac only".
    - **PC:** `h264_amf` through the pinned `ffmpeg.exe`. A confirmation run in the first attended AMF session (M1:130).
    - **Windows CI:** lossless I_PCM on every PR. Regression only.

    M1 closes only when the Mac and PC corpora are both green.
    - Real captures are *not* gate-1 clips: `gfxcapture` is unstable (M1:27, R08:386), and SP2 owns that question.
    - **Conflict:** architecture made the PC corpus formal and required 3 real captures to pass.
20. **How defects are made.** Content defects (drop, duplicate, judder, 50 ms A/V, corrupted bars, swapped content) are injected in `synth`. B-frames come from an encoder setting. Container defects (no faststart, edit list, wrong `colr`) are box-level edits by `kairo-verify mutate`, so each clip carries exactly one change.
    - Every clip, the good ones included, is muxed from a video-only intermediate plus its own WAVs. Stream-copying a *finished* clip's video lengthens video frame 0 a second time (584 → 912 ticks), which shifts the video 21.3 ms: `av50` then reads +28.7 ms and passes [Mac; crit].
    - **Conflict:** container.md used FFmpeg remuxes, which are proven but silently drop `udta/name` titles [Mac]. Its remux matrix stays as an opt-in Mac test of the parser (§9).
21. **No Stereo Mix.** Gate 2 is video-only. A/V, routing and tones rest on the golden clips (M1:187). All four notes agree.
22. **Gate 2 runs as one `\Kairo\Run` job of `kairo-harness run`.**
    - The testbed runs borderless at 180 fps vsync on the 180 Hz monitor.
    - Capture: `gfxcapture` by **hwnd** (no window enumeration, CL:16) with `max_framerate=360`, so MinUpdateInterval is half a refresh period (exactly one undershoots: R08:391–399, CB:301) → `fps=60` → `hwdownload` → swscale with BT.709 limited range and left-sited chroma → NV12 → `setparams` → `h264_amf`. One D3D11VA device is created on the LUID-matched adapter index, and FFmpeg's log must confirm that adapter.
    - Then `kairo-verify --profile capture`.
    - Estimated 52–68 s against 300 s; `max_seconds` is 240.
    - In the attended window it starts on the owner's "go", with the displays still on (§8.1).
    - **Conflict:** requirements' `window_exe` regex enumerates windows. Architecture's command lacked `setparams`.
    - **Conflict:** architecture required the runner's safety stops (M1:97) before gate 2. They're needed only for runs *while the owner is away*. A ~70 s job with the owner home and hands-off is the "short job" M1:97 allows, and nobody is at the keyboard.
23. **Exit codes for `kairo-verify check`:** 0 pass (warnings allowed), 1 fail, 2 usage, 3 cannot evaluate, 4 internal. Reports carry file names only (CL:16).

---

## 3. Architecture

### 3.1 Crates and modules

| Crate | Kind | `unsafe` | Dependencies (exact pins, CL:30) | Holds |
|---|---|---|---|---|
| `kairo-core` (exists) | lib | forbidden (core/lib.rs:15) | none new | + `testsig`, `testbed_log`, `cadence::steps` |
| `kairo-verify` | lib + bin | forbidden | `serde`, `serde_json`, `kairo-core`; Windows only: `ffmpeg-next` =9.0.0 (`default-features = false`, `codec`, `format`) and `ffmpeg-sys-next` =9.0.1 (`default-features = false`, `avcodec`, `avformat`, as the M0 `ffcheck` prototype does: its defaults would add avdevice, avfilter, swresample and swscale through feature unification). Feature `dev-tools` (default) | `mp4/`, `h264.rs`, `aac.rs`, `decode/{linked,cli,synthetic}.rs`, `analyze/{barcode,flash,cadence,bars,levels,hdr,tones,av,routing,cue}.rs`, `checks/`, `profile.rs`, `report.rs`, `synth.rs` (incl. I_PCM), `mutate.rs`, `corpus.rs` |
| `kairo-testbed` | bins `testbed`, `kairo-tone` | only in `gpu`, `audio`, `platform`, with `// SAFETY:` (CL:32) | `windows` =0.62.2, `wasapi` =0.26.0, `kairo-core`, `kairo-display` | window, swap chain, pacing, renderer, beeps, logs |
| `kairo-display` | lib | only in `platform` | `windows` =0.62.2 | QueryDisplayConfig: device path → GDI name → HMONITOR → rect, refresh, HDR state → adapter LUID → DXGI index and vendor:device (R08 §7.2–7.3) |
| `kairo-harness` | bin | only in `platform`, with `// SAFETY:`: `GetDiskFreeSpaceExW`, `SetThreadExecutionState`, and a Toolhelp `TH32CS_SNAPPROCESS` snapshot matched by image name (allowed, ci:58). It opens no other process | `kairo-core`, `kairo-display`, `serde_json`, `windows` =0.62.2 | verbs `run`, `corpus`, `probe`, `displays` |

- **Windows-only crates** build a stub `main` elsewhere, as devrun does.
- **Linux CI** adds `-p kairo-verify` to its test step (ci:50).
- **cargo-deny:** ADR-0002 records the WTFPL exceptions for `ffmpeg-next` and `ffmpeg-sys-next`, which deny:33–34 already anticipates. deny.toml also bans `sysinfo`: process-listing crates may open other processes beyond `PROCESS_QUERY_LIMITED_INFORMATION` (house rule 3), and CI's API scan reads only Kairo's own crates (ci:60).
  - It is decided by Claude within the plan: ADR-0001 item 3 chose these crates, and the ADR adds no host, house-rule change or elevated part (docs/adr/README.md).
- **FFmpeg's own messages.** The `kairo-verify` binary calls `ffmpeg::util::log::set_level(Level::Quiet)` (safe) at start and reports error codes, so FFmpeg never prints a path to stderr or into a job's `err.log` (CL:16). The library leaves FFmpeg's global log state alone.

### 3.2 Data flow

```
kairo_core::testsig (pattern v1: Scene(N) = solid rects + 1-D bands + texture band; tone bursts)
  ├─► testbed.exe ── D3D11 copy/clear ─► 180 Hz screen ─► gfxcapture → swscale → AMF ─► capture.mp4 ─┐
  │        └─ 1 kHz beep (own PID) ── testbed.log (QPC truth) ─────────────────────────────────────┤
  ├─► kairo-tone.exe ── 2 kHz beep (sibling PID) ── tone.log ──────────────────────────────────────┤
  ├─► kairo-verify synth ── NV12 ─► ffmpeg (AMF | VT | I_PCM copy) ─► v-*.mp4 (video only) ──┐       │
  │                      └─ WAVs ─────────────────► ffmpeg mux (-c:v copy, aac) ─► corpus/*.mp4 ─┤
  └─► kairo-verify analysis: the same functions give the expected pixels and samples ◄──────────────┘
```

### 3.3 Decoding and the timing model

- **Structure.** `mp4/` reads the top-level headers, then the whole `moov` (capped at 64 MiB), then, for each video sample, its NAL lengths plus at most the first 64 bytes of each NAL, and whole SPS/PPS units up to 4 KiB. It never loads `mdat` payloads or AAC payloads.
  - Supported: largesize, `size == 0`, `co64`, version-1 boxes, `ctts` v0 and v1. Fragmented files are detected and fail (C0, exit 1).
- **Decoder trait.** Frames are delivered in presentation order, borrowed in place. Audio comes as f32 per track at its native channel count, with the AAC priming included; shared code downmixes (L+R)/2 for analysis.
  - `linked` (Windows): opens a file only after Kairo's parser has accepted it, through `format::input_with_dictionary` with `ignore_editlist=1`, `protocol_whitelist=file` and `format_whitelist=mov,mp4,m4a,3gp,3g2,mj2`. Linked avformat would otherwise open URLs, and only `kairo-net` may open sockets (CL:15). The decoder gets `err_detect=+crccheck+bitstream+buffer` and frame threading. Paths must be valid UTF-8, since ffmpeg-next's path conversion unwraps (format/mod.rs `from_path`); otherwise exit 3.
  - `cli` (`dev-tools` only): run with the clip's folder as working directory, `ffmpeg -v warning -nostdin -protocol_whitelist file,pipe -err_detect +crccheck+bitstream+buffer -ignore_editlist 1 -c:v h264 -i file:<file name> -map 0:v:0 -fps_mode passthrough -f rawvideo -pix_fmt yuv420p -`, and `-c:a aac … -map 0:a:N -f f32le -` per track at native channels. No filters. Any line FFmpeg prints counts as a decode error, warnings included: FFmpeg 8.0.1 reports a frame it had to conceal (the flags `has_decode_errors()` reads) only as the warning `corrupt decoded frame`, so at `-v error` most decoder-detected corruption passed [Mac, PR 4a's review]. `file:` keeps a name like `take:2.mp4` from being read as a protocol. An FFmpeg that fails before decoding anything didn't start: exit 3, not a failed clip. Stderr reaches a report only as its last line, with the path replaced by the file name.
  - `synthetic` (tests): frames straight from `testsig`, with optional scaling, noise and quantisation.
- **Timing comes only from Kairo's parser.**
  - Video sample i is presented at its `stts`(+`ctts`) time, plus a single-entry `elst` when present.
  - Decoded audio sample s plays at `(first_dts + s)/48 kHz − elst.media_time`.
  - With these rules a correct no-`elst` clip measured flash vs beep at 0.08 ms [Mac].

### 3.4 What runs where

| Where | What |
|---|---|
| Mac | Edit; `cargo test -p kairo-core -p kairo-verify` (pure and synthetic); `cargo xwin check/clippy --target x86_64-pc-windows-msvc` of all new crates with `FFMPEG_DIR=~/kairo-deps/ffmpeg-9.0-lgpl-shared`, after refreshing it from n9.0.2-29 to the pinned n9.0.2-17 zip (bootstrap.ps1:159–161); opt-in tests through Homebrew's `ffmpeg` (spawned, never linked); gate 1's formal corpus into `.dogfood/corpus-mac/` |
| Linux CI | fmt, clippy (Windows crates as stubs), tests of `kairo-core` and `kairo-verify`'s pure parts, house-rule scans (ci:47–70) |
| Windows CI | Before Build, a step reads `$FfmpegUrl`/`$FfmpegSha256` from tools/pc/bootstrap.ps1, downloads, checks and caches the pinned zip (keyed by SHA-256), and sets `FFMPEG_DIR`, `LIBCLANG_PATH` and `PATH` for tests (bootstrap-e2e.yml already fetches the same zip). Then: build, clippy and tests of everything; the testbed's WARP render test; the I_PCM corpus build and check. Nothing is uploaded but test reports (PLAN:370) |
| PC | Only `\Kairo\Run` jobs (M1:91): the testbed, `kairo-tone`, the harness, and `kairo-verify` in the owner's limited session (never in the elevated SSH session, M0:86). `pc deploy` also copies the FFmpeg DLLs that `kairo-verify.exe` imports, directly or through each other (confirmed with `dumpbin /dependents` in PR 6; expected: avcodec, avformat and avutil only) from `C:\kairo\tools\ffmpeg\bin`, after checking `KAIRO-PIN.json` against the pin, into the admin-owned build folder, so no DLL can be planted beside it (deploy.ps1:37, M0:61) |

---

## 4. The testbed (`testbed.exe`)

### 4.1 Window modes

- **Borderless (default).** A `WS_POPUP` window covering the monitor's rect, shown with `SW_SHOWNOACTIVATE`. Window capture needs no focus.
- **Windowed.** A framed window of client `--size` (default 1280×720), centred on the monitor.
- **Exclusive.** It needs keyboard focus: `SetFullscreenState` fails with `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` when "the output window does not have keyboard focus" (MS). So exclusive implies `--activate`, then:
  1. `SetFullscreenState(TRUE, output)` on the `IDXGIOutput` whose `Monitor` equals the HMONITOR;
  2. `ResizeBuffers`, which flip-model swap chains need after the switch (MS);
  3. `GetFullscreenState` must report TRUE: `e fse result=ok`, otherwise `e fse result=failed` and exit 5.

  Losing fullscreen later logs `e fullscreen_lost`. It keeps the desktop's display mode, uses vsync only, and runs on the 180 Hz monitor only. It exists for M1:159 and SP6.
- **`--activate`** (exclusive mode, and M1:159's fullscreen test, which `watch.json` can only see as a fullscreen *foreground* window, M0:74). The testbed waits until the owner's last input (`GetLastInputInfo`) is older than the foreground lock time-out (`SPI_GETFOREGROUNDLOCKTIMEOUT`, 200 s by default) plus 5 s. Then it shows the window with `SW_SHOW`, calls `SetForegroundWindow`, checks `GetForegroundWindow` and retries for 10 s: `e activate result=ok`, otherwise `e activate result=failed` and exit 5.
  - An expired lock time-out is one of the conditions MS lists, but "it is possible for a process to be denied the right to set the foreground window even if it meets these conditions" (MS) [Unverified for a task-launched process; PR 8's attended run settles it].
  - It never synthesises input, presses keys or attaches to another thread's input (CL:10, ci:59).
  - Fallback: one job with `abort_on_input:false` in which the owner clicks the testbed once (M1:92 allows it when the owner is meant to use the window; asked in §11 Q2).
- **Displays.** While it runs, the testbed holds `SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED)`, so the screens stay on (§8.1).
- The process is per-monitor-v2 DPI aware (set at start, before any window), so the swap chain maps 1:1 to pixels (R08:767). The cursor is hidden over the window.
- The window title is "Kairo testbed". The testbed never sends input (CL:10) and never opens a capture device or the mic (M1:15).

### 4.2 Monitor and adapter

- `--monitor <monitorDevicePath>` is required; there is no "primary" and no index (M1:54, R08:770). A default could put patterns on the wrong screen, the OLED included.
- `kairo-display` resolves the path to the HMONITOR, rect, refresh and adapter LUID. D3D11 is created on the `IDXGIAdapter1` with that LUID, and the LUID is logged (M1:63).
- Exit 3 if the path isn't found.

### 4.3 Pacing and frame numbering

- **`--pace vsync` (default).** `Present(k, 0)` with k = refresh/fps and k ≤ 4, since SyncInterval runs 1–4 (MS). It refuses rates that are not within 0.5% of refresh/k: 60, 90 and 180 on the 180 Hz monitor; 90, 120, 180 and 360 at 360 Hz. 60 and 72 at 360 Hz need `--pace timer`.
- **`--pace timer`.** A `CreateWaitableTimerExW(HIGH_RESOLUTION)` on absolute QPC slots from core/timeline.rs's tick maths, then `Present(0, 0)` with flip-discard and no tearing flag. Composition shows the newest frame each vblank. Rates are 60–360 fps.
- The process opts out of EcoQoS (CB:57, P16).
- **Frame number N is time-based.** N is the index of the vblank group (vsync) or the slot (timer) the frame targets. A late frame shows up as a skipped N plus an `m` log line, the way a game's clock keeps running, so a testbed miss is never mistaken for a capture drop.
- **Start.** Frame 0 is presented at the first vblank (vsync) or slot (timer) at or after the next whole QPC second. That anchors the flash and the game beep to the voice beep's QPC grid (§2 item 8).
- **Swap chain.** `FLIP_DISCARD`, 2 buffers, a frame-latency waitable object, `SetMaximumFrameLatency(1)`. Format: BGRA8, or `R16G16B16A16_FLOAT` with `SetColorSpace1(RGB_FULL_G10_NONE_P709)` in HDR mode (R08:666–669).

### 4.4 Rendering

- Every element of pattern v1 is either a solid rectangle or a horizontal 1-D band. There is no HLSL and nothing from OBS (CL:17, M1:192).
  - **Background:** `ClearRenderTargetView` black, so OLED pixels are off.
  - **Solid rectangles** (barcode cells, flash, patches): `ID3D11DeviceContext1::ClearView` on the back buffer's RTV with one rect list per colour.
  - **1-D bands** (bars, ramp): each frame the CPU evaluates one row with `testsig` at device pixel centres, uploads it with `UpdateSubresource` into a W×1 texture of the back-buffer format, and fills the band by doubling `CopySubresourceRegion` (1 → 2 → 4 … rows).
  - **Load band:** `CopySubresourceRegion` from a texture made once, one period of the band wide: 512·H/gcd(H, 135) px (4,096 at 1080p, 8,192 at 720p, 16,384 at 1440p), read with a moving source box that splits into two copies where it wraps (the motion pattern copies each of its 12 block rows that way). Every frame's band is a window of that texture, whole pixels included. Heights whose period is above D3D11's 16,384 px (1200 and 1600, for example) have the CPU evaluate the 12 block rows each frame instead. Per-frame upload is about 20 KB.
- **Pixel rounding.** Every u→px conversion uses `testsig::px(x_u) = round_half_away(x_u·H/1080)`, everywhere.
- **WARP test** (Windows CI, PLAN:342). The same ops on a WARP device and an offscreen texture must equal `testsig`'s CPU raster byte for byte, for N in {0, 1, 179, 180, 181, 5000}, at 1920×1080 and 2560×1440.
- **[Unverified]** `ClearView` with rects and copies into a flip-model back buffer on the AMD driver. Fallback: copy from pre-made solid tiles, or a 10-line pass-through shader that only `Load`s those textures (runtime `D3DCompile`, CB:377). Either way no pattern maths lives in HLSL.

### 4.5 Pattern v1: geometry

1 u = H/1080 px. x is measured from the client's left edge; W_u = W/u. Elements centred horizontally are placed at W_u/2 ± half-width, so a 9:16 centre crop (W_u/2 ± 303.75 u) keeps the barcodes, the flash and at least two full bars (PLAN:238, PLAN:245).

| Element | y (u) | x (u) | Content |
|---|---|---|---|
| Barcode A | 32–128 | centred, 384 wide | §4.6 |
| Bars | 160–352 | full width | 8 SMPTE bars at 75% (sRGB 191): white, yellow, cyan, green, magenta, red, blue, black; 160 u each (period 1,280 u); 32 u raised-cosine transitions; scroll **+3 u per frame** |
| Ramp | 352–416 | full width | grey triangle 0→255→0 over 1,024 u; scroll **−2 u per frame** |
| Flash | 448–640 | centred, 192 × 192 | white while `N mod F < round(0.1·F)`, else black. F = nominal fps |
| Patches | 672–800 | row of 12 × 128 u, pitch 160 u | drifts **+1 u per frame** and wraps over max(W_u, 1,920 u). SDR: sRGB grey codes 0, 8, 16, 32, 64, 96, 128, 160, 191, 224, 240, 255. HDR: 0, 10, 40, 80, 120, 160, 203, W, 400, 600, 1,000 and 4,000 nits |
| Load band | 832–928 | full width | `--load scroll` (default): seeded random 8 u RGB blocks (seed `0x4B41_4952_4F00_0001`), scroll **+5 u per frame**. `churn`: new random blocks every frame (stress runs only). `none` |
| Barcode B | 952–1048 | centred | same code as A |

- **Why soft, moving bars.** The 32 u edges make any reasonable scaler reproduce the pattern: modelled error ≤0.07 code for 1440p→1080p, and the analytic reference matches a lossless pipeline at 59–62 dB [Mac]. Static bars would be coded once and then skipped, so P-frame corruption would go unseen.
- **Why the load band.** `scroll` makes the encoder spend real bits (7.8 of 8 Mbps) while the bars stay ≥49.3 dB [Mac]. The seeded texture is also M8's byte-search needle (PLAN:503); flat black and white would match everywhere.
- **Conversions used by `synth` and the verifier.** sRGB codes → BT.709 limited (CB:347–351; R09:365–382, e.g. 75% yellow = 168/44/136). 4:2:0 chroma follows `chroma_sample_loc_type` 0 (ITU-T H.273; H.264 Annex E): co-sited with the even luma columns and between rows vertically, filtered [1 2 1]/4 horizontally and [1 1]/2 vertically. `testsig` writes that kernel from those definitions; nothing comes from CB §3.3 or R09 §3.3, which are derived from OBS (CB:356–357, R09:390–392; CL:17, PLAN:314). The SDR desktop is sRGB-coded with the 709 matrix, with no transfer conversion (R09:384).
- **`--pattern motion`** (gate 3 and the OLED) is the same scene without the barcodes and the flash, scrolled down 1 u per frame (wrapping) so no row keeps a band. Its patches stop at SDR white unless `--hdr-patches` is given (an SP3 run that asks for them).

### 4.6 Barcode encoding

| Item | Rule |
|---|---|
| Grid | 16 × 4 cells of 24 u (32 px at 1440p, 24 at 1080p, 16 at 720p, 12 at 540p). Black is sRGB 0 and white sRGB 255 (Y 16/235, Cb = Cr = 128), so 4:2:0 can't touch it. In HDR, white is SDR white |
| Row 0 | References: cell j is white iff (j + N) is even. The phase flips every frame; the decoder learns N's parity from it |
| Rows 1–3 | 48 bits, MSB first, row-major, 1 = white. Bits 0–7 hold N mod 256, raw. Bits 8–47 hold (N[23:8] ‖ hdr ‖ tag[6:0] ‖ crc16) XOR `mask40(N mod 256)` |
| `crc16` | CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF) over the bytes `0x4B 0x01 N[23:16] N[15:8] N[7:0] (hdr<<7 \| tag)`. The prefix is "K" plus format version 1; it isn't drawn, so another layout version fails the CRC instead of misreading |
| `mask40(b)` | `splitmix64(0x4B01_0000_0000_0000 \| b) >> 24`, low 40 bits |
| Range | 24 bits wrap after 12.9 h at 360 fps; the verifier unwraps: `N_i = N_{i−1} + ((n_i − n_{i−1} + 2²³) mod 2²⁴) − 2²³` |
| Run tag | 7 bits, random per run, in the log and `expect.json`. It catches a clip verified against the wrong log |

**Decoding** (signals.md §1.2, adapted for the flipping reference row):
1. **Geometry.** By default the clip frame is the testbed's client area scaled to the clip; otherwise `--testbed-rect x,y,w,h` or a known export crop. A wrong geometry fails every frame with "barcode not found: check geometry", not as drops.
2. **Cell value.** The mean luma of each cell's central 50% × 50% (at least 1 px).
3. **References.**
   - The phase comes from whichever reference cells (even or odd positions) are brighter.
   - W and B are the medians of the white and black reference cells.
   - The copy is unreadable if W − B < 110 codes. T = (W + B)/2, and every reference cell must fall on its expected side of T.
4. **Bits and checks.** Bit = cell > T. Unwhiten, check the CRC, and require parity(N) = phase. Margin = min \|cell − T\| / ((W − B)/2); the minimum is reported, with a warning below 0.5.
5. **Two copies.** A frame is readable if either copy passes. If both pass and disagree, the frame is **torn** (labelled with both values).

### 4.7 Audio (the 1 kHz game beep)

- **Signal.** 1 kHz sine, phase 0 at onset, **−30 dBFS peak** (amplitude 0.0316, CL:26, M1:93), 100 ms with 2 ms raised-cosine ramps, both channels. Silence between beeps.
- **Device.** WASAPI shared, event-driven, 48 kHz f32 stereo, on the console-role default device. It never switches devices (M1:41).
- **It plays in the testbed's own process,** so an include-tree on the testbed's PID captures it as "Game" in M3 (CB:1192).
- **Aiming.** Beep m targets the predicted display time of flash frame N_f = F·m: the last known vblank plus whole refresh periods. The sample index is computed from `AudioClock::get_position() → (pos, qpc)` (wasapi 0.26.0 api.rs:1999) at least 200 ms ahead.
- **Logging.** The target time, the played time (mapped from the device position) and the flash frame's display time all go to the log.
- **Audio failure** (no device) logs `e audio_failed` and the video carries on.
- **[Unverified]** How IAudioClock's play time relates to the timestamps process loopback puts on the same samples. M3 task 9 measures that constant (CB:1193).

### 4.8 HDR

- **`--hdr auto` (default).** FP16 scRGB when the monitor's `IDXGIOutput6::GetDesc1().ColorSpace` is G2084, else BGRA8.
- **In HDR mode:**
  - The SDR-style elements use sRGB-decoded values × W/80, as Windows composes SDR apps.
  - The patches show nits/80 (1.0 = 80 nits, R08:667), capped at W in `--pattern motion` unless `--hdr-patches`.
  - W is the OS SDR white level (`SDRWhiteLevel/1000 × 80`, R09:462). L_w is `MaxLuminance`, clamped to 400–4,000 (R09:463).
  - The barcode's `hdr` bit is set, and the log records W and L_w.
- **HDR toggles** (M5 G2) recreate the swap chain, set the `hdr` bit from the next frame and log `e hdr_changed`.
- **Expected SDR codes** come from the published PQ and BT.2390 formulas only (M1:192). At L_w 1,000 and W 240 they reproduce the plan's Natural ≈ Y 215 and Match SDR apps Y 235 (PLAN:430; the table is in signals.md §4.2).

### 4.9 QPC log (`testbed.log`)

```
# kairo-testbed 1
h pattern=1 run=93 mode=borderless pace=vsync fps=180 F=180 size=2560x1440 refresh_mhz=179960 hdr=0 sdr_white_nits=- max_nits=- start_hns=<QPC of frame 0>
f <N> <target_hns> <present_hns> <display_hns|->
b <m> <N_f> <target_hns> <played_hns|->
m <N> vblank|slot
e <name> [k=v …]   (ready, activate result=ok|failed, fse result=ok|failed, fullscreen_lost, hdr_changed,
                    device_lost, audio_failed, audio_device_changed, end reason=…)
```

- **Times** are absolute QPC in 100 ns (core/timeline.rs:88).
  - `display`: the vblank where frame N first appeared, from `GetFrameStatistics` (PresentCount → SyncQPCTime); `-` if unknown or never shown.
  - **[Unverified]** for composed borderless and windowed modes. The fallback is the vblank grid from `DwmGetCompositionTimingInfo`.
- **Writing.** Each line goes out in one `write_all`. The parser drops a torn last line, as `Trace::parse` does (core/trace.rs:130–165).
- **Privacy.** No device paths, monitor, adapter or PC names, window titles or wall-clock time (CL:16). It is a raw result: `C:\kairo\logs` and `.dogfood/` only (M1:72–74). A committed fixture would be rebased to 0 and needs the owner's OK (M1:195).

### 4.10 CLI, ready file and exit codes

```
testbed.exe --monitor <monitorDevicePath> [--mode borderless|windowed|exclusive] [--size WxH] [--activate]
            [--fps N] [--pace vsync|timer] [--seconds S] [--pattern full|motion] [--load scroll|churn|none]
            [--hdr auto|off] [--hdr-patches] [--beep on|off] [--run-tag 0..127] [--log FILE] [--ready FILE]
            [--stop-on-stdin-eof]
```

- `--fps` defaults to the rounded refresh rate. `--seconds` defaults to 60 and has no cap of its own: devrun's `max_seconds` bounds every run (soaks with verified saves run for hours, PLAN:345), and the OLED's 600 s cap is enforced where the OLED is known (§4.11).
- The testbed exits at `--seconds`, at WM_CLOSE, or at EOF on stdin only with `--stop-on-stdin-eof`, which the harness passes along with a piped stdin. devrun starts every job with a NUL stdin (devrun/run.rs:160), which reads as EOF at once.
- **`ready.json`** is written after the first present: pid, hwnd, client rect, refresh in mHz, adapter LUID, DXGI adapter and output index (valid in this process only), format, mode, fps and `start_hns`. It stays local.
- **Exit codes:** 0 ok · 2 usage · 3 monitor not found or rate refused · 4 device or swap-chain failure · 5 focus or exclusive fullscreen not obtained.

### 4.11 Screens, OLED and the 180 Hz monitor

- Patterns and harness runs go on the 180 Hz monitor (CL:26, M1:93).
- **The full pattern never runs on the OLED**, in M1 or later. Its barcodes and flash sit at fixed places, so they would wear rectangles into the panel even though every cell flips. Every job writer that can target the `oled` alias (`pc harness`) refuses `capture` and `corpus` there, forces `--pattern motion`, caps the run at 600 s and passes `--oled` to `kairo-harness`, which checks the same. OLED runs happen only on agreed days (M1:39, M1:193).
- The 180 Hz monitor must be in SDR for `capture` runs in M1, because the expected codes are SDR: the harness exits 3 if it's in HDR.

### 4.12 PC acceptance (M1:131)

- Each run lasts 20 s on the 180 Hz monitor:
  - borderless at 60, 90 and 180 (vsync);
  - windowed 1280×720 at 60 and 180;
  - borderless timer at 240 and 360;
  - in the attended window, with `--activate` (after its ≥205 s wait): exclusive at 180, and M1:159's checks of the borderless and exclusive testbed as the foreground window, read through `pc status --json`.
- **Pass:**
  - missed vblanks ≤0.1% at fps ≤ refresh in borderless or exclusive mode;
  - display times present and advancing at the refresh;
  - beep played − target within ±1 ms [Estimate];
  - `e activate result=ok` and `e fse result=ok` on the exclusive row;
  - gate 2's capture reads ≥99.9% of barcodes ("a barcode that survives H.264").

---

## 5. The voice-tone process (`kairo-tone.exe`)

```
kairo-tone.exe beeps [--hz 2000] [--offset-ms 500] [--seconds S] [--dbfs -30] [--run-tag N] [--log FILE] [--stop-on-stdin-eof]
```

- **Signal.** A 2 kHz beep, same envelope as the game beep, starting at the absolute QPC whole second + 500 ms.
  - `--dbfs` above −30 is refused (CL:26).
  - On the half-second grid, even 2nd-harmonic distortion of the 1 kHz beep (measured −87 to −107 dB [Mac]) lands at game-beep times, never voice times.
- **Device.** WASAPI shared render only, console default device, the same `audio` module as the testbed.
- **Own image and PID** (M1:55). It exits at `--seconds`, or at EOF on stdin with `--stop-on-stdin-eof` (§4.10).
- **Log** (`tone.log`): `# kairo-tone 1`, an `h` header line, then `v <k> <target_hns> <played_hns|->`, plus `e` lines.
- **Process tree: always a sibling of the testbed, never its child.** The harness starts both. An include-tree on the testbed's PID would otherwise capture the voice tone as Game (PLAN:185, CB:1192).
- **M3 contract items** for the engine harness:
  1. The game-root walk-up "within the install folder" (PLAN:185, CB:772–773) would climb from `testbed.exe` to `kairo-harness.exe`, which sits in the same folder, and from there include `kairo-tone`. The harness hands the engine the testbed's PID explicitly.
  2. Voice chat captures by an image list (PLAN:187), so the harness hands the engine `kairo-tone`'s PID for Voice chat (or adds `kairo-tone.exe` to a dev-only voice list).
  3. A save's cue plays at the save, after the clip's end (PLAN:179; post-roll 0 s, PLAN:264), so "cue absent" in a single clip is vacuous. The harness saves twice, 10 s apart, and checks the second clip (signals.md:307).
- **In M1** it is built, unit-tested and accepted in PR 8's attended run (log only). Gate 2 doesn't start it (video-only); `kairo-harness run --voice` exists from day one.

---

## 6. kairo-verify

### 6.1 Inputs and CLI

```
kairo-verify check <clip.mp4> --profile test|capture|smoke|engine|share|discord|whatsapp|reels|original [--gate m3]
      [--expect FILE] [--testbed-log FILE] [--voice-log FILE] [--cue FILE.wav] [--av-tight] [--voice-included]
      [--decoder linked|cli|none] [--ffmpeg PATH] [--json FILE|-] [--summary FILE|-]
kairo-verify synth --preset 1080p60|1440p60|WxH@fps [--source-hz 180] [--seconds N] [--defect D]
      [--hdr natural|match --lw NITS --sdr-white NITS] [--codec nv12|ipcm [--gop N]] --wav-dir DIR --expect FILE
      (frames on stdout)
kairo-verify mutate undo-faststart|add-elst|set-colr P T M R|remove-colr|set-default TRACK|perturb-stts S D  IN OUT
kairo-verify corpus build --encoder h264_amf|h264_mf|h264_videotoolbox|ipcm --ffmpeg PATH [--hw-index N] --out DIR
kairo-verify corpus check DIR [--ffprobe PATH]
kairo-verify boxes <clip.mp4>      (box tree; file name and `ilst` keys only; replaces prototypes/scripts/mp4boxes.py)
```

- **`synth --hdr`** renders the HDR patch row at the chosen curve's SDR codes (Natural: PQ + BT.2390; Match SDR apps: clip) and leaves every other element at its SDR codes, so it tests the patch check alone. It sets the barcode's `hdr` bit and writes `expect.hdr`.
- **`perturb-stts S D`** moves D ticks from sample S + 1 to sample S, so the total duration stays the same.
- **Default decoder:** `linked` on Windows; `cli` when `--ffmpeg` is given in a `dev-tools` build elsewhere; otherwise `none` (structure only). A required content check that can't run gives verdict `error` and exit 3.
- **`expect.json`** (written by `synth`, the harness, or later the engine harness from the save request):

  ```json
  {"kairo_expect":1,"pattern":1,"size":[1920,1080],"fps":60,"source_hz":180,"seconds":30,"run_tag":0,
   "hdr":null,"trim":null,
   "tracks":[{"title":"Mix","channels":2,"default":true,
              "tones":{"game":"present","voice":"present","mic":"present","cue":"absent"}},
             {"title":"Game","channels":2,"default":false,
              "tones":{"game":"present","voice":"absent","mic":"absent","cue":"absent"}}]}
  ```

  - `hdr` is `{"curve","lw_nits","sdr_white_nits"}` for HDR-sourced clips.
  - `trim` is `{"first_N","last_N"}` for a Precise-trim export (M7).
  - Tone values are `present`, `absent` or `any` (report-only).
- **Without `--expect`:**
  - `engine` uses PLAN:446's defaults (30 s, Mix + Game + Mic) and runs content checks only if the barcode is found in the first 60 frames. Otherwise they report `skip: no testbed pattern`, which is M4's real game clips.
  - `test` requires `--expect`.
- **`--ffprobe`** is a PC-only parser cross-check. It compares per-packet pts, dts and key flags and the stream fields with ffprobe's JSON; differences are warnings. `ffprobe.exe` never ships (M1:70).

### 6.2 Checks, part 1: structure (own parser, no decoding)

`container.structure` gates the rest: if it fails, the other structure checks report `skip`. Rules and fixtures are in container.md §6; the essentials:

| Id (container.md) | Rule | Sources |
|---|---|---|
| `container.structure` (C0) | `ftyp` first; valid headers; no box overruns; exactly one `moov`; ≥1 `mdat`; no `moof`/`mvex`/`mfra` (fragmented = fail, exit 1); one of each required `stbl` box; tables agree (N = stsz = Σ stts = Σ ctts); one self-contained `dref`; every sample's bytes inside an `mdat` | PLAN:179, PLAN:271 |
| `container.faststart` (C1) | `moov` before the first `mdat`; only `free`, `skip` or `wide` between `ftyp` and `moov` | PLAN:179, R06:132 |
| `container.no_editlist` (C2) | no `edts` in any track; identity edits fail too | PLAN:179, R09:590, PLAN:237 |
| `video.colr` (C3) | exactly one `colr` of type `nclx`, 1/1/1, full_range 0 | M1:12, R09:683–686 |
| `video.params` (C4) | VUI colour = `colr` when both exist (no `colr`: C3's job); a VUI with no colour description fails in `test`/`capture` and warns elsewhere (CB:478); in-band SPS = `avcC` SPS (bytes differ but checked fields equal: warning); High (100); 4:2:0; 8-bit; `frame_mbs_only`; cropped size = `avc1` = `tkhd`, even; SAR unspecified or 1:1, and `pasp` absent or 1:1; CABAC (`entropy_coding_mode_flag`); level below Table A-1: warning; x264/x265 SEI: fail in `test` | R09:558, CB:486, R13:801, container.md:218–219 |
| `video.cfr` (C5) | D = T/fps is an integer; samples 2…N (the last included) last exactly D; sample 1 lasts D + E | PLAN:446, PLAN:161, core/cut.rs:221 |
| `av.start_alignment` (C6) | with audio: E ∈ [⌈1,024·T/48,000⌉, ⌈2,047·T/48,000⌉] (328–656 ticks at T = 15,360) and every audio track's first delta = 1,024; video-only: E = 0; the hold (≤42.7 ms) is reported. A lead of 0–1,023 is a fail in `test`/`engine` and a warning for library clips [proposed] | PLAN:179, CB:49 (P8), core/cut.rs:7–12, :50–52; replaces R09:978–979 (M1:189) |
| `video.no_reorder` (C7) | no `ctts`, or only zeros; no B slices; `max_num_reorder_frames` is 0 *when present* (VideoToolbox writes no `bitstream_restriction`, and H.264 E.2.1 would infer MaxDpbFrames); POC rising per IDR period | PLAN:166, core/ring.rs:89, container.md:253 |
| `video.idr_start` (C8) | sample 1 is a sync sample, all-IDR, I slices, `first_mb` 0, parameter sets resolve | PLAN:179, R09:969 |
| `video.gop` (C9) | sync samples = {1, 1+G, …}, G = fps × 1 s; IDR samples = sync samples; no recovery-point SEI on a non-IDR. With `bf=0` this *is* the closed-GOP test (container.md §6 C9) | PLAN:166, CB:537 |
| `tracks.layout` (C10) | one `vide` track first; then the expected `soun` tracks in role order with `hdlr` name = `udta/name` = title (Mix, Game, Mic, Voice chat, Everything else [proposed name]); only Mix enabled; one shared non-zero `alternate_group`; channels Mix 2, Game 2, **Mic 1**, Voice chat 2, Everything else 2; no other track types | PLAN:185–191, CB:50 (P9), CB:573, CB:598 |
| `audio.format` (C11) | `mp4a`, 48 kHz, OTI 0x40, AOT 2 (an SBR-absent extension is allowed), channels = `channelcount`, 1,024-sample frames (the last may be short) | PLAN:191, R09:702–714 |
| `duration` (C12) | `mdhd` = Σ `stts`; `tkhd` and `mvhd` consistent; length per profile (`capture`, `smoke`: `expect.seconds` × fps ± 2 frames); each audio track ends within [−1 tick, +1,024 samples + 1 tick] of the video's end | PLAN:446, core/cut.rs:227–246 |
| `video.single_session` (C13) | one sample entry per track; every `stsc` index is 1 | PLAN:162 |
| `video.bitrate` (C14) | average and peak over sliding 1 s windows; >50 Mbps peak is a warning in recording profiles; export caps fail | CL:31, R13:139, PLAN:245 |
| `export.size`, `export.shape`, `export.tags` | §6.5 | PLAN:243–249 |
| `export.trim` | with `expect.trim`, the first and last barcode N fall within ±1 output frame of `first_N` and `last_N` | PLAN:496 |

### 6.3 Checks, part 2: content (decoded)

| Id | What it reads and computes | Rule |
|---|---|---|
| `decode.clean` | No decode errors: `Frame::has_decode_errors()` and `is_corrupt()` (safe in ffmpeg-next 9.0.0, util/frame/mod.rs:88–99) with the decoder's `err_detect=+crccheck+bitstream+buffer`; in the CLI backend, any decoder error line. Decoded video frames = samples. Decoded audio samples = 1,024 × packets (AAC-LC): with no edit list nothing trims the short last packet, so its surplus (1,024 − the last `stts` delta) is reported, not failed [Mac: 189 packets, Σ `stts` 193,024, decoded 193,536] | any error or other mismatch fails (R09:981) |
| `content.barcode` | §4.6 decode on every frame: readable share, torn frames, minimum margin, run tag vs log/expect | per profile (§6.5) |
| `cadence.drops_dups` | **Barcode line:** robust LS of `pts_i = α + β·N_i` over frames i ≥ 1 (frame 0 is the held P8 frame), refitted 3 times rejecting \|r − median(r)\| ≥ 2 ticks. **Steps:** ΔN between consecutive readable output frames, fed to `cadence::steps` in testbed frames at the nominal source rate F_s (log `fps`, else round(1/β)) over readable runs. Repeats and skips are judged as the excess over `cadence.judder`'s ideal (0 at `test`'s source rates). Events are listed as (output frame, N before, N after). With a log, events at testbed misses count as *source* and are excluded | per profile |
| `cadence.judder` | judder-step share (§2 item 12, exact integers on ΔN) minus the ideal: every-vblank delivery at the refresh from the barcode line (vsync: k/β; else the log's display grid; else `expect.source_hz`), replayed through `trace::replay` and classified the same way, worst of 7 phases. p99 \|step − tick\| in 100 ns is reported (from the log's display times, else β·ΔN) | §2 item 12 |
| `content.flash` | the flash patch's central-50% mean luma vs the frame's barcode midpoint (125.5 if unreadable); onset = first lit frame after an unlit one; F from the log, else round(1/β) checked against lit frames (F ± 1) | `test`: every expected flash found |
| `content.bars` | per frame, PSNR of Y, Cb and Cr over the bars and ramp bands (2 px margin at the band's top and bottom; ramp chroma must stay at 128 ± 2, a warning in exports) against the analytic float reference at clip pixel centres, `x_u = (x + 0.5)·1080/H_out`, converted to BT.709 limited 4:2:0. An unreadable frame is scored against every N between its nearest readable neighbours' N (at most 64; else the line's N̂ ± 2·F_s/fps), keeping the best, so corruption that also hits the barcode can't hide as "unreadable" | min-plane ≥40 dB on **every** frame; never averaged |
| `content.colour_matrix` | the same PSNR against BT.601-limited, BT.709-full, BT.601-full and Cb↔Cr-swapped references | fail if any beats BT.709 limited by ≥6 dB [proposed], naming it |
| `content.bars_lines` | per frame on Y: the largest \|mean residual\| of any row and of any column inside the bands, leaving 16 px at each side out of the columns: the bars scroll right and the ramp left, so new content enters there each frame, which VideoToolbox's good 1080p clip shows as 4.6 codes at x = 0 against 2.3 inside (25/40 Mbps; 3.98 inside at 8 Mbps) [Mac, PR 4b's review] | §6.5 |
| `content.levels` | SDR patches: central-50% mean Y vs 16 + 219·code/255 | ±2 codes |
| `hdr.curve_levels` | HDR patches vs PQ + BT.2390 (Natural), clip (Match SDR apps), or SP3's Reinhard, with L_w and W from the log or `expect.hdr` | ±2 codes; patches at 235 one-sided (PLAN:430) |
| `source.stability` | testbed `m` lines and `-` display times from the log | testbed misses ≤0.1% of frames [proposed], else "rerun" (the clip isn't blamed) |

**Audio** (detectors from signals.md §3.3, measured [Mac]):
- **Onset.** Demodulate with a centred 1 ms boxcar, which nulls the other tones (all multiples of 1 kHz). Take the 50% envelope crossing minus the template's own, then refine with the carrier phase over [onset + 10, onset + 90] ms. A beep counts only at ≥−60 dBFS narrowband and f₀ ± 1 Hz. Error through AAC 96–160 kbps: <1 µs.
- **Expected windows.** The game beep at each flash. Voice windows come from Voice chat or Mix onset detections, else from the voice log, else from a 1.000 s clip-time grid phased by one search (signals.md:274). The voice beep follows QPC and the flash follows the panel, 0.22 ms/s apart at 179.96 Hz, so a flash-anchored window would miss the beep after about 7 minutes of testbed time. The mic marker (+250 ms) and the cue placeholder (+750 ms) appear in synthetic clips only.

| Id | Rule |
|---|---|
| `av.offset` | Per track carrying the game beep: offset_m = a_m − (α_m + β·N_f(m)), where α_m is the 5th percentile of (pts_i − β·N_i) over frames within ±0.5 s of flash m, and N_f = F·m (or from the log). Median, maximum and every event are reported. With logs: exact A/V = median(a_m − played_m) − p05(pts_i − display(N_i)). Voice-only tracks: from the voice log, or offset(Mix) + median(Voice − Mix onsets). Tracks without the expected beep are `skip` (routing owns presence) |
| `av.offset_tight` | `engine`: median ≤10 ms, maximum ≤21.3 ms (CB:1193). Warning; required with `--av-tight` |
| `av.inter_track` | the same beep on two tracks agrees within ±0.5 ms [proposed] (one 1,024 grid, PLAN:191) |
| `audio.tone_snr` | per beep: least-squares sinusoid fit over [onset + 10, onset + 90] ms with the frequency refined; SNR = tone power / residual; the frequency from an FFT peak with parabolic interpolation. Mix uses a joint fit where windows overlap |
| `audio.routing` | the narrowband level (single-bin Hann DFT, 80 ms) of each tone in each track's windows against the expected matrix (present / absent / any); thresholds as in §2 item 16 |
| `audio.cue` | an FFT matched filter against the cue's PCM (`--cue`, or the embedded placeholder until M4 embeds the real asset with `include_bytes!`). NCC ≥0.5 [proposed] and ≥−70 dBFS: present; 0.2–0.5: warning; windows under −90 dBFS RMS count as silence. Expected absent everywhere (PLAN:213). The cue must keep its energy ≥200 Hz from 1, 2 and 3 kHz (requirement for M4) |
| `build.lgpl` | Linked backend: avutil, avcodec and avformat `license()` all contain "LGPL" (else exit 3 before analysis); the report gives their numeric versions and a SHA-256 of `configuration()`, since ffmpeg-next exposes no safe `av_version_info()` and `kairo-verify` forbids unsafe. With nothing linked (CLI or synthetic backend) it passes, and the report notes "spawned, not linked: <licence>" |

### 6.4 Routing matrices

| Configuration | Mix | Game | Mic | Voice chat | Everything else |
|---|---|---|---|---|---|
| `test` golden clips | game, voice, mic ✓; cue ✗ | game ✓; others ✗ | mic ✓; others ✗ | voice ✓; others ✗ | — |
| `engine`, voice consented, plan A | game ✓, voice ✓, cue ✗ | game ✓, voice ✗, cue ✗ | any | voice ✓, game ✗ | game ✗, voice ✗ |
| `engine`, no voice consent | game ✓, voice ✗ | as above | any | track absent | game ✗, voice ✗ |
| plan B / Win10 before consent (PLAN:188, PLAN:523) | game ✓, voice ✗, cue ✗ | — | any | — | voice ✗, cue ✗ |
| exports (one track) | game ✓; voice ✗ unless `--voice-included` (PLAN:249, PLAN:496); cue ✗ | | | | |

### 6.5 Profiles

R = required (a violation fails the clip); W = warning only; – = not run.

| Check | `test` (gate 1) | `capture` (gate 2, SP2) | `engine` (M3+) | exports |
|---|---|---|---|---|
| structure, faststart, no_editlist | R | R | R | R (`original`: `elst` W, PLAN:237) |
| colr, params, idr_start, no_reorder, gop, single_session | R | R | R (params: VUI without colour W) | R; `reels` closed GOP (R06:156) |
| cfr | R | R | R | R |
| start_alignment | R | R (E = 0) | R | R |
| tracks.layout | from expect | from expect (video only in M1) | Mix (default) + Game + Mic [+ Voice chat, Everything else] | one stereo Mix track (except `original`) |
| audio.format | R | – | R | R |
| duration | expect ± 1 frame | `expect.seconds` × fps ± 2 frames | 30 s ± 1 GOP (PLAN:446) | source ± 1 frame |
| bitrate | W >50 Mbps | W >50 | W >50 | `reels` average ≤25 Mbps R; `discord` by size |
| decode.clean | R | R | R | R |
| content.barcode | 100% readable | ≥99.9% [proposed] | ≥99.9% [proposed] | W |
| drops_dups (excess over the ideal) | 0 repeats, 0 skips, 0 backward | ≤0.5% [proposed], 0 backward | ≤0.5%, 0 backward, longest hold ≤2, max step ≤2 (core/cadence.rs:13–14) | ≤0.5% |
| judder (excess) | 0 | ≤5 pp [proposed] | ≤5 pp | W |
| bars PSNR | ≥40 dB every frame | ≥40 dB | ≥40 dB (W for HDR-sourced clips until SP3) | ≥30 dB [proposed] |
| colour_matrix | R | R | R (W for HDR-sourced clips until SP3) | R |
| bars_lines | R ≤4 codes [proposed]; PC corpus: its calibrated limit (§7.4) | W >4 | W >4 | – |
| levels | R ±2 | R ±2 | R ±2 | W |
| hdr.curve_levels | R when `expect.hdr` | – (SDR in M1) | R when HDR-sourced | W |
| av.offset | R ≤40 ms every event | – (no audio) | R ≤40 ms; tight pair W (R with `--av-tight`) | R ≤40 ms |
| av.inter_track | R ±0.5 ms [proposed] | – | R | – |
| tone_snr | R ≥35 dB, ±1 Hz [proposed] | – | R on Game and Voice chat; W on Mix with a mic | R ≥35 dB |
| routing, cue | R | – | R (Mic `any`) | R |
| source.stability | – | R ≤0.1% [proposed] | R | – |
| build.lgpl | R (linked code only) | R | R | R |
| export.trim | – | – | – | R with `expect.trim` |

- **`smoke`** (§8.7) is `capture` with drops_dups, judder and source.stability as W, and the length from `expect.seconds`.
- **`--gate m3`** (M3's timed gate, PLAN:446). The verdict uses only `container.structure` (the precondition), `duration` (30 s ± 1 GOP), `video.cfr` (60), `video.idr_start`, `video.no_reorder`, `container.faststart`, `container.no_editlist`, `video.colr`, `tracks.layout` (Mix default + Game + Mic) and `av.offset` ≤40 ms on Mix and Game; `--av-tight` adds `av.offset_tight` (task 9, CB:1193). Every other check is still reported but can't flip the verdict, so a miss on a limit Claude chose goes to an ADR, not the C# switch (M1:107).

**Export rules** (`export.*`):
- `share`: H.264 ≤1080p60; exactly one audio track (stereo Mix, enabled); `ilst` keys ⊆ {©nam, ©too}, no `desc`; faststart; no `elst` (PLAN:246, PLAN:249).
- `discord`: `share` plus ≤19,000,000 bytes and (size, fps) on the ladder 1080p60 → 1080p30/720p60 → 720p → 540p (PLAN:243).
- `whatsapp`: `share` plus ≤1080p30 and AAC 128 kbps, both [proposed] (PLAN:244).
- `reels`: `share` plus exactly 1080×1920; 23–60 fps; average ≤25 Mbps; 3 s–15 min; ≤300 MB; AAC ≤48 kHz (PLAN:245, R06:156).
- `original`: `engine` without the duration and alignment rules. It also reports whether the plain Copy rule allows handing over the original (faststart, `avc1`, all `mp4a`, no Voice chat track; PLAN:249).
- Cadence at a non-integer frame rate reports `skip`.

Every limit lives in `profile.rs` with its source string; [proposed] limits carry their Mac evidence; the report repeats both.

### 6.6 Report and exit codes

```json
{"kairo_verify":1,"tool":"0.1.0","pattern":1,"profile":"capture","verdict":"pass",
 "file":{"name":"capture.mp4","bytes":94211873},
 "decoder":{"kind":"linked","libs":{"avutil":"<x.y.z>","avcodec":"<x.y.z>","avformat":"<x.y.z>"},
            "config_sha256":"<hex>","license":"LGPL version 3 or later"},
 "summary":{"video":"h264 High 1920x1080 CFR 60, 1800 frames, peak 38.4 Mbps","audio":"none",
            "content":"1800/1800 barcodes, 0 drops, 0 dups, judder excess 0.1 pp, bars min 47.2 dB"},
 "checks":[{"id":"cadence.drops_dups","status":"pass","required":true,
            "value":{"repeats":0,"skipped":0,"backwards":0,"share":0.0,"ideal_share":0.0,"source_events":0},
            "limit":{"max_excess":0.005,"source":"PLAN:351 [proposed]"},"events":[]}],
 "timings_ms":{"parse":9,"decode":6100,"analysis":900}}
```

- **Statuses:** `pass`, `fail`, `warn`, `skip` (with a reason) and `error` (a required check couldn't run).
- **Verdict:** `fail` if any required check fails; else `error` if any is `error`; else `pass`.
- **Events:** the first 100 are listed, plus a count.
- **The text summary** fits one screen: one line each for container, video, content and audio.
- **The FFmpeg build string** (`n9.0.2-17-g2a571b6068`) comes from `KAIRO-PIN.json` into `harness.json`, not from `kairo-verify`. The example's licence string assumes the pinned LGPL build is LGPL v3, as deny:34 records.
- **Never in a report:** directories, device paths, PC, monitor or adapter names, `ilst` tag values (keys only), pixels or samples (CL:16, M1:96).
- **Exit codes** (`check`): 0 pass · 1 fail (a fragmented file included: a fragmented Kairo save is a defect) · 2 usage · 3 cannot evaluate (not an MP4, no `avc1` track, unreadable, a required decoder missing, a non-LGPL FFmpeg, a non-UTF-8 path) · 4 internal.
  - `corpus check`: 0 only if every clip behaves as its manifest says; otherwise 1.

### 6.7 Performance budget

| Step | 30 s 1440p60 clip on the PC |
|---|---|
| Parse | <50 ms |
| Decode (linked, frame threads) | 3–6 s [Estimate] |
| Analysis | ≈0.3 ms per frame [Estimate] |
| Total | **≤15 s** |

- **How analysis stays cheap.** One pass per frame sums each band's columns (Σy, Σy²) and rows (Σy). The reference is 1-D, so each reference's PSNR follows from those sums in O(width), and the row and column line errors come from the same sums. The four colour-matrix references then cost almost nothing; scoring pixel by pixel would be about 6 M operations per 1440p frame.
- Memory stays flat (streaming), so M13's 60-minute files work (PLAN:544). The loop filter is never skipped.
- On the Mac the CLI pipe moves about 5.6 GB of yuv420p for a 30 s 1080p clip: 30–60 s [Estimate].
- In-process structure-only mode (`--decoder none`) takes milliseconds. Recommendation for M3: the engine's post-save verify calls `kairo_verify::check` structure-only instead of `avformat_find_stream_info` (CB:675). It also sees moov-first, `elst`, `colr` and the GOP. M3 decides.

---

## 7. Gate 1: good and bad clips

### 7.1 The synthetic source

- `kairo-verify synth --preset 1080p60 --source-hz 180 --seconds 30` renders pattern v1 at the clip size: output frame k shows content N = k·(source/60), so 3k at 180 Hz.
- It writes NV12 to stdout (or Annex B I_PCM with `--codec ipcm`), one 48 kHz WAV per track, and `expect.json`.
- **Tracks:**

  | Track | Content | AAC |
  |---|---|---|
  | Mix (default) | game + voice + mic marker | 160 kbps stereo |
  | Game | 1 kHz at k s | 160 kbps stereo |
  | Mic | 3 kHz marker at k s + 0.25 s, mono | 128 kbps mono |
  | Voice chat | 2 kHz at k s + 0.5 s | 160 kbps stereo |

  All tones are −30 dBFS peak and never overlap, so the Mix also peaks at −30 dBFS.
- **Good clips:**
  - `good-1080p60`: source 180 Hz, Recommended 25/40 Mbps.
  - `good-1440p60`: source 360 Hz, High 36/50 Mbps (PLAN:173–174, R13:126–127).
  - `good-hdr-1080p60` (an extra): `--hdr natural --lw 1000 --sdr-white 240`, Recommended. It must pass `hdr.curve_levels`; Natural and Match differ by 5.6–20 codes at 203, 240 and 400 nits (requirements.md §4.4).

### 7.2 Encode recipes (identical mux on every encoder)

Each corpus encodes every video variant once into a video-only MP4, then muxes each clip from it with `-c:v copy` and that clip's WAVs. The intermediate has no audio lead (`stts` (N, 256)); the mux applies the P8 hold once (sample 1 = 584 ticks), exactly as a one-pass encode does. Stream-copying a *finished* clip applies it twice (912 ticks) [Mac: crit `work/from_vonly.mp4` vs `remux_audio_only.mp4`]. The mux opens no encoder session.

```
# 1. video, one encoder session per video variant
kairo-verify synth --preset 1080p60 --source-hz 180 --seconds 30 [--codec ipcm] --wav-dir . --expect good-1080p60.expect.json |
ffmpeg -hide_banner -nostdin -loglevel verbose -y [-init_hw_device d3d11va=kd:<index>]    (PC: -loglevel debug, §8.5)
  <INPUT> <VIDEO> -an -movflags +faststart -use_editlist 0 v-good-1080p60.mp4
# 2. mux, no encoder session
ffmpeg -hide_banner -nostdin -loglevel verbose -y -i v-good-1080p60.mp4
  -i mix.wav -i game.wav -i mic.wav -i voice.wav -map 0:v -map 1:a -map 2:a -map 3:a -map 4:a -c:v copy
  -c:a aac -b:a:0 160k -b:a:1 160k -b:a:2 128k -b:a:3 160k
  -metadata:s:a:0 title=Mix -metadata:s:a:0 handler_name=Mix   (likewise Game, Mic, Voice chat; language=und)
  -disposition:a:0 default -disposition:a:1 0 -disposition:a:2 0 -disposition:a:3 0
  -movflags +faststart -use_editlist 0 good-1080p60.mp4
```

`SETPARAMS` below stands for `-vf setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv`.

| Corpus | `<INPUT>` | `<VIDEO>` |
|---|---|---|
| **PC** (AMF, NV12 in system memory on the LUID device; CL:31) | `-f rawvideo -pix_fmt nv12 -video_size 1920x1080 -framerate 60 -i pipe:0` | `SETPARAMS -pix_fmt nv12 -c:v h264_amf -usage transcoding -quality balanced -rc vbr_peak -b:v 25M -maxrate 40M -bufsize 40M -g 60 -vbaq 1 -preanalysis 0 -high_motion_quality_boost_enable 0 -preencode 0 -bf 0 -max_b_frames 0 -frame_skipping 0 -filler_data 0 -coder cabac -profile:v high -header_spacing 60 -async_depth 4` (PLAN:166, CB:483–488, R13:802) |
| **Mac** (stand-in) | as PC | `SETPARAMS -c:v h264_videotoolbox -b:v 25M -maxrate 40M -g 60 -bf 0 -profile:v high` |
| **CI** (lossless, `--codec ipcm`) | `-f h264 -framerate 60 -i pipe:0` (the raw demuxer's default is 25 fps) | `-c:v copy -color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv`. A stream copy can't filter, but colour options on a copy do write `colr` [Mac] |

- **The I_PCM writer** (about 300 lines on top of the parser tests' bit writer) produces 480×270 (coded 480×272 with SPS cropping), 60 fps, 4 s clips.
  - High profile with CAVLC (`expect` sets `cabac:false`), VUI colour 1/1/1/0, `poc_type` 2.
  - IDR I slices every `--gop` frames (default 60). In between, **P slices made only of I_PCM macroblocks**, so the stream is lossless while only IDRs are sync samples.
  - Each clip is about 47 MB.
- **The resulting file** (P8, as in Kairo's saves): video sample 1 lasts 584 ticks (256 + 328), so E = 328, an audio lead of 1,024. No `elst`. A/V exact [Mac]. `corpus check` asserts E = 328 at T = 15,360 for every clip except `bframes`.
- **Spacing and log checks.** On the PC the builder runs one encode at a time, ≥1.5 s apart (M1:94). An encode fails on `has not been used for any stream`, and unless its log shows:
  - `Using device kd (type d3d11va) with h264_amf encoder.` (ffmpeg_enc.c `hw_device_setup_for_encode`) and `AMF via D3D11.` (hwcontext_amf.c:760);
  - `Selecting d3d11va adapter` / `Using device vvvv:dddd` matching the LUID's adapter (hwcontext_d3d11va.c:660, :673);
  - the mapping line `-> h264 (h264_amf)` and, under `Output #0`, a video stream line containing `nv12(tv, bt709` (line shapes checked on 8.0.1 [Mac]).
- **B-frame warnings.** `h264_amf` runs its B-frame capability block whenever `-bf` or `-max_b_frames` is set, 0 included, and warns on a GPU without H.264 B-frames or when `GetCaps` fails (amfenc_h264.c:476–526). Those warnings are fatal only for the `bframes` clip, where they trigger the `h264_mf` fallback; for `bf=0` encodes C7 on the output decides.
- **AMF's VUI colour [Unverified].** `colr` 1/1/1/0 is certain: movenc writes it from the stream's colour fields (movenc.c:2944–2952), which the CLI copies from the `setparams` frames (ffmpeg_enc.c:265–270). The VUI is the unknown, since `h264_amf` only passes the properties on (amfenc_h264.c:277–296). If the PC corpus fails C4's colour rule, the builder adds the LGPL bitstream filter `-bsf:v h264_metadata=video_full_range_flag=0:colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1` (after checking `ffmpeg -h bsf=h264_metadata`), records it in the manifest's `finishing`, adds it to the §8.4 capture command, and notes for M3 that the engine needs the same fix.

### 7.3 Bad clips: one defect each (M1:26, M1:188)

All are made from `good-1080p60`'s recipe with one change. "Synth" means the same video command with one `--defect`, a new video encode. Audio-only defects mux the good clip's video-only intermediate with their own WAVs (§7.2), so they open no encoder session. Container defects are box edits of the finished good clip. Frame positions scale with the clip: k_d = 60·⌊frames/120⌋ + 30, midway between two flashes near the middle (frame 930 of 1,800; 150 of CI's 240).

| Clip | Defect | Exact recipe | `must_fail` | `may_fail` |
|---|---|---|---|---|
| `drop1` | dropped frame | synth `drop@k_d`: N_k = 3k for k < k_d, then 3(k + 1) | `cadence.drops_dups` (1 skip) | — (after the drop the video runs one tick early, so A/V reads +16.7 ms, under 40) |
| `dup1` | duplicated frame | synth `dup@k_d`: N_k = 3k for k < k_d, then 3(k − 1) | `cadence.drops_dups` (1 repeat) | — (A/V reads −16.7 ms after it) |
| `judder` | 90/s on 180 Hz under a 60 fps pacer | synth `judder`: N_k = 3k − (k mod 2), steps of 11.1 and 22.2 ms (core/trace.rs:410) | `cadence.judder` (100% vs 0%) | — |
| `av50` | audio 50 ms late | every WAV starts with 2,400 samples of silence (end trimmed), muxed with the good video-only intermediate; the manifest requires it to measure +50 ± 1 ms | `av.offset` | — |
| `bars` | corrupted bars | synth `corrupt-bars`: 1 px luma lines (Y ±48) every 16 px, both directions, inside the bars band, every frame, applied to the encoder's input as DCC damage would be (R13:113) | `content.bars` | `content.bars_lines` |
| `bframes` | B-frames | the same synth with `-bf 2 -max_b_frames 2` on `h264_amf`; if AMF warns and disables them (amfenc_h264.c), `h264_mf -bf 2`; if that fails too, the Mac's VideoToolbox `-bf 2` clip is copied into the PC corpus | `video.no_reorder` | per encoder, filled in from its first run (VideoToolbox also fails `video.cfr`, `video.gop` and `av.start_alignment` [Mac]) |
| `nofaststart` | moov last | `mutate undo-faststart` (ftyp, mdat, moov; every `stco`/`co64` shifted) | `container.faststart` | — |
| `editlist` | edit list | `mutate add-elst`: an identity `elst` in every trak, offsets fixed | `container.no_editlist` | — |
| `colr` | wrong `colr` | `mutate set-colr 6 6 6 0` in place; the VUI untouched | `video.colr`, `video.params` | — |
| `swapped` | Game ↔ Voice chat | synth `swap-game-voice`: the Game WAV carries the 2 kHz beeps and the Voice chat WAV the 1 kHz beeps; titles in place; muxed with the good video-only intermediate. The swap the container can't see (both stereo, container.md C10) | `audio.routing` | — |

**Extras** (not gate items; one failing clip for each remaining check, M1:119):

| Clip | Recipe | `must_fail` | `may_fail` |
|---|---|---|---|
| `nocolr` | `mutate remove-colr` | `video.colr` | — (C4 compares colour only when a `colr` exists) |
| `two-defaults` | `mutate set-default Game` | `tracks.layout` | — |
| `vfr` | `mutate perturb-stts 100 128`: sample 100 lasts 384 ticks and sample 101 lasts 128, so the total is unchanged | `video.cfr` | — |
| `gop2s` | encode with `-g 120` (I_PCM: `--gop 120`) | `video.gop` | — |
| `bars-sparse` | synth, +24 every 64 px | `content.bars_lines` | `content.bars` (40.9 dB on VideoToolbox, 0.9 dB above the limit [Mac]) |
| `unreadable` | synth: barcodes garbled in 5 frames | `content.barcode` | — |
| `cue` | placeholder cue in Mix: 600 Hz for 100 ms, then 1,500 Hz for 120 ms, at +750 ms, −30 dBFS | `audio.cue` | — |
| `leak` | voice tone at −40 dBFS in Game | `audio.routing` | — |
| `noisy` | white noise at −55 dBFS RMS in Game: tone SNR ≈22 dB, while the absent windows peak at −77 dBFS, so routing still passes [crit: noisy_routing.py, re-run] | `audio.tone_snr` | — |
| `curve-swap` | synth `--hdr match`, expected `natural` | `hdr.curve_levels` | — |

Structure checks without a real-file extra are covered by the in-code builder (§9).

### 7.4 Manifest, judging and evidence

```json
{"kairo_corpus":1,"pattern":1,"encoder":"h264_amf","ffmpeg":"n9.0.2-17-g2a571b6068","finishing":[],
 "calibration":{"content.bars_lines":{"limit_codes":4.0,"source":"max(4, 2 x worst good clip)"}},
 "clips":[{"file":"good-1080p60.mp4","expect":"pass","profile":"test"},
          {"file":"drop1.mp4","defect":"drop@930","must_fail":["cadence.drops_dups"],"may_fail":[]}]}
```

- `ffmpeg` is the spawned `ffmpeg -version` line.
- `corpus check` runs `check --profile test --expect <clip>.expect.json` on every clip. It passes only if:
  - every good clip passes;
  - every bad clip fails all of its `must_fail` checks;
  - no clip fails anything outside `must_fail ∪ may_fail`;
  - every clip except `bframes` has E = 328, and `av50` measures +50 ± 1 ms.
- **PC calibration** (risk 10). The builder checks the good clips first, sets the PC corpus's line-error limit to max(4, 2 × their worst line error), records it in `calibration`, and only then judges the bad clips. The Mac and CI corpora keep 4 codes.
- **Gate-1 evidence**, one line each in the gate report (M1:209):
  1. **Mac corpus** (VideoToolbox): formal (M1:119).
  2. **PC corpus** (`h264_amf`): run in the first attended AMF session (M1:130). It also gives the AMF SPS fields, which the bit writer rebuilds as a parser test vector so no PC bytes are committed (M1:74, M1:195), plus the clean AMF PSNR floor and line error.
  3. **CI I_PCM corpus** green on `main`: every defect except `bframes`.
- **Precision assertions** (tests, not profile limits): golden clips measure \|A/V\| ≤1 ms, bars ≥55 dB (I_PCM) and margin ≥0.9.

### 7.5 Where clips live

- **Mac:** `.dogfood/corpus-mac/<stamp>/` (the newest 2 are kept).
- **PC:** `C:\kairo\logs\corpus-<stamp>\` (the harness keeps the newest 2). Only reports come back to `.dogfood/corpus-pc/<stamp>/`.
- **CI:** the runner's temp folder, never uploaded.
- **Nothing is committed** (CL:18, M1:119). The VideoToolbox SPS vector is a Mac parameter set; AMF's is rebuilt in code.

---

## 8. Gate 2: the unattended harness flow

### 8.1 Preconditions

- **Merged:** PRs 6 (linked decoder and DLL deploy), 7 (display), 8 (testbed) and 9 (harness).
- **The PC's FFmpeg re-pinned** to n9.0.2-17 (M1:156), and the first attended AMF session done (M1:130).
- **Displays on, nobody at the keyboard.** `pc status`'s IDLE needs 10 minutes without input (M1:90), and M1:35 leaves Windows' screen-off timer as it is. On a timer of 10 minutes or less the screens go dark exactly when IDLE begins [the owner's timer is Unverified]. So:
  - In the attended window the owner says "go" in chat and steps away. Claude checks `pc status` (no fullscreen window in front) and starts each job at once with `--force`, which needs the owner's OK in that session (tools/mac/pc:8). `abort_on_input` stays on.
  - `kairo-harness` and the testbed hold `ES_CONTINUOUS | ES_DISPLAY_REQUIRED` from phase 0 to exit, and phase 2 exits 3 "display off" if display times don't advance. Nothing wakes a display with input.
  - [Unverified] Whether `ES_DISPLAY_REQUIRED` wakes a display that is already dark, and whether WGC delivers while the displays sleep. PR 8's attended run checks both with the owner present.
  - Runs while the owner is away wait for the safety stops (M1:97, M1:129, M1:196) and the agreed test days (M1:35).
- `pc doctor` passes. The owner is signed in and unlocked, with no RDP (CB:1175). The 180 Hz monitor is in SDR.

### 8.2 The job (`.dogfood/jobs/`, since it names a monitor; M1:73)

```json
{"id":"harness-<stamp>","cmd":["C:\\kairo\\bin\\current\\kairo-harness.exe","run","--monitor","<180 Hz device path>",
 "--testbed-fps","180","--mode","borderless","--output","1920x1080@60","--seconds","30","--profile","capture"],
 "cwd":"C:\\kairo\\logs","max_seconds":240,"abort_on_input":true,"window":"hidden"}
```

The schema is M0's (M0:72, core/job.rs:37). devrun stops the whole job tree on owner input or on time (devrun/run.rs:187–203). It waits for every process of the job to exit, so the harness kills any leftover child before it exits.

### 8.3 `kairo-harness run`: phases and limits

| Phase | What | Limit | Typical [Estimate] |
|---|---|---|---|
| 0 setup | Hold `ES_DISPLAY_REQUIRED`; run folder `harness-<stamp>\`; prune to the newest 5; ≥2 GB free (`GetDiskFreeSpaceExW`); refuse if any process named `ffmpeg.exe` is running (a Toolhelp snapshot by name; one session at a time, M1:94); resolve device path → rect, refresh, HDR (exit 3 if HDR), LUID → DXGI index and vendor:device; pick a run tag | 5 s | <1 s |
| 1 testbed | `testbed.exe --monitor … --mode borderless --fps 180 --seconds 150 --stop-on-stdin-eof --run-tag T --log testbed.log --ready ready.json` (stdin piped); wait for `ready.json`. 150 s outlasts phases 1–3 (≤78 s) | 15 s | 1–2 s |
| 2 warm-up | Steady presents, and display times (or the DWM composition grid where those are unavailable, risk 7) advancing at the refresh ±1% (else exit 3, "display off") | 3 s | 3 s |
| 3 capture | §8.4, `-t 30`, killed 30 s after the expected end | 60 s | 32 s |
| 4 stop | Close the testbed's stdin, wait for `e end`, kill after 5 s | 5 s | <1 s |
| 5 verify | `kairo-verify check capture.mp4 --profile capture\|smoke --expect expect.json --testbed-log testbed.log --json verify.json --summary verify.txt` | 120 s | 10–15 s |
| 6 report | `harness.json`: phase times, exit codes, the adapter check, the read-back table (§8.5), the FFmpeg pin from `KAIRO-PIN.json` and its licence, verify's verdict; the summary on stdout (the job's `out.log`) | — | <1 s |

- The harness exits **0 whenever a report exists**: gate 2's check is the report, not a pass (M1:27).
- **Other exit codes:** 1 a component failed or timed out · 2 usage · 3 environment refused (monitor not found, OLED, HDR, disk, another ffmpeg, display off) · 4 encoder read-back failed (adapter mismatch, a missing device, AMF or NV12 line, an unused option) · 5 internal.

### 8.4 Capture command

```
ffmpeg.exe -hide_banner -nostdin -loglevel debug -y
  -init_hw_device d3d11va=kd:<index from the LUID> -filter_hw_device kd
  -filter_complex "gfxcapture=hwnd=<from ready.json>:max_framerate=360:capture_cursor=0:display_border=0,
     fps=60,hwdownload,format=bgra,
     scale=w=1920:h=1080:flags=bicubic+accurate_rnd+full_chroma_int+full_chroma_inp:out_color_matrix=bt709:out_range=tv:out_chroma_loc=left,
     format=nv12,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv[v]"
  -map "[v]" -t 30 -fps_mode cfr -r 60
  -pix_fmt nv12 -c:v h264_amf <the §7.2 AMF options, -usage … -async_depth 4> -b:v 25M -maxrate 40M -bufsize 40M -g 60
  -movflags +faststart -use_editlist 0 capture.mp4
```

| Choice | Why (sources) |
|---|---|
| `gfxcapture` by `hwnd` | WGC, the engine's API (PLAN:149, CL:8). Window capture keeps notifications out (PLAN:154). By handle, so FFmpeg enumerates no other window titles (CL:16) |
| `max_framerate=360` | MinUpdateInterval = 1/max_framerate (vsrc_gfxcapture_winrt.cpp:377–378): 2.78 ms, half a refresh, so every composition is eligible (R08:415). One full refresh (180) undershoots: R08 measured about 45/s at 1/60 s on 60 Hz (R08:391–399) and CB:301 forbids it. Under R08's model a 179.96 Hz panel then gives ≈115/s with 46–49% judder steps, against 180/s and ≤1.9% at 360 [crit: interval_sim.py, re-run]. `fps=60` before `hwdownload` reads back only 60 frames/s |
| One D3D11VA device | Without it, `gfxcapture` creates its device on the default adapter (winrt.cpp:1221) and the encoder on another. The CLI hands h264_amf the single d3d11va device (ff: ffmpeg_enc.c `hw_device_setup_for_encode`; ffmpeg_hw.c returns none if there are two). The log must show gfxcapture's `Using provided hw_device_ctx` (winrt.cpp:1219; otherwise `Created internal hw_device_ctx`, :1229) and `AMF via D3D11.`. The PC has an iGPU and a dGPU (M1:63) |
| swscale BT.709 limited, left-sited chroma, then NV12 | AMF gets NV12 only (CL:31). `scale_d3d11` sets no colour space, and D3D11 then defaults to BT.601 (architecture.md §2.6); the `content.colour_matrix` check would catch that. `colormatrix` is GPL-only (never used). `full_chroma_inp` + `out_chroma_loc=left` match the reference's chroma siting: Cb 60.2 / Cr 58.9 dB before encoding, against 50.5 / 51.8 dB without, with a 30.9 dB colour-matrix margin and the same speed [Mac: crit swscale_chain_results.json] |
| `setparams`, `-pix_fmt nv12` | §2 item 5: the only way the CLI tags primaries and transfer on 9.0.2; NV12 stated, not negotiated |
| `-use_editlist 0 +faststart -bf 0 -g 60`, every AMF option listed | PLAN:166, PLAN:179, CB:483–488; peak 40 Mbps ≤ 50 (CL:31) |
| Video only | Stereo Mix stays off (M1:187) |
| `ddagrab` fallback (risk 14) | `ddagrab=output_idx=<from kairo-display>:framerate=360:draw_mouse=0:offset_x=…:offset_y=…:video_size=<testbed client rect>`. Its defaults are 30 fps with the cursor drawn, its pts are arrival times (vsrc_ddagrab.c:112–117, :1161, :1232), and the crop keeps the rest of the owner's screen out of the clip (PLAN:154, CL:16). The harness refuses `ddagrab` without the crop |

- **Expected result.** Content steps of 3 testbed frames, with a ±1 testbed-frame beat blip about every 25 s on a 179.96 Hz panel [Estimate]. That is ≪5 pp of judder and no drops, so the clip should pass `capture`.
- **Optional, not a gate item.** `--capture-rate 90` sets `max_framerate=120` (8.33 ms, 1.5 refresh periods), which R08:415 says delivers 90/s at 180 Hz; the clip must fail `cadence.judder`. `max_framerate=90` would sit exactly on two refresh periods and undershoot like 180.

### 8.5 Read-back (M1:27, "every option set and read back")

The CLI can't call `av_opt_get` (CB:534), and `h264_amf` ignores the result of almost every property it sets (amfenc_h264.c:268–296, :548–590). A quiet log therefore proves little. The gate report lists each option as verified or "set, not read back":
1. **From the bitstream** (`kairo-verify`): profile High; CABAC; IDR every 60 (`-g`); no B slices (`bf`, `max_b_frames`); SPS/PPS in band at each IDR (`header_spacing`); no filler NAL units (`filler_data`); one sample per output tick (`frame_skipping`); VUI colour and `colr` 1/1/1/0; the measured 1 s peak (`rc`, `maxrate`, indirectly).
2. **From the log:** NV12 into `h264_amf` (the `Output #0` line); the single d3d11va device on the LUID-matched adapter (`Using device vvvv:dddd`, `Using provided hw_device_ctx`, `Using device kd (type d3d11va) with h264_amf encoder.`, `AMF via D3D11.`); no `has not been used for any stream`.
3. **AMF's own trace.** PC encodes run at `-loglevel debug`, the least verbose level at which FFmpeg prints AMF's trace (it logs it at `AV_LOG_DEBUG`, hwcontext_amf.c:73–75, :644–685). If the trace prints the applied properties, the harness parses them [Unverified; the first attended AMF session checks]. If debug logging changes the capture's cadence in the rehearsal, gate 2 runs at `verbose` and the corpus encode's trace (same build, options and driver) stands in.
4. **Set, not read back** until M3 task 6's `av_opt_get` (CB:1190): `usage`, `quality`, `vbaq`, `preanalysis`, `high_motion_quality_boost_enable`, `preencode`, `async_depth`, unless step 3 shows them.

B-frame capability warnings are recorded but never fatal at `bf=0` (§7.2).

### 8.6 Outputs

- `C:\kairo\logs\harness-<stamp>\` holds `harness.json`, `ready.json`, `testbed.log`, `expect.json`, `capture.mp4`, `ffmpeg.log`, `verify.json` and `verify.txt`.
- devrun's `<id>.result.json` and the out/err logs sit beside the folder (M0:73).
- `ffmpeg.log` (debug level) names the GPU model, a PC detail, and may carry paths, so it stays on the PC and in `.dogfood/`.
- The gate report gets one line with the hardware class only (M1:209, PLAN:377), plus the read-back table.

### 8.7 `tools/mac/pc harness`

```
pc harness capture [--monitor 180hz] [--seconds 30] [--testbed-fps 180] [--mode borderless|windowed|exclusive]
                   [--capture gfxcapture|ddagrab] [--capture-rate 90] [--output 1080p60|1440p60] [--voice]
                   [--smoke] [--fetch-clip] [--force]
pc harness corpus  [--presets 1080p60,1440p60] [--force]
pc harness probe   --monitor 180hz|oled --seconds N [--force] -- <probe args>      (gate 3)
pc harness displays
```

- It maps aliases to device paths through `.dogfood/pc/monitors.json`, which `displays` fills (M1:38). It refuses `oled` for `capture` and `corpus`; for `probe` on `oled` it forces `--pattern motion`, caps the run at 600 s and passes `--oled` (§4.11).
- **`probe`** writes one job of `kairo-harness probe --monitor <path> --pattern motion --seconds N -- <program args>`: start the testbed, wait for `ready.json`, run the probe from `C:\kairo\bin\current`, stop both (M1:28; a job holds one program, M0:72). Launcher work then never falls on counted `wgc-probe` days (M1:104).
- It writes the job and reuses `cmd_run`'s refusals and wait (tools/mac/pc:641).
- It fetches reports with a new read-only `remote/harness-fetch.ps1` (Base64 between markers); the clip only with `--fetch-clip`.
- **Instant Replay.** Before each job it records, read-only: `encoder-usage.ps1 -Seconds 3` (once it exists, M1:17, M1:80) and CB:1066's registry hints, shown next to the verdict. It never toggles AMD's settings (CB:1066, M0:40, R08:776).
- It reads System events 4101 and 41 since the job started (read-only). Any of them is a P0: exit 7, stop and report (M1:204).
- It prints the verdict, the failing checks and **start → report seconds**, gate 2's timing.
- **Exit codes:** those of `pc run` (tools/mac/pc:317–319), plus 7 for a TDR.
- **`--smoke`** is a 10 s capture under `--profile smoke`: the "green harness smoke" (CL:34, M1:194) and the "short recording test with AMD Software back" (M1:155).
  - **Green** = harness exit 0 and a `pass` verdict. `smoke` requires the structure checks (structure, faststart, no_editlist, colr, params, idr_start, no_reorder, gop), barcode ≥99.9%, bars, colour_matrix and levels. It reports drops, judder and source stability as warnings while the capture is the FFmpeg CLI: gfxcapture's rate isn't stable (M1:27), and one 50 ms stall in 10 s is already 0.5%. In M3 the smoke becomes a 30 s engine save under `engine`, where cadence is required.
  - **Media PRs** (M1:194): until PR 9 merges, each `wgc-probe` or `audio-soak` PR carries its own short `\Kairo\Run` smoke, the testbed plus that probe for 10 s on the 180 Hz monitor with the trace replayed through the pacer. After PR 9, a media PR's smoke is `--smoke` plus the PR's own probe run. M3's engine mode replaces both. This definition goes into M1's contract.

### 8.8 Time budget

| Step | Time [Estimate] |
|---|---|
| `pc harness`: SSH, status, job file, start the task | 3–8 s |
| Phases 0–2 | 5–6 s |
| Capture with AMF open/close and the faststart rewrite | ≈32–33 s |
| Testbed stop | <1 s |
| `kairo-verify` (linked) | 10–15 s |
| Polling and fetch | 2–5 s |
| **Total** | **≈52–68 s against 300 s** |

`max_seconds` 240 kills a hung AMF session inside the budget. A killed run has no report, so it fails the gate.

### 8.9 Failures

| Failure | Result |
|---|---|
| Owner input or a game start | Exit 6; partial outputs are kept (an MP4 without a moov is simply invalid) |
| AMF hang | Phase 3's limit, then `max_seconds`; the TDR check runs, and a TDR is P0 (M1:204) |
| AMF won't open | Exit 1, then `h264_mf`, checking which adapter it took (M1:205) |
| Adapter mismatch, missing log line, unused option | Exit 4; nothing is verified |
| Displays off (no display times) | Exit 3 "display off"; nothing wakes the screen |
| Focus refused (exclusive rows) | Testbed exit 5, reported as "not obtained"; then the one-click fallback (§4.1) |
| Testbed missed >0.1% of vblanks | `source.stability` fails: rerun, and the capture isn't blamed |

---

## 9. Tests

| Tier | Where | What | Fixtures |
|---|---|---|---|
| **U** (unit) | Mac and Linux CI (plus Windows CI) | `testsig`: barcode round trips with whitening, CRC and parity; geometry; the `px` rounding; BT.709 codes vs R09:372–382; the chroma kernel against H.273's siting; PQ and EETF vs an independent Python model (as core/cadence.rs:117 does). `testbed_log` parse, including torn lines. `cadence::steps` = `cadence()` in 100 ns at 30/60/120 fps; ΔN steps at 24–360 fps; the judder and drop ideals at 50, 75, 144, 165, 175 and 180 Hz (`design-scripts/judder_rule.py`'s table, exact fractions). **Parser:** an in-code MP4 builder with one good configuration plus one single-mutation test per structure check that leaves the others passing (gate 1 in miniature); largesize, `co64`, `size==0`, version-1 boxes; truncation at every byte of a built `moov` and seeded byte flips (no panic, always fail). **H.264:** a bit writer for SPS/PPS/slice/SEI. Golden SPS vectors: VideoToolbox's `2764001fac56805005ba6a04040204` (720p, no `aspect_ratio_info`, no `bitstream_restriction`, VUI 2/2/1 because that clip had no `setparams`, decoded here) must pass C7 and C4's SAR rule and fail C4's colour rule against `colr` 1/1/1/0; AMF's is rebuilt from the fields the first PC run reports. **Analysis:** the synthetic backend with scaling, noise, quantisation and sub-pixel shifts; drop/dup/judder sequences from `trace::replay` of vblank traces (core/trace.rs:406–413); bars on unreadable frames (corruption that also hits the barcode still fails); the column/row sums equal direct PSNR and line errors; onsets, SNR, routing and cue on synthetic PCM with noise, delay and resampling; voice windows on a drifting grid; `decode.clean` with a short last AAC packet. **`mutate`:** each operation flips exactly its own check | Built in code; no files, so nothing needs the owner's OK |
| **W** (Windows CI) | `windows-latest` | Build, clippy and tests with the linked decoder; the testbed WARP render = CPU raster; **the I_PCM corpus**: `corpus build --encoder ipcm` with the pinned BtbN `ffmpeg.exe` (real 9.0.2 muxer and native AAC), then `corpus check` with the linked decoder, on every PR (every defect except `bframes`); this also confirms the P8 first-frame hold on 9.0.2 itself | Generated at test time; never uploaded |
| **M** (Mac, opt-in) | `KAIRO_FFMPEG=/opt/homebrew/bin/ffmpeg cargo test -p kairo-verify -- --ignored` | The I_PCM corpus through the CLI backend; container.md's remux matrix (e20, b00–b10, m01–m06, regenerated at test time) to test the parser against FFmpeg's real output; **assert outcomes, never bytes** (VideoToolbox isn't byte-reproducible); a gate-2 conversion rehearsal (synth BGRA 2560×1440 → the §8.4 swscale chain → VideoToolbox) for the bars PSNR; the mux control (the two-step mux keeps 584 ticks, a finished-clip copy gives 912) | Temporary folder |
| **G** (gate corpora) | Mac (formal), PC (confirmation) | §7 | `.dogfood/`, `C:\kairo\logs` |
| **P** (PC) | `\Kairo\Run` jobs | Testbed acceptance (§4.12), the AMF corpus, gate 2 | — |

**How the Mac and Linux cover the analysis without the PC.**
- Every detector runs on frames and PCM the synthetic backend makes from the same `testsig` functions the testbed draws, with distortions from the measured worst cases.
- The Mac adds real lossy H.264 (VideoToolbox) and the real FFmpeg muxer and AAC through the CLI backend.
- Only AMF's own numbers (PSNR floor, line error, SPS shape, B-frame support) and WGC's cadence need the PC.

---

## 10. PR plan

Branches follow `m1/<slug>`; Conventional Commits with DCO; squash after CI (CL:44). Each PR adds its names to M1's contract tables and a CHANGELOG entry. Sizes are lines of code plus tests [Estimate].

**The clock.** These PRs are instrument work (M1:7, M1:104), with two edges that go into the day log's day 1 for the owner's veto (M1:109):
- `cadence::steps` extends skeleton maths, so PR 1 lands before the first `wgc-probe` commit, and later edits to kairo-core's cadence count as engine days.
- `kairo-display` (the cookbook's capture `monitors.rs` and `device.rs`, CB:106) and PR 6's FFmpeg link, CI and deploy work are built before the clock. Fixes to them after it starts count as capture or encode code.

| # | Branch | Scope | Size | Tested on | PC and owner |
|---|---|---|---|---|---|
| 1 | `m1/testsig` | `kairo-core`: `testsig` (HDR patch codes included), `testbed_log`, `cadence::steps` (any integer rate; 100 ns or testbed-frame units) | S ~1,000 | Mac, Linux CI | none |
| 2 | `m1/verify-structure` | the `kairo-verify` crate: CLI, the `mp4/`, H.264 and AAC parsers, every §6.2 check, profiles (`smoke`, `--gate m3`), report, exit codes, `boxes`; the builder tests | L ~2,500 | Mac, Linux CI | none |
| 3 | `m1/verify-mutate` | box serializer with offset fixing; `mutate`; flip tests | M ~700 | Mac, Linux CI | none |
| 4 | `m1/verify-video` | decoder trait; synthetic and CLI backends; barcode, flash, ΔN cadence and its ideals, bars (unreadable frames included), colour matrix, lines, levels, HDR curve; `synth` video, HDR patches and defects; I_PCM writer with `--gop` | L ~1,700 | Mac, Linux CI, Mac opt-in | none |
| 5 | `m1/verify-audio-corpus` | native-channel audio and (L+R)/2; onsets, voice windows from detections, A/V, inter-track, SNR, routing, cue; `decode.clean`'s audio rule; `synth` WAVs and audio defects; `--voice-log`, `--cue`, `--av-tight` and `--voice-included`; the two-step mux; `corpus build/check` with the manifest guards and PC calibration; **the Mac gate-1 corpus (formal)** | L ~1,500 | Mac, Linux CI, Mac opt-in | none |
| 6 | `m1/verify-ffmpeg` | the linked backend (quiet log, whitelists, `err_detect`) and the three-library licence guard; `dev-tools` feature; ADR-0002 and deny.toml (WTFPL exceptions, `sysinfo` ban); `ffmpeg-sys-next` with `avcodec` + `avformat` only; Windows CI download, cache and the I_PCM corpus; `cargo xwin check` on the Mac (**settles M1:181**; CB:1181); `pc deploy` copies the imported DLLs after a pin check | M ~650 + CI/ps1 | Mac (xwin), Windows CI; PC: one hidden `check` job on a copied I_PCM clip | First Claude re-pins the PC's FFmpeg between sessions (M1:156); the job runs when `pc status` is IDLE |
| 7 | `m1/display` | `kairo-display`; `kairo-harness displays`; `pc harness displays` → `.dogfood/pc/displays.json` and `monitors.json` | M ~700 | Mac (pure parts), Windows CI; PC: one hidden job | PC idle |
| 8 | `m1/testbed` | `testbed.exe` (modes, `--activate`, exclusive with `ResizeBuffers` and checks, pacing, renderer, HDR, motion scroll and SDR cap, `ES_DISPLAY_REQUIRED`, `--stop-on-stdin-eof`, log, ready file, beep) and `kairo-tone.exe beeps`; the WARP test; `kairo-verify --testbed-log` and `source.stability` (assigned here in PR 4b's review: no PR named them) | L ~2,300 | Mac (pure parts), Windows CI; PC attended (§4.12) | **Owner home, says "go", hands-off**; patterns on the 180 Hz monitor; beeps audible at −30 dBFS; settles risks 22–23 (about 15 min) |
| 9 | `m1/harness` | `kairo-harness run`, `corpus` and `probe`, its `platform` module, the read-back table, pruning; `pc harness` (capture, corpus, probe, fetch, TDR check, `--smoke`, the OLED guard, the Instant Replay record) | M ~1,700 | Windows CI; PC | One attended session: the first AMF CLI session (M1:130) → the **PC corpus** (gate-1 confirmation, about 6–10 min) → harness rehearsals (one at `verbose`, one at `debug`) → **gate 2** with the owner hands-off. Later, gate 2 again while the owner is away, once the safety stops land and test days are agreed |

- **Dependencies.** 1 → 2 → 3 → 4 → 5 → 6. PRs 7 and 8 need only PR 1, so they can run alongside 2–6 as PC time allows. PR 9 needs 5–8.
- **The devrun safety stops** (M1:97, M1:129) are a separate M1 PR; only runs while the owner is away depend on them.
- **`wgc-probe`** reuses PRs 7–9. Recommendation: start it after PR 9 merges, so its smoke and the gate-3 launcher already exist and no launcher work falls on counted days. If PC time delays PR 9, the interim smoke rule of §8.7 applies. Its first commit starts the clock (M1:103).

---

## 11. Owner questions (real decisions only)

1. **Test days** (M1:35): when may the PC run tests while you're out?
   - Recommendation: two fixed weekday windows of about an hour.
   - Until you answer, everything (gate 2 included) runs only while you're home, have said "the PC is free", and stay off the keyboard and mouse.
2. **One attended window of about 40 minutes**, at a time of your choosing (or two shorter ones, if the testbed is ready well before the harness). In it: the first AMF recording test, gate 1's PC confirmation, the testbed's checks on the 180 Hz screen (exclusive fullscreen included), and gate 2.
   - You say "go" in chat and step away from the keyboard and mouse. Claude then starts each job straight away, which needs your OK for that session, so your screens are still on. Any touch of keyboard or mouse stops the running job.
   - You'll see test patterns on the 180 Hz screen and hear quiet beeps (−30 dBFS) every second.
   - The exclusive-fullscreen check waits about 4 minutes after your last touch, because only then does Windows let a program come to the front by itself. If Windows still refuses, Claude asks you to click the test window once; that one job doesn't stop on your click.
   - Instant Replay: recommendation: leave it on. Claude records whether it is using the encoder during the jobs and never changes AMD's settings. Say if you'd rather switch it off for that half hour.
   - Recommendation: yes, any evening that suits you.
3. **Stereo Mix** (M1:187). Recommendation: leave it off; nothing changes in Windows' sound settings. Gate 2 records video only. Only answer if you want it on.
4. **Committing test clips** (M1:119). Recommendation: none, ever. CI makes its own clips at test time, and the clips stay in `.dogfood/` and `C:\kairo\logs`. The AMD encoder's settings header used in one parser test is rebuilt by code from the values the run reports, so no PC output is committed either. Nothing to approve.
5. **OLED runs** (M1:193). Recommendation:
   - a "run" is one job of at most 10 minutes;
   - the testbed's OLED pattern always moves (no barcode and no flash, ever) and stays at SDR brightness;
   - the first OLED run (gate 3) happens with you present, which the border prompt needs anyway, so no cover is needed for it.

   Please confirm.

**Later, not now.**
- M3: may harness clips include your mic track? Its gate needs "mix + game + mic" (PLAN:446). Recommendation: yes, on agreed days; Claude asks in M3.
- SP3/M5: may clip metadata hold your monitor's peak brightness and SDR white level? The HDR checks need them. It's a privacy setting.

---

## 12. Risks, fallbacks and [Unverified] items

| # | Item | How it gets settled | Fallback |
|---|---|---|---|
| 1 | AMF H.264 B-frames on the RX 9000-series card [Unverified] | The first PC corpus | `h264_mf -bf 2` [Unverified], then the Mac's VideoToolbox clip copied over |
| 2 | AMF's SPS VUI colour is 1/1/1/0 [Unverified]; `colr` is certain (movenc.c:2944–2952) | The PC corpus `video.params` | `-bsf:v h264_metadata=…` recorded in `finishing` and added to the capture command (§7.2); note for M3: the engine needs the same |
| 3 | 9.0.2's CLI only warns on unused options [Unverified; 8.0.1 warns, exit 0] | The first PC log | The warning is fatal either way |
| 4 | libclang on `windows-latest` [Unverified] | PR 6's CI | Install a pinned LLVM with a checksum |
| 5 | DXGI adapter order per process [Unverified] | `Using device vvvv:dddd`, `Using provided hw_device_ctx` and `AMF via D3D11.` on every run | Exit 4; retry with the index that matches |
| 6 | `ClearView` with rects, and copies into flip back buffers on AMD [Unverified] | PR 8's WARP test, then the attended run | Solid-tile copies, or a pass-through shader |
| 7 | Display times per frame in composed modes [Unverified] | PR 8's attended run | `DwmGetCompositionTimingInfo` grid; else `-` and log-free analysis |
| 8 | WGC delivers ≈180/s at `max_framerate=360` on the 180 Hz monitor [Unverified; modelled] | The first harness captures | Report as found; SP2 decides; not a verifier bug |
| 9 | AMF's clean bars PSNR at 25 Mbps [Unverified; VideoToolbox ≥49.3 dB] | The PC corpus | Below 43 dB: revisit load and bitrate and propose a per-preset limit in the gate report; 40 dB stays the plan's rule |
| 10 | Line-error threshold on AMF [proposed 4 codes] | The PC corpus calibrates max(4, 2 × clean) (§7.4) | A warning outside `test` |
| 11 | Beep scheduling through IAudioClock within ±1 ms [Estimate] | Every testbed log | Matters from M3 task 9 |
| 12 | Audio leads of 1,025–2,047 samples, derived from source only (C6) | M3 task 9's real cuts (CB:1193) | Widen the window with evidence |
| 13 | swscale's chroma vs the reference: 60.2 / 58.9 dB on the Mac with the §8.4 flags | The PC's first capture | Fit swscale's chroma response black-box from measured frames; never port swscale or OBS code |
| 14 | `gfxcapture` window capture of a borderless flip-model window [Unverified] | The first harness run | `ddagrab`, cropped and without the cursor (§8.4) |
| 15 | Barcode at AMF low bitrates [Unverified; 24 u has ≥2× margin on VideoToolbox] | The PC corpus margins | Bigger cells (pattern v2) (M1:200) |
| 16 | Real cue asset (M4), Reinhard variant (SP3) | Those milestones | The placeholder cue; Natural and Match only |
| 17 | The M3 harness: game-root walk-up reaching `kairo-harness`, Voice chat by image list, the cue after a clip's end | M3 harness design | §5's three contract items |
| 18 | CI time and download size | PR 6 | Cache keyed by SHA-256; the I_PCM corpus is ~0.9 GB, under 2 min [Estimate] |
| 19 | AMF hang or TDR on RX 9000 (M1:204) | `max_seconds`, ≥1.5 s spacing, the 4101 check | P0: stop and report |
| 20 | A 9.0.2 stream copy dropping `udta/name` [Unverified] | Audio is always re-encoded at the mux with titles re-specified | — |
| 21 | Raw I_PCM H.264 into the 9.0.2 muxer: timestamps at `-framerate 60`, I_PCM-only P slices not marked as keyframes, and `colr` from copy options [Unverified] | PR 6's first CI corpus (`corpus check` fails loudly) | Write the I_PCM stream as MP4 samples directly with `mutate`'s serializer |
| 22 | `SetForegroundWindow` from a task-launched process after the lock time-out [Unverified] | PR 8's attended run | The owner's one click in an `abort_on_input:false` job (§4.1) |
| 23 | `ES_DISPLAY_REQUIRED` wakes a dark display; WGC delivers while the displays sleep [Unverified] | PR 8's attended run, owner present | Attended runs start on "go" with the displays on; away runs wait for M1:196 |
| 24 | AMF's debug trace prints the applied properties [Unverified] | The first attended AMF session | Those options are listed "set, not read back" until M3 task 6 |
| 25 | `-loglevel debug` changes the capture's cadence [Unverified] | The verbose/debug rehearsal pair (PR 9) | Gate 2 at `verbose`; the corpus encode's trace stands in |

---

## Changes after critique

Each critique issue, verified against the repo, the FFmpeg 9.0.2 and crate sources, Microsoft Learn and the critiques' own scripts (re-run here where cheap). F = feasibility, C = completeness, R = rules. One piece was rejected (R10's peak-bitrate gate); the rest are fixed, a few in a different form than proposed, as noted.

| Issue | What changed |
|---|---|
| **F1** (major) Audio-only remuxes stream-copy the finished clip: frame 0 grows 584 → 912 ticks, `av50` reads +28.7 ms and passes | Fixed. Every clip is muxed from a video-only intermediate (§2 item 20, §7.2–7.3); `corpus check` asserts E = 328 and `av50` = +50 ± 1 ms (§7.4). Reproduced with `stss.py`: `from_vonly` 584, `remux_audio_only` 912 |
| **F2** (major) `max_framerate=180` = one refresh period; capture and smoke red by construction | Fixed: `max_framerate=360`; the judder demo uses 120; explicit `ddagrab` options (§2 item 22, §8.4, risk 8). Confirmed `interval = 1/max_framerate` (winrt.cpp:377); `interval_sim.py` re-run: 114–119/s vs 180/s |
| **F3** C4/C7 lost "when present" / "absent or 1:1" | Fixed (§6.2), plus the unit test (§9). Decoding the golden SPS here showed VUI 2/2/1, so the test expects C7 and the SAR rule to pass and the colour rule to fail |
| **F4** Risk 2 asks about `colr`; the unknown is AMF's VUI | Fixed: risk 2 reworded, `h264_metadata` fallback in `finishing`, M3 note, `set-colr` finishing step dropped (§7.2, §12). Confirmed movenc.c:2944–2952 and amfenc_h264.c:277–296 |
| **F5** B-frame capability warning fatal even at `bf=0` | Fixed: fatal only for the `bframes` clip (§7.2, §8.3, §8.5). Confirmed the block runs whenever `bf`/`max_b_frames` ≠ −1 (amfenc_h264.c:479) |
| **F6** The CLI "read-back" is weaker than claimed | Fixed: honest verified / "set, not read back" table; `-loglevel debug` for AMF's trace; risks 24–25 (§8.5). Bitstream checks now also cover `header_spacing`, `filler_data` and `frame_skipping` |
| **F7** Smoke flaky because cadence is required | Fixed: `smoke` profile with cadence as warnings until M3 (§6.5, §8.7) |
| **F8** Unreadable frames skip the bars check | Fixed (§6.3). Candidates come from the readable neighbours' N rather than a fixed ±2, since a stale frame can lag the line by a whole output frame; `decode.clean` uses `has_decode_errors()`/`is_corrupt()` and `err_detect` |
| **F9** CI defect positions and `gop2s` don't fit 4 s I_PCM clips | Fixed: k_d = 60·⌊frames/120⌋ + 30 (mid-gap between flashes, exact); I_PCM `--gop` (§7.2–7.3) |
| **F10** Exclusive fullscreen needs keyboard focus | Fixed with R1 (§4.1) |
| **F11** Harness Win32 calls need a `platform` module | Fixed (§3.1, §8.3). Matching `ffmpeg.exe` by name opens no process at all, so no path is read or logged |
| **F12** `build.lgpl` outcome on the Mac undefined | Fixed with R3 (§2 item 18, §6.3) |
| **F13** `ffmpeg-sys-next` defaults pull avdevice, avfilter, swscale, swresample | Fixed (§3.1); confirmed defaults via crates.io JSON in `critique-rules/net/` |
| **F14** No `av_log` control under `forbid(unsafe)`; linked avformat accepts URLs | Fixed with R9 (§3.1, §3.3) |
| **F15** "≈0.3 ms per frame" impossible pixel by pixel | Fixed: column/row sums (§6.7) |
| **F16** Default swscale chroma spends ~9 dB of headroom | Fixed: `full_chroma_inp` + `out_chroma_loc=left` (§8.4); risk 13 reduced to a PC confirmation. Numbers from `swscale_chain_results.json` |
| **F17** Adapter log check passes even if gfxcapture ignored the device | Fixed: also require `Using provided hw_device_ctx` and `AMF via D3D11.` (§7.2, §8.4–8.5) |
| **F18** `FrameRate` has only 30/60/120 | Fixed: `cadence::steps` takes integer units per second and output rate (§2 item 1) |
| **F19** Fragmented: fail or exit 3? | Fixed: fail, exit 1 (§3.3, §6.2, §6.6) |
| **C1** (major) Same as F1; audio-only remuxes also fail `duration` | Fixed with F1, which removes both effects |
| **C2** (major) `decode.clean` audio vs Σ `stts` fails every FFmpeg-made clip | Fixed: decoded = 1,024 × packets, surplus reported (§6.3) |
| **C3** `hdr.curve_levels` never meets a clip | Fixed: `synth --hdr`, `good-hdr-1080p60`, extra `curve-swap` (§6.1, §7.1, §7.3). Only the patch row goes through the curve, so the extra fails exactly one check; HDR-sourced engine clips keep bars as warnings until SP3 (signals.md:354, restored) |
| **C4** Extras fail checks outside their manifest | Fixed: `nocolr` by rule (C4 compares colour only when `colr` exists); `vfr` by a zero-sum `stts` perturbation instead of adding `duration`; `noisy` at −55 dBFS RMS (`noisy_routing.py` re-run: max −77.3 dBFS, SNR ≈22 dB); `bars-sparse` may fail `content.bars` (§7.3) |
| **C5** Cadence false-fails 144 Hz (judder) and 75/50 Hz (drops) | Fixed: exact integer ΔN classification, and drops judged as excess over the ideal (§2 items 11–12, §6.3). Recomputed: 75 Hz 25.1%, 50 Hz 16.7%, 144 Hz 0% |
| **C6** Voice windows anchored to the flash drift off after ~7 min | Fixed: from detections, the log, or a phased 1 s grid (§6.3) |
| **C7** `engine` makes Claude-chosen limits gate the C# switch | Fixed: `--gate m3` (§6.5) |
| **C8** Harness `unsafe`; process crates could break house rule 3 | Fixed with F11, plus a `sysinfo` ban in deny.toml (§3.1) |
| **C9** M1:194's interim smoke undecided | Fixed (§8.7, §10); `wgc-probe` recommended after PR 9 |
| **C10** No gate-3 launcher | Fixed: `kairo-harness probe` and `pc harness probe` in PR 9 (§3.1, §8.7) |
| **C11** VUI-without-colour fails every `h264_mf` clip | Fixed: fail in `test`/`capture`, warn elsewhere (§2 item 17, §6.2, §6.5) |
| **C12** M3 harness: vacuous cue check; Voice chat by image list | Fixed: §5's contract items 2–3 |
| **C13** `bars_lines` ≤4 could block M1 on an uncalibrated limit | Fixed: PC calibration (§7.4, §6.5) |
| **C14** `-ac 1` reads stereo 3 dB hot | Fixed: native channels, shared (L+R)/2 (§2 item 3, §3.3) |
| **C15** Claude-chosen limits not marked [proposed] | Fixed throughout §2, §6.3, §6.5 |
| **C16** SyncInterval is 1–4 | Fixed (§4.3); confirmed on MS Learn |
| **C17** 600 s cap applied everywhere | Fixed: OLED only, enforced by job writers and `--oled` (§4.10–4.11) |
| **C18** Testbed `--seconds 60` shorter than phases 1–3 | Fixed: 150 (§8.3) |
| **C19** Same as F9 | Fixed with F9 |
| **C20** Read-back covers few options; Instant Replay state unrecorded | Fixed with F6 and R16 |
| **C21** Cross-references to §6.4 | Fixed: §6.2 |
| **C22** No check for M7's "Precise trim ±1 frame" | Fixed: `expect.trim`, `export.trim` (§6.1–6.2, §6.5) |
| **R1** (major) No allowed way to give the testbed focus | Fixed: `--activate` after the foreground lock time-out, `ResizeBuffers`, state checks, exit 5, one-click fallback (§4.1, §4.12, §11 Q2, risk 22). MS Learn lists the expired time-out among the sufficient conditions but says a process can still be denied, so it stays [Unverified] |
| **R2** (major) IDLE (10 min) vs displays on | Fixed: `ES_DISPLAY_REQUIRED`, a display-on check, "go" + `--force` in the attended window (§8.1, §8.3, §11 Q2, risk 23) |
| **R3** `build.lgpl` required but only "records" on the CLI | Fixed: linked code only; the CLI backend and spawner behind `dev-tools` (§2 items 3 and 18, §6.3) |
| **R4** Chroma model cited from the OBS-derived HLSL | Fixed: H.273 / H.264 Annex E, written from scratch; risk 13's fallback reworded (§4.5, §12) |
| **R5** M1:104 misread; plumbing moved ahead of the clock silently | Fixed: corrected reading, PR 1 before the clock, edges listed for day 1's veto (§1, §2 item 1, §10) |
| **R6** Same as F11 | Fixed with F11 |
| **R7** devrun's NUL stdin makes "exit at EOF" quit at once | Fixed: `--stop-on-stdin-eof` (§4.10, §5, §8.3); confirmed devrun/run.rs:160 |
| **R8** `ddagrab` fallback records the cursor and the whole screen | Fixed: `draw_mouse=0`, crop to the client rect, refused otherwise (§8.4) |
| **R9** FFmpeg messages carry full paths; URLs open sockets | Fixed: quiet log level, whitelists, working directory + file name, `boxes` keys only (§3.1, §3.3, §6.1). Reproduced path leak in `critique-rules/exp/err.txt` |
| **R10** 10 s smoke vs "1,800 ± 2 frames"; "green" undefined | Fixed with F7: duration from `expect.seconds`, green defined and recorded in M1's contract (§6.2, §8.7). **Rejected one piece:** "peak ≤50 Mbps required in the smoke". A conforming VBV stream (maxrate 40 Mbps, 40 Mbit buffer) may hold up to 80 Mbit in a 1 s window, so a measured peak above 50 doesn't prove a setting was ignored. It stays a printed warning |
| **R11** Same as F13 | Fixed with F13 |
| **R12** Licence guard checks one library; `av_version_info()` unavailable safely | Fixed: three `license()` calls, numeric versions + config hash, pin string from `KAIRO-PIN.json` (§6.3, §6.6, §8.3); confirmed no `version_info` wrapper in ffmpeg-next 9.0.0 |
| **R13** Committing AMF's SPS bytes needs the owner's OK | Fixed: rebuilt in code from reported fields (§7.4, §9, §11 Q4) |
| **R14** Bitstream 4:2:0 doesn't prove NV12 into AMF | Fixed: `-pix_fmt nv12` and the `Output #0` / mapping-line checks (§2 item 5, §7.2, §8.4–8.5); line shapes checked on 8.0.1 |
| **R15** OLED gaps: full pattern only refused in M1; HDR patches to 4,000 nits | Fixed: permanent full-pattern ban in every job writer plus `--oled`; motion patches capped at SDR white; §1 notes OLED cadence comes from traces (§1, §4.5, §4.8, §4.11). Added: the motion scene also scrolls vertically, so no row keeps a band |
| **R16** Instant Replay unrecorded | Fixed: read-only record before each job, never toggled, asked in Q2 (§8.7, §11) |
