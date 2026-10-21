# 13: The user's rig. AMD RX 7000/9000 encoding with minimal game impact, driver interactions, measuring

*Code-level pre-implementation research for the Kairo engine, written for the implementation cookbook. Prepared 2026-10-21. Nothing is built yet.*

**Tags**
- **[H]**: checked in a primary source today. That means vendor docs or SDK docs at their current revision, source code at a named tag or `master`, a vendor release note on amd.com, a GitHub issue thread with vendor staff answers, or crate source from crates.io.
- **[M]**: a strong secondary source (third-party code, a well-known project's docs), or a direct inference from primary sources.
- **[Unverified]**: not confirmed. It needs a run on the user's PC or more sources.
- **[Estimate]**: arithmetic or judgement. Measure it.
- **[Hypothesis]**: a proposed explanation that no source states. Treat it as a test to run, not a fact.
- **[Corrected]**: changes a claim in the brief or in reports 01, 08, 09 or 11.

**Scope.** This report covers four things. First, how to configure AMD's encoder (AMF) through FFmpeg 9.0.2 so it costs the game as little as possible. Second, why OBS was expensive on AMD in LTT's August 2026 test, and what that implies for Kairo. Third, how AMD driver features and Adrenalin interact with capture. Fourth, how to measure Kairo's cost against the ≤3% FPS budget, over SSH. It builds on report 09 (encoder plumbing, options), 08 (capture), 11 (SSH and PresentMon basics) and the brief. It does not repeat them.

---

## TL;DR

1. **What actually costs the game FPS.** The video encoder (VCN) is separate hardware. The game pays for four other things:
   - work on the GPU's 3D or compute queue: Kairo's copy and NV12 shader, AMF Pre-Analysis (PA), and DWM composition for capture;
   - memory bandwidth for full-resolution frame copies;
   - board power;
   - GPU scheduling contention.

   So on AMD the levers are: no PA (its kernels run on **OpenCL by default** [H]), NV12 input only, a low capture rate, fewer full-frame copies, and no raised GPU priority. VCN presets (balanced vs quality) barely matter for FPS. They matter for VCN headroom.
2. **AMF defaults for Kairo (H.264, through FFmpeg 9.0.2).** Use `usage=transcoding quality=balanced rc=vbr_peak vbaq=1 preanalysis=0 high_motion_quality_boost_enable=0 preencode=0 bf=0 async_depth=4 frame_skipping=0`. Set every one explicitly.
   - `usage=high_quality` silently turns on PA, VBAQ, high-motion boost and TAQ with a lookahead of 11 [H].
   - Bitrates: 1080p60 25/40 Mbps and 1440p60 **36/50 Mbps**. **[Corrected]** The brief's 1440p60 peak was 60 Mbps. Lower it to 50 because of the open h264_amf output stall at high bitrates on an RX 9060 XT (AMF #614).
3. **HEVC and AV1 (opt-in).**
   - HEVC has no B-frames on AMD. Use 16/25 Mbps at 1080p60 and 24/36 at 1440p60.
   - AV1: add `latency=power_saving_real_time aq_mode=caq bf=0`.
   - **RDNA3 AV1 still codes 1080p as 1082 lines.** AMD staff confirmed on 2026-10-14 that it is a VCN hardware alignment limit (64×16, with a 1080p exception) [H]. FFmpeg handles it with a 2-line crop and an MP4 `clap` box [H].
   - **2560×1440 is 64×16-aligned**, so 1440p AV1 on RDNA3 has no padding.
   - VCN 5 (RX 9000) reportedly lifts the restriction **[M, Jellyfin docs; verify on the PC]**.
4. **The encoder hardware is shared and finite. There is no session cap.**
   - AMD removed the 16-session limit from the driver [H, AMF #589].
   - **RX 9000 has one VCN, shared by encode and decode.** RX 7700/7800/7900 have two, one used for decode and one for encode by default. RX 7600 has one. AV1 encode runs on one instance only [H].
   - So Kairo plus Adrenalin Instant Replay (IR) both encode on the same VCN. On RX 9000, a hardware-decoded video on the second monitor also lands there.
5. **LTT's OBS number was misread in the brief.** **[Corrected]** The primary article is now retrieved. OBS used **Game Capture (the hook) with an empty scene**. It lost **13% at 1080p and 17% at 1440p on the RX 9060 XT**, against 5% and 19% on the RTX 5060. NVIDIA's 1440p figure was pulled down by an unstable F1 24 run; Cyberpunk and RDR2 alone lost 6–7%. The "−16 vs −5 fps" gap is the 1080p result only.
   - AMD's own ReLive lost **4% at 1080p and 7% at 1440p**. That is over Kairo's 3% budget.
   - Game Bar lost about 1 fps, so roughly 1%.
   - LTT gave no cause.
6. **Why OBS is heavy, from its source [H code; the AMD-specific weighting is a Hypothesis]:**
   - The hook copies the game's backbuffer **inside the game's process, on the game's queue**, up to **2× the OBS frame rate** unless "Limit capture framerate" is ticked.
   - In D3D12 games (all three LTT titles are D3D12-capable), every captured frame goes through D3D11On12: `AcquireWrappedResources`, `CopyResource`, `ReleaseWrappedResources`, then `Flush`.
   - OBS then renders its scene and the NV12 conversion.
   - Its AMF encoder runs on a **second D3D11 device**, opening the NV12 texture through `OpenSharedResource` with a keyed mutex, then `CopyResource` and `Flush` on every frame.
   - The RX 9060 XT also has about 30% less memory bandwidth than the RTX 5060 (320 vs 448 GB/s) [M].
   - Kairo's design avoids all of these.
7. **AFMF and capture.** Whether WGC or DDA sees AFMF-generated frames is **[Unverified]**. LTT explains that compositor-based capture only sees frames that pass through DWM. Frame generation that flips around DWM (NVIDIA's flip metering) is invisible to capture.
   - **This barely matters for 60 fps clips.** The rendered frame rate is almost always ≥60, so every pacer tick still gets a new real frame.
   - Warn only when the clip frame rate is higher than the game's rendered frame rate.
   - PresentMon 2.6 can label AFMF frames (`FrameType = "AMD AFMF"`, with `--track_frame_type`) [H].
8. **Adrenalin coexistence.** Detect IR through two signals:
   - **per-process video-engine use in PDH**, which is robust and vendor-neutral;
   - AMD's registry values, `HKLM\SOFTWARE\AMD\DVR\InstantReplayEnabled` and `HKCU\Software\AMD\DVR\DvrEnabled`. These are **[Unverified]**, from third-party code; confirm with the diff script in §4.2.

   Warn, and never change AMD's settings. AMD's default hotkeys (Ctrl+Shift+S/E/I/G/C/M/J/U/O/L, Alt+R/Z/U, Alt+Shift+G) **do not clash** with Kairo's Alt+F8 and Alt+F6.
9. **Driver timeouts are the AMD reliability risk, not FPS.** These are open in 2026:
   - an RX 9070 XT hang when encoder sessions are created back to back (Adrenalin 26.6.4);
   - h264_amf output stalls (RX 9060 XT);
   - an encoder overload that never recovers (RX 9070 XT);
   - `AMF_INPUT_FULL` followed by a driver timeout after hours of a **1440p OLED replay buffer on an RX 7900 GRE with Adrenalin 26.8.1**, almost exactly the user's setup;
   - Adrenalin 26.10.1 known issues with overlays opening and closing.

   Kairo needs:
   - a lazy, sequential encoder probe, cached per driver version;
   - an output-stall watchdog;
   - encoder rebuild on device loss;
   - 4–8 h soak tests.
10. **Measuring.**
    - **PresentMon 2.6.0** CLI over SSH. It needs admin or Performance Log Users. Use a unique `--session_name`: Adrenalin ships its own `PresentMon-x64.exe`.
    - Read columns by name: `MsBetweenPresents`, `PresentMode`, `MsGPUBusy`, `MsGPUWait`, `FrameType`.
    - Read Kairo's own encoder load from PDH `\GPU Engine(*)\Utilization Percentage`. **On AMD the engine is `engtype_VideoCodec`, not `VideoEncode`** [M].
    - Run interleaved A/B with ≥6 paired runs per workload and a synthetic GPU-load app for automation. Pass when average-FPS loss ≤3% and the upper 95% CI bound ≤4%.

---

## 0. What the user's rig implies

| Fact | Consequence for Kairo | Source |
|---|---|---|
| GPU is RX 7000 (RDNA3, VCN 4.0) or RX 9000 (RDNA4, VCN 5.0); exact SKU unknown | Probe the VCN count by SKU table (§1.5). Treat AV1 1080p differently per generation | [H] AMF wiki table |
| RX 906x/907x: **1 VCN**; RX 75xx/76xx: 1; RX 77xx/78xx/79xx: **2** | On RX 9000, Kairo, IR and any hardware video decode share one engine | [H] AMF wiki, AMF #587/#548 |
| 1440p 360 Hz OLED (main, HDR sometimes) + 1440p 180 Hz | Capture rate must be decoupled from refresh. `MinUpdateInterval` is refresh-aware (§2.4). HDR doubles capture bandwidth (FP16) | [H] report 08; [Estimate] bandwidth |
| AMD Adrenalin Instant Replay installed and in use | It also encodes on VCN and captures. ReLive cost 4–7% FPS in LTT's test. Kairo must detect it, warn, and benchmark with it **off** | [H] LTT |
| Desktop Ryzen CPUs (7000/9000) have an iGPU; if it is enabled, a second adapter exists | Create the capture device, the conversion and AMF on the **adapter that owns the game's monitor** (by LUID), never the default adapter | [M] |
| 8 GB-class cards exist in this family (RX 9060 XT 8 GB) | Keep VRAM small: `async_depth=4`, a 2–3 buffer WGC pool | [Estimate] |

---

## 1. AMF through FFmpeg 9.0.2: settings for minimal game impact

### 1.1 Where each AMF feature spends its cost

The rule: **VCN work is nearly free for the game. Shader, compute, copy and bandwidth work is not.**

| Feature (FFmpeg option) | Runs on | Game impact | Default for transcoding usage | Kairo default | Evidence |
|---|---|---|---|---|---|
| Encode itself, all presets (`quality`) | VCN | Indirect only: power, a little memory bandwidth | balanced | **balanced** (quality if headroom) | [H] AMF docs presets |
| **Pre-Analysis** (`preanalysis`, `pa_*`) | **OpenCL kernels by default** (`AMF_PA_ENGINE_TYPE` = `AMF_MEMORY_OPENCL`; FFmpeg does not expose the engine) | **Direct: compute on the game's GPU** | false | **0** | [H] PA API doc; FFmpeg 9.0.2 sets no engine type |
| High Motion Quality Boost (`high_motion_quality_boost_enable`) | "pre-analysis of the motion"; engine not documented | Possibly direct | false | **0** (OBS also forces it off) | [H] AMF doc wording; OBS texture-amf.cpp; engine **[Unverified]** |
| VBAQ (`vbaq`, AV1 `aq_mode=caq`) | Not documented; believed to be inside VCN rate control | Expected ~0 | false | **1** | [H] doc; engine **[Unverified]** |
| Pre-encode RC (`preencode`) | "rate control pre-analysis"; engine not documented | Expected VCN time | disabled | **0** (1 in the "Quality" profile) | [H] doc; engine **[Unverified]** |
| B-frames (`bf`, `max_b_frames`) | VCN | VCN throughput; reorder delay; pts ≠ dts | n/a | **0** (report 09) | [H] report 09 |
| RGB→YUV colour conversion inside AMF | **Hardware EFC if `CAP_COLOR_CONVERSION == AMF_ACCEL_HARDWARE`, otherwise shaders** | Direct, if shaders | n/a | **Never feed RGB.** Kairo's shader writes NV12 | [H] AMD staff, AMF #574 |
| `async_depth` (surfaces in flight) | VRAM | VRAM: 16 × 5.5 MB ≈ 88 MB at 1440p NV12 | FFmpeg default **16** | **4** | [H] amfenc_h264.c |
| AV1 `latency` | VCN clocks and power | `lowest_latency` "causes highest power consumption" | none | **`power_saving_real_time`** | [H] FFmpeg 9.0.2 option text, AV1 API doc |

**`usage` is a bundle, so always set it.** Per the AMF H.264 docs [H], `HQ` (high_quality) turns on PA, VBAQ, HMQB, profile High, `QUERY_TIMEOUT=50`, and in PA, `TAQ_MODE_2` with a lookahead of 11. `transcoding` turns all of these off and selects quality preset BALANCED. FFmpeg's `usage` defaults to −1, meaning not set. FFmpeg also *keeps* PA on if the usage enabled it (`amfenc_h264.c:420-427`) [H].

**Never feed BGRA.**
- AMD staff explained (AMF #574, 2025-12 and 2026-09) that RGB **render-target** textures can be DCC-compressed. VCN can't read DCC, and D3D11 gives no way to see or disable it, so `CreateSurfaceFromDX11Native` produces gridline corruption on RX 7000 from driver 25.10.2. "YUV doesn't have DCC" [H].
- A separate FFmpeg PR (#24688, open) reports silent corruption when an encoder input format is "accepted but non-native" [M].
- Kairo renders NV12 into FFmpeg's D3D11 pool (report 09 §2). Keep it that way, and drop the BGRA-into-encoder fallback idea for AMD.

### 1.2 Recommended options per codec (AMD, 1080p60 and 1440p60)

Report 09 §4 covers the generic `AVCodecContext` fields: GOP 60, colour VUI, `GLOBAL_HEADER`, NV12 D3D11 frames. Below are the **AMD private options**. Every name and value was checked against the FFmpeg 9.0.2 AVOption tables [H].

| | H.264 `h264_amf` (default) | HEVC `hevc_amf` (opt-in) | AV1 `av1_amf` (opt-in, RX 7000+) |
|---|---|---|---|
| usage | `transcoding` | `transcoding` | `transcoding` |
| quality / preset | `balanced` (`quality` if VCN headroom > 50%) | `balanced` | `balanced` (`quality`; `high_quality` exists for AV1) |
| rc | `vbr_peak` | `vbr_peak` | `vbr_peak` (see the av1 RC note below) |
| bitrate / maxrate / bufsize, 1080p60 | 25M / 40M / 40M | 16M / 25M / 25M | 12M / 20M / 20M |
| bitrate / maxrate / bufsize, 1440p60 | **36M / 50M / 50M** | 24M / 36M / 36M | 18M / 28M / 28M |
| adaptive quantisation | `vbaq=1` | `vbaq=1` | `aq_mode=caq` |
| PA / HMQB / pre-encode | `preanalysis=0`, `high_motion_quality_boost_enable=0`, `preencode=0` | same | same |
| B-frames | `bf=0`, `max_b_frames=0` | none exist | `bf=0`, `max_b_frames=0` |
| headers | `header_spacing=60` | `gops_per_idr=1`, `header_insertion_mode=idr` | `header_insertion_mode=gop` |
| never drop frames | `frame_skipping=0`, `filler_data=0` | `skip_frame=0`, `filler_data=0` | `skip_frame=0`, `filler_data=0` |
| other | `profile=high`, `coder=cabac`, `async_depth=4` | `profile=main`, `bitdepth=8`, `async_depth=4` | `latency=power_saving_real_time`, `align=none`, `async_depth=4` |

**Notes**
- **Bitrates are starting points [Estimate].** Validate each with VMAF on the user's GPU (§5.6).
  - AMD H.264 has historically trailed NVENC and QSV in quality; Jellyfin says VCN 5 "greatly improved" it [M].
  - **For AMD owners, HEVC is the better "quality" option.** Its playback limits are covered in brief §7.3.
- **50 Mbps peak cap on H.264.** AMF #614 (open, 2026-09-30, RX 9060 XT, FFmpeg 9.0.2): h264_amf stops emitting packets for about 20 s after an ordinary scene change at a target of about 100 Mbps. It reportedly never happens at 50 Mbps [H issue; the cause is unknown]. Until tested, keep the peak at or below 50 Mbps.
- **`enforce_hrd`: leave it unset and A/B it.** OBS sets it true. Sunshine defaults it off with the warning "HRD is known to cause encoding artifacts … on certain cards" [H]. Kairo's ring is capped in MB anyway.
- **av1_amf RC quirk.** `amfenc_av1.c:414-417` sets the *context variable* to CBR whenever `bit_rate > 0`, after the property is assigned. Read the stream back in DEBUG logs to confirm VBR behaviour **[Unverified effect]**. Report 09 flagged the same.
- **"Quality" profile** (user-selectable, for PCs with headroom): `quality=quality`, `preencode=1`. Keep `preanalysis=0`. PA's compute cost is the one thing that hits the game directly.

### 1.3 Rust sketch: set, verify and log the options

`ffmpeg-next`'s `open_with` silently drops unknown options (report 09). Set each one with `av_opt_set` and fail loudly.

```rust
// kairo-engine/src/media/amf_opts.rs  (Windows-only; ffmpeg-next =9.0.0 / ffmpeg-sys-next =9.0.1)
use std::ffi::{CStr, CString};
use ffmpeg_next::ffi;

pub const H264_AMF_PERF: &[(&str, &str)] = &[
    ("usage", "transcoding"), ("quality", "balanced"), ("rc", "vbr_peak"),
    ("vbaq", "1"), ("preanalysis", "0"), ("high_motion_quality_boost_enable", "0"),
    ("preencode", "0"), ("bf", "0"), ("max_b_frames", "0"), ("frame_skipping", "0"),
    ("filler_data", "0"), ("profile", "high"), ("coder", "cabac"),
    ("header_spacing", "60"), ("async_depth", "4"),
];
pub const AV1_AMF_PERF: &[(&str, &str)] = &[
    ("usage", "transcoding"), ("quality", "balanced"), ("latency", "power_saving_real_time"),
    ("rc", "vbr_peak"), ("aq_mode", "caq"), ("preanalysis", "0"),
    ("high_motion_quality_boost_enable", "0"), ("preencode", "0"), ("bf", "0"),
    ("max_b_frames", "0"), ("header_insertion_mode", "gop"), ("skip_frame", "0"),
    ("align", "none"), ("async_depth", "4"),
];

/// Apply private options to an *unopened* encoder context. Any rejection is an error.
pub unsafe fn apply(ctx: *mut ffi::AVCodecContext, opts: &[(&str, &str)]) -> anyhow::Result<()> {
    for (k, v) in opts {
        let (ck, cv) = (CString::new(*k)?, CString::new(*v)?);
        let r = ffi::av_opt_set((*ctx).priv_data, ck.as_ptr(), cv.as_ptr(), 0);
        anyhow::ensure!(r >= 0, "AMF option {k}={v} rejected (AVERROR {r})");
    }
    Ok(())
}

/// Read back what will actually be used, for the capability report and tracing.
pub unsafe fn dump(ctx: *mut ffi::AVCodecContext, keys: &[&str]) -> Vec<(String, String)> {
    keys.iter().filter_map(|k| {
        let ck = CString::new(*k).ok()?;
        let mut out: *mut u8 = std::ptr::null_mut();
        (ffi::av_opt_get((*ctx).priv_data, ck.as_ptr(), 0, &mut out) >= 0).then(|| {
            let s = CStr::from_ptr(out as *const _).to_string_lossy().into_owned();
            ffi::av_free(out as *mut _);
            (k.to_string(), s)
        })
    }).collect()
}
```

- Get the AMF **runtime** version from `AVAMFDeviceContext.version`, which FFmpeg fills from `AMFQueryVersion` (`hwcontext_amf.c:612-628`) [H]. **Don't** read the file version of `amfrt64.dll`. 25.x drivers returned a truncated ProductVersion (AMF #579, fixed internally in Feb 2026 and since closed) [H]. A 2026-08 driver still "reports no version string" (AMF #614) [H].
- **Put the encoder on its own thread** (report 09 §2.6 already says so). Here is the AMD-specific reason.
  - When AMF's queue is full, FFmpeg loops in `ff_amf_receive_packet` with `av_usleep(1000)` (`amfenc.c:660-685`) [H]. On Windows that becomes `Sleep(1)` or a `nanosleep` built on it.
  - Unless the process holds a raised timer resolution, `Sleep(1)` lasts up to about 15.6 ms. Report 11 notes that Windows 11 may not honour `timeBeginPeriod` for invisible processes [M].
  - FFmpeg reduces this by setting `AMF_VIDEO_ENCODER_QUERY_TIMEOUT = 1` ms, so QueryOutput blocks inside the driver when supported (`amfenc_h264.c:550-552`) [H]. The sleep path still exists.
  - So hand frames to the encoder through a 2–4 slot channel. The pacer must never call `send_frame` itself.

### 1.4 VCN headroom: budget and a governor

**Throughput anchors**

| Data point | Value | Source |
|---|---|---|
| RX 7900 XT, driver 25.3.1, 1080p transcode with software decode (encode-bound) | H.264 ≈ 382–399 fps; HEVC ≈ 537–566 fps; AV1 ≈ 228 fps | [M] numbers quoted in AMF #548 from VCEEnc's author |
| RX 9070 XT, H.264 1080p hardware decode + encode on its single VCN | ≈ 375 fps; AMD staff: "in line with expectations for RX 9xxx" | [H] AMF #548 |
| RX 9070 XT, OBS, 1440p sessions | Each ≈ 30–35% of Task Manager's "Video Codec"; two ≈ 65–75%; three ≈ 98–100%, with "encoding lag" | [M] user in AMF #587 |
| Scaling rule | "encoder throughput is scaled almost linearly if you count total number of pixels in a second" | [H] AMD staff, AMF #587 |

**Budget [Estimate].** H.264 1080p60 at `balanced` uses roughly 15% of one VCN 4/5 instance, and 1440p60 roughly 25–30%. AV1 1080p60 is about 26%. Kairo plus IR at the same settings doubles that. 1440p120 doubles it again. RX 9000 shares the same engine with any hardware video decode, such as a YouTube video on the second screen.

**Governor (proposed).** OBS does something similar at init: `check_preset_compatibility` drops the preset from highQuality to quality, balanced, then speed when `CAP_MAX_THROUGHPUT − CAP_REQUESTED_THROUGHPUT` is smaller than the session's macroblocks per second [H]. Kairo can't read those AMF caps through FFmpeg, so it drives the governor from observable signals:
1. Every 1 s, read the PDH video-engine utilisation for **all** processes (§5.3), plus Kairo's own encoder backlog: frames sent but not yet received, and the time spent blocked in `receive_packet`.
2. If total utilisation stays above 85% for 5 s, or the backlog stays above `async_depth`: step quality down (quality to balanced to speed). Then, if the user allowed it, step fps down (120 to 60). Never go below 60 by default. Raise a `encoderPressure` notification naming the competing process (for example "AMD Instant Replay").
3. If no packet has come out for more than 2 s while frames are going in, **tear down and recreate the encoder session**. Keep counting ticks, and start a new GOP with an IDR. This answers AMF #614 (a 20 s stall) and OBS #13670 (an overload that never recovers) [H].

### 1.5 RDNA3 vs RDNA4: what differs

| | RX 7000 (Navi31/32/33, VCN 4.0) | RX 9000 (Navi44/48, VCN 5.0) | Source |
|---|---|---|---|
| VCN instances | 79xx and 78xx/77xx: **2**; 76xx/75xx: 1 | **1**, shared by encode and decode | [H] AMF wiki (edited 2025-12-05); AMF #587 |
| Default use of two VCNs | "one is used for decode and another for encode" | n/a | [H] AMD staff, AMF #548 |
| Choosing an instance | `AMF_VIDEO_ENCODER_INSTANCE_INDEX` (0/1) exists in AMF; **FFmpeg 9.0.2 doesn't expose it** | n/a | [H] AMF doc; FFmpeg option tables |
| AV1 encode | One VCN instance only | One VCN | [H] AMF wiki |
| **AV1 1080p** | **Coded 1920×1082** with 2 black lines. AMD staff, 2026-10-14: "requires the coded resolution to be aligned to 64x16, with an exception for 1080p"; it is a VCN hardware limit, and AV1 can't carry crop info in the bitstream. FFmpeg reads `CAP_*_ALIGNMENT_FACTOR` (64×16) and special-cases crop 8 → 2. movenc then writes `clap`. **1440p (2560×1440) is aligned, so no padding.** | Reported fixed in VCN 5 **[M: Jellyfin]**. AMF also has `ALIGNMENT_MODE_8X2_ONLY` **[H enum; which GPUs report it is Unverified]** | [H] AMF #617, `amfenc_av1.c:666-690`, AV1 API doc |
| AV1 B-frames | No | Yes (`AV1_CAP_BFRAMES`); not used (`bf=0`) | [H] AMF docs, report 01 |
| H.264 quality | Baseline | AMD claims better low-bitrate AVC and +11% HEVC (launch claims only) | [M] |
| HAGS support | RX 7900/7800/7700 since Adrenalin 23.12.1 (Win11 22H2+); RX 7600 excluded at launch | **[Unverified]**: log it | [M] press coverage of AMD's 23.12.1 notes |

**What to do about AV1 at 1080p on RDNA3**
- Don't offer it as a choice.
- If the output is 1440p, AV1 is clean.
- If the user insists, encode it and verify playback of the `clap` box in the S6 player matrix.
- Detect which case applies: ffprobe a 1-s AV1 probe clip for `coded_height` and a `clap` box (§5.6 test 4).

### 1.6 Known AMF and driver issues relevant to capture (2025–2026)

| # | Issue | Hardware / driver | Status on 2026-10-21 | Kairo mitigation | Source |
|---|---|---|---|---|---|
| 1 | Sunshine creates H.264, then HEVC, then AV1 sessions at start-up. HEVC/AV1 fail with "Failed to create encoder D3D11 device", then the GPU hangs and AMD Crash Defender puts the GPU in reduced mode | RX 9070 XT, Adrenalin **26.6.4** (not on 25.9.1), Win11 25H2 | Open | **Lazy, sequential probe**: H.264 only at first run; HEVC/AV1 only when opted in. Wait ≥1 s between sessions, release each before the next, and cache results per (driver store version, AMF version). Never probe on every start | [H] Sunshine #5385 |
| 2 | h264_amf stops output for ~20 s after a scene change at ~100 Mbps | RX 9060 XT, driver 32.0.31041.1004 (2026-08-17), FFmpeg 9.0.2 | Open | Peak ≤50 Mbps; output-stall watchdog (§1.4) | [H] AMF #614 |
| 3 | AMD encoder overload never recovers after a transient overload (4K playback while encoding) | RX 9070 XT, 26.6.4, OBS 32.1.2 | Open | Governor plus session recreate | [H] OBS #13670 |
| 4 | `amf_encode_tex: SubmitInput timed out: AMF_INPUT_FULL`, an `AMD_WATCHDOG` live dump, then a driver timeout after hours. **Replay buffer, `h264_texture_amf` at 2560×1440, two 240 Hz monitors including a 1440p OLED, HAGS on** | RX 7900 GRE; 24.3.1 to **26.8.1** (2026-08-30 comment) | Open | Soak tests (§7); device-lost rebuild; record TDR events (§5.4) | [H] OBS #10594 |
| 5 | VCN encode deadlocks against a compute fence with no TDR. AMD couldn't reproduce it | RX 7900 XT, 26.5.2 and 26.6.1 (VR runtime) | Open | Output-stall watchdog | [H] AMF #599 |
| 6 | RGB render-target input corrupts output (DCC) | RX 7000 from 25.10.2 | By design per AMD; FFmpeg PR #24696 (Duplicate) open | NV12 input only | [H] AMF #574, Forgejo PR |
| 7 | A non-native input format (BGRA) silently corrupts output | Radeon 860M | FFmpeg PR #24688 open | NV12 only | [M] PR text |
| 8 | Truncated `amfrt64.dll` ProductVersion | 25.x | Fixed internally, closed 2026-07 | Use `AVAMFDeviceContext.version` | [H] AMF #579 |
| 9 | HEVC 10-bit fails Init with QVBR or HQVBR | – | Open | No QVBR/HQ modes; 8-bit in v1 | [H] AMF #591 |
| 10 | B-frames + HQVBR hang in FFmpeg | – | Open | `bf=0`, no HQVBR | [H] AMF #586 |
| 11 | AV1 PA → random `AMF_INPUT_FULL` | RDNA4 | Closed 2025-07 | PA off | [H] AMF #562 |
| 12 | VBAQ blurs I-frames in motion | RX 5500 XT (VCN 2), 25.9.1; AMD couldn't reproduce on a 9070 XT | Open | Keep VBAQ; VMAF A/B in S2 | [H] AMF #571 |
| 13 | MHW crashes/timeouts with Anti-Lag + **Instant Replay** | Known 25.4.1–25.6.1 | Fixed 25.8.1 | – | [H] AMD release notes |
| 14 | MHW with RT, recording with AMD Software → timeouts | RX 7600/7700/7800 | Fixed 25.9.1 | – | [H] AMD RN 25.9.1 |
| 15 | BF6 texture flicker with AMD Record and Stream | Known 25.10.2 → 26.7.1 | Not listed from 26.8.1 | Specific to driver-level capture | [H] AMD RN |
| 16 | Stutter with alt-tab while streaming to Discord on multiple monitors | Known 25.6.1 | Not tracked later | Include multi-monitor alt-tab in the A/B | [H] AMD RN 25.6.1 |
| 17 | Multiple ≥120 Hz monitors over HDMI → crash | Fixed 25.3.2 | Fixed | Log monitor connection types | [H] AMD RN 25.3.2 |
| 18 | Crash/timeout "when opening or closing other overlays, such as Xbox Game Bar, while the … Performance Metrics Overlay is enabled" | All Radeon, **26.10.1 known** | Known | Kairo's toast is overlay-like: create it once and hide/show it, don't create and destroy; test with AMD's overlay on; sound cue is the primary feedback | [H] AMD RN 26.10.1 (live page) |
| 19 | Crash/hang "when switching between applications or toggling overlays" with in-game FSR Frame Generation | RX 9000, 26.10.1 known | Known | Same as #18 | [H] AMD RN 26.10.1 |
| 20 | Cyberpunk + FSR + Anti-Lag + in-game FG + **video playback on a secondary display** → timeouts | RX 9000, 26.10.1 known | Known | Test with a video on the 180 Hz screen (shared VCN on RDNA4) | [H] AMD RN 26.10.1 |

Adrenalin 26.10.1 is "Driver Version 26.20.15.07 … Windows Driver Store Version 32.0.32015.7029" [H]. Log the store version (§3.3). It is the only version string every API agrees on.

---

## 2. Why OBS cost 13–17% on the RX 9060 XT, and what Kairo copies from that

### 2.1 What LTT actually measured

The primary article was retrieved through a reader proxy because Cloudflare blocks direct fetches [H]:
- **Rig.** Ryzen 7 7800X3D, 32 GB DDR5-6000, PULSE RX 9060 XT **8 GB** or Ventus 2X RTX 5060 **8 GB**, Windows 11.
- **Recording settings.** "H.264 codec, with a bitrate of 6 Mbps where possible, at 60 FPS, and with hardware encoding enabled."
- **Games.** Cyberpunk 2077, F1 24 and RDR2. Results are the geomean of the three at 1080p and 1440p.
- **OBS setup.** "To keep it simple, we used basic game capture with an otherwise empty scene."
- **Baselines.** 1080p: AMD 119 fps, NVIDIA 108. 1440p: AMD 87, NVIDIA 78.

| Tool | RX 9060 XT 1080p | RX 9060 XT 1440p | RTX 5060 1080p | RTX 5060 1440p |
|---|---|---|---|---|
| Game Bar ("Standard" ≈15 Mbps) | ≈ −1 fps (0 in RDR2) | ≈ −1 fps | ≈ −1 fps | ≈ −1 fps |
| Steam Recording | −3 to −5% | −3 to −5% | −3% | −3% |
| ReLive (AMD) | **−4%** | **−7%** | n/a | n/a |
| ShadowPlay | n/a | n/a | −3% (avg and 1% low) | −3% |
| **OBS** | **−13%** | **−17%** | −5% | −19% (F1 24 unstable; Cyberpunk −6%, RDR2 −7%) |
| Streamlabs | −18% | −20% | −11% | −19% |

LTT offered two possible causes, with no measurement behind either:
- "the additional overhead from OBS' scene rendering", which they note both vendors share;
- NVIDIA's longer optimisation partnership with OBS (2019, against AMD's in 2022).

**[Corrected]** The brief's table ("OBS −16 fps vs −5") is the 1080p column only. The brief can now drop the "[Uncertain] … primary page not fetched" tag.

### 2.2 What the OBS source shows [H: code read today, OBS `master`]

| Mechanism | Code | Cost lands on |
|---|---|---|
| Game Capture frame rate: `interval = 1/fps`; **halved unless "Limit capture framerate" is ticked** (default **off**). The comment says unlimited capture "will dramatically reduce performance" | `game-capture.c` `reset_frame_interval`; default `limit_framerate=false` | The game's own GPU queue, up to 120 copies/s for a 60 fps canvas |
| D3D12 games: per captured frame `AcquireWrappedResources`, then `CopyResource`, then `ReleaseWrappedResources`, then `context11->Flush()` through D3D11On12 on the game's queue | `graphics-hook/d3d12-capture.cpp:275-305` | The game's queue and its submission batching |
| Scene compositing plus NV12 conversion at 60 fps (and the preview) | libobs | 3D queue |
| AMF texture encoder: its **own** `D3D11CreateDevice`; `OpenSharedResource` on libobs's NV12 texture; `IDXGIKeyedMutex::AcquireSync`; `CopyResource`; `Flush`; `ReleaseSync`; then `CreateSurfaceFromDX11Native` | `texture-amf.cpp:426-460, 830-860` | 3D/copy queue plus cross-device sync, every frame |
| AMF defaults: CBR 6000, preset **quality**, profile high, `bf=2` (but `bf=0` when >1920×1088 or >60 fps), PA off, **VBAQ on**, **PREENCODE on**, ENFORCE_HRD on, HMQB off, usage transcoding | `texture-amf.cpp:1226-1231, 1461-1472, 1572-1609, 1694-1702` | Mostly VCN |
| GPU priority: if built with `GPU_PRIORITY_VAL` (official CI passes it as a secret) and the GPU isn't Intel, `D3DKMTSetProcessSchedulingPriorityClass(REALTIME`, or `HIGH` with HAGS on), plus `SetGPUThreadPriority`. **Only succeeds as admin** (it logs "not admin?") | `d3d11-subsystem.cpp:435-459, 645-660`; `build-project.yaml:410` | If OBS runs elevated, OBS's work pre-empts the game |
| HAGS: OBS logs a **warning** when HAGS is enabled | `d3d11-subsystem.cpp:651-653` | – |

### 2.3 Plausible causes of the AMD-specific gap, ranked [Hypothesis]

1. **In-game hook copies plus D3D11On12 flushes in D3D12 titles, at up to 2× 60 fps.** These run inside the game's submission stream, and each `Flush` splits the game's work. How costly that is depends on the driver. A difference here between AMD's and NVIDIA's drivers would produce exactly a vendor-specific gap.
2. **Memory bandwidth.** The RX 9060 XT 8 GB has about 320 GB/s against about 448 GB/s on the RTX 5060 [M, spec sites]. OBS moves several full-resolution frames per output frame:
   - hook copies of 1440p RGBA, ≈14.7 MB, at up to 120/s;
   - scene render;
   - NV12;
   - the cross-device copy.

   That is several GB/s, so the same bytes are a larger share on the AMD card [Estimate].
3. **8 GB of VRAM at 1440p in Cyberpunk and RDR2.** Any extra allocation can push a game into paging. Both cards are 8 GB, but drivers budget VRAM differently [Hypothesis].
4. **Cross-device keyed-mutex copy and flush in OBS's AMF path.** These are per-frame synchronisation points. NVENC's OBS path is a different, NVIDIA-optimised integration (LTT's second hypothesis).
5. **Admin-only GPU priority** applies only if LTT ran OBS elevated. That is unknown.

**What all five have in common:** extra work in the game's queue or extra bandwidth. Encoder settings don't appear in the list.

### 2.4 Design rules for Kairo

| Rule | Why (evidence) | How |
|---|---|---|
| **Capture out of process (WGC/DDA), never hook** | Game Bar, Windows' own recorder with no hook [M], lost about 1% on the same RX 9060 XT [H] | Brief house rule; report 08 |
| **One D3D11 device on the game's adapter for capture, conversion and AMF; no keyed mutex, no shared handles** | OBS pays a cross-device copy and sync per frame | Report 09 §2: `av_hwdevice_ctx_create_derived(AMF ← D3D11VA)` from Kairo's device |
| **NV12 only into AMF** | AMF #574/DCC; EFC vs shader CSC | §1.1 |
| **No PA; HMQB off** | PA runs OpenCL kernels on the game's GPU | §1.1 |
| **Capture rate tied to the clip, not the game or monitor** | OBS: unlimited capture "will dramatically reduce performance" | WGC `MinUpdateInterval` (26100+): a refresh-aware value (below); DDA: poll at Kairo's own rate |
| **Refresh-aware MinUpdateInterval** for the 360 Hz and 180 Hz monitors | WGC delivery is quantised to vblanks (report 08) | k = floor(refresh / (1.2 × clip fps)). If k ≥ 2, interval = k / refresh: 360 Hz at 60 fps → k = 5 → **13.9 ms (72/s)**; 180 Hz → k = 2 → **11.1 ms (90/s)**. If k ≤ 1 (60–144 Hz panels), request 1/(1.5 × fps), the brief's 1/90 s, so that every vblank is eligible. **[Estimate]**: verify duplicate-tick rates (§5.6 test 3) |
| **Fewer full-frame copies: candidate "convert-on-arrival"** | Copy-on-arrival at 72–90/s of 1440p BGRA (14.7 MB) is a 2.1–2.65 GB/s copy (read + write ≈ 4.2–5.3 GB/s). On a 320 GB/s card that is about 1.5% of bandwidth on its own [Estimate] | On each WGC frame, run the scale + NV12 shader **directly from the WGC surface** into a small ring of pool NV12 frames, then close the frame (OBS closes right after its `CopyResource`, so ordering is safe the same way). The pacer submits the newest finished NV12 frame. A screenshot sets a flag and the *next* arriving frame is copied full-res. **A/B it against copy-on-arrival in S2** |
| **Window capture A/B vs monitor capture** | Monitor capture makes DWM compose the whole desktop into the capture surface even when the game is in Independent Flip; window capture may avoid that **[Unverified]** | `CreateForWindow` on Win11 (brief Q8). Measure both (§5.6) |
| **Never raise Kairo's GPU priority. Optionally lower it (experiment)** | OBS raises priority to keep its own frames; Kairo's goal is the opposite | Default: untouched. Hidden flag `--gpu-priority=-2`: `IDXGIDevice::SetGPUThreadPriority(-2)` (relative range −7..7, 0 = normal, −7 = idle) [H MS Learn]. Measure FPS and clip duplicate ticks. Under HAGS, enforcement is done by the GPU's firmware scheduler **[Unverified]** |
| **Normal CPU priority; MMCSS only for the capture/encode thread; opt out of EcoQoS** | Game Mode favours the foreground game | Report 11 §2.6 |
| **No visible topmost window over the game when idle** | A 1×1 invisible topmost window is enough to keep DWM in **Composed Flip** (ForceComposedFlip's README, with MPO off) [M]. That adds a frame of latency and can break VRR | Create the toast once and `ShowWindow(SW_HIDE)` when idle. Show it ≤2 s with `SW_SHOWNOACTIVATE` (brief §5.4). Default feedback is the sound cue |

```rust
// Experiment only. windows =0.62.2, features "Win32_Graphics_Dxgi".
use windows::{core::Interface, Win32::Graphics::{Direct3D11::ID3D11Device, Dxgi::IDXGIDevice}};
pub fn set_gpu_priority(dev: &ID3D11Device, rel: i32) -> windows::core::Result<()> {
    let dxgi: IDXGIDevice = dev.cast()?;            // relative mode: −7..=7, 0 = normal
    unsafe { dxgi.SetGPUThreadPriority(rel) }        // signature: fn(&self, priority: i32) -> Result<()>
}
```

### 2.5 Mitigation catalogue

| Mitigation | Evidence | Kairo default | Measure |
|---|---|---|---|
| Encoder usage/preset | VCN-side only; no PA | `transcoding` + `balanced` | VideoCodec % (PDH), VMAF |
| Hardware-accelerated GPU scheduling | AMD supports it on RX 7700+ (Win11 22H2+); OBS warns when it's on; may change how priorities are honoured | Don't touch; log state | One A/B with HAGS on and off on the user's PC |
| Process / GPU priority | OBS raises it (admin only) | Normal; GPU-priority experiment | FPS, 1% low, duplicate ticks |
| Capture rate | OBS hook default = 2× canvas | Refresh-aware `MinUpdateInterval` | WGC arrival histogram |
| Texture-sharing path | OBS: second device + keyed mutex + Flush | One device, zero-copy into AMF | PDH 3D/Copy engine time for Kairo's PID |
| Full-frame copies | Bandwidth | Convert-on-arrival candidate | A/B |
| Capture target | Unknown cost difference | Window (Win11) vs monitor A/B | A/B |

---

## 3. AMD driver features vs capture

### 3.1 Feature matrix

| Feature | What it does | Seen by WGC/DDA? | Effect on clips | Effect on measurement | Detect | Kairo action |
|---|---|---|---|---|---|---|
| **AFMF 2/2.1** (driver frame generation; RX 7000+ borderless, RX 6000 exclusive only; needs VSync off; Adrenalin 25.3.1+; DX11/12/Vulkan/OpenGL; toggle **Alt+Shift+G**) | Inserts interpolated frames outside the game | **[Unverified]**. Compositor capture sees only frames that pass through DWM (LTT; ForceComposedFlip). A driver-inserted flip that bypasses DWM would be invisible | **None at a 60 fps clip if the rendered fps ≥ 60.** Only a clip fps above the rendered fps shows duplicates | FPS tools see app presents; PresentMon labels generated frames with `--track_frame_type` | PresentMon `FrameType == "AMD AFMF"`; ADLX `IADLX3DAMDFluidMotionFrames::IsEnabled` (licence caveat, §3.3) | Log it. Warn only when the clip fps is above the rendered fps. Test in §5.6 |
| **Anti-Lag** (driver) / **Anti-Lag 2** (in-game SDK, DX11/12) | Limits CPU run-ahead; v2 is called by the game before input sampling | Not relevant | None | Changes the frame queue; keep it constant across A/B | ADLX `IADLX3DAntiLag` | Log only. (Anti-Lag + IR timeouts in MHW were fixed in 25.8.1) |
| **Radeon Chill** (dynamic fps between min and max with input) | Lowers fps when idle | Yes (fewer frames) | Duplicate ticks when fps < 60 during idle | **Must be off** for A/B | ADLX `IADLX3DChill` | Log; benchmark checklist |
| **Radeon Boost** (lowers render resolution during motion; supported games only) | Output stays native, upscaled | Yes | Softer motion in clips | **Must be off** for A/B (it raises fps) | ADLX `IADLX3DBoost` | Log; checklist |
| **Enhanced Sync** | Tear-free without VSync latency; drops frames at scan-out | Not relevant | None | Keep constant | ADLX `IADLX3DEnhancedSync` | Log only. (Old black-screen bugs were fixed in 22.9.1 [M]) |
| **FreeSync / VRR at 360 Hz** | Refresh follows fps | – | None | Capture must not demote the game from Independent Flip; watch `PresentMode` | ADLX `IADLXDisplayFreeSync`; PresentMon | PresentMode check in S1; the monitor's Hz OSD with Kairo on |
| **HDR** (OLED, sometimes) | DWM composes FP16 scRGB | Yes (FP16 pool) | Tone-mapped SDR (report 09) | **2× capture bandwidth** (29.5 MB/frame at 1440p) | `IDXGIOutput6::GetDesc1().ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020` | Inform; run the A/B with HDR on as well |
| **10 bpc colour depth (SDR)** | Scan-out bit depth | DWM still composes at 8 bpc without HDR (MS docs quoted in OBS #4164) | None | None | `DXGI_OUTPUT_DESC1.BitsPerColor` | Log only |
| **"10-bit pixel format"** (Graphics → Advanced, needs a reboot) | Driver override for 10-bit GL output | In 2021 it **broke DDA/BitBlt display capture**; WGC worked (OBS #4164) [H thread, old] | Frozen or black DDA frames | – | **[Unverified]**: no known API; detect the symptom | If DDA freezes on AMD, switch to WGC and say why |
| **RSR / VSR / integer scaling** | Driver-side scaling or a larger desktop | VSR raises the captured resolution | – | VSR raises capture cost | ADLX `IADLX3DRadeonSuperResolution`, `IADLXDisplayVSR` | Log; RSR once broke OBS's hook (#6697), not relevant to WGC |
| **Adrenalin Metrics Overlay** | AMD's in-game overlay; uses an AMD-shipped `PresentMon-x64.exe` [M] | Captured if on screen | Shows in clips | Its own ETW session | `HKCU\Software\AMD\CN\Performance\EnableMetricsOverlay` [Unverified, third-party code] | Off for A/B; 26.10.1 overlay-toggle timeouts (§1.6 #18) |
| **HAGS** | GPU-firmware scheduling | – | – | Can change priority behaviour | D3DKMT (§3.3) | Log; one A/B |

**Mutual exclusions** (ADLX docs, [M]): enabling Boost disables Chill. Enabling Chill disables Boost, Anti-Lag and RSR.

### 3.2 AFMF test (S1, 30 minutes)

1. Turn AFMF on globally (Alt+Shift+G), turn VSync off, and run a borderless DX12 game capped at about 50 fps in-game. The displayed rate should then be about 100.
2. Run `PresentMon --track_frame_type` and confirm rows with `FrameType=AMD AFMF`.
3. Capture with Kairo's WGC probe at `MinUpdateInterval` 1/120 s, then with DDA. Count unique frames per second from the `SystemRelativeTime` deltas, plus a frame-difference check on the decoded clip.
4. Reading the result:
   - **≈50 unique/s**: AFMF frames are invisible to capture.
   - **≈100 unique/s**: they are visible.

   Record the outcome per capture method and per present mode.

### 3.3 Detection code (Rust, `windows` =0.62.2)

Features needed: `Win32_Foundation`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_System_Registry`, `Wdk_Graphics_Direct3D` (D3DKMT), `Win32_System_Performance` (PDH, §5.3).

```rust
// HAGS state, as OBS does it. D3DKMT_WDDM_2_7_CAPS bit 0 = HwSchSupported, 1 = HwSchEnabled,
// 2 = HwSchEnabledByDefault, 3 = IndependentVidPnVSyncControl. Microsoft marks the struct
// "reserved for system use"; treat the result as diagnostics only.
use windows::Wdk::Graphics::Direct3D::*;
use windows::Win32::Foundation::LUID;
pub fn hags(luid: LUID) -> Option<(bool, bool)> {
    unsafe {
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: luid, hAdapter: 0 };
        if D3DKMTOpenAdapterFromLuid(&mut open).is_err() { return None; }
        let mut caps = D3DKMT_WDDM_2_7_CAPS::default();
        let mut q = D3DKMT_QUERYADAPTERINFO {
            hAdapter: open.hAdapter, Type: KMTQAITYPE_WDDM_2_7_CAPS,
            pPrivateDriverData: &mut caps as *mut _ as *mut core::ffi::c_void,
            PrivateDriverDataSize: core::mem::size_of::<D3DKMT_WDDM_2_7_CAPS>() as u32,
        };
        let ok = D3DKMTQueryAdapterInfo(&mut q).is_ok();
        let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: open.hAdapter });
        let bits = caps.Anonymous.Value;                       // packed union, u32
        ok.then_some((bits & 1 != 0, bits & 2 != 0))          // (supported, enabled)
    }
}

// Driver store version ("32.0.32015.7029") for the capability report. DXGI returns the
// user-mode driver version; that it equals AMD's "Windows Driver Store Version" is [M].
use windows::core::Interface;                       // brings IDXGIDevice::IID into scope
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIDevice};
pub fn umd_version(a: &IDXGIAdapter1) -> Option<[u16; 4]> {
    let v = unsafe { a.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok()? as u64; // Result<i64>
    Some([(v >> 48) as u16, (v >> 32) as u16, (v >> 16) as u16, v as u16])
}
```

- **HDR and bit depth.** Use `IDXGIOutput6::GetDesc1()`, which returns `DXGI_OUTPUT_DESC1 { BitsPerColor, ColorSpace, MaxLuminance, … }` [H]. Report 09 adds SDR white.
- **ADLX** would cover AFMF, Chill, Boost, Anti-Lag, Enhanced Sync, FreeSync, colour depth and RSR. Facts [H]:
  - v2.0 (build 125) was released 2026-09-20. The DLL is `amdadlx64.dll`, with C-callable exports `ADLXInitialize`, `ADLXQueryFullVersion` and `ADLXTerminate`, and a COM-like vtable interface.
  - The only Rust crate, `adlx`, is **0.0.0-alpha.1 (2024-05-01)**.
  - **ADLX ships under "ADLX SDK License Agreement.pdf", not MIT.** Check that it is compatible with GPL-3.0 before vendoring headers or generated bindings.
  - **Recommendation:** v1 uses DXGI, D3DKMT, the registry and PDH only. ADLX is a v1.x item after the licence check.
  - Note that ADLX reads *global* 3D settings. Per-game profile overrides are not covered **[Unverified]**.
- **AMF caps** (`CAP_MAX_THROUGHPUT`, `CAP_REQUESTED_THROUGHPUT`, `CAP_NUM_OF_HW_INSTANCES`, `CAP_COLOR_CONVERSION`) would need a raw AMF C vtable binding. `AVAMFDeviceContext.factory` gives the entry point.
  - `shiguredo_amf` 2026.3.0 is **Linux-only** (libamfrt64.so, Vulkan) and doesn't apply.
  - Whether `REQUESTED_THROUGHPUT` counts other processes' sessions is **[Unverified]**. If it does, it is the best "is IR encoding?" signal. Treat this as v1.x.

---

## 4. Coexisting with Adrenalin

### 4.1 Encoder sharing

| Fact | Source |
|---|---|
| No session cap: "The limit of 16 was removed from the driver. The app can create as many sessions as it needs. The HW throughput limitations need to be considered" | [H] AMD staff, AMF #589 (2026-03-16) |
| `CAP_NUM_OF_HW_INSTANCES` = number of VCNs. `CAP_NUM_OF_STREAMS` still reports 16 and "need[s] to be corrected" | [H] same |
| "RX 9xxx has a single VCN. It is shared HW between decode and encode jobs. Most of RX 7xxx have two VCNs. By default, one is used for decode and another for encode" | [H] AMD staff, AMF #548 |
| "Spreading sessions between two VCNs improves performance" (needs `INSTANCE_INDEX`, which FFmpeg doesn't expose) | [H] AMF #589 |
| ReLive's own cost: −4% (1080p) and −7% (1440p) on the RX 9060 XT | [H] LTT |
| IR records continuously to the media folder by default ("Instant Replay video files stored in media save location folder"), so it also adds disk writes | [H] AMD DH3-023 |
| Adrenalin Record "Enhanced Filtering … may have an impact on gaming performance" | [H] AMD DH3-023 |

So Kairo plus IR means:
- two captures and two encodes;
- roughly double the VCN load;
- the sum of both GPU costs.

On RDNA3 Navi31/32, both encodes probably share the default "encode" VCN **[Inference]**.

**Product rule.** At onboarding, and again when a game starts, if IR (or Record Desktop) is detected, show:

> "AMD Instant Replay is also recording. Running both roughly doubles the encoder load and the FPS cost. Turn off Instant Replay in AMD Software → Record & Stream to let Kairo take over."

Kairo **must not** write AMD's registry or settings.

### 4.2 Detecting Instant Replay

| Signal | How | Confidence |
|---|---|---|
| **PDH video-engine use by a non-Kairo process** | Sample `\GPU Engine(*)\Utilization Percentage`. Instances whose engine type is `VideoEncode` or `VideoCodec`, PID ≠ Kairo, and value > 1% for 5 s. Map PID → image path with `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `QueryFullProcessImageNameW`. An AMD process under `C:\Program Files\AMD\CNext\CNext\` (e.g., `AMDRSServ.exe`) means AMD is recording **[path from third-party pages; Unverified]** | Robust, vendor-neutral (it also catches OBS, the NVIDIA app, Medal and Discord). Whether PDH shows other processes to a standard user is **[Unverified]** |
| Registry: `HKLM\SOFTWARE\AMD\DVR` `InstantReplayEnabled`, `InGameReplayEnabled` (DWORD); `HKCU\Software\AMD\DVR` `DvrEnabled`, `DesktopRecordingEnabled`, `CameraEnabled` (DWORD), `ShowRSOverlay` (REG_SZ "true"/"false") | `RegGetValueW` | **[Unverified]**: read by third-party overlay code (AMDLSS5). Tweak scripts also write `HKCU\Software\AMD\DVR\PrevInstantReplayEnable`. Confirm with the diff script below |
| Processes | `RadeonSoftware.exe` (UI), `AMDRSServ.exe` (host service; ReLive), `AMDRSSrcExt.exe`, `amdow.exe` (overlay) | [M] third-party file-info pages; presence ≠ recording |
| Overlay window | `FindWindowExW(…, w!("AMDDVROVERLAYWINDOWCLASS"), …)` | **[Unverified]**, third-party code |

**Confirm the registry values on the PC.** Run this over SSH. The user toggles Instant Replay in Adrenalin when prompted. OpenSSH loads the user's HKCU.

```powershell
$paths = 'HKCU:\Software\AMD\DVR','HKLM:\SOFTWARE\AMD\DVR','HKCU:\Software\AMD\CN','HKCU:\Software\AMD\CN\Performance'
function Snap { $o = @{}
  foreach ($p in $paths) { if (Test-Path $p) {
    (Get-ItemProperty $p).PSObject.Properties | Where-Object Name -notlike 'PS*' |
      ForEach-Object { $o["$p\$($_.Name)"] = "$($_.Value)" } } }
  $o }
$before = Snap; Read-Host 'Toggle Instant Replay (then Record Desktop, then the Metrics Overlay) in AMD Software, then press Enter'
$after = Snap
$after.Keys + $before.Keys | Sort-Object -Unique | Where-Object { $before[$_] -ne $after[$_] } |
  ForEach-Object { '{0}: {1} -> {2}' -f $_, $before[$_], $after[$_] }
```

```rust
// Registry probe. windows =0.62.2, feature "Win32_System_Registry".
use windows::{core::*, Win32::System::Registry::*};
fn reg_dword(root: HKEY, sub: PCWSTR, val: PCWSTR) -> Option<u32> {
    let (mut v, mut len) = (0u32, 4u32);
    unsafe { RegGetValueW(root, sub, val, RRF_RT_REG_DWORD, None,
                          Some(&mut v as *mut u32 as *mut _), Some(&mut len)) }
        .is_ok().then_some(v)                                // WIN32_ERROR::is_ok()
}
pub fn amd_instant_replay_hint() -> Option<bool> {
    reg_dword(HKEY_LOCAL_MACHINE, w!("SOFTWARE\\AMD\\DVR"), w!("InstantReplayEnabled"))
        .or_else(|| reg_dword(HKEY_CURRENT_USER, w!("Software\\AMD\\DVR"), w!("InstantReplayEnabled")))
        .map(|v| v != 0)                                     // a hint only; PDH is the ground truth
}
```

### 4.3 Adrenalin's default hotkeys

Hotkeys are stored as `HKLM\SOFTWARE\AMD\DVR\<Name>` = "Mod+Mod,Key". AMD states that "Some hotkeys require the use of CTRL and/or ALT keys" [H, DH3-023/DH3-038].

| Action | Default | Registry value | Source |
|---|---|---|---|
| Open AMD overlay | **Alt+R** | `ToggleRsHotkey` | [M] AMDHotkeys module (2025); AMD 2026 RN mention Alt+R |
| Performance overlay | **Ctrl+Shift+O** | `ToggleRsPerfUiHotkey` | [H] AMD DH3-038 |
| Performance logging | **Ctrl+Shift+L** | `ToggleRsPerfRecordingHotkey` | [H] AMD DH3-038 |
| Record & Stream toolbar | **Alt+Z** (registered even when hotkeys are "disabled", per the module's author) | `ToggleDvrToolbarHotkey` | [M] |
| Start/stop recording | **Ctrl+Shift+E** | `ToggleDvrRecordingHotkey` | [M] |
| Start/stop streaming | **Ctrl+Shift+G** | `ToggleStreamingHotkey` | [M] |
| Camera / mic | **Ctrl+Shift+C** / **Ctrl+Shift+M** | `ToggleCameraHotkey` / `ToggleMicrophoneHotkey` | [M] |
| Screenshot | **Ctrl+Shift+I** | `TakeScreenshotHotkey` | [M] |
| **Save Instant Replay** | **Ctrl+Shift+S** | `SaveInstantReplayHotkey` | [M]; brief/report 03 agree |
| Save Instant GIF / In-Game Replay | **Ctrl+Shift+J** / **Ctrl+Shift+U** | `SaveInstantGifHotkey` / `SaveInGameReplayHotkey` | [M] |
| Upscaling toggle | **Alt+U** | `ToggleUpscaling` | [M] |
| Display rotation | Ctrl+Alt+arrow keys | `Rotation*Hotkey` | [M] |
| AFMF toggle | **Alt+Shift+G** | – | [H] AMD AFMF release notes |
| All hotkeys off | `HKCU\Software\AMD\DVR\HotkeysDisabled` = 1 | – | [M] |

- **Kairo's Alt+F8 and Alt+F6 clash with none of these.**
- AMD loads an in-game hotkey module into games (`amdihk64.dll`, seen by third-party code) [M]. So an AMD combination may be caught *inside the game* while `RegisterHotKey` for the same keys still succeeds. **Detect conflicts by parsing AMD's registry**, not by relying on `ERROR_HOTKEY_ALREADY_REGISTERED` (1409).
- Third-party "how to clip with AMD" pages listing Alt+F8/F9/F10 for AMD conflict with each other and look like copies of NVIDIA guides. Don't rely on them.

---

## 5. Measuring

### 5.1 PresentMon 2.6.0 over SSH

- **Version.** v2.6.0, released 2026-09-21 (`PresentMon-2.6.0-x64.exe`). 2.5.1 and 2.4.1 are older [H].
- **Privileges.** Run as admin, or as a member of **Performance Log Users** (`DOMAIN_ALIAS_RID_LOGGING_USERS`). PresentMon also enables `SeDebugPrivilege` to name other processes [H, `Privilege.cpp`]. An admin's OpenSSH session has the elevated token (report 11 §8.8).
- **Session name.** The default session name is "PresentMon". Use `--session_name KairoBench`, so `--stop_existing_session` can never kill another tool's session (Adrenalin ships its own PresentMon for its overlay [M]).

```powershell
& C:\kairo\tools\PresentMon-2.6.0-x64.exe `
  --process_name Cyberpunk2077.exe `
  --output_file "C:\kairo\bench\$Tag.csv" `
  --session_name KairoBench --stop_existing_session `
  --delay 5 --timed 60 --terminate_after_timed --no_console_stats `
  --qpc_time --track_gpu_video --track_frame_type
```

- `--qpc_time` writes `CPUStartQPC`, so game frames line up with Kairo's QPC-stamped tracing, for example the moment a save starts.
- `--track_gpu_video` adds `MsVideoBusy` for the **target** process (the game), not for Kairo.
- `--track_frame_type` adds `FrameType`, whose values are `Application`, `Intel XeSS-FG` and `AMD AFMF`. It needs driver instrumentation.

**Columns in the default 2.6.0 CSV** (neither `--v1_metrics` nor `--v2_metrics`), in order, from `CsvOutput.cpp` [H]:

`Application, ProcessID, SwapChainAddress, PresentRuntime, SyncInterval, PresentFlags, AllowsTearing, PresentMode, [FrameType], TimeInMs, MsBetweenSimulationStart, MsBetweenPresents, MsBetweenDisplayChange, MsInPresentAPI, MsRenderPresentLatency, MsUntilDisplayed, CPUStartTimeInMs (CPUStartQPC with --qpc_time), MsBetweenAppStart, MsCPUBusy, MsCPUWait, MsGPULatency, MsGPUTime, MsGPUBusy, MsGPUWait, [MsVideoBusy], MsAnimationError, AnimationTime, MsFlipDelay, MsAllInputToPhotonLatency, MsClickToPhotonLatency`

The README's table lags behind the code, so **read columns by header name**.

| Metric | Column | Definition used by Kairo |
|---|---|---|
| Average FPS | `MsBetweenPresents` | frames ÷ Σ frame time × 1000 (application presents, main swap chain) |
| 1% low | `MsBetweenPresents` | **1000 ÷ P99(frame time)**, nearest rank. State the definition in every report: some tools average the slowest 1% instead |
| 0.1% low | same | 1000 ÷ P99.9 |
| Present mode share | `PresentMode` | % of rows per value (e.g., `Hardware: Independent Flip` vs `Composed: Flip`) |
| GPU contention | `MsGPUBusy`, `MsGPUWait`, `MsGPUTime` | Mean and P99. **Kairo's or DWM's work interleaving with the game shows up as higher `MsGPUWait` and `MsGPUTime` at the same `MsGPUBusy`** [Inference from the column definitions] |
| Displayed cadence | `MsBetweenDisplayChange` | Secondary; affected by VRR and tearing |

### 5.2 CSV statistics (PowerShell 7)

```powershell
# pm-stats.ps1 — one PresentMon 2.6 CSV, one process → JSON line
param([Parameter(Mandatory)][string]$Csv, [Parameter(Mandatory)][string]$Exe, [string]$Tag = '')
$inv = [Globalization.CultureInfo]::InvariantCulture
function Num($v) { $d = 0.0; if ([double]::TryParse($v, 'Float', $inv, [ref]$d)) { $d } else { $null } }
function Pct([double[]]$s, [double]$p) { $s[[math]::Max(0, [math]::Ceiling($p * $s.Count) - 1)] }
$rows = Import-Csv -LiteralPath $Csv | Where-Object Application -eq $Exe
if (-not $rows) { throw "no rows for $Exe in $Csv" }
$main = ($rows | Group-Object SwapChainAddress | Sort-Object Count -Descending)[0].Group   # main swap chain
[double[]]$ft = $main | ForEach-Object { Num $_.MsBetweenPresents } | Where-Object { $_ -gt 0 } | Sort-Object
$n = $ft.Count; $sum = ($ft | Measure-Object -Sum).Sum
$busy = $main | ForEach-Object { Num $_.MsGPUBusy } | Where-Object { $_ -ne $null }
$wait = $main | ForEach-Object { Num $_.MsGPUWait } | Where-Object { $_ -ne $null }
[pscustomobject]@{
  tag = $Tag; frames = $n
  avg_fps   = [math]::Round(1000 * $n / $sum, 2)
  low1_fps  = [math]::Round(1000 / (Pct $ft 0.99), 2)
  low01_fps = [math]::Round(1000 / (Pct $ft 0.999), 2)
  gpu_busy_ms = [math]::Round(($busy | Measure-Object -Average).Average, 3)
  gpu_wait_ms = [math]::Round(($wait | Measure-Object -Average).Average, 3)
  present_modes = ($main | Group-Object PresentMode | ForEach-Object { '{0}={1:P1}' -f $_.Name, ($_.Count / $main.Count) }) -join '; '
  frame_types   = ($main | Group-Object FrameType   | ForEach-Object { '{0}={1}' -f $_.Name, $_.Count }) -join '; '
} | ConvertTo-Json -Compress
```

```powershell
# ab-stats.ps1 — paired loss with a 95% CI from per-run avg_fps (A = Kairo off, B = Kairo on, same order index)
param([double[]]$A, [double[]]$B)
$t975 = @{2=12.706;3=4.303;4=3.182;5=2.776;6=2.571;7=2.447;8=2.365;9=2.306;10=2.262;12=2.201;16=2.131}
$d = for ($i = 0; $i -lt $A.Count; $i++) { 100 * ($A[$i] - $B[$i]) / $A[$i] }   # % loss per pair
$n = $d.Count; $m = ($d | Measure-Object -Average).Average
$sd = [math]::Sqrt((($d | ForEach-Object { ($_ - $m) * ($_ - $m) }) | Measure-Object -Sum).Sum / ($n - 1))
$h = $t975[$n] * $sd / [math]::Sqrt($n)
'{0} pairs: loss {1:N2}% (95% CI {2:N2}..{3:N2}%)  PASS={4}' -f $n, $m, ($m - $h), ($m + $h), (($m -le 3) -and ($m + $h -le 4))
```

### 5.3 Encoder load from PDH "GPU Engine"

**Counter set and instance names**
- Use `\GPU Engine(*)\Utilization Percentage`. (`Running Time` also exists.)
- Instance names look like `pid_<PID>_luid_0x<HI>_0x<LO>_phys_<N>_eng_<N>_engtype_<Type>` [M: many tools, e.g. a Zabbix forum post and Rust and Go code on GitHub].
- `Get-Counter` returns the instance names **lower-cased**.

**Engine types on AMD**
- Task Manager shows AMD's engine as **"Video Codec"** (users report it on RX 6700 XT and RX 7900 XTX; on an RX 9070 XT, OBS sessions show up as "video codec" load).
- Third-party PDH code treats both `engtype_VideoEncode` and `engtype_VideoCodec` as encode [M].
- So match `^Video(Encode|Codec)$`. On AMD, `VideoCodec` includes **decode** work too. Log every engine type once in the capability report.

**Rate counters** need two collections before they give a value. Instances appear and disappear with processes, so use the `*` wildcard and filter in code.

```powershell
# encoder-usage.ps1 — who uses the video engines (run while gaming, Kairo on/off, IR on/off)
param([int]$Seconds = 30)
$s = Get-Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 1 -MaxSamples $Seconds
$rows = foreach ($set in $s) { foreach ($c in $set.CounterSamples) {
  if ($c.InstanceName -match '^pid_(\d+)_luid_\w+_phys_\d+_eng_(\d+)_engtype_(video\w*)$') {
    [pscustomobject]@{ pid = [int]$Matches[1]; eng = $Matches[2]; type = $Matches[3]; pct = $c.CookedValue } } } }
$rows | Group-Object pid, type | ForEach-Object {
  $p = Get-Process -Id $_.Group[0].pid -ErrorAction SilentlyContinue
  [pscustomobject]@{ process = $p.ProcessName; path = $p.Path; engine = $_.Group[0].type
    avg = [math]::Round(($_.Group | Measure-Object pct -Average).Average, 1)
    max = [math]::Round(($_.Group | Measure-Object pct -Maximum).Maximum, 1) } } |
  Sort-Object avg -Descending | Format-Table -AutoSize
```

```rust
// In-engine sampler. windows =0.62.2, feature "Win32_System_Performance". Signatures verified:
// PdhOpenQueryW(PCWSTR, usize, *mut PDH_HQUERY) -> u32; PdhAddEnglishCounterW(PDH_HQUERY, PCWSTR, usize, *mut PDH_HCOUNTER) -> u32;
// PdhCollectQueryData(PDH_HQUERY) -> u32;
// PdhGetFormattedCounterArrayW(PDH_HCOUNTER, PDH_FMT, *mut u32, *mut u32, Option<*mut PDH_FMT_COUNTERVALUE_ITEM_W>) -> u32
use windows::{core::*, Win32::System::Performance::*};
pub struct EngineSampler { q: PDH_HQUERY, c: PDH_HCOUNTER, buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> }
impl EngineSampler {
    pub fn new() -> Option<Self> { unsafe {
        let mut q = PDH_HQUERY::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) != 0 { return None; }
        let mut c = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(q, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut c) != 0 { PdhCloseQuery(q); return None; }
        PdhCollectQueryData(q);                                // prime the rate counter
        Some(Self { q, c, buf: Vec::new() })
    } }
    /// (pid, engine type, %), video engines only; call about once a second.
    pub fn video(&mut self) -> Vec<(u32, String, f64)> { unsafe {
        if PdhCollectQueryData(self.q) != 0 { return vec![]; }
        let (mut bytes, mut n) = (0u32, 0u32);
        if PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE, &mut bytes, &mut n, None) != PDH_MORE_DATA { return vec![]; }
        let items = (bytes as usize).div_ceil(size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
        self.buf.resize(items, Default::default());           // aligned buffer
        if PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE, &mut bytes, &mut n, Some(self.buf.as_mut_ptr())) != 0 { return vec![]; }
        self.buf[..n as usize].iter().filter(|i| i.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA || i.FmtValue.CStatus == PDH_CSTATUS_NEW_DATA)
            .filter_map(|i| {
                let name = i.szName.to_string().ok()?;          // pid_123_luid_..._engtype_VideoCodec
                let pid = name.strip_prefix("pid_")?.split('_').next()?.parse().ok()?;
                let ty = name.rsplit_once("engtype_")?.1.to_string();
                ty.starts_with("Video").then(|| (pid, ty, i.FmtValue.Anonymous.doubleValue))
            }).collect()
    } }
}
impl Drop for EngineSampler { fn drop(&mut self) { unsafe { PdhCloseQuery(self.q); } } }
```

### 5.4 Kairo self-instrumentation (built in; also feeds the capability report)

| Signal | How | Why |
|---|---|---|
| GPU time of Kairo's shader per tick | D3D11 `D3D11_QUERY_TIMESTAMP_DISJOINT` + two `D3D11_QUERY_TIMESTAMP` around the pass | Separates Kairo's 3D cost from DWM's |
| WGC arrival histogram | `SystemRelativeTime` deltas | Validates `MinUpdateInterval` and counts duplicate ticks |
| Encoder backlog | frames sent − packets received; time blocked in `receive_packet`; count of resubmits | Governor (§1.4) |
| Video-engine % (own and others) | PDH sampler (§5.3) | Headroom, IR detection |
| Kairo PID 3D/Copy engine % | Same PDH set, `engtype_3D`/`engtype_Copy` | Shows whether AMF copies internally (report 09 open item) |
| TDR history | Event ID **4101**, provider "Display" (e.g., "Display driver amdwddmg stopped responding and has successfully recovered"); `C:\Windows\LiveKernelReports\WATCHDOG\*.dmp` | Correlate clip failures with driver timeouts [M] |

```rust
// GOTCHA: windows-rs maps S_FALSE ("not ready") to Ok(()) because HRESULT::ok() accepts any hr >= 0.
// Call through the vtable and compare with S_OK yourself. windows =0.62.2, "Win32_Graphics_Direct3D11".
use windows::{core::Interface, Win32::{Foundation::S_OK, Graphics::Direct3D11::*}};
unsafe fn ready<T: Default>(ctx: &ID3D11DeviceContext, q: &ID3D11Query) -> Option<T> {
    let mut v = T::default();
    let hr = (Interface::vtable(ctx).GetData)(Interface::as_raw(ctx), q.as_raw(),
        &mut v as *mut T as *mut _, size_of::<T>() as u32, D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32);
    (hr == S_OK).then_some(v)                    // S_FALSE = not ready yet; poll next tick
}
// disjoint: D3D11_QUERY_DATA_TIMESTAMP_DISJOINT { Frequency: u64, Disjoint: BOOL }; stamps: u64
```

```powershell
Get-WinEvent -FilterHashtable @{ ProviderName = 'Display'; Id = 4101 } -MaxEvents 20 -ErrorAction SilentlyContinue | Select TimeCreated, Message
Get-ChildItem C:\Windows\LiveKernelReports -Recurse -Filter *.dmp -ErrorAction SilentlyContinue | Select FullName, LastWriteTime
```

### 5.5 Workloads for a repeatable A/B

1. **`kairo-loadgen`** (new, small, part of the test harness). It extends the brief's D3D11 test-pattern window and is fully automatable over SSH:
   - a flip-model swap chain (`DXGI_SWAP_EFFECT_FLIP_DISCARD`, `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING`, `Present(0, DXGI_PRESENT_ALLOW_TEARING)`), borderless fullscreen on the 360 Hz OLED, uncapped;
   - two modes:
     - **ALU-bound**: the shader below, with `iters` tuned to about 8 ms per frame;
     - **bandwidth-bound**: random reads from a 256 MB texture. Kairo's cost is partly bandwidth, so this mode matters;
   - deterministic, time-based animation and a fixed duration.
2. **Two or three real games with built-in benchmarks** (fixed camera paths). One should be a GPU-bound AAA title. One should be a high-fps esports title, which is CPU-bound, to expose Kairo's CPU cost. Use what the user actually plays where possible.
3. **The user's own play session**, for the soak test only. It is not repeatable enough for the 3% gate.

```hlsl
// loadgen ALU mode: deterministic per-pixel cost; iters sets GPU time per frame.
cbuffer P : register(b0) { float t; uint iters; float2 res; }
float4 main(float4 pos : SV_Position) : SV_Target {
    float2 uv = pos.xy / res; float3 c = 0;
    [loop] for (uint i = 0; i < iters; ++i) {
        uv = frac(uv * 1.0001 + float2(sin(t + i * 0.01), cos(t - i * 0.013)) * 0.001);
        c += 0.5 + 0.5 * cos(6.2831 * (uv.xyx + float3(0, 0.33, 0.67) + t));
    }
    return float4(c / iters, 1);
}
```

### 5.6 Matrix and tests (S1/S2 on the user's PC)

| # | Test | Conditions | Pass / output |
|---|---|---|---|
| 1 | **Noise floor (A/A)** | Kairo off, 6 runs per workload | Run-to-run CV of avg FPS. If CV > 1%, add runs or fix the setup |
| 2 | **Gate (A/B)** | Kairo default (WGC monitor, H.264 1080p60, `balanced`), IR **off**, HDR off. Pattern ABBA ×3 = 12 runs per workload | Loss ≤3% and upper CI ≤4%; 1%-low loss ≤5%; `PresentMode` shares unchanged |
| 3 | Capture rate on 360 Hz and 180 Hz | `MinUpdateInterval` 16.67 / 13.9 / 11.1 / 8.33 ms | Duplicate-tick % at 60 fps when game fps ≥ 90 (target ≤1%), plus FPS loss |
| 4 | Codec and preset | H.264 balanced/quality; HEVC; AV1 (and the AV1 1080p `clap` check: `ffprobe -show_streams` coded_height plus `clap` in the trace) | ΔFPS ≈ 0 expected; VideoCodec % per config; RDNA3 vs RDNA4 alignment behaviour |
| 5 | Capture path | WGC window vs WGC monitor vs DDA; copy-on-arrival vs convert-on-arrival | Pick the lowest-cost default |
| 6 | GPU priority | 0 vs −2 (`SetGPUThreadPriority`) | FPS and 1% low vs clip duplicate ticks |
| 7 | HAGS | On vs off (reboot) | Sensitivity; log only |
| 8 | HDR | HDR on (FP16 path + tone-map) | Loss ≤3%? If not, document it |
| 9 | **Coexistence** | IR alone; Kairo + IR; Kairo + video playing on the 180 Hz screen (RDNA4 decode on the same VCN) | Inform the warning text; governor behaviour; no encoder deadlock |
| 10 | AFMF | §3.2 | Visible vs invisible; warn rule |
| 11 | Toast vs AMD overlay | Toast shown and hidden 100× with the Adrenalin Metrics Overlay on (26.10.1 known issue) | No TDR (4101); `PresentMode` back to Independent Flip within 1 s of hiding |
| 12 | Clip quality (offline) | Record a 60 s high-bitrate mezzanine with `ddagrab` + `hevc_amf -rc cqp -qp_i 12 -qp_p 12`. Re-encode with each AMF profile on the user's GPU. VMAF with a GPL FFmpeg build that has libvmaf (dev machine only, never shipped) | VMAF ≥93 (1080p) at the chosen bitrates; decide `balanced` vs `quality` and VBAQ |
| 13 | **Soak** | 4–8 h of real play, buffer on, 1440p OLED, HAGS as the user has it | Zero 4101 events; zero stalls over 2 s; watchdog recoveries logged |
| 14 | Optional: AMD DisplayCapture | FFmpeg 9.0.2 has `vsrc_amf` (`monitor_index`, `framerate`, `capture_mode=keep_framerate\|wait_for_present\|get_current`) outputting `AMF_SURFACE` [H]. Run `ffmpeg -filter_complex vsrc_amf=framerate=60 -c:v h264_amf …` vs `gfxcapture`. Check `ffmpeg -filters` for it in the BtbN LGPL build | Understand what ReLive's path costs. Adopting it would change house rule 2 ("WGC or DDA only"), so it is information only |

---

## 6. Plan impacts

1. **S1 gets the AMD protocol:**
   - IR off; AMD features off (§7 checklist);
   - PresentMon 2.6.0 with `--session_name`, `--qpc_time`, `--track_gpu_video` and `--track_frame_type`;
   - PDH video-engine logging;
   - `kairo-loadgen`;
   - tests 1–3, 5, 7 and 10 of §5.6.
2. **Encoder options (replaces brief §2.4 "AMD" row and amends report 09 §4.2):**
   - set `usage=transcoding`, `preanalysis=0`, `high_motion_quality_boost_enable=0`, `preencode=0` and `async_depth=4` explicitly;
   - AV1 also gets `latency=power_saving_real_time` and `aq_mode=caq`;
   - 1440p60 H.264 bitrate becomes **36/50 Mbps** (the brief had 40/60);
   - a "Quality" profile (`quality`, `preencode=1`, still no PA) is user-selectable.
3. **Encoder probe becomes lazy and sequential:** H.264 only by default; ≥1 s between sessions; results cached per driver store version and AMF version (Sunshine #5385). It runs in the engine's "probe" role so a failure can't take the buffer down.
4. **Add an encoder governor and output-stall watchdog** (§1.4): preset step-down, then fps step-down, then session recreate; an `encoderPressure` notification over the pipe.
5. **Capture rate:** use the refresh-aware `MinUpdateInterval` rule. Add a convert-on-arrival experiment to S2 (it touches reports 08 and 09).
6. **Capability report adds:**
   - driver store version (DXGI);
   - AMF runtime version (`AVAMFDeviceContext.version`);
   - VCN count from the SKU table;
   - HAGS (D3DKMT);
   - HDR and BitsPerColor per output; refresh rates;
   - IR / Record Desktop / Metrics Overlay hints (registry);
   - video-engine users (PDH);
   - TDR count in the last 7 days (event 4101).
7. **Onboarding:** detect IR and show the coexistence warning. Parse AMD's hotkey registry to flag conflicts with user-chosen Kairo hotkeys.
8. **Toast:** create the window once and hide it when idle; never leave a transparent topmost window up. Keep the sound cue as the primary feedback. Gate the toast on test #11.
9. **Budget statement:** "≤3% average FPS loss **with AMD Instant Replay off**". Measure the coexistence cost separately and show it in the warning.
10. **Test harness deliverables:** `pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1`, an AB runner, and `kairo-loadgen`.
11. **ADLX:** deferred to v1.x behind a licence review. v1 detection uses DXGI, D3DKMT, the registry and PDH.
12. **Brief corrections:** the LTT table (§2.1), the 1440p bitrate, and the RDNA3 AV1 status ("confirmed hardware limitation, 1440p unaffected").

---

## 7. Open items / [Unverified]

- Whether WGC/DDA capture AFMF-generated frames (§3.2).
- Whether an active WGC session (monitor or window) demotes a game from Independent Flip. Indirect evidence: ForceComposedFlip needs a topmost window to force Composed Flip while capture is running, which suggests capture alone doesn't. PresentMon decides.
- The real game-FPS cost difference between WGC window and monitor capture, and between copy-on-arrival and convert-on-arrival.
- Which engine runs VBAQ, pre-encode and HMQB (VCN or shader).
- Whether VCN 5 reports `8X2_ONLY` or no alignment for AV1 (i.e., no 1082 at 1080p).
- AMD's registry value names for IR (HKLM vs HKCU) and the IR host process. Confirm with the diff script and PDH.
- Whether PDH "GPU Engine" instances of other processes are visible to a standard (non-elevated) user.
- Whether `SetGPUThreadPriority(-2)` changes anything under HAGS on RDNA3/4.
- Whether `AMF_VIDEO_ENCODER_CAP_REQUESTED_THROUGHPUT` includes other processes' sessions.
- WGC cursor capture: OBS maintainers said in 2021 that WGC "disables the hardware cursor (which makes the cursor run at a lower frame rate)". Check cursor smoothness on the 360 Hz screen with `IsCursorCaptureEnabled(true)` and `false`.
- The "10-bit pixel format" setting's effect on DDA in 2026 drivers.

---

## 8. Recommended AMD defaults

| Area | Default | Notes |
|---|---|---|
| Codec | **H.264 High, `h264_amf`** | HEVC as the "better quality on AMD" option; AV1 opt-in on RX 7000+ (avoid 1080p AV1 on RDNA3) |
| AMF options | `usage=transcoding quality=balanced rc=vbr_peak vbaq=1 preanalysis=0 high_motion_quality_boost_enable=0 preencode=0 bf=0 max_b_frames=0 frame_skipping=0 filler_data=0 coder=cabac profile=high header_spacing=60 async_depth=4` | Set each with `av_opt_set`; fail on rejection; log the read-back |
| Bitrate (target/peak/VBV) | 1080p60 25/40/40 Mbps; 1440p60 36/50/50 | HEVC 16/25 and 24/36; AV1 12/20 and 18/28. Validate with VMAF |
| GOP | 60 (1 s), every keyframe IDR | Report 09 |
| Input to AMF | NV12 rendered by Kairo into FFmpeg's D3D11 pool on the game's adapter | Never BGRA |
| Capture rate | Refresh-aware `MinUpdateInterval` (360 Hz → 13.9 ms; 180 Hz → 11.1 ms; 60 Hz → 1/90 s) | Verify the duplicate rate |
| Copies | Copy-on-arrival in S2; switch to convert-on-arrival if the A/B shows a gain | – |
| GPU / CPU priority | Untouched GPU priority; normal process priority; MMCSS for the capture/encode thread; EcoQoS opt-out | `SetGPUThreadPriority(-2)` stays an experiment |
| Encoder governance | 85% video-engine ceiling → step the preset down; >2 s output stall → recreate the session | Notify about competing users of the encoder |
| Probe | Lazy, sequential, cached per driver version; H.264 only unless opted in | Sunshine #5385 |
| Coexistence | Detect IR (PDH + registry); warn; never touch AMD settings | – |
| Feedback | Sound cue; toast window created once and hidden when idle | 26.10.1 overlay known issues |
| Diagnostics | Capability report fields from §6.6 | – |

---

## 9. Measurement procedure (step by step, on the gaming PC)

1. **Freeze the environment and record it in `bench/env.json`:**
   - Adrenalin and driver store versions, Windows build;
   - HAGS, HDR, VRR, refresh rates;
   - game settings; Kairo commit and config.
2. **Turn off these AMD features**, globally *and* in the game's profile:
   - Instant Replay, Record Desktop, In-Game Replay;
   - the Metrics Overlay and logging;
   - AFMF, Chill, Boost, FRTC, Enhanced Sync, RSR.

   Also: VSync off in the game; Game Bar capture off; no OBS, Medal, Discord streaming or Steam recording. The second monitor shows a static desktop with no video.
3. **Tools:** PresentMon 2.6.0; the four PowerShell scripts; `kairo-cli`, which starts and stops the buffer over the pipe; `kairo-loadgen`. Launch the game or loadgen into the interactive session with the `schtasks /it` task (report 11).
4. **Warm-up:** run the workload once for 2 minutes and discard it.
5. **A/A noise floor** (test 1). If CV > 1%, fix the noise first (thermals, background tasks, power plan) or increase the run count.
6. **A/B gate** (test 2):
   - order ABBA ×3; 60 s per run with `--delay 5`; 30 s idle between runs;
   - "A" means the Kairo engine process is **not running**; "B" means it runs with the default config and the buffer armed;
   - run `encoder-usage.ps1` in parallel during one B run.
7. **Analyse:**
   - `pm-stats.ps1` on each CSV, then `ab-stats.ps1` per workload;
   - check `present_modes` and `gpu_wait_ms` deltas;
   - read Kairo's tracing for duplicate ticks, encoder backlog and shader GPU time.
8. **Variations** (tests 3–8, 10–12). Each is a smaller A/B against the default: 3 pairs, expanding to 6 if the result is within 1% of a decision threshold.
9. **Coexistence** (test 9). Report "IR alone", "Kairo alone" and "both". This feeds the onboarding warning numbers.
10. **Soak** (test 13). Run overnight-length play. Afterwards check event 4101, LiveKernelReports and Kairo's watchdog log.
11. **Publish** a `bench/<date>-amd.md` summary: a table of loss % with CI per workload and configuration, plus the env.json. Attach it to the capability-report baseline for the beta.

---

## 10. Gotchas

1. **`usage=high_quality` turns on PA (OpenCL), TAQ, HMQB and VBAQ** behind your back. Always set `usage=transcoding` and `preanalysis=0` explicitly.
2. **Pre-Analysis runs on the game's GPU** (OpenCL kernels by default) and FFmpeg can't move it to another engine.
3. **Don't feed BGRA/RGBA to AMF.** It can hit DCC-compressed render targets (RX 7000, 25.10.2+) or non-native-format corruption. NV12 only.
4. **`async_depth` defaults to 16** (≈88 MB of NV12 at 1440p held in flight). Set 4.
5. **FFmpeg's AMF receive path may `Sleep(1)`.** With the default timer resolution that is up to about 15.6 ms. Never call the encoder from the pacer thread.
6. **Creating several AMF sessions back to back can hang an RX 9070 XT** on 26.6.4 (Sunshine #5385). Probe lazily and sequentially; cache the results.
7. **h264_amf can stall for ~20 s at a very high target bitrate** (RX 9060 XT, #614). Cap the peak at 50 Mbps and watchdog output gaps.
8. **The encoder can stay "overloaded" after a transient spike** (RX 9070 XT, OBS #13670). Recreate the session when the backlog doesn't drain.
9. **Long sessions can end in `AMF_INPUT_FULL` and a TDR** (RX 7900 GRE, 1440p OLED replay buffer, 26.8.1). Soak test, rebuild on device loss, and keep the TDR history in diagnostics.
10. **RDNA3 AV1 1080p is coded 1082 lines tall**, a hardware limit confirmed by AMD in Oct 2026. FFmpeg crops 2 lines (`clap`). Players may not honour `clap`. 1440p is aligned and fine.
11. **RX 9000 has one VCN for both encode and decode.** A video on the second monitor, IR and Kairo all share it.
12. **FFmpeg can't select the VCN instance** (`INSTANCE_INDEX`). On RX 7700+, Kairo and IR probably land on the same encode VCN.
13. **`amfrt64.dll`'s file version string is unreliable** on recent drivers. Use `AMFQueryVersion` (`AVAMFDeviceContext.version`) and the DXGI driver store version.
14. **On AMD the PDH engine type is usually `VideoCodec`** (encode + decode), not `VideoEncode`. `Get-Counter` lower-cases instance names.
15. **Adrenalin ships its own PresentMon.** Always pass `--session_name`; `--stop_existing_session` with the default name could kill another tool's session.
16. **PresentMon's `MsVideoBusy` is the game's video work, not Kairo's.** Use PDH for Kairo's.
17. **The PresentMon 2.6 default CSV mixes old and new metric names**, and the README lags the code. Parse by header name.
18. **Chill, Boost, AFMF, FRTC and Enhanced Sync invalidate an A/B.** Turn them off globally *and* in the game's profile. Boost and Chill switch each other off.
19. **A transparent topmost window over a game can force Composed Flip** (+1 frame latency, VRR loss). Hide Kairo's toast when idle. 26.10.1 also lists timeouts when overlays open and close alongside AMD's metrics overlay.
20. **windows-rs `ID3D11DeviceContext::GetData` returns `Ok(())` for S_FALSE** ("not ready"). Use the raw vtable call and compare with `S_OK`.
21. **D3DKMT `WDDM_2_7_CAPS` is "reserved for system use".** Use it for logging only, the same way OBS does.
22. **AMD's in-game hotkey hook (`amdihk64.dll`) means `RegisterHotKey` success doesn't prove there is no conflict.** Parse `HKLM\SOFTWARE\AMD\DVR\*Hotkey`.
23. **ADLX is not MIT** (it has its own SDK licence PDF). Clear it before using it in GPL-3.0 Kairo.
24. **Desktop Ryzen iGPUs add a second adapter.** Bind capture, conversion and AMF to the adapter that drives the game's monitor (by LUID), or every frame crosses PCIe.
25. **The LTT numbers are 6 Mbps H.264 on 8 GB cards in three D3D12 games.** Kairo's defaults (25 Mbps, 1080p from 1440p, a 16 GB card perhaps) differ. Re-measure on the user's rig and don't quote LTT as Kairo's cost.

---

## Sources

**LTT Labs and coverage**
- LTT Labs, "Your Screen Recorder Wants to Steal Your Frames!" (2026-08-24): https://www.lttlabs.com/articles/2026/08/24/your-screen-recorder-wants-to-steal-your-frames (read through https://r.jina.ai/ because Cloudflare blocks direct fetches)
- GIGAZINE (2026-08-26): https://gigazine.net/gsc_news/en/20260826-pc-screen-recorder/

**OBS Studio (`master`, read 2026-10-21)**
- https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-subsystem.cpp (set_priority, HAGS status)
- https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/CMakeLists.txt · https://github.com/obsproject/obs-studio/blob/master/CMakePresets.json · https://github.com/obsproject/obs-studio/blob/master/.github/workflows/build-project.yaml (GPU_PRIORITY_VAL)
- https://github.com/obsproject/obs-studio/blob/master/plugins/obs-ffmpeg/texture-amf.cpp
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/game-capture.c · https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/graphics-hook/graphics-hook.h · https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/graphics-hook/d3d12-capture.cpp · https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/data/locale/en-US.ini
- Issues: https://github.com/obsproject/obs-studio/issues/10594 · https://github.com/obsproject/obs-studio/issues/13670 · https://github.com/obsproject/obs-studio/issues/8929 · https://github.com/obsproject/obs-studio/issues/4164 · https://github.com/obsproject/obs-studio/issues/13337 · https://github.com/obsproject/obs-studio/pull/12759
- OBS forum thread (10-bit advice, RX 9070 XT): https://obsproject.com/forum/threads/new-amd-rx-9070-xt-in-obs-av1-jumps-over-selected-bit-rate-up-to-90000-and-youtube-drops-connection.186757/

**AMD AMF SDK (v1.5.3 docs, commit 57ee0cfb, 2026-10-16) and wiki (HEAD ea768b9)**
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_Encode_API.md
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_Encode_HEVC_API.md
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_Encode_AV1_API.md
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_PreAnalysis_API.md
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Display_Capture_API.md
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/GPU%20and%20APU%20HW%20Features%20and%20Support (edited 2025-12-05) · https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/AV1%20Encoder · https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/Recommended%20FFmpeg%20Encoder%20Settings
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/releases (v1.5.3, 2026-09-29)
- Issues: #617 https://github.com/GPUOpen-LibrariesAndSDKs/AMF/issues/617 · #614 …/614 · #599 …/599 · #593 …/593 · #591 …/591 · #589 …/589 · #587 …/587 · #586 …/586 · #579 …/579 · #574 …/574 · #571 …/571 · #562 …/562 · #548 …/548

**FFmpeg 9.0.2 (tarball ffmpeg-9.0.2.tar.xz; GitHub tag n9.0.2)**
- libavcodec/amfenc.c (receive loop, `av_usleep`), amfenc_h264.c (options, QUERY_TIMEOUT, PA keep-on), amfenc_hevc.c, amfenc_av1.c (align, crop 8 → 2, RC quirk, latency modes); libavutil/hwcontext_amf.h/.c (`AVAMFDeviceContext.version`, `AMFQueryVersion`); libavutil/time.c (`av_usleep`); libavfilter/vsrc_amf.c (AMF DisplayCapture source); configure (`amf_capture_filter_deps`)
- FFmpeg Forgejo PRs: https://code.ffmpeg.org/FFmpeg/FFmpeg/pulls/24696 · https://code.ffmpeg.org/FFmpeg/FFmpeg/pulls/24688

**AMD support and release notes (amd.com)**
- DH3-023 Record & Stream: https://www.amd.com/en/resources/support-articles/faqs/DH3-023.html · DH3-038 Performance Metrics: https://www.amd.com/en/resources/support-articles/faqs/DH3-038.html · DH3-033 Radeon Boost · DH-033 Radeon Chill · DH3-008 colour depth
- AFMF: https://www.amd.com/en/products/software/adrenalin/afmf.html · AFMF preview RN: https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-23-40-01-10-PREVIEW.html
- Release notes (RN-RAD-WIN-…): 23-12-1 (HAGS), 25-3-2, 25-4-1, 25-6-1, 25-8-1, 25-9-1, 25-10-2, 26-7-1, 26-8-1, 26-9-2, 26-10-1, e.g. https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-26-10-1.html (25-4-1, 25-10-2, 26-8-1 and 26-10-1 were re-fetched live today)

**ADLX**
- https://github.com/GPUOpen-LibrariesAndSDKs/ADLX (v2.0, 2026-09-20; SDK/Include/ADLX.h, ADLXVersion.h, I3DSettings*.h, IDisplaySettings.h, IApplications.h; "ADLX SDK License Agreement.pdf")
- https://crates.io/crates/adlx (0.0.0-alpha.1) · https://crates.io/crates/shiguredo_amf (2026.3.0, Linux-only)

**PresentMon (tag v2.6.0, e13fce6a)**
- https://github.com/GameTechDev/PresentMon/releases/tag/v2.6.0 · README-ConsoleApplication.md · PresentMon/CsvOutput.cpp · PresentMon/CommandLine.cpp · PresentMon/Privilege.cpp

**Microsoft**
- IDXGIDevice::SetGPUThreadPriority: https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgidevice-setgputhreadpriority
- D3DKMTSetProcessSchedulingPriorityClass: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmthk/nf-d3dkmthk-d3dkmtsetprocessschedulingpriorityclass
- D3DKMT_WDDM_2_7_CAPS (docs source): https://github.com/MicrosoftDocs/windows-driver-docs-ddi/blob/staging/wdk-ddi-src/content/d3dkmdt/ns-d3dkmdt-d3dkmt_wddm_2_7_caps.md

**Rust crates (crates.io source, read today)**
- windows 0.62.2 (still the latest stable; 0.100 family exists for windows-core and windows-registry): `Win32/System/Performance` (PDH), `Win32/System/Registry` (`RegGetValueW`), `Win32/Graphics/Dxgi` (`SetGPUThreadPriority`, `CheckInterfaceSupport`, `DXGI_OUTPUT_DESC1`), `Win32/Graphics/Direct3D11` (`GetData`, timestamp queries), `Wdk/Graphics/Direct3D` (D3DKMT); windows-result 0.4.1 (`HRESULT::ok`)
- https://crates.io/api/v1/crates/windows · https://crates.io/api/v1/crates/windows-registry · https://crates.io/api/v1/crates/windows-core

**Third-party code and docs used as [M] evidence**
- Sunshine: https://github.com/LizardByte/Sunshine/issues/5385 · https://github.com/LizardByte/Sunshine/blob/master/docs/configuration.md (AMD options)
- Jellyfin AMD hardware acceleration: https://jellyfin.org/docs/general/post-install/transcoding/hardware-acceleration/amd/
- ForceComposedFlip README: https://github.com/fernandoenzo/ForceComposedFlip
- AMDHotkeys (default hotkey table): https://github.com/johngibbs/AMDHotkeys/blob/51fa86e94c7c5d44e4d9f78d7c0ff316414b1b26/AMDHotkeys.psm1
- AMDLSS5 AMD DVR registry probe: https://github.com/Wehaveall/AMDLSS5/blob/51e8f342929f8632e4d5f27bd3e84a81db5b3b4a/src/helper/runtime/overlay_amd.cpp
- AMD DVR tweak scripts: https://github.com/HickerDicker/SapphireOS (AMD Dwords by imribiy.bat) · https://github.com/ToXTweaks/ToX-Free-Utility (GPUTweaks.cs)
- PDH usage in Rust: https://github.com/shm11C3/HardwareVisualizer/blob/main/core/src/infrastructure/providers/windows/pdh_provider.rs · engine-type test: https://github.com/McHauge/Spout-Multi-Recorder (internal/hwstat/parse_windows_test.go)
- AMD community, Task Manager "Video Codec 0": https://community.amd.com/t5/pc-drivers-software/dont-showing-video-ecoder-task-manager/m-p/639495
- GPU specs (RX 9060 XT 8 GB ≈320 GB/s; RTX 5060 ≈448 GB/s): https://www.techspot.com/specs/gpu/308914-amd-radeon-rx-9060-xt-8gb.html · https://www.databasemart.com/blog/amd-rx9060xt-vs-nvidia-rtx5060
- HAGS on RX 7000 (23.12.1): https://www.neowin.net/news/amd-finally-brings-windows-hardware-accelerated-gpu-scheduling-to-rx-7900-7800-7000-series/
- Anti-Lag 2 SDK: https://gpuopen.com/learn/integrating-amd-radeon-anti-lag-2-sdk-in-your-game/
- Event 4101 background: https://gaming-st.com/trouble-settings/event-id-4101-display-driver-stopped-responding-2026/

**Kairo research (this folder)**
- 00-brief.md, 01-video-capture.md, 03-triggers-safety-detection.md, 04-competitors.md, 08-capture-rust.md, 09-encode-mux-rust.md, 11-engine-shell-rust.md
