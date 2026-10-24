# kairo-verify: container and bitstream checks (design)

*2026-10-24 · Design only: no repo code was written. This covers the checks that need no decoding. Drops and duplicates, cadence, bars PSNR, A/V by flash and beep, routing and tones are decode-based and designed elsewhere.*

**Citation keys.** `PLAN` = docs/PLAN.md · `M1` = docs/milestones/M1.md · `CL` = CLAUDE.md · `R01/R06/R09/R10/R12/R13` = docs/research/NN-*.md · `CB` = docs/research/15-implementation-cookbook.md · `core/x.rs` = crates/kairo-core/src/x.rs · `deny.toml`, `Cargo.toml` at the repo root. Repo line numbers are from `main` at 21f0770; against 3775e7c only M1.md lines after 152 moved, by +3. FFmpeg sources at tag **n9.0.2**, the PC's line (fetched read-only from raw.githubusercontent.com/FFmpeg/FFmpeg/n9.0.2/…; copies in `container/upstream/`): `movenc.c`, `mov.c`, `mux.c`, `amfenc_h264.c`, `vf_setparams.c`, `fftools/ffmpeg_enc.c`. **Experiment IDs** (e01–e21, b00–b10, m01–m06) are files I made on the Mac under `verify-design/container/clips/` with Homebrew FFmpeg **8.0.1** (CLI only, `h264_videotoolbox` and native `aac`, LGPL filters only). Their dumps are in `container/dumps/`. None of them goes into git (CL:18, M1:119).

---

## 0. Decisions in brief

1. **Own parser, no crate.** A small, pure-Rust, read-only MP4 box reader plus a minimal H.264 and AAC config parser, all in `kairo-verify`'s `container` module. It has zero dependencies, `#![forbid(unsafe_code)]` and no `unwrap`, and it builds and tests on the Mac, Linux CI and Windows. No crate fits a verifier (§3).
2. **H.264, parse headers only, never decode.** The parser covers NAL framing, the SPS through the VUI (colour, profile/level, cropping, reorder limits), the first four PPS fields, slice headers through `pic_order_cnt_lsb`, and SEI payload types. That is enough for IDR at 0, closed GOP, no B-frames, `colr` = VUI and one session per file (§4).
3. **Fifteen checks** (C0–C14, §6), grouped into profiles (`clip`, `harness`, `selftest`, `share`, `discord`, `whatsapp`, `reels`, `original`; §7). Each check reports measured against expected. A failed structure check (C0) stops the dependent checks, which are reported as not run.
4. **Two rules the sources didn't spell out, now proven on real files:**
   - **CFR with a held first frame.** In a correct Kairo clip, video sample 1 is longer than the others, by the audio pre-roll. In e20, `stts` is `[[1,584],[179,256]]`: 584 = 256 + 328. So "CFR" means every sample after the first is exactly one tick, and the first sample's extra length must equal an audio lead of 1,024–2,047 samples (C5, C6).
   - **Research 09's start-offset test can't work.** Its ffprobe `start_time` test (R09:978-979) reads 0.000 for every stream in a good clip, in an edit-list clip and in a misaligned clip alike (e20, e03, b09). C6 replaces it.
5. **A real harness pitfall, found while testing.** With the FFmpeg 8 CLI, `-color_primaries` and `-color_trc` never reached the encoder. The result was VUI 2/2/1 and **no `colr`** (e01). `setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv` fixes it (e20). Gate 2's capture command must use it (§10).
6. **Gate 1's container defects are stream-copy remuxes of a good clip, one defect each.** On the Mac, each copy failed exactly its own check, and the control copy passed all 13 checks the model implements (§9.3 matrix). Only the B-frame bad clip needs a re-encode.
7. **ffprobe is a cross-check, never a dependency.** It never ships (M1:56, M1:70). "Test my setup" runs kairo-verify's analysis on friends' PCs (PLAN:271). Several checks are invisible to it (§8).

---

## 1. Scope and sources

What the plan asks kairo-verify to check without decoding: IDR at 0, a closed 1 s GOP, pts = dts, moov first, no edit list, `colr`, CFR and export profiles (PLAN:351, M1:12). Gate 1 needs it to flag no faststart, edit list, B-frames, wrong `colr` and swapped tracks (M1:26, PLAN:417). M3's timed gate adds "30 s ± 1 GOP, CFR 60 … mix + game + mic" (PLAN:446). The engine's own save verify covers "duration, IDR at 0, tracks, moov first" (PLAN:179).

The constraints that bind this design:
- **Licences.** No GPL deps (CL:17). Crates must be on deny.toml's permissive allow-list (deny.toml:17-34).
- **Safety.** `unsafe` only in capture/gpu/encode/audio/platform/input modules, and no `unwrap` (CL:32, Cargo.toml:40-48).
- **Privacy.** Never log raw paths (CL:16, PLAN:269).
- **No ffprobe.** Its own analysis must work without `ffprobe.exe` (M1:56).

---

## 2. Evidence: what FFmpeg actually writes

The good clip **e20**: 3 s of 1280×720 at 60 fps, plus Mix (stereo, 160k), Game (stereo, 160k) and Mic (mono, 128k). It used `-movflags +faststart -use_editlist 0`, `setparams` for BT.709, `-g 60 -bf 0` and a default disposition on Mix only. Its full tree is in `dumps/e20_good_setparams.tree.txt`. Abridged:

```
ftyp @0 32  isom/512 [isom iso2 avc1 mp41]
moov @32 8021
  mvhd timescale 1000 duration 3022 next_track_id 5
  trak  tkhd flags 3 id 1 alt_group 0 1280x720 | mdhd 15360 dur 46408 und | hdlr vide "Video"
        stsd 1 entry: avc1 1280x720 { avcC High@3.1 len4 SPS 2764001fac56805005ba6a04040204 PPS 28ee3cb0,
                                      colr nclx 1/1/1 full_range 0, pasp 1:1, btrt }
        stts [[1,584],[179,256]]  stss [1,61,121]  (no ctts)  stsc/stsz/stco (141 chunks)
  trak  tkhd flags 3 id 2 alt_group 1 | mdhd 48000 dur 145024 | hdlr soun "Mix"  | udta/name "Mix"
        mp4a ch 2 48000 { esds OTI 0x40 type 5 ASC 11 90 56 e5 00 } stts [[141,1024],[1,640]] sgpd/sbgp roll −1
  trak  tkhd flags 2 id 3 alt_group 1 | hdlr "Game" | udta/name "Game" | mp4a ch 2 (same ASC)
  trak  tkhd flags 2 id 4 alt_group 1 | hdlr "Mic"  | udta/name "Mic"  | mp4a ch 1 ASC 11 88 56 e5 00
  udta/meta/ilst ©too "Lavf62.3.100"
free @8053 8
mdat @8061 2284927
```

