# Testbed signals and `kairo-verify` detectors (design, M1)

*Design draft, 2026-10-24, written against `main` = 21f0770. Nobody writes repo code from this yet. The numbers come from experiments on the Mac (Apple M2): Python + numpy render the frames and audio, `/opt/homebrew/bin/ffmpeg` 8.0.1 encodes them with `h264_videotoolbox` (standing in for `h264_amf`) and the native `aac` encoder, and Python decodes and measures them. Scripts and raw JSON are in `signals/`. No GPL encoder or filter was used. "u" is a layout unit: 1 u = client height / 1080.*

## 0. Decisions at a glance

| Signal | Recommendation | Measured basis (details below) |
|---|---|---|
| Barcode | Two identical 16 × 4 blocks of **24 u** cells (top and bottom centre). Row 0 holds the black and white reference cells. 48 data bits: 24-bit frame number, 1-bit HDR flag, 7-bit run tag, then CRC-16/CCITT-FALSE over a format prefix `K`,1 | 11,100 decoded frames at 0.7–35 Mbps actual, 540p–1440p: **0 unreadable, 0 false accepts**, worst margin 0.72. Cells of 4.5 px or less fail under starvation; cells of 6 px or more never did. 24 u is 12–32 px |
| Barcode → cadence | Fit `pts = α + β·N`. Content time = `N·β` (log-free) or the frame's display QPC (with the log). Feed it to `kairo_core::cadence::cadence` and list every event by frame | The real `kairo-core` module agrees with an exact port on decoded clips. It finds one duplicate, one drop, one backward step, and 90/s judder (p99 = 55,556.3 hns, just over one 180 Hz period) |
| Bars | Moving 75 % SMPTE bars with 32 u raised-cosine edges, plus a moving grey ramp. Regenerate them analytically from N at the output pixel centres, in BT.709 limited range 4:2:0 | Reference model vs lossless pipeline: about 59–62 dB (8-bit rounding floor) |
| PSNR rule | Every frame, min(Y, Cb, Cr) **≥ 40 dB** for recording presets. Bitrate-free relative test: fail if a BT.601, full-range or Cb↔Cr reference fits ≥ 6 dB better. Export profiles: ≥ 30 dB [proposed] | Clean at Kairo presets: ≥ 46.0 dB (Light), ≥ 49.3 dB (Recommended, High). Corrupted: 6.9–32.6 dB. Wrong matrix, range or swap: the right alternative reaches 48–55 dB |
| Testbed load | Default `--load scroll` (moving texture band); `churn` only for stress runs | `scroll` makes the encoder spend real bits (7.8 of 8 Mbps) and keeps bars ≥ 49.3 dB. `churn` starves 8 Mbps down to 40.3 dB |
| Flash | 192 u white square at the centre. Lit for `round(0.1·F)` frames when `N mod F = 0`, where F is the testbed's nominal fps | Detected in every case: lit ≥ 226, unlit ≤ 21 codes (normally 235 / 16) |
| Beeps | 1 kHz "game" and 2 kHz "voice": −30 dBFS peak, 100 ms, 2 ms raised-cosine ramps, sine phase 0 at onset. Game beep plays at the flash's predicted display instant. Voice beep plays from `voice-tone.exe` on the QPC grid at k s + 0.5 s | — |
| Beep onset | 1 ms boxcar quadrature demodulation, 50 % envelope crossing, then carrier-phase refinement | Through AAC 96–160 kbps and a 44.1 kHz resample: **< 1 µs** with phase, < 0.18 ms envelope only |
| A/V per track | Median over beeps of `audio onset − (α₀₅ + β·N_f)`, where α₀₅ is the 5th percentile of the barcode line residuals. T4 is measured through T1 or the voice log | Bias ≤ 0.36 ms. The plan's "beep vs first lit frame" method is biased by up to one frame: it read +27…31 ms on a +41.6 ms clip, so it would pass a +50 ms bad clip under ±40 ms |
| Tone SNR | ≥ 35 dB on single-source stems, and the tone frequency within ±1 Hz | Clean AAC: 41.0–52.2 dB. Faults: 11.7–33.6 dB. A 44.1/48 kHz mix-up reads 918.75 Hz |
| Routing | Narrowband level per tone per track in the beep windows. Present ≥ −60 dBFS, absent ≤ −70 dBFS, in between counts as a leak. The expected matrix comes from a routing profile | Present tones −30.0 dBFS. Absent tones ≤ −93 dBFS (noisy mic), −121.6 (harmonic), −240 (silence) |
| Kairo's cue | Matched filter against the cue's own PCM, embedded in `kairo-verify`. Present if NCC ≥ 0.5 and level ≥ −70 dBFS | No cue: NCC ≤ 0.048. Cue at −60 dBFS: 0.87; at playback level: 0.999 |
| HDR | Same drifting 12-patch row. In HDR mode it shows grey nits in scRGB; the expected SDR codes come from PQ + BT.2390 + sRGB + BT.709 | Reproduces the plan: Natural ≈ Y 215.0, Match SDR apps Y 235 at 1000 / 240 nits |
| QPC log | Text, one line per frame and per beep, plus the voice log. `kairo-verify` runs without it; with it, verify excuses testbed hiccups and measures A/V against exact ground truth | — |

## 1. Block barcode

### 1.1 What the testbed draws

Requirements: a barcode that "survives scaling and H.264" (docs/milestones/M1.md:11, docs/PLAN.md:350). The fallback is "bigger blocks" (docs/milestones/M1.md:200). Drop and duplicate checks are built from it (docs/milestones/M1.md:12).

- **Grid.** 16 columns × 4 rows of square cells, each **24 u**: 1/45 of the client height, so 32 px at 1440p, 24 px at 1080p, 16 px at 720p and 12 px at 540p. One block is 384 × 96 u.
- **Placement.** Copy A has its top edge at y = 32 u. Copy B has its bottom edge at y = 1048 u. Both are centred horizontally, so both survive the 9:16 centre crop (607.5 u wide) and the "fit" layout of vertical exports (docs/PLAN.md:238). The background around them is black.
- **Row 0: references.** White, black, white, black, … from left to right: 8 white and 8 black cells.
- **Rows 1–3: 48 data bits**, MSB first and row-major, 1 = white.
  - `payload` (32 bits) = `N mod 2²⁴` (24 bits) ‖ `hdr` (1 bit) ‖ `run_tag` (7 bits, random per run, logged).
  - `crc` = CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF) over the bytes `0x4B 0x01` + the payload as 4 bytes big-endian. The prefix is "K" plus the format version. It is not drawn, so a decoder for another layout version fails the CRC instead of misreading the frame.
  - The experiments used the same layout with an 8-bit tag (`signals/spec.py`). Splitting off the HDR flag changes nothing for robustness.