| # | Fact | Proof (Mac, FFmpeg 8.0.1) | Same in 9.0.2? (source) |
|---|---|---|---|
| F1 | Faststart order is `ftyp moov free mdat`. Without it the order is `ftyp free mdat moov`. The 8-byte `free` is the placeholder for a 64-bit mdat header | e20 vs b01; e02 | movenc.c:6180-6187 writes `free` then `mdat`; 8993-9014 overwrites the `free` when mdat > 4 GiB |
| F2 | The default `use_editlist` (auto → 1) puts an `edts/elst` in **every** track: video `media_time` 0, AAC 1024 (the priming) | e03 | movenc.c:122 (default −1), 8203-8213 (→ 1), 4410-4416 (edts for every track with samples) |
| F3 | With `use_editlist 0` there is no `edts`. The negative AAC start shifts all streams (make_zero), and video sample 1 is forced to dts 0 and **lengthened**: `stts [[1,584],[179,256]]`, 584 = 256 + ⌈1024·15360/48000⌉ | e01, e20 | movenc.c:8222-8224 (make_zero), 7282-7290 (first sample forced to 0); mux.c:636-679 (lowest ts across the queue, rescaled per stream with `AV_ROUND_UP`) |
| F4 | `colr nclx` is written only if primaries, transfer and matrix are all specified. Layout: 3 × u16 plus a byte holding `full_range<<7`. It is the second child of `avc1`, after `avcC` | e04 (none), e20 | movenc.c:2944-2953, 2611-2658 (as R09:683-684 says) |
| F5 | **CLI colour pitfall.** As output options, `-color_primaries` and `-color_trc` did not reach the encoder; `-colorspace` and `-color_range` did. VUI 2/2/1, no `colr`. `setparams` fixes it | e01, e05, e06 vs e20 | ffmpeg_enc.c:263-270 copies the encoder's colour from the frame; vf_setparams.c:149-171 negotiates only colorspace and range; :225 `AVFILTER_FLAG_METADATA_ONLY` |
| F6 | A stream copy with colour options writes a `colr` that disagrees with the VUI (`colr` 5/5/5, VUI unchanged). `h264_metadata` changes only the VUI | e14, b03; e15, b05 | (CLI behaviour; bsf is LGPL) |
| F7 | B-frames: `ctts` with non-zero offsets, 131 of 180 samples with B slices, `stss` [1,58,115,172], irregular `stts` [328,256,1,511,…]. With an edit list, video `media_time` is 256 | e07, e08 | movenc.c:7330-7331 (CTTS only if dts ≠ pts), 3423-3428 |
| F8 | The default disposition becomes the `tkhd` enabled flag: flags 3 on video and Mix, 2 on the others. Audio `alternate_group` is 1. **Two defaults** give two enabled tracks and per-stream groups [1,2,3]. No default enables the first audio track | e20; e09, b08; e10 | movenc.c:7885-7925, 4011-4014, 4023-4024; mov.c:5738 (enabled → default on read) |
| F9 | `title` → `trak/udta/name`; `handler_name` → `mdia/hdlr` name. The defaults are "VideoHandler" and "SoundHandler". **A stream-copy remux drops `udta/name`** | e20; e16, e17; e13 | movenc.c:4373, 3624. mov.c (8.0.1) ignores `name`; 9.0.2 reads it as tag `name` (mov.c:396), but movenc writes it from `title`, so copies probably still drop it [Unverified on 9.0.2] |
| F10 | AAC: `mp4a` samplerate 48000; esds OTI 0x40, stream type 5; ASC = AOT 2, index 3 (48 kHz), channels 2 or 1, 1024-sample frames, **plus an explicit "SBR absent" extension** (0x2B7, AOT 5, flag 0). The CLI's last packet is short (`[1,640]`); `roll` sgpd −1 | e20, e21 | movenc.c:3305-3395 (roll) |
| F11 | `mdhd` duration = Σ `stts`. `tkhd` duration = ⌈mdhd × 1000 / timescale⌉ (3022 for 3.0214 s). `mvhd` uses timescale 1000 | e20 | movenc.c:3993-3995 (`AV_ROUND_UP`) |
| F12 | `stco` below 4 GiB, `co64` only when an offset needs it. A 64-bit mdat uses the `free` placeholder. Simulated by byte mutation (m01–m03): the parse and every check are unchanged, and FFmpeg decodes all tracks with 0 errors | m01-m03 | movenc.c:177-182, 201-220, 8993-9014 |
| F13 | Video timescale is `time_base.den` doubled to ≥ 10,000, so 30, 60 and 120 fps all get 15,360 (deltas 512, 256, 128) | e20 (256), e21 (512 at 30 fps) | movenc.c:8493-8494 |
| F14 | A new extradata mid-stream adds a second sample entry (`stsd` count 2) | (source) | movenc.c:7046-7072, 3165-3176 |
| F15 | ffprobe `start_time` is 0.000 for all streams in e20, e03 and b09. The first video packet's **duration** (584) carries the alignment | e20, e03, b09 | — |
| F16 | SPS cropping works: e21 is coded 1088 wide, cropped to 1080×1920. VideoToolbox: High, level 3.1 at 720p60 (below the 3.2 that 720p60 needs), `poc_type` 0, one reference frame, no timing info, SEI before the IDR, no in-band SPS. FFmpeg's own CBS parser (`trace_headers`) agrees with my reference parse on every field | e20, e21; `dumps/trace_headers_e20.txt` | — |

---

## 3. Decision: own MP4 parser, not a crate

Checked on crates.io and in the `.crate` sources on 2026-10-24:

| Crate | Latest | Licence | `colr` | largesize / `co64` / fragments | Why not |
|---|---|---|---|---|---|
| `mp4` (alfg) | 0.14.0, 2023-08-01 | MIT | **no** | yes / yes / yes | No release in over 3 years; no `colr` |
| `re_mp4` (Rerun) | 0.5.1, 2026-07-08 | MIT | **no** | yes / yes / yes | Fork of `mp4`; no `colr`; MSRV 1.92 |
| `mp4-atom` | 0.16.2, 2026-10-06 | MIT OR Apache-2.0 | yes | yes / yes / yes | Typed decoder. Without `strict` it drops unexpected boxes with a log line (src/error.rs:79-87); with `strict` it errors on any one. It keeps only the **last** of duplicate children (the avc1 decode loop). Four breaking 0.x releases (0.13 → 0.16) between 2026-07-14 and 2026-09-25. Pulls `tracing`, `num`, `derive_more`, `pastey`, `thiserror` 1 |
| `mp4parse` (Mozilla) | 0.17.0, 2023-05-29 | **MPL-2.0** | — | — | Not on the allow-list (deny.toml:18-28); MPL only by ADR exception (deny.toml:33-34) |
| `mp4box` (alfg) | 0.14.0, 2026-09-16 | MIT | yes | yes / yes / yes | 11 months old, about 5.9k downloads, `anyhow` errors, default features pull `clap` |
| `h264-reader` | 0.9.0, 2026-09-14 | MIT/Apache-2.0 | n/a | n/a | Sound SPS/VUI/slice parser, but 5 dependencies for about 350 lines Kairo needs; golden vectors give the same independence (§4) |