- **Colours.** sRGB 0 and 255 on grey only, which gives Y 16 and 235 with Cb = Cr = 128, so 4:2:0 cannot touch it. In HDR mode, white is SDR white (§4).
- **Range of N.** 24 bits wrap after 12.9 h at 360 fps and 77 h at 60 fps; the 8 h soaks fit (docs/PLAN.md:345). `kairo-verify` unwraps: `N(i) = N(i−1) + ((n(i) − n(i−1) + 2²³) mod 2²⁴) − 2²³`.

### 1.2 Decoding rule (`kairo-verify`)

1. **Geometry.** By default the clip frame is the testbed's client area, scaled to the clip size: borderless or fullscreen monitor capture, or window capture. Output pixel x maps to `x_u = (x + 0.5) · 1080 / H_out`. Other geometries take `--testbed-rect x,y,w,h` (clip pixels) or the export's crop transform, which Kairo knows for its own exports (docs/PLAN.md:238). If the geometry is wrong, the reference row and the CRC fail on every frame, and the report says "barcode not found: check geometry" rather than reporting drops.
2. **Cell value.** Mean luma over the central 50 % × 50 % of each cell, at least 1 px. Only luma is used.
3. **Levels.** W = median of the 8 white reference cells, B = median of the 8 black ones. If W − B < 110 codes (half the nominal 219), the block is unreadable. Threshold T = (W + B)/2. The reference row itself must decode as 1010…; that catches half-cell misregistration.
4. **Bits and CRC.** Bit = cell > T. The block is accepted only if the CRC matches. Margin = min over data cells of |cell − T| / ((W − B)/2), where 1.0 is perfect. Report the minimum margin per clip and warn below 0.5. There is no error correction: a block whose CRC fails is unreadable.
5. **Two copies.** Accept the frame if either copy passes. If both pass and disagree, the frame is unreadable, and the report labels it **"torn frame (top N₁, bottom N₂)"**: a free tear detector for exclusive-fullscreen tests.
6. **Run tag.** If a QPC log is given, the tag must match the log's run tag, which catches verifying a clip against the wrong log.

**Measured** (`signals/results/video_matrix.json`, `run_video_matrix.py`). The testbed rendered at 2560×1440 and 180 fps, captured at 60 fps (`N = 1000 + 3i`). The capture conversion was modelled on research 09 §3.1–3.3: 2-tap bilinear scaling in sRGB, then the BT.709 limited matrix (docs/research/09-encode-mux-rust.md:360) with left-sited [1 2 1]/4 × [1 1]/2 chroma. Encoding used `h264_videotoolbox`, High profile, 1 s GOP, no B-frames.

| Output, load, target (actual) | Production 24 u: unreadable / frames | Test cells (L + R blocks) unreadable: 18 / 12 / 9 / 6 u | Worst margin |
|---|---|---|---|
| 1080p, none, 8–50 Mbps (1.0–1.2) | 0 / 600 each | 0 / 0 / 0 / 0 (18–6 px) | 0.99 |
| 1080p, churn, 4.6–50 Mbps (4.8–35.1) | 0 / 600 each | 0 / 0 / 0 / 0 | 0.92 |
| 720p, churn, 1 Mbps (3.0) | 0 / 600 | 0 / 0 / 0 / **100** (6 u = 4 px) | 0.79 |
| 720p, churn, 4 Mbps (4.0) | 0 / 600 | 0 / 0 / 0 / **46** | 0.96 |
| 540p, scroll, 1.2 Mbps (0.8) | 0 / 300 | 0 / 0 / **1** (4.5 px) / **73** (3 px) | 0.93 |
| 540p, scroll, 0.6 Mbps (0.7) | 0 / 300 | 0 / 0 / **28** / **282** | 0.72 |
| 1440p, churn, 36 / 50 Mbps (35.1) | 0 / 300 | 0 / 0 / 0 / 0 (24–8 px) | 1.00 |

Across all 11,100 frames, the production copies had 0 unreadable frames and no block in any size ever passed the CRC with a wrong frame number. Cells of 6 px or more never failed; cells of 4.5 px or less failed only under starvation. **So 24 u keeps at least 2.7× linear margin at 720p and 2× at 540p.** A smaller cell gains nothing that matters, because the block already fits the centre crop.

**Mac stand-in caveat (it matters on the PC too).** Fed untagged `yuv420p`, VideoToolbox *re-matrixed* the frames as if they were BT.601. Green went from Y 133 to 154 and greys moved by −2 codes, and bars PSNR fell to 28 dB. Tagging the input (`setparams=…bt709…:range=tv`) removed the conversion; levels error then fell to ≤ 0.48 code. The bars check caught this silent colour conversion immediately, which is its job. Two consequences:
- The PC's FFmpeg CLI capture (`gfxcapture`/`ddagrab` → NV12 → `h264_amf`, docs/milestones/M1.md:27) must convert with an explicit BT.709 limited matrix and tag the stream. Otherwise the report will say "matrix: BT.601", which is correct but noisy for gate 2.
- research 12's VideoToolbox fixture script (docs/research/12-tauri-ui.md:1156–1162) has no tags. That is fine for UI fixtures; never reuse it for verify fixtures.

### 1.3 From barcodes to `kairo_core::cadence`

`cadence(content_hns, rate)` takes the content time shown at each output frame, "the testbed frames a clip's barcodes name" (crates/kairo-core/src/cadence.rs:5–9, :60).

1. Decode N for every frame (§1.2) and unwrap it.
2. **Video line.** Robust least squares `pts_i = α + β·N_i` over frames i ≥ 1, refit three times with |residual − median| < 2 ticks. Frame 0 is excluded because Kairo's muxer holds it ≤ 42.7 ms longer (docs/PLAN.md:179; crates/kairo-core/src/cut.rs:10). β is the testbed's frame period on the clip's clock: it fitted 179.962 Hz against a simulated 179.96 Hz panel.
3. **Content times.** Without a log: `content_hns_i = round(N_i · β · 10⁷)`. With the QPC log: the frame's logged display time (§5), so a frame the testbed itself showed twice shows up as the testbed's hiccup, not the capture's.
4. Call `cadence(content_hns, FrameRate::try_from(clip_fps))`. Supported rates are 30, 60 and 120 (crates/kairo-core/src/timeline.rs:46–56).
5. **Events.** Next to the statistics, list each repeat, skip and backward step with the output frame index and both testbed frame numbers. M1.md recommends listing every drop and duplicate by frame number (docs/milestones/M1.md:188).
6. **Unreadable frames.** In the `gate` profile any unreadable frame fails the clip as "cadence incomplete". In the `recording` profile, cadence runs over maximal readable runs and sums the counts, and p99 comes from the longest run.

**Measured** (`run_cadence.py`; real `kairo-core` called through the scratch crate `signals/cadence_rs`, which has a path dependency and builds outside the repo; `git status` stayed clean). 720p, 8 Mbps, 360 frames, every frame decoded exactly:

| Clip | repeats | skipped | backwards | longest hold | max step (ticks) | p99 deviation (hns) | (repeats + skipped) / steps |
|---|---|---|---|---|---|---|---|
| good (3 testbed frames per tick) | 0 | 0 | 0 | 1 | 1 | 0.67 | 0 % |
| one duplicate | 1 | 0 | 0 | 2 | 1 | 0.67 | **0.28 %** |
| one drop | 0 | 1 | 0 | 1 | 2 | 0.67 | **0.28 %** |
| 90/s judder (steps 2, 4, 2, 4 frames) | 0 | 0 | 0 | 1 | 1 | **55,556.3** | 0 % |
| two frames swapped | 0 | 2 | 1 | 1 | 2 | 0.67 | 0.56 % |

**Thresholds** (my calls; M1.md leaves them open, docs/milestones/M1.md:188–189):
- **`gate` profile** (gate 1's test clips): any repeat, skip or backward step fails, as M1.md recommends; a single drop is only 0.28 % here.
- **`recording` profile:** (repeats + skipped)/steps ≤ 0.5 % (docs/milestones/M1.md:12) and no backward step.
- **Judder.** The `gate` profile flags judder when p99 > ¼ tick (41,667 hns at 60 fps) *and* at least 5 % of steps deviate by more than ¼ tick. The 90/s clip has 50 % of steps at 5.56 ms; a good capture has rare single misses. The `recording` profile uses SP2's ADR rule once it exists (docs/PLAN.md:428), because the 90/s case sits exactly on SP2's limit, as M1.md predicted (docs/milestones/M1.md:189).

## 2. Moving bars and the PSNR check

### 2.1 What the testbed draws

- **Bars band** (y 160–352 u, full width). The 8 SMPTE colours at 75 % (sRGB 191): white, yellow, cyan, green, magenta, red, blue, black. Each bar is 160 u wide, so the pattern repeats every 1280 u. Neighbouring bars blend over a **32 u raised-cosine transition**. The band scrolls **+3 u per testbed frame**.
- **Ramp band** (y 352–416 u). A grey triangle wave from 0 to 255 and back over 1024 u, scrolling **−2 u per frame**. It catches range, clipping and banding faults.
- Both are 1-D in x and a pure function of N, so the verifier regenerates them exactly.
- **Why soft edges.** Hard edges would make the reference depend on whichever scaler the capture used. Band-limited edges make any reasonable kernel reproduce the pattern: modelled error ≤ 0.07 code for 1440p → 1080p. The analytic reference matches the lossless pipeline at about **59–62 dB**, the 8-bit rounding floor, at 720p, 1080p and 1440p.
- **Why motion.** Static bars are coded once and then skipped, so P-frame corruption would go unseen.

### 2.2 Reference and region

- **Reference.** For each output pixel, evaluate the analytic bars at `x_u = (x + 0.5)·1080/H_out`. Convert with the **BT.709 limited** matrix (docs/research/09-encode-mux-rust.md:360–382; PLAN calls for the published BT.709 formulas, docs/PLAN.md:163), then apply the capture's chroma filter, left-sited [1 2 1]/4 horizontally, to get 4:2:0. The vertical [1 1]/2 is the identity on a band that is constant in y. Keep the reference as float; do not round it.
- **Region.** Bars band rows minus a 2 px margin at the top and bottom (the hard horizontal band edges); chroma rows whose two luma rows are inside; every column. The ramp band is luma only, with the same margin; its chroma must stay at 128 ± 2.
- **Metric.** Per plane, PSNR = 10·log₁₀(255² / MSE), using the 8-bit convention of FFmpeg's `psnr` filter. **Frame PSNR = min(Y, Cb, Cr). Never average over frames:** a fault that hits 1 frame in 10 (garbage blocks) must fail the clip.
- **Frame number.** Computing PSNR needs N from the barcode. Frames without a readable N are reported as unreadable (§1.3).
- **SDR desktop transfer.** The SDR desktop is sRGB-coded and gets the BT.709 matrix like every desktop recorder (docs/research/09-encode-mux-rust.md:384). The reference therefore keeps the testbed's sRGB code values: no transfer conversion.

### 2.3 Rules

1. **Absolute, recording profiles** (Light, Recommended, High; docs/PLAN.md:173–176): **every frame ≥ 40 dB** (docs/PLAN.md:351).
2. **Relative, every profile and bitrate.** Also compute PSNR against the BT.601-limited, BT.709-full, BT.601-full and Cb↔Cr-swapped references. If any of them beats BT.709-limited by ≥ 6 dB, fail and name it: "matrix BT.601", "full range", "Cb/Cr swapped". This check works at any bitrate.
3. **Absolute, low-bitrate export profiles** (Discord 540p/720p rungs, WhatsApp; docs/PLAN.md:243–246): ≥ 30 dB [proposed]. It catches gross faults only; rule 2 covers the colour mistakes.
4. **Decode errors.** Count libavcodec decode errors per stream. Any error fails the clip, matching research 09's "decodes cleanly, every track" (docs/research/09-encode-mux-rust.md:981).
5. **Diagnostic only, never a gate.** For each frame, the largest mean residual of any row (and of any column) inside the band. On clean encodes, rows stay ≤ 0.24 code and columns ≤ 2.0 codes, except the outermost 16 px, where scrolling brings new content in each frame (4.6 codes at x = 0 on VideoToolbox at 1080p, 25/40 Mbps): kairo-verify leaves those columns out [Mac, research 16's PR 4b]. A faint +6-code gridline every 64 px scored 6.3 on columns but only 50 dB PSNR, so the report shows it. Starved encodes reach 9–22 on columns, which is why it can't gate.

### 2.4 Measured

**Clean clips** (`video_matrix.json`, `run_corrupt.py`). Columns are the minimum over all frames; the minima come from the first IDR.

| Case | Actual Mbps | Bars PSNR min Y / Cb / Cr (dB) | Median Y / Cb / Cr | Level patch max error (codes) |
|---|---|---|---|---|
| 1080p, scroll, 8 target | 7.8 | 53.9 / 49.5 / 49.7 | 58.5 / 56.0 / 57.2 | 0.48 |
| 1080p, scroll, 25 (Recommended) | 12.2 | 55.1 / 50.5 / 49.3 | 58.9 / 57.7 / 58.3 | 0.48 |
| 1080p, churn, 25 (Recommended) | 25.1 | 55.1 / 50.5 / 49.3 | 58.2 / 56.0 / 57.0 | 0.48 |
| 720p, churn, 12 (Light) | 12.0 | 54.1 / **46.0** / 47.6 | 56.2 / 47.5 / 49.9 | 0.49 |
| 1440p, churn, 36 / 50 (High) | 35.1 | 55.9 / 50.4 / 50.1 | 57.9 / 55.1 / 56.6 | 0.48 |
| 720p, none, 4 | 1.0 | 50.8 / 44.1 / 46.1 | — | 0.74 |
| 1080p, scroll, 4.6 (Discord-like) | 3.2 | 49.2 / 43.8 / 46.2 | 52.1 / 47.2 / 49.5 | 1.26 |
| 1080p, scroll, 2.5 | 1.8 | 41.9 / **38.2** / 39.6 | 45.9 / 40.6 / 42.5 | 3.0 |
| 720p, scroll, 2 | 1.4 | 42.8 / **36.4** / 39.6 | 45.1 / 39.2 / 41.5 | 2.07 |
| 540p, scroll, 1.2 | 0.8 | 42.2 / **37.5** / 39.6 | 46.5 / 40.0 / 42.6 | 2.48 |
| 540p, scroll, 0.6 | 0.7 | **32.5** / 33.4 / 34.9 | 35.1 / 35.4 / 37.8 | 8.87 |
| 1080p, churn, 8 (starved) | 8.0 | 44.9 / 40.3 / 41.7 | 47.4 / 42.7 / 44.2 | 1.87 |
| 1080p, churn, 4.6 (starved) | 4.8 | 35.4 / 34.5 / 36.9 | 37.2 / 36.0 / 38.5 | 6.0 |

**Corrupted clips.** The fault is applied to the encoder input, as DCC corruption would be (docs/research/13-amd-performance.md:113). Encoded at 25 and 8 Mbps with the scroll load, 120 frames each.

| Fault | Worst frame min-plane PSNR | Frames < 40 dB | Best-fitting reference |
|---|---|---|---|
| Clean | 49.3 | 0 / 120 | BT.709 |
| Gridlines every 32 px, +24 codes | 32.6 | 120 / 120 | BT.709 (rows 24.5) |
| Garbage 16 × 16 blocks in every 10th frame | 27.9 | **12 / 120** (exactly the hit frames) | BT.709 |
| Chroma planes shifted 8 px | 25.9 | 120 / 120 | BT.709 |
| Cb ↔ Cr swapped | 11.5 | 120 / 120 | **BT.709-swap** (55.1 / 49.3 / 50.5) |
| BT.601 matrix, tagged 709 | 24.9 | 120 / 120 | **BT.601** (54.4 / 49.1 / 49.4) |
| Full range, tagged limited | 27.2 | 120 / 120 | **BT.709-full** (54.2 / 48.4 / 50.1) |
| Bars band frozen (skip or reference bug) | 6.9 | 119 / 120 | — (no reference fits) |
| Bitstream bytes flipped (1 in 20,000) | 18.7 | 66 / 120, plus decode errors | — |
| Bitstream bytes flipped (1 in 200,000) | 49.3 in the bars; 1 frame's barcode unreadable | 0, plus decode errors | — |
| +6-code gridline every 64 px | 50.0 | 0: **missed by PSNR**; column diagnostic 6.3 | BT.709 |

**Is 40 dB reachable while corruption falls far below?** Yes, for Kairo's own presets. Clean frames stay ≥ 46.0 dB (Light) and ≥ 49.3 dB (Recommended, High). The worst clean case tested, 720p at 4 Mbps, still reached 44.1 dB. Corruption lands at ≤ 32.6 dB, so 40 dB has ≥ 6 dB of margin on each side.

It stops working below about 3 Mbps at 1080p or 2 Mbps at 720p: clean frames reach 36–38 dB there. That is why exports get the relative rule plus a 30 dB floor [proposed]. 540p at 0.6 Mbps (32.5 dB) is the limit even for that floor.

AMF may land elsewhere: the first PC run re-measures the clean floor before the threshold is frozen [Unverified for AMF].

## 3. Flash, beeps, A/V offset, tone SNR, routing and the cue

### 3.1 Testbed flash and game beep

- **Flash.** A 192 u white square centred at x = centre, y = 448–640 u. It is black when unlit. Lit while `N mod F < round(0.1·F)`, i.e. 100 ms per second. **F is the testbed's nominal fps**: the rounded refresh rate in vsync mode, or `--fps` in QPC-paced mode. It goes in the log header. Without a log, `kairo-verify` takes F = round(1/β) and checks it against the observed lit frames, trying F ± 1 if they disagree.
- **Game beep.**
  - Tone: 1 kHz, sine with phase 0 at the onset, **−30 dBFS peak** (house rule, CLAUDE.md:26; docs/milestones/M1.md:93).
  - Envelope: 100 ms with 2 ms raised-cosine ramps at both ends.
  - Path: WASAPI shared, console role, 48 kHz float stereo. Silence between beeps.
- **"Same QPC instant"** (docs/research/10-audio-rust.md:808). The beep's first sample is scheduled to *play* at the **predicted display time** of flash frame N_f. The prediction is the last known vblank QPC plus whole refresh periods, from frame statistics; the vblank is the moment the flash becomes visible.
  - Placement: from `IAudioClock::GetPosition` (position plus QPC), the testbed writes the beep at the sample index that maps to that QPC, about 200 ms ahead, beyond the shared-mode buffer.
  - Logging: the testbed logs the predicted time, the actual display QPC and the played QPC (§5), so verify can remove the testbed's own error.
  - [Unverified] how IAudioClock's play QPC relates to the QPC timestamps that process loopback stamps on the same samples. That constant is what M3 task 9 measures (docs/research/15-implementation-cookbook.md:1193).
- **Pacing.** Default `--pace vsync`: `Present(1)`, one frame per refresh on the monitor chosen by device path (docs/milestones/M1.md:54). Content time equals display time, and the flash period is F frames. `--pace qpc --fps R` covers rates that aren't the refresh rate, such as SP2's 360 fps on 180 Hz (docs/PLAN.md:428). In that mode the beep is scheduled at content time plus the measured average display latency, and the log carries the truth.
- **Long runs.** In vsync mode the flash period is F/refresh: 1.000222 s on a 179.96 Hz panel. The game beeps therefore drift against the voice process's QPC grid and overlap about every 37 min. The detectors tolerate that: §3.3's demodulator nulls the other tone, and SNR fits both tones jointly in T1.

### 3.2 Voice-tone process

- **Binary.** `voice-tone.exe`, a second `[[bin]]` of crate `kairo-testbed` (M1 lets Claude pick the name, docs/milestones/M1.md:55). It shares the testbed's WASAPI render and log code.
- **Signal.** 2 kHz, same envelope and level. **Onsets on the QPC grid at k s + 0.500 s** (QPC converted to 100 ns, crates/kairo-core/src/timeline.rs:88), so it needs no IPC with the testbed. It writes its own log (§5).
- **Process tree.** It **must not be a descendant of the testbed**: process loopback include-tree on the game captures the game's children (docs/PLAN.md:185), and the voice tone would then land on T2.
  - Recommendation: `kairo-devrun` starts both as siblings. That means a second program in the job file, a contract addition (docs/milestones/M1.md:73).
  - For M3, the engine's game-root walk-up ("walk up within the install folder; stop at launchers", docs/PLAN.md:185) must stop at `kairo-devrun.exe`. Otherwise it would pick devrun, which lives in the same folder, as the root and include the voice process [Unverified until SP4 or M3].
  - M1 itself has no engine and is unaffected.

### 3.3 Detectors

- **Flash onset** (in-house; `blackframe` is GPL-only, docs/milestones/M1.md:192, CLAUDE.md:17). Per frame, take the mean luma of the patch's central 50 %. The frame is lit if that mean is above the midpoint of the same frame's barcode white and black references; if both barcodes fail, use 125.5. Onset = first lit frame after an unlit one.
  - Measured: every flash found in every case; lit ≥ 226, unlit ≤ 21 even at 720p, 1 Mbps with churn. Normally 235 and 16.
  - Frame-accurate by construction.
- **Beep onset** (envelope plus correlation):
  1. Demodulate: `z = x·e^(−i2πf₀t)`, centred **1 ms boxcar**. One millisecond nulls every offset that is a multiple of 1 kHz, so the 1 kHz and 2 kHz detectors are blind to each other and to their 2f₀ images. A 0.5 ms boxcar for 2 kHz leaked the 1 kHz beeps at −34 dBFS in an early run; fixed.
  2. Envelope = 2|z|.
  3. Plateau = 95th percentile in a ±250 ms search window.
  4. Coarse onset = the first upward crossing of 50 % of the plateau, after ≥ 4 ms below 25 %, interpolated linearly, minus the template's own crossing time (0.953 ms at 1 kHz, 0.986 ms at 2 kHz).
  5. Fine onset = carrier phase over [onset + 10, onset + 90] ms: `τ = −(arg Σx·e^(−i2πf₀t) + π/2)/(2πf₀) mod 1/f₀`, wrapped to the cycle nearest the coarse estimate.
  6. A beep counts only if the narrowband level is ≥ −60 dBFS and the frequency is f₀ ± 1 Hz. A tone 217 Hz away (the cue's second note) tripped the wideband envelope, so frequency is checked.

**Measured onset accuracy** (`run_audio.py`, `run_audio2.py`). 59 beeps per case, onsets at random sub-sample times, compared with truth:

| Path | Envelope error max (ms) | Phase-refined error max (µs) | Tone SNR min / median (dB) |
|---|---|---|---|
| AAC 160 kbps stereo, 1 kHz / 2 kHz | 0.141 / 0.078 | 0.34 / 0.20 | 41.8 / 47.6 · 41.3 / 47.5 |
| AAC 128 kbps mono (mic settings) | 0.143 | 0.34 | 42.1 / 47.2 |
| AAC 96 kbps (export-like), 1 kHz / 2 kHz | 0.146 / 0.085 | 0.34 / 0.21 | 41.8 / 47.5 · 41.3 / 45.9 |
| 48 → 44.1 → 48 kHz resample, then AAC 160 kbps | 0.141 | 0.34 (no constant delay) | 41.8 / 47.6 |
| Beep at −50 / −55 dBFS, gate −60 dBFS | — | 0.37 / 0.17 | 41.0 / 41.4 |
| −50 dBFS mic noise in the same track | 0.175 | 0.94 | 21.0 (noise-limited) |

### 3.4 A/V offset per track

The flash-frame method of research 09 and 10 (docs/research/09-encode-mux-rust.md:993; docs/research/10-audio-rust.md:808) is quantised by the sampling delay of the output frame, 0–16.7 ms at 60 fps. In the simulation it read −22.9…−18.7 ms for a true −8 ms. On the +50 ms bad clip it read **+27…31 ms**: under 40 ms, so the bad clip would pass. **Use the barcode line instead.**

1. **Video line** (§1.3 step 2). Residuals `r_i = pts_i − α − β·N_i`. Put the "freshest frame" line at **α₀₅ = α + p05(r)**.
2. **Expected clip time of flash m:** `e_m = α₀₅ + β·N_f(m)`.
3. **Audio onset** a_m on each track: detector §3.3, searched in e_m ± 250 ms.
4. **Track offset = median_m(a_m − e_m)**. Positive means audio late. Also report max − min (drift within the clip) and two cross-checks: the "viewer" offset (median of r instead of p05) and the raw flash-frame offsets.
5. **T4 and other voice-only tracks.** With the voice log, compute `median(a_k − played_k) − p05(pts_i − display(N_i))`. Without it, compute offset(T1) + median(T4 onset − T1 onset) on matching voice beeps. If T1 carries no voice either, report "A/V unmeasurable on T4 without the voice log".
6. **Inter-track alignment.** The same beep on T1 vs T2, and on T1 vs T4, must agree within ±0.5 ms. All tracks share one 1,024-sample grid (docs/PLAN.md:191).
7. **Frame-0 hold.** Report `pts₁ − pts₀ − tick`; it must be ≤ 42.7 ms in Kairo's clips (docs/PLAN.md:179; docs/research/15-implementation-cookbook.md:678). Exclude frame 0 from the fit.

**Measured end to end** (`run_av.py`, `results/av.json`). The simulation:
- testbed vsync at 179.96 Hz, a flash every 180 frames, the game beep at the flash's display instant;
- voice beeps on the QPC grid;
- WGC sees each frame λ after display; a 60 fps QPC pacer picks the newest frame at each tick;
- five AAC tracks: T1 = game + voice + mic noise at −70 dBFS, T2 game, T3 mic noise, T4 voice, T5 silence or cue;
- 20 s, 720p, 12 Mbps.

| Case | True offset (ms) | Barcode line, T1 / T2 (ms) | Viewer (ms) | Raw flash frames (ms) | Frame-0 hold (ms) | T4 − T1 voice (µs) |
|---|---|---|---|---|---|---|
| Kairo-style mux: negative audio pre-roll, `use_editlist=0`; λ = 8 ms | −8 | −8.36 / −8.36 | −10.25 | −22.9…−18.7 | 80.7 | −0.9…+0.3 |
| Same, λ = 0 | 0 | −0.21 / −0.21 | −3.43 | −6.2…−2.0 | 80.7 | −0.9…+0.3 |
| Default edit list | −8 | −8.29 | −10.19 | −22.8…−18.6 | 16.7 | −0.8…+0.4 |
| Naive: audio from 0, `use_editlist=0` | −8 | −8.31 | −10.21 | −22.8…−18.6 | 38.0 | −0.8…+0.4 |
| Bad: audio +50 ms | +42 | **+41.64** | +39.75 | **+27.1…+31.3** | 80.7 | — |
| Bad: audio −50 ms | −58 | **−58.35** | −60.25 | −72.9…−68.7 | 80.7 | — |
| Bad: T2 ↔ T4 swapped | −8 | T2: no game beeps; T4: −8.36 | — | — | 80.7 | — |

Findings:
- The barcode line is accurate to 0.2–0.4 ms. It fitted the testbed at 179.962 Hz against a true 179.96.
- **FFmpeg 8.0.1's MP4 muxer behaves as cookbook P8 says** (docs/research/15-implementation-cookbook.md:49). With `use_editlist=0`, `make_zero` shifts every stream and lengthens video frame 0, so A/V stays exact. Here the hold is 64 ms, because the CLI's fresh encoder adds a priming packet in front of the 2,048-sample pre-roll; Kairo's mid-stream ring packets keep it ≤ 42.7 ms.
- An *audio-only* file with `use_editlist=0` decodes 1,024 samples (21.3 ms) late relative to the file start. Verify must take audio times from the decoder's frame pts and honour `elst` and skip samples. It must never assume sample 0 is at t = 0.
- **Recipe for gate 1's good clips:** audio from t = 0, `-use_editlist 0`, video in the same mux. That gives no `elst`, exact sync and a 38 ms frame-0 hold (16.7 + 21.3 ms).
- The CLI's rawvideo output duplicates the held frame 0 unless `-fps_mode passthrough` is set. `kairo-verify` reads libavcodec frames and is unaffected, but any CLI-based check must pass that flag.

**Thresholds:**
- `recording` profile: |offset| ≤ 40 ms (docs/PLAN.md:351).
- `m3` and `gate` profiles: ≤ 21.3 ms, the cookbook's M3 gate; its target is < 10 ms (docs/research/15-implementation-cookbook.md:1193).
- **Gate 1's 50 ms bad clip must shift audio away from zero**, in the direction of the good clip's measured sign. With the −8.4 ms baseline here, +50 ms lands at only +41.6 ms; on a −15 ms baseline it would be +35 ms and pass a 40 ms gate.

### 3.5 Tone SNR and frequency

- **SNR.** Least-squares sinusoid fit over the plateau [onset + 10, onset + 90] ms. The frequency is refined from the phase slope between the two halves, which absorbs resampling drift. Residual = noise + distortion. In T1, fit 1 kHz and 2 kHz jointly.
- **Frequency.** FFT peak with parabolic interpolation over the same window.
- **Rules:** **SNR ≥ 35 dB on single-source stems** (T2 and T4; T1 too when no mic is enabled) and **|f − f₀| ≤ 1 Hz**. With a mic in T1, the SNR is reported only, because it is limited by room noise: T1 measured 26–33 dB with −70 dBFS mic noise.
- This is not research 10's −50 dB soak alarm (docs/research/10-audio-rust.md:821), which judges raw PCM. AAC alone caps a −30 dBFS tone at about 41–52 dB, so the clip check sits below that. The soak keeps its own detector, and its −50 dB figure gets rechecked at −30 dBFS (docs/milestones/M1.md:190).

**Measured** (`results/audio2.json`; 1 kHz at −30 dBFS through AAC 160 kbps, 20 beeps each):

| Tone | SNR min (dB) | Frequency (Hz) | Result |
|---|---|---|---|
| Clean | 41.3 (all clean runs: 41.0–52.2) | 999.97–1000.05 | Pass |
| 5 ms dropout in each beep | 11.7 | 1000.0 | Fail |
| Crackle: sparse impulses at −40 dBFS | 33.6 | 1000.0 | Fail at 35 dB, by a small margin |
| Clipped 6 dB below peak | 12.7 | 1000.0 | Fail |
| 44.1 kHz played as 48 kHz | −55 | **918.75** | Fail on both |

### 3.6 Routing

Requirements: "routing (voice tone only on T4; Kairo's cue absent)" (docs/PLAN.md:351). The tracks: T1 mix, T2 game, T3 mic, T4 voice, T5 everything else (docs/PLAN.md:185–189). Win10 must have no voice tone and no cue in T1/T5 before consent (docs/PLAN.md:523). Shares carry no voice tone unless ticked (docs/PLAN.md:496).

- **Measurement.** Narrowband level: a single-bin DFT with a Hann window over [onset + 10, onset + 90] ms, in dBFS, for each tone in each track.
  - Game windows come from the barcode line (§3.4).
  - Voice windows come from T4 or T1 detections, else from the voice log, else a sliding 80 ms scan over the whole track that skips the game windows ± 10 ms.
- **Decision per (track, tone).**
  - *Present*: ≥ −60 dBFS in ≥ 90 % of windows.
  - *Absent*: ≤ −70 dBFS in every window.
  - Anything else is a *leak*: it fails an "absent" route and warns on a "present" one, e.g. acoustic pickup by a mic.
- **Expected matrix.** Chosen with `--routing <profile>`; the clip's JSON tag lists its tracks and roles (docs/research/09-encode-mux-rust.md:696).

| Profile | T1 Mix | T2 Game | T3 Mic | T4 Voice | T5 Else |
|---|---|---|---|---|---|
| `harness` (M3+: game = testbed, voice = voice-tone.exe consented, plan A) | 1k ✓, 2k ✓, cue ✗ | 1k ✓, 2k ✗, cue ✗ | any (mic can hear the speakers) or ✗ with headphones | 2k ✓, 1k ✗, cue ✗ | 1k ✗, 2k ✗, cue ✗ |
| `no-voice-consent` | 1k ✓, 2k ✗, cue ✗ | as above | any | track absent | 1k ✗, 2k ✗, cue ✗ |
| `plan-b` (exclude-tree on control) | 1k ✓, 2k ✓, cue ✗ | as above | any | 2k ✓ | 1k ✓, 2k ✓, cue ✗ |
| `win10` before consent | 1k ✓, **2k ✗, cue ✗** | — | any | — | **2k ✗, cue ✗** |
| `share` (exports) | single track: 1k ✓, 2k ✗ unless "include voice chat", cue ✗ | | | | |

**Measured** (`results/av.json` good clip; `results/audio.json` floors; dBFS):
- Present tones: −30.0 on every expected route.
- Absent tones:
  - T2 at 2 kHz: −240, or −121.6 in game windows (a harmonic);
  - T4 at 1 kHz: −133.7;
  - T3 mic noise (−70 dBFS RMS): −93 to −97;
  - an −60 dBFS noise track: ≤ −84;
  - T5: −240.
- The swapped clip showed 2 kHz at −30 on T2 and 1 kHz at −30 on T4, so both routes fail. Between −60 and −70 dBFS, the guard band has ≥ 23 dB of margin to every floor.

### 3.7 Recognising Kairo's cue

- **How.** Kairo's cue is a fixed asset under 300 ms, played at 60 % volume and never recorded (docs/PLAN.md:213, :260). `kairo-verify` embeds the **same PCM** (`include_bytes!` from the asset) and runs an FFT matched filter over every track. NCC = correlation / (‖template‖ · ‖window‖), with windows quieter than −90 dBFS RMS treated as silence: without that floor, AAC's exact digital silence produced absurd NCC values.
- **Rule.**
  - *Present*: NCC ≥ 0.5 and projected level ≥ −70 dBFS.
  - *Possible*: NCC from 0.2 to 0.5 (warn).
  - *Absent*: otherwise.
  - NCC ignores the gain, so the 60 % volume setting doesn't matter.
- **Harness use.** To make "cue absent" a real test, the harness saves twice, 10 s apart, so the second clip spans the first save's cue.
- **Measured** with a 220 ms two-note placeholder cue through AAC 160 kbps:
  - no cue: NCC ≤ 0.048 on tracks with beeps and −70 dBFS noise;
  - cue at −16 / −30 / −40 / −50 / −60 / −70 dBFS: 0.9995 / 0.9998 / 0.998 / 0.985 / **0.87** / 0.48 (the last is at the noise floor);
  - the leaked-cue clip: T1 0.999 and T5 1.000, located to the sample.
- **Requirement on the cue asset** (M4): keep its energy ≥ 200 Hz away from 1 kHz and 2 kHz. The placeholder's 2,217 Hz note passed the wideband 2 kHz envelope, so beep acceptance also checks frequency (§3.3). Until the cue exists, verify takes `--cue <wav>`. [Unverified] whether the Windows engine's resampling or effects lower the NCC of a leaked cue; it stays ≥ 0.98 here after a 44.1 kHz resample of the beeps.

## 4. HDR patches and the curve-levels check

Rules: only the published BT.2100 PQ (SMPTE ST 2084) and BT.2390 EETF formulas, not the cookbook's OBS-derived shader (docs/milestones/M1.md:192; docs/PLAN.md:163; CLAUDE.md:17). HDR patches must move on the OLED (docs/milestones/M1.md:193; CLAUDE.md:26).

### 4.1 What the testbed shows

- **One drifting row** of 12 patches, each 128 × 128 u with a 160 u pitch, at y 672–800 u. The row moves **+1 u per frame** and wraps, so every pixel sees changing content. At 180 fps a patch crosses the screen in about 11 s, which suits the ≤ 10 min OLED rule.
- **SDR mode** (8-bit swap chain): grey sRGB codes `0, 8, 16, 32, 64, 96, 128, 160, 191, 224, 240, 255`. Expected Y = 16 + 219·code/255.
- **HDR mode** (`--hdr`):
  - Swap chain: `R16G16B16A16_FLOAT` + `DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709` (scRGB, 1.0 = 80 nits, docs/research/08-capture-rust.md:667).
  - Patches: grey D65 at **0, 10, 40, 80, 120, 160, 203, W, 400, 600, 1000, 4000 nits**, written as nits/80. W = the OS SDR white level, from `SDRWhiteLevel/1000 × 80` (docs/research/09-encode-mux-rust.md:462).
  - The SDR-style elements (barcode, bars, ramp, flash, texture) use sRGB-decoded values × W/80, as Windows composes SDR apps.
  - The barcode's `hdr` bit is set, and the log records W and `MaxLuminance` (L_w, from `IDXGIOutput6::GetDesc1`).
- **Grey patches** make the colour method irrelevant: maxRGB, per channel and luminance all agree on grey. Only the transfer curve is tested.

### 4.2 What an SDR clip of HDR content should measure

Expected Y = 16 + 219 · sRGB_OETF(min(curve(L)/W, 1)) (`signals/hdr_table.py`), where curve(L) is:
- **Natural**: the BT.2390 EETF in the PQ domain, knee at KS = 1.5·maxLum − 0.5, Hermite spline, b = 0 (docs/PLAN.md:164);
- **Match SDR apps**: L, clipped;
- **Reinhard**: candidate only; SP3 fixes the variant.

Sanity checks against the plan: PQ(100) = 0.5081 and PQ(1000) = 0.7518, and the EETF maps 240 → 193.2 nits (80.5 %). That is research 09's 193 nits (docs/research/09-encode-mux-rust.md:469) and SP3's "Natural ≈ Y 215, Match SDR apps Y 235" (docs/PLAN.md:430).

**Expected Y codes, L_w = 1000 nits, SDR white W = 240 nits** (knee 114.2 nits):

| Patch (nits) | 0 | 10 | 40 | 80 | 120 | 160 | 203 | 240 (W) | 400 | 600 | 1000 | 4000 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Natural (BT.2390) | 16.0 | 65.4 | 113.5 | 150.1 | 177.0 | 195.3 | 207.8 | **215.0** | 229.4 | 234.0 | 235.0 | 235.0 |
| Match SDR apps (clip) | 16.0 | 65.4 | 113.5 | 150.1 | 177.0 | 199.1 | 219.4 | **235.0** | 235.0 | 235.0 | 235.0 | 235.0 |
| Reinhard candidate L/(1+L) in W units [Unverified until SP3] | 16.0 | 64.4 | 106.7 | 133.6 | 150.1 | 161.7 | 170.9 | 177.0 | 193.9 | 204.8 | 215.2 | 229.5 |
| *Failure signature: no tone map, scRGB clipped at 80 nits* | 16.0 | 101.1 | 177.0 | 235 | 235 | 235 | 235 | 235 | 235 | 235 | 235 | 235 |

`results/hdr_table.json` also covers W = 200 (knee 85.8), L_w = 600 (knee 149.9; Natural at W gives 222.1) and W = 80. The table depends on **L_w and W**, so `kairo-verify` needs both. For engine clips, the clip's JSON tag should carry `curve`, `lw_nits` and `sdr_white_nits`; the schema already has `hdr_source` and `curve` (docs/PLAN.md:223–224) [proposed addition]. The same golden table should serve the shader's WARP test (docs/PLAN.md:342).

### 4.3 Check rule

- For each fully visible patch, take the mean Y of its central 50 % and compare it with the table: **±2 codes** (SP3, docs/PLAN.md:430). Patches at 235 count as one-sided.
- Measured patch accuracy in SDR mode: ≤ 0.48–0.74 code at Kairo's presets, 1.87 at 8 Mbps with churn, and 2–10 codes only under export-level starvation. ±2 holds for recordings, and exports report it only.
- The curves are 11.6 codes apart at 203 nits and 20 codes apart at W, so the check also tells which curve was used.
- **Bars in HDR mode.** The bars carry colour, and the curve's colour handling (maxRGB vs per channel) is SP3's call. Until its ADR, HDR clips run the relative bars rule (§2.3, rule 2), the decode check and the grey patches. After it, the bars reference passes through the same published curve, and the absolute rule returns.
- In Natural mode, barcode white maps to about 215 and the flash too. Contrast stays at 199 codes, well above the 110 minimum.

## 5. The QPC log (ground truth)

### 5.1 Format

UTF-8 text in `C:\kairo\logs\` (docs/milestones/M1.md:72), written line by line in the same style as the trace format (docs/milestones/M1.md:83; crates/kairo-core/src/trace.rs:1–23).

```
# kairo-testbed 1
# run 5a hdr 0 pace vsync fps 180 flash_frames 180 flash_len 18 size 2560x1440 qpc_freq 10000000 sdr_white_nits - max_nits -
f <N> <content_hns> <present_hns> <display_hns|->
b <m> <N_f> <scheduled_hns> <played_hns|->
```

- **Times** are QPC converted to 100 ns (crates/kairo-core/src/timeline.rs:88). They stay absolute, so the voice process needs no shared zero.
- `content`: the time the frame was rendered for.
- `present`: just after `Present` returns.
- `display`: the QPC of the vblank where the frame first appeared, from DXGI frame statistics (`PresentCount` → `SyncQPCTime`); `-` if unknown or never shown.
- `played`: the beep's first sample, from `IAudioClock::GetPosition` (position plus QPC). [Unverified] which API gives reliable per-present display times in windowed and borderless (composed) modes; without frame statistics the testbed takes its vblank grid from the monitor's own clock (`IDXGIOutput::WaitForVBlank`) and logs `-` (README §4.9). DWM's composition clock isn't used: it follows the primary or fastest display.
- **Voice log** `voice-<run>.log`: header `# kairo-voice 1`, `# run 5a`, then lines `v <k> <scheduled_hns> <played_hns|->`.
- **Privacy.** No monitor, adapter or PC names, no window titles, no paths and no wall-clock time (CLAUDE.md:16, :18; docs/milestones/M1.md:96). These are raw results, so they stay in `C:\kairo\logs` and `.dogfood/` (docs/milestones/M1.md:74). A committed fixture would be rebased to 0 and needs the owner's OK, like the trace fixture (docs/milestones/M1.md:195).

### 5.2 How `kairo-verify` uses it

The log is **optional**. "Test my setup" and friends' PCs have none (docs/PLAN.md:271; docs/milestones/M1.md:56). Every check in §1–4 runs log-free, using the barcode line, F, the flash-frame rule and the QPC grid for voice. With `--log` (and `--voice-log`), verify:

1. Checks the run tag against the barcode (1.2 step 6).
2. **Excuses testbed hiccups.** Frames whose display gap exceeds 1.5 refresh periods, or that were never shown, are marked "testbed missed vblank". Repeats and skips at those frames count as the testbed's, not the capture's.
3. **Measures exact A/V.** Video delay `p05(pts_i − display(N_i))`, audio delay `median(a_m − played_m)`. That removes the testbed's prediction error and display latency, which the log-free method folds into its result: at most one refresh period on a vsynced testbed.
4. Gives absolute A/V for T4 from the voice log (§3.4 step 5).
5. Uses display times as cadence content times (§1.3 step 3).
6. Reports the testbed's own beep-vs-flash error (`played − display(N_f)`). That error is what research 10's "same QPC instant" means in practice.

## 6. Report fields and profiles (summary)

The report layout is Claude's call at implementation time (docs/milestones/M1.md:84).

| Field | `gate` (gate-1 clips) | `m3` | `recording` (plan) | `export:<preset>` |
|---|---|---|---|---|
| Barcode unreadable frames | 0 | 0 | listed; cadence by runs | listed |
| Repeats + skips | 0, every event listed | ≤ 0.5 % | ≤ 0.5 % (docs/milestones/M1.md:12) | at the export's fps |
| Judder | p99 > ¼ tick and ≥ 5 % off-steps fails | SP2 ADR | SP2 ADR | — |
| Bars | min-plane ≥ 40 dB every frame; relative rule | same | same | relative rule; ≥ 30 dB [proposed] |
| Levels / curve | ±2 codes | ±2 | ±2 | report |
| A/V per track | ≤ 21.3 ms | ≤ 21.3 ms (target < 10) | ≤ 40 ms | ≤ 40 ms |
| Inter-track | ±0.5 ms | ±0.5 ms | ±0.5 ms | — |
| Tone SNR / frequency | ≥ 35 dB / ±1 Hz | same | same | ≥ 35 dB / ±1 Hz |
| Routing, cue | per routing profile | per profile | per profile | `share` |
| Frame-0 hold | ≤ 42.7 ms (+ 1 tick) | same | same | — |

The container checks (IDR at 0, closed GOP, pts = dts, faststart, no `elst`, `colr`) are another part of `kairo-verify`. `mp4boxes.py` (docs/research/prototypes/scripts/mp4boxes.py:1–3) already walks top-level order and `elst`; the experiments used it to confirm the edit-list cases.

## 7. Unverified items and risks

- **AMF vs VideoToolbox.** Clean PSNR floors, VBR undershoot on easy content, and how a starved AMF encodes the barcode are Mac stand-in numbers. The first PC run re-measures the 40 dB margin with the default `scroll` load.
- **Harness colour path.** The FFmpeg CLI capture must convert with BT.709 limited range and tag the stream (§1.2 caveat). Which filter does this on the `gfxcapture` path is [Unverified].
- **Display and play times.** The display QPC per frame in composed modes, and the IAudioClock-vs-loopback QPC relation (§3.1, §5.1), are [Unverified]. M3 task 9 measures the constant.
- **Gate-1 good clips.** A real WGC capture may have occasional off-steps; if they exceed 5 %, the `gate` judder rule would fail a good clip. SP2's traces decide; fallback: build the good clip from a QPC-perfect source.
- **Fractional refresh rates.** F = round(1/β) is ambiguous at x.5 Hz. The log header removes the ambiguity, and verify tries F ± 1.
- **Process tree for the voice process** (§3.2): a contract addition for M3's harness.
- **Cue asset.** It doesn't exist until M4; the detector uses a placeholder.
- **Reinhard variant.** SP3 decides.
- **Clips.** The synthetic clips stayed in the scratchpad and are deleted. Committing any clip needs the owner's OK (docs/milestones/M1.md:119).

## 8. Files

All under `verify-design/signals/`:
- `spec.py`: layout, payload/CRC, bars, ramp, flash, HDR formulas.
- `codes.py`: barcode blocks and the reader.
- `render.py`: the testbed renderer and the Kairo conversion model.
- `analyze.py`: video detectors.
- `audio.py`: beeps and audio detectors.
- `cadence.py`: an exact port.
- `cadence_rs/`: calls the real `kairo_core::cadence`.
- `harness.py`: FFmpeg CLI glue.
- `run_video_matrix.py`, `run_corrupt.py`, `run_av.py`, `run_audio.py`, `run_audio2.py`, `run_cadence.py`, `hdr_table.py`: the experiments.
- `results/*.json` and `*.log`: raw numbers.