**Recommendation: write our own.** Reasons:
- **A verifier must see the file as written.** That means box order (top level and inside sample entries), duplicates (two `colr`, two `stss`), unknown boxes, header form (32- or 64-bit), byte offsets for messages, overruns and truncation. Typed decoders normalise these away, so they hide exactly the defects kairo-verify exists to catch.
- **Small scope.** About 25 box types (§5.2), read-only, from one muxer family. That is about 600–800 lines of safe Rust with zero dependencies: nothing to pin (CL:30) or licence-check, and it tests everywhere.
- **Memory.** It reads `moov` plus a few bytes per NAL and never loads `mdat`. That matters because an M13 manual recording of 60 min at 50 Mbps is about 22 GB, which needs `co64` and a 64-bit mdat (PLAN:544).
- **Fragmented files are detected, not parsed.** A `moof`, `mvex` or `mfra` fails C0 with "fragmented MP4". Kairo never writes them; research 01's recoverable `hybrid_fragmented` recordings end with a normal moov.
- **64-bit support is required from day one.** That means `size==1` largesize, `size==0` (to end of file, top level only), `co64`, version-1 `mvhd`/`tkhd`/`mdhd`/`elst`, and `ctts` v0 and v1.

---

## 4. Decision: how much H.264 and AAC to parse

| Layer | Parse | Skip |
|---|---|---|
| Framing | `avcC`: version 1, profile/compat/level, `lengthSizeMinusOne`, SPS/PPS lists and the High-profile tail. Each sample is split by the length prefix; the NAL header gives `nal_ref_idc` and type; RBSP is unescaped (00 00 03) | Annex B start codes, which never appear in MP4 samples |
| SPS (H.264 §7.3.2.1.1, E.1.1) | All of it through the VUI: profile, constraints, level, `chroma_format_idc`, bit depths, **scaling lists** (must be walked to reach the VUI), POC type and fields, `max_num_ref_frames`, size and **cropping**, `frame_mbs_only`; VUI aspect ratio, `video_signal_type` (full range, colour description), timing info, both HRDs (skipped by syntax), `bitstream_restriction` (`max_num_reorder_frames`, `max_dec_frame_buffering`) | SPS extensions, subset SPS (MVC/SVC) |
| PPS | `pps_id`, `sps_id`, `entropy_coding_mode_flag` (report: CABAC per CB:486), `bottom_field_pic_order_in_frame_present_flag` | Everything after |
| Slice header | `first_mb_in_slice`, `slice_type` (mod 5), `pps_id`, `colour_plane_id` if separate planes, `frame_num`, field flags, `idr_pic_id`, `pic_order_cnt_lsb` | Ref lists, weights, `dec_ref_pic_marking`, macroblocks: **no decoding** |
| SEI | Payload type list. Recovery point (6) on a non-IDR picture marks an open GOP | Payload contents |
| AAC config | `esds` descriptors (ES, DecoderConfig: OTI, stream type, buffer, max/avg bitrate), AudioSpecificConfig (AOT incl. escape, frequency index or explicit, channels, GASpecificConfig, the 0x2B7 sync extension with AOT 5 and `sbrPresentFlag`) | AAC frames |

Size: about 350 lines for H.264, 120 for AAC.

**Independent check of the parser.** FFmpeg's CBS (`-bsf:v trace_headers`, LGPL, a dev tool) agrees with the reference model on e20 (F16). The Rust tests carry **golden SPS vectors**:
- VideoToolbox's `2764001fac56805005ba6a04040204`, with the fields `trace_headers` printed: profile 100, level 31, POC 0, 1 reference, no crop, colour 1/1/1, range 0;
- AMF's SPS from the first PC run, checked the same way.

That gives an independent oracle without a dependency. A single SPS is a parameter set, not a clip, so committing it doesn't need the owner's OK.

---

## 5. Architecture

### 5.1 Crate and modules

`crates/kairo-verify` is a **library plus a thin binary**. The library is product code: the engine's save verify (PLAN:179) and "Test my setup" (PLAN:271) both run it.

```
kairo-verify/src/
  container/mod.rs     #![forbid(unsafe_code)]  pub fn verify(src, &Expect) -> ContainerReport
  container/read.rs    ReadAt trait (File via positioned reads; &[u8] for tests), bounded reads, caps
  container/boxes.rs   BoxTree: kind, offset, size, header_len (8/16/+16 uuid), children, problems
  container/mp4.rs     typed views: ftyp mvhd tkhd mdhd hdlr elst stts ctts stss stsz stsc stco co64
                       stsd avc1 avcC colr pasp btrt mp4a esds sgpd sbgp udta/name meta/ilst dref
  container/samples.rs per-sample offset, size, dts, duration; mdat containment
  container/h264.rs    NAL split, RBSP, Exp-Golomb, SPS/VUI, PPS head, slice head, SEI types
  container/aac.rs     esds + AudioSpecificConfig
  container/checks.rs  C0..C14 → Vec<Finding>
  container/profile.rs Expect + profiles (§7)
  report.rs            JSON (serde, workspace pins) + text summary
  decode/…             #[cfg(windows)] feature "decode" (ffmpeg-next =9.0.0): other workers' checks
  main.rs              CLI
```

- **Dependencies.** The `container` module uses only `std`, plus `serde` for findings (pinned in the workspace, Cargo.toml:28-29). It never links FFmpeg, so it runs in the Linux CI job next to `kairo-core` (ci.yml:29).
- **Reading.** Top-level headers first. Then `moov` whole, capped at 64 MiB [proposed; a 2-min 4-track clip's moov is about 0.5 MB]. Entry counts are bounded by box size before any allocation, and depth is capped at 16. For video it reads, per NAL, the length plus the first ≤ 64 bytes, or the whole NAL for SPS/PPS up to 4 KiB, by positioned reads. It never reads AAC payloads or the bulk of `mdat`.
- **Errors.** Every finding carries the box path and offset, e.g. `moov/trak[2]/mdia/minf/stbl/stsz @3359`. Every read returns `Result`; there is no `unwrap` (Cargo.toml:44-45).
- **Speed** [Estimate]. Parsing the moov takes well under 10 ms. NAL headers for 1,800 samples take a few ms from the page cache. That fits the save → file p95 < 1 s gate (PLAN:446).
- **Report** (M1:84 leaves the layout to Claude). One JSON object per file:
  - `file` is the **basename only**;
  - `profile`;
  - `container`, a list of findings: `{check, status: pass|fail|warn|not_run, measured, expected, detail}`;
  - `verdict`.
- **What the report never prints.** Tag values: it lists `ilst` keys only (e.g. `©nam, ©too, desc`), since the library JSON tag holds game data. Never directories (CL:16, PLAN:269).
- **Exit codes.** 0 = pass, 1 = any fail, 2 = unreadable or usage error.
- **CLI.** `kairo-verify check FILE --profile clip --fps 60 --gop 1 --length 30 --tracks mix,game,mic [--json] [--container-only]`. `--container-only` is what the Mac runs.
- **Engine reuse (proposal).** CB:675 has M3's save verify call `avformat_open_input` + `find_stream_info`. Recommendation: call `kairo_verify::container::verify` instead, with the `clip` profile. It needs no FFmpeg calls or `unsafe` and runs in milliseconds; it also checks moov-first, the `elst`, `colr` and the GOP, which avformat can't report. The `Expect` comes from the save request (length, fps, enabled roles). The engine depends on `kairo-verify` with `default-features = false`.

### 5.2 Expectations

`Expect { profile, fps: 30|60|120, gop_s: 1.0, length_s, roles: [Mix, Game, Mic, Voice chat, Everything else], lead_samples: 1024..=2047, require_udta_name, max_bytes, … }`. Profiles fill the defaults (§7); flags override them. The `desc` JSON tag, the library mirror (R09:696), is **cross-checked** against the layout but never used as the expectation, since a wrong muxer would write both wrong.

---

## 6. The checks

Each check lists what it reads, the rule, the pass/fail logic, the threshold with its source, and a fixture. The fixture tiers are:
- **U**: a unit test with the in-code builder (§9.1);
- **M**: a Mac test that makes real VideoToolbox clips at test time (§9.2);
- **P**: the PC, gate 1's good clips and their bad copies (§9.3).

### C0 Structure (gates all other checks)
- **Reads.** File length; every box header at top level and inside `moov`; `dref`; sample tables; mdat ranges.
- **Rule.** The file must satisfy all of these:
  - `ftyp` comes first;
  - every header is valid: size ≥ header length; `size==0` only for the last top-level box; `size==1` → largesize ≥ 16;
  - no box overruns its parent or the end of the file, and children fill their parent exactly;
  - exactly one `moov`, at least one `mdat`, and no `moof`/`mvex`/`mfra` (fragmented);
  - each `trak` has exactly one `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf`, `stbl`, `stsd`, `stts`, `stsc`, `stsz` and exactly one of `stco`/`co64`; at most one `stss`, `ctts`, `edts`;
  - the sample tables agree: N = stsz count = Σ stts = Σ ctts (if present); `stss` strictly increasing within [1, N]; `stsc` first chunks increasing from 1 and the chunks it expands to match the offset count;
  - the `dref` holds one self-contained `url ` (flags & 1);
  - **every sample's byte range lies inside an mdat payload and inside the file**.
- **Pass/fail.** Any violation fails, and C1–C14 then report `not_run`. Caps (moov 64 MiB, depth 16) fail with "too large", never a panic.
- **Threshold and source.** No numeric threshold. This is the precondition for PLAN:179's verify and for "Test my setup" on unknown files (PLAN:271).
- **Fixtures.**
  - U: truncate at every byte offset of a built moov (no panic, always fail); seeded byte flips in moov (no panic); size ±1; largesize; `size==0`; an appended `moof`; stsz count ≠ Σ stts; a chunk offset past the end.
  - M: m04 (cut mid-mdat → "mdat overruns"), m05 (cut mid-moov), m06 (`frag_keyframe` → `moof`), m01–m03 (64-bit, must pass).

### C1 Faststart (moov first)
- **Reads.** Top-level order.
- **Rule.** The `moov` comes before the first `mdat`. Boxes between `ftyp` and `moov` may only be `free`, `skip` or `wide`.
- **Pass/fail.** Fail if any mdat precedes moov. Measured: the order and the moov offset.
- **Threshold and source.** Moov first (PLAN:179, M1:12, PLAN:249); Reels needs "moov atom at the front" (R06:132); YouTube too (R01:413).
- **Fixtures.** U: builder `faststart(false)`. M/P: **b01** (`-c copy` without `+faststart` → `ftyp free mdat moov`).

### C2 No edit list
- **Reads.** `trak/edts/elst` in every track.
- **Rule.** No `edts` in any track. Identity edits (one entry, `media_time` 0, whole duration) fail too.
- **Pass/fail.** Fail, listing each track's entries (segment duration, `media_time`, rate).
- **Threshold and source.** `use_editlist=0` (PLAN:179, R09:590, CB:581); Reels rejects edit lists (PLAN:237, R06:132).
- **Fixtures.** U: builder `edts(...)`, including an empty edit (`media_time` −1). M/P: **b02** (`-c copy -use_editlist 1` → identity `elst` on all 4 tracks), e03 (FFmpeg's default: video 0, AAC 1024), e08 (video 256 from B-frames).

### C3 `colr`
- **Reads.** Children of the `avc1` entry.
- **Rule.** Exactly one `colr`, of type `nclx`, with primaries 1, transfer 1, matrix 1 and `full_range_flag` 0. The 7 reserved bits must be 0; if not, it is a warning.
- **Pass/fail.**
  - Fail if it is missing, duplicated, of type `nclc`, `prof` or `rICC`, or has any other value.
  - All v1 outputs are SDR, HDR sources included (PLAN:629 "SDR clips", PLAN:164), so `colr` is checked the same way in every profile.
- **Threshold and source.** BT.709 limited range, signalled in the VUI and in `colr` (R01:436, R09:558, R09:683-686, CB:498-500; `colr set` PLAN:163). It is written only when all three fields are set (F4, movenc.c:2944-2953).
- **Fixtures.** U: builder colr None / (6,6,6,0) / (1,1,1,1) / type `nclc` / two boxes. M/P: **b03** (`-c copy -color_primaries bt470bg -color_trc gamma28 -colorspace bt470bg` → `colr` 5/5/5), **b04** (`-color_primaries unspecified` → no `colr`), e01/e04/e05.

### C4 `colr` agrees with the VUI, and the parameter sets are sound
- **Reads.** The `avcC` SPS/PPS, in-band SPS/PPS in samples, `avc1` and `tkhd` dimensions, `pasp`.
- **Rule.**
  1. The VUI has `video_signal_type_present` 1 and `colour_description_present` 1, and its (primaries, transfer, matrix, `video_full_range_flag`) equal `colr`'s (primaries, transfer, matrix, `full_range`).
  2. Every in-band SPS with the same id is byte-identical to the `avcC` SPS. If the bytes differ but the checked fields are equal, it is a warning [Unverified for AMF with `header_spacing=60`, CB:487].
  3. Profile 100 (High, CB:486, R09:965), and `avcC` profile and level equal the SPS's.
  4. 4:2:0, 8-bit, `frame_mbs_only` 1 (progressive).
  5. The cropped SPS size equals the `avc1` size equals the `tkhd` size (16.16), and both are even.
  6. SAR unspecified or 1:1, and `pasp` absent or 1:1.
- **Pass/fail.** Fail on 1–5. A level below Table A-1's need for the size and fps is a **warning only**, since VideoToolbox already declares 3.1 for 720p60 (F16) [proposed]. Under the `h264_mf` safe level (PLAN:136), a VUI with no colour description is a warning, because MF writes none (R09:571, CB:478) [proposed]. For AMF it is a fail.
- **Threshold and source.** Both signals come from the same four encoder fields (R09:558); AMF sets the VUI from them (amfenc_h264.c:277-296).
- **Fixtures.** U: builder VUI ≠ `colr`; in-band SPS with a different VUI; profile 77; crop mismatch; SAR 4:3. M/P: **b05** (`h264_metadata=colour_primaries=5:…` → VUI 5/5/5, `colr` 1/1/1), b03 (`colr` 5/5/5, VUI 1/1/1), e14.

### C5 CFR (`stts`)
- **Reads.** Video `mdhd` timescale T, `stts`; the expected fps.
- **Rule.**
  - D = T / fps must be an integer (T = 15,360 for 30/60/120; F13).
  - Samples 2…N, the last included, each have a delta of exactly D.
  - Sample 1 has D + E with E ≥ 0; C6 judges E.
  - The measured fps is T / D.
- **Pass/fail.** Fail on any other delta (listing the first indices), a non-integer D, or an fps other than expected. This is timestamp CFR only; repeated content is the cadence check's job (core/cadence.rs:1-15).
- **Threshold and source.** CFR 60 (PLAN:446); pacer pts = tick (PLAN:161); rates 30/60/120 (CB:616, core/timeline.rs:21-28); every packet one tick long (R09:608, core/cut.rs:221).
- **Fixtures.** U: builder with a gap, a double tick, a last delta of 0, timescale 1000. M/P: **b10** (`-bsf:v setts=ts=TS+if(gte(N,100),256,0)` → 512 at sample 100), e07 (B-frame dts gives deltas 1 and 511).

### C6 Start alignment (cookbook P8)
- **Reads.** Video `stts[0]`; each audio track's `stts[0]`.
- **Rule.**
  - With audio: E = stts_v[0] − D must lie in [⌈1024·T/48000⌉, ⌈2047·T/48000⌉], which is **328…656 ticks** at T = 15,360. That is an audio lead of 1,024–2,047 samples (21.3–42.7 ms).
  - **And** every audio track's first delta is exactly 1,024. movenc stretches the first sample of any track that starts late (movenc.c:7282-7290).
  - Without audio: E = 0.
- **Pass/fail.**
  - Fail outside the window.
  - A lead of 0–1,023 is the "track's very first packet" case (core/cut.rs:50-51): a truncated start. It is a fail in the M3-gate and selftest profiles and a warning in library clips [proposed].
  - Measured: E, the lead in samples, and each audio track's first delta.
- **Threshold and source.** PLAN:179 (video frame 0 held ≤ 42.7 ms), CB P8 (line 49), CB:678, core/cut.rs:7-12, core/cut.rs:570-577 (first packet −2,047…−1,024); the ⌈ ⌉ comes from mux.c:677-679. This **replaces** R09:978-979's `start_time` test, which is blind (F15), and it is the "<22 ms … separate check of how far apart the audio and video tracks start" that M1:189 mentions.
- **Fixtures.** U: leads 0, 1023, 1024, 2047, 2048, and an audio first delta of 1375. M/P: e20/b00 (E 328 → 1,025 samples, pass), **b09** (`-ss 0.5 -copyinkf`: E 0, audio first delta 1375, fail), e03 (edit list instead of a held frame, fail).
- **[Unverified].** Only lead 1,024 was measured, from the CLI's priming. 1,025–2,047 rest on reading the source; M3 task 9's real cuts confirm them (CB:1193).

### C7 pts = dts (no B-frames)
- **Reads.** Video `ctts`; the `slice_type` of every VCL NAL; SPS `max_num_reorder_frames`; POC.
- **Rule.**
  - No `ctts`, or every offset 0 (movenc writes it only when some pts ≠ dts, movenc.c:7330-7331).
  - No slice with `slice_type % 5 == 1`.
  - `max_num_reorder_frames` is 0 when present.
  - With `poc_type` 0, POC strictly increases in decode order inside each IDR period [proposed]. `poc_type` 2 implies no reordering by definition.
- **Pass/fail.** Fail on any of these. Measured: `ctts` entries and how many are non-zero, the B-sample count, the first out-of-order POC.
- **Threshold and source.** `bf=0` (PLAN:166, R09:46, R09:588-591); the ring already refuses pts ≠ dts (core/ring.rs:214-219); gate 1's B-frame clip (M1:26).
- **Fixtures.** U: builder `bframes(true)` (`ctts` plus B slices), a POC regression. P/M: the B-frame bad clip, a re-encode (§9.3); e07/e08 (`ctts` non-zero, 131 B samples).

### C8 IDR at frame 0
- **Reads.** `stss[0]`; sample 1's NAL units; PPS ids.
- **Rule.**
  - Sample 1 is a sync sample (`stss[0] == 1`, or no `stss`).
  - **All** its VCL NALs are type 5 with I slices (2 or 7), and the first slice has `first_mb_in_slice` 0.
  - At least one SPS and PPS exist (in `avcC`, or in-band before the slice), and the slice's PPS id resolves.
- **Pass/fail.** Fail otherwise. Measured: sample 1's NAL types and `stss[0]`.
- **Threshold and source.** "IDR first" (PLAN:179), "IDR at 0" (M1:12, PLAN:446), "first row 0,0,K__" (R09:969).
- **Fixtures.** U: first sample P, non-IDR I, a missing PPS. M/P: **b09** (first sample P, `stss[0]` = 32).

### C9 Closed 1 s GOP
- **Reads.** `stss`; the NAL types of every sample; the expected fps and GOP.
- **Rule.**
  - G = fps × `gop_s` (60).
  - Sync samples == {1, 1+G, 1+2G, … ≤ N} exactly. The last GOP may be short.
  - **IDR samples == sync samples exactly.**
  - No non-IDR I-only sample appears in `stss`.
  - No recovery-point SEI on a non-IDR picture.
- **Why "every random-access point is an IDR" is the closed-GOP test with `bf=0`.**
  - C7 guarantees output order = decode order, so there are no leading pictures. A B-picture can't reach back into the previous GOP.
  - The only way left to cross a GOP boundary is a P-picture referencing a picture before a **non-IDR** I-picture. H.264 allows several references.
  - An IDR marks every reference picture unused, so nothing after it can reference across it.
  - AMF maps `gop_size` to its IDR period (amfenc_h264.c:574-575) and flags only IDRs as keys (R09:611-618, CB:537), so a mismatch here means an encoder or muxer bug.
- **Pass/fail.** Fail, listing the expected vs actual sync and IDR sets (first five of each).
- **Threshold and source.** "1 s closed IDR GOP with `gop_size` = fps" (PLAN:166); "a K every 60 rows" (R09:969); keyframes every 60 packets (R09:958); every keyframe an IDR (R01:430).
- **Fixtures.** U: builder G = 120; an I-not-IDR at 61; an IDR missing from `stss`; a recovery SEI. M/P: e11 (`-g 120` → [1,121]), e07 (every 57), b09.

### C10 Track layout ("swapped tracks")
- **Reads.** Per track: `tkhd` (id, flags, `alternate_group`), `hdlr` type and name, `udta/name`, `mdhd` language, sample entry, `channelcount`; `mvhd.next_track_id`.
- **Rule** (`clip` profile; expected roles R come from the save request):
  1. Track 1 is the only `vide` track: `avc1`, enabled (flags & 1), `alternate_group` 0.
  2. Then exactly |R| `soun` tracks in role order. **Both** the `hdlr` name and `udta/name` equal the role title. The engine sets both (CB:573). Titles: "Mix", "Game", "Mic", "Voice chat" (CB:598), and "Everything else" for T5 [proposed, from PLAN:188's name].
  3. **Only Mix is enabled**: the `tkhd` enabled flag is the MP4 form of "default" (mov.c:5738).
  4. All audio tracks share one non-zero `alternate_group`. Two enabled audio tracks switch movenc to per-stream groups (F8).
  5. Channels per role: Mix 2, Game 2, **Mic 1**, Voice chat 2, Everything else 2.
  6. No other track types (text, `tmcd`, hint, meta).
  7. Track ids are unique, increase in file order and stay below `next_track_id`.
  8. Language is reported only (`und`).
- **Pass/fail.** Fail on any of 1–7, with each mismatch listed.
- **What it catches and what it can't.**
  - A swap where titles follow their streams fails on title order plus channels (b06).
  - A swap of Game and Mic with titles relabelled fails on channels (b07).
  - **A swap of Game and Voice chat with titles relabelled is invisible here** (both stereo). The decode routing check, "voice tone only on T4" (M1:12), owns it.
- **Threshold and source.** T1 Mix default plus stems (PLAN:185-189); players play only the default track (PLAN:194); P9 (CB:50); dispositions and groups (R09:673-679); mic mono (PLAN:186, CB:556).
- **Fixtures.** U: a roles permutation, two enabled tracks, per-stream groups, a missing `udta/name`, a mono Game. M/P: **b06**, **b07**, **b08** (`-disposition:a:1 default` → enabled ×2, groups [1,2,3]), e09, e13 (a plain CLI remux loses `udta/name`, F9).

### C11 AAC LC, 48 kHz
- **Reads.** Each `mp4a` entry (`channelcount`, `samplerate`), `esds`, `mdhd` timescale, `stts`, `sgpd`/`sbgp`.
- **Rule.**
  - Entry `mp4a`; samplerate and `mdhd` timescale 48000; OTI 0x40 with stream type 5.
  - ASC: AOT 2; frequency index 3 (or explicit 48000); `channelConfiguration` ∈ {1, 2} and equal to `channelcount`; `frameLengthFlag` 0; `dependsOnCoreCoder` 0.
  - If the 0x2B7 extension is present, it must say AOT 5 with `sbrPresentFlag` 0 (FFmpeg writes exactly that, F10).
  - Every `stts` delta is 1,024, except the last, which may be 1…1,024. Engine clips are all 1,024 (core/cut.rs:243); CLI encodes end short (F10).
- **Pass/fail.** Fail on any mismatch. Average bitrate vs nominal (160k stereo, 128k mono) and the `roll` −1 group are reported only.
- **Threshold and source.** AAC-LC 48 kHz, 160 kbps, mic 128, one 1,024 grid (PLAN:191); R09:705-710; CB:562; core/ring.rs:22.
- **Fixtures.** U: AOT 5 (HE-AAC), `sbrPresentFlag` 1, 44.1 kHz, `frameLengthFlag` 1 (960), `esds` 2 vs entry 1, a delta of 1000.

### C12 Durations
- **Reads.** `mvhd`, every `tkhd`, `mdhd`, Σ `stts`, N; the expected length L.
- **Rule.**
  1. `mdhd` duration == Σ `stts`, exactly.
  2. `tkhd` == ⌈mdhd × mvhd timescale / mdhd timescale⌉ ± 1 (movenc.c:3993-3995).
  3. `mvhd` == the largest `tkhd` ± 1.
  4. Video length N/fps ∈ [L − GOP, L + GOP]. Below L alone is a warning: the clip was shortened, which the pop-up announces (PLAN:162).
  5. Every audio track ends within [−1 video tick, +1,024 samples + 1 tick] of the video's end. Audio runs to the end of the last video frame, rounded up to a whole AAC frame (core/cut.rs:227-246).
- **Pass/fail.** Fail on 1–3 and 5, and on 4 outside ±1 GOP. Measured: frames, seconds, the per-track end offsets.
- **Threshold and source.** "30 s ± 1 GOP" (PLAN:446, R09:994, CB:1193). The ring cut gives 1,801–1,860 frames for 30 s at 60 fps with post-roll 0 (core/cut.rs:143-147); see the note below.
- **Fixtures.** U: `mdhd` ≠ Σ `stts`; L = 30 with N = 1739 (fail), 1740 (pass, edge), 1860 (pass), 1861 (fail). M/P: **b10** (video one frame longer → audio ends 801 samples early → fail).
- **Note on the frame window.** The cut's key K is the newest keyframe ≤ t − 1,800, which keeps the clip inside [L, L + 1 GOP]. ±1 GOP is the gate's tolerance [proposed reading].

### C13 One encoder session per file
- **Reads.** `stsd` entry count, `stsc` sample-description indices, in-band SPS.
- **Rule.** One sample entry per track, every `stsc` index 1, and in-band SPS equal to `avcC` (C4.2).
- **Pass/fail.** Fail on two entries or an index of 2. movenc adds an entry when new extradata arrives mid-stream (F14), so this is the container's evidence that "saves never span epochs".
- **Threshold and source.** PLAN:162; CB:617; R09:637.
- **Fixtures.** U: builder with two entries; an in-band SPS that changes resolution.

### C14 Video bitrate (report; export limits)
- **Reads.** `stsz` and `stts` → per-sample time and size.
- **Rule.** Report the average Mbps and the peak over sliding 1 s windows.
  - `clip`/`harness`: a peak above 50 Mbps is a **warning**. A VBV of 1 s at peak can legitimately put up to about bufsize + maxrate in a 1 s window, so this is evidence for the setting, not a gate [proposed].
  - Export profiles: **fail** if the average exceeds the profile's cap.
- **Threshold and source.** H.264 peak ≤ 50 Mbps (CL:31, R13:139); Reels ≤ 25 Mbps (PLAN:245).
- **Fixtures.** U: builder sizes giving 49/51 Mbps windows.

---

## 7. Profiles (including export profiles)

Base set for every profile: C0–C5, C7–C9, C11, C13, C14. C6, C10 and C12 vary as shown.

| Profile | Used for | Differences from the base set |
|---|---|---|
| `clip` | Engine saves; M3's timed gate | Full C10 with roles from the save; C6 strict; C12 with L from the binding; the C4 `h264_mf` exception off |
| `selftest` | "Test my setup", 5 s (PLAN:271) | `clip` with L = 5 s; roles as configured |
| `harness` | M1 gate 2's FFmpeg CLI capture (M1:27) | Roles from the job (video only unless a loopback device exists, M1:187); `udta/name` optional; C6 on audio only; L = the job length |
| `share` | Quick share copy, Copy (S2), share upload (PLAN:249, PLAN:577) | H.264 ≤ 1080p60; **exactly 1 audio track** (the mix, stereo) and enabled; titles not required (`-map_metadata -1` resets them to "SoundHandler", e17); `ilst` keys ⊆ {`©nam`, `©too`}; no `desc`, `©cmt`, `©day`, `©xyz`; `creation_time` 0 in `mvhd`/`tkhd`/`mdhd` [proposed]; average 6–8 Mbps reported (R06:392) |
| `discord` | Copy for Discord (PLAN:243) | `share`, plus file size ≤ 19,000,000 bytes (PLAN:58); (size, fps) on the ladder 1080p60 → 1080p30 / 720p60 → 720p → 540p |
| `whatsapp` | WhatsApp / phone (PLAN:244) | `share`, plus ≤ 1080p30 [proposed] and AAC 128k [proposed]; SP8 sets the rest |
| `reels` | TikTok / Shorts / Reels (PLAN:245, PLAN:496 `--profile reels`) | `share`, plus exactly 1080×1920, 23 ≤ fps ≤ 60, average video ≤ 25 Mbps (C14 fail), duration 3 s – 15 min, size ≤ 300 MB, closed GOP (R06:156) |
| `original` | "Original (all tracks)" stream copy (PLAN:247) | `clip` without C6/C12. It also yields `shareable_original = faststart && avc1 && all mp4a && no Voice-chat track`: the plain Copy rule (PLAN:249), which the engine can reuse |

e21 is the `reels` shape from VideoToolbox: 1080×1920 at 30 fps, `colr` 1/1/1/0, `stts [[1,840],[119,512]]` (lead 328 at T = 15,360). It passes everything except the `clip`-only title rule, which is why titles are profile-dependent.

---

## 8. ffprobe: what it could do, and why kairo-verify must not need it

| Check | ffprobe route | Limit |
|---|---|---|
| C0 | Errors on stderr | No overrun or containment report; repairs silently |
| C1 faststart | Only `-v trace` log text | Log text is not an interface |
| C2 `elst` | Only `-v trace` text; mov.c applies the edits | `start_time` and durations shift silently |
| C3/C4 `colr`, VUI | `stream=color_*` | **Merges `colr` and VUI.** e01 shows `color_space=bt709` with no `colr` at all, so a missing `colr` or `colr` ≠ VUI is invisible |
| C5 CFR | `packet=pts,duration` | Works |
| C6 alignment | The first packet's `duration` (584) works; `start_time` doesn't (F15) | — |
| C7 pts = dts | `packet=pts,dts` | Works; B slices need `-show_frames` (decodes) or `ffmpeg -bsf:v trace_headers` text |
| C8/C9 IDR, GOP | `flags` K = `stss` | IDR vs non-IDR I needs `trace_headers` text or decoding |
| C10 tracks | `disposition:default`, `handler_name`, channels | `udta/name` title: invisible in 8.0.1, tag `name` in 9.0.2 (F9); `alternate_group` not shown |
| C11–C14 | Profile LC, sample rate, durations, sizes | Mostly works |

**Why kairo-verify's own analysis must stand alone:**
1. `ffprobe.exe` never ships (M1:56, M1:70), while "Test my setup" runs this analysis on friends' PCs (PLAN:271). Only the LGPL DLLs and `ffmpeg.exe` for exports ship (PLAN:555).
2. The engine verifies every save in-process within the save latency budget (PLAN:179, PLAN:446). Spawning a process and parsing text per save is wrong there.
3. About half of the checks (C1, C2, C3/C4, the IDR part of C8/C9, part of C10) are invisible to ffprobe's structured output.
4. Checking FFmpeg's muxer with FFmpeg's demuxer is a common-mode oracle: mov.c repairs exactly what we test (edit lists, colour merging).
5. The tests must run on the Mac and Linux CI without FFmpeg. The Mac's FFmpeg is GPL and must never be linked (CL:17).
6. FFmpeg's text output drifts between versions (the title behaviour changed from 8.0.1 to 9.0.2).

**Keep ffprobe as a dev-only cross-check.** `pc harness --cross-check` runs `ffprobe -show_streams -show_packets -of json` on the PC and compares what it can see: packet counts, pts/dts, durations, K flags, dispositions, channels, rates. A disagreement is a kairo-verify bug and is reported. CB:682-687's three ffprobe lines stay in the harness as secondary evidence.

---

## 9. Fixtures and tests

### 9.1 U: in-code builder (Mac + Linux CI)
`container/testkit.rs` is `#[cfg(test)]`. A builder writes MP4 bytes in memory:
- `ftyp`, `moov` (`mvhd`, one `trak` per stream with `tkhd`, `edts`?, `mdia`/`mdhd`/`hdlr`/`minf`/`dinf`/`stbl`), optional `udta/name`, `udta/meta/ilst`, `free`, `mdat`;
- options: faststart, largesize, `co64`, version-1 boxes;
- **H.264 access units** from a small Exp-Golomb bit writer (SPS with any VUI/HRD/scaling-list combination, PPS, slice headers, SEI), followed by filler bytes. kairo-verify never decodes, so filler is enough.
- AAC payloads are arbitrary bytes.

Every check gets a good-config pass test and a **single-mutation fail test that leaves the other checks passing**: gate 1 in miniature. Plus:
- SPS writer/parser round trips over generated combinations;
- golden SPS vectors (§4);
- the truncation sweep and seeded byte flips from C0, with no panics allowed.

Nothing is written to disk and nothing is a clip, so no owner OK is needed (CL:18, M1:119). Expected values can come from the reference model in `container/scripts/`, the way core/cut.rs's tests use an independent Python model.

### 9.2 M: real encoder output on the Mac (opt-in)
- `#[ignore]` + `cfg(target_os = "macos")` + env `KAIRO_FFMPEG=/opt/homebrew/bin/ffmpeg`.
- The test runs the CLI (never linked) with `-c:v h264_videotoolbox` and `-c:a aac` **always explicit**, since an unspecified encoder would pick libx264. It uses only LGPL filters (`setparams`, `format`, `h264_metadata`, `setts`).
- It writes into a temp dir, rebuilds the e20/b00–b10/m01–m06 set (`container/scripts/make_bad_copies.sh`), and asserts the §9.3 matrix.
- **Assert check outcomes, never bytes.** VideoToolbox output differs by a byte from run to run; the outcomes reproduced exactly.

### 9.3 P: gate 1 on the PC (recipes, proven on the Mac)
A good testbed clip G gets one defect per copy (M1:188). Copies stay in `.dogfood/` or `C:\kairo\logs`.
- **Every remux re-specifies the per-stream `title` and `handler_name`.** A plain stream copy drops `udta/name` (F9, e13), and that would be a second, unwanted defect.
- Remuxes are stream copies, so they open no encoder session. Only the B-frame clip encodes (one session at a time, M1:94).

| Defect (M1:26) | Recipe (FFmpeg CLI) | Fails (proven on the Mac) |
|---|---|---|
| Control | `-map 0 -c copy <titles> -movflags +faststart -use_editlist 0` | none (b00) |
| No faststart | as the control, without `-movflags +faststart` | C1 (b01) |
| Edit list | as the control with `-use_editlist 1` | C2 (b02: identity `elst` on 4 tracks) |
| Wrong `colr` | `-color_primaries bt470bg -color_trc gamma28 -colorspace bt470bg` on a copy | C3, C4 (b03) |
| Missing `colr` (extra) | `-color_primaries unspecified` on a copy | C3, C4 (b04) |
| VUI ≠ `colr` (extra) | `-bsf:v h264_metadata=colour_primaries=5:transfer_characteristics=5:matrix_coefficients=5` | C4 (b05) |
| Swapped tracks (a) | `-map 0:v -map 0:a:0 -map 0:a:2 -map 0:a:1 -c copy` (titles follow) | C10 (b06) |
| Swapped tracks (b) | the same mapping with canonical titles | C10 by channels (b07). Use a Game ↔ Voice chat swap for the routing check's own bad clip |
| Two defaults (extra) | `-disposition:a:1 default` | C10 (b08) |
| Not IDR at 0 (extra) | `-ss 0.5 -copyinkf` on a copy | C8, C9, C6, C11 (b09) |
| VFR gap (extra) | `-bsf:v "setts=ts=TS+if(gte(N\,100)\,256\,0)"` | C5, C12 (b10) |
| B-frames | Re-encode G's video: `h264_amf -bf 2` if the card supports H.264 B-frames (FFmpeg warns and falls back otherwise, amfenc_h264.c:504-512) [Unverified on RX 9000]. Fallback: `h264_videotoolbox -bf 2` on the Mac (proven, e07) | C7, C9, C5 |

Matrix from the Mac (`dumps/matrix_repro.txt`; reproduced twice):

```
file                        struct fastst no_edi colr   colr_m cfr    start_ pts_eq idr_at closed track_ aac_lc durati
e20_good_setparams.mp4      PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b00_copy_control.mp4        PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b01_nofaststart.mp4         PASS   FAIL   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b02_editlist.mp4            PASS   PASS   FAIL   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b03_wrong_colr.mp4          PASS   PASS   PASS   FAIL   FAIL   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b04_no_colr.mp4             PASS   PASS   PASS   FAIL   FAIL   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b05_vui_mismatch.mp4        PASS   PASS   PASS   PASS   FAIL   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
b06_swapped_follow.mp4      PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   FAIL   PASS   PASS
b07_swapped_relabelled.mp4  PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   FAIL   PASS   PASS
b08_two_defaults.mp4        PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   FAIL   PASS   PASS
b09_starts_on_p.mp4         PASS   PASS   PASS   PASS   PASS   PASS   FAIL   PASS   FAIL   FAIL   PASS   FAIL   PASS
b10_vfr_gap.mp4             PASS   PASS   PASS   PASS   PASS   FAIL   PASS   PASS   PASS   PASS   PASS   PASS   FAIL
m01_largesize.mp4           PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
m02_co64.mp4                PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
m03_co64_largesize.mp4      PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS   PASS
```

In the naive-CLI series (`dumps/checks_e.txt`), every clip made with `-color_primaries` as an encoder option fails C3/C4 (F5). e07/e08 fail C5, C6, C7 and C9; e11 fails C9. m04–m06 fail C0.

---

## 10. Implications for the harness and the engine

1. **Gate 2's capture command must tag colour on the frames.** Put `setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv` as the last filter before `h264_amf`.
   - It is metadata-only (vf_setparams.c:225), so it should pass D3D11 frames through.
   - It is needed because the CLI copies encoder colour from frames (ffmpeg_enc.c:263-270).
   - Also: `-g 60 -bf 0` plus CB:483-488's AMF options, `-fps_mode cfr -r 60`, `-movflags +faststart -use_editlist 0`.
   - The first gate-2 report's C3/C4 confirms it [Unverified on 9.0.2 with `gfxcapture`].
2. **Bad-clip remuxes always re-specify `title` and `handler_name`** (§9.3).
3. **The engine** sets all four colour fields on the context (CB:495-500), so its `colr` and VUI come out right without `setparams`. It writes `pkt.duration` (C5's last delta), the P8 audio start (C6) and the mix-only default (C10). C6 and C12 then check the P8 premise directly.
4. **Research 12's mock fixture script (R12:1156-1166) makes clips that fail C3/C4/C10.** It has no `setparams`, no titles, no dispositions and maps the same sine twice. That is fine for the UI. Never reuse those files as kairo-verify fixtures.

---

## 11. [Unverified] and open points

1. FFmpeg 9.0.2's CLI with `-color_primaries`/`-color_trc`: 8.0.1 drops them (F5). `setparams` is recommended either way.
2. AMF (RX 9000) details:
   - whether its in-band SPS is byte-identical to `avcC` with `header_spacing=60`;
   - whether it writes VUI timing or `bitstream_restriction`;
   - whether it uses `slice_type` 2 or 7;
   - its level choice;
   - whether it can make H.264 B-frames for the bad clip.

   The first `pc harness` run answers all of these and supplies the AMF golden SPS.
3. Whether 9.0.2 stream copies keep `udta/name` (F9).
4. Audio leads of 1,025–2,047 samples are derived from source only (mux.c:677-679, movenc.c:7282-7290); only 1,024 was measured. M3 task 9 confirms them.
5. Whether `gfxcapture` with `-fps_mode cfr` gives CFR `stts` (it should: duplicates go into the content, which the cadence check sees).
6. Which title players show (`udta/name` vs `hdlr`): the SP8 player matrix (PLAN:434).
7. Proposals to confirm:
   - the engine's save verify reuses `kairo_verify::container` instead of avformat (§5.1);
   - the level check is a warning only;
   - the `share` profile zeroes `creation_time`;
   - a truncated start (lead < 1,024) fails in `clip`/`selftest` but is a warning in library clips;
   - T5's title is "Everything else".

---

## Appendix: artefacts (scratch, never committed)

- `container/scripts/mp4inspect.py`: the reference box, SPS/slice and esds parser, with tree and JSON output.
- `container/scripts/checks.py`: the reference model of C0–C12 as written here (C13/C14 are trivial and not modelled).
- `container/scripts/mutate.py`: the largesize and `co64` transforms. The second mirrors movenc.c:8993-9014.
- `container/scripts/make_clips.sh`: e01–e18.
- `container/scripts/make_bad_copies.sh`: e20, b00–b10, m01–m06, e21.
- `container/dumps/*.tree.txt` and `*.json.txt` (every clip), `checks_b.txt`, `checks_e.txt`, `matrix_repro.txt`, `trace_headers_e20.txt`.
- `container/upstream/`: the FFmpeg n9.0.2 and n8.0.1 sources read for the citations.
- `container/crate-src/`: unpacked `.crate` sources, inspected with grep only, never built.
