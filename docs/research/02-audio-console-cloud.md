# Kairo research 02: Windows audio capture, console capture cards, cloud-gaming capture

- **Prepared:** 2026-10-21, for the Kairo planning team (Windows-only instant-replay clipper).
- **Scope:** WASAPI system vs per-process loopback; mic and multi-track audio; audio encoders; clock drift; device changes; console capture cards; cloud-gaming capture; importing console-native clips; v1 vs later recommendations.
- **Method:** I checked primary sources where possible: Microsoft Learn, OBS Studio source code and GitHub issues, FFmpeg source and docs, Elgato's help center (read through its Zendesk API), and the Sony, Nintendo, NVIDIA and Microsoft support and legal pages. Every source carries its date or the date I checked it.
- **Confidence labels:** **[High]** means verified in a primary source or source code. **[Med]** means a vendor statement or several consistent secondary reports. **[Low]** means a single user report, an inference, or something I could not verify.
- **Research limitations (honest disclosure):**
  - The session's web-search budget ran out part-way through. The later checks used direct fetches of known primary URLs only.
  - The OBS forum and OBS knowledge base returned HTTP 502 for the whole session. Claims that depend on OBS forum threads come from search-engine summaries and are marked **[Low]**.
  - amazon.com/luna showed an outage page. Boosteroid's site gave only minimal information.
  - Nothing here was tested on real Windows hardware. Section 8 lists what to test on the gaming PC.

---

## 0. Executive summary

1. **Per-process audio capture is viable on every OS Kairo targets, but its documented minimum and its working minimum differ.**
   - Microsoft documents `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` as requiring **"Windows 10 Build 20348"**. 20348 is the Windows Server 2022 build, so for consumers that means Windows 11.
   - OBS registers its Application Audio Capture source on **build 19041+ (Windows 10 2004)**. The code comment reads *"MS says 20348, but process filtering seems to work earlier"*.
   - Windows 10 22H2 (19045) therefore works in practice, but Windows 10 is outside the documented support. **[High]**
2. **Long-session audio corruption is a real risk for an always-on buffer.**
   - OBS issue #8064 ("sound becomes noisy after a long time", 2023 to 2025) was traced by OBS maintainers to a **Microsoft-side bug**. In October 2025 they said *"Fixes will only be made to Windows 11"*.
   - Users confirmed the fix on Windows 11 25H2 (26200.6899). Windows 10 users can still hit crackling or noise after roughly 15 minutes to several hours.
   - Kairo needs a watchdog that periodically re-creates loopback clients, plus multi-hour soak tests. **[Med]**
3. **The process-loopback `IAudioClient` is a limited object.**
   - Microsoft staff confirmed that `GetMixFormat` and `IsFormatSupported` return `E_NOTIMPL`.
   - wasapi-rs (Rust bindings for WASAPI) documents more gaps: `GetDevicePeriod` and `GetCurrentPadding` are not implemented, `GetBufferSize` returns garbage, and there is no `IAudioClock` or `IAudioSessionControl`.
   - You must pass a fixed format with `AUTOCONVERTPCM` and rely on the QPC timestamps from `IAudioCaptureClient::GetBuffer`.
   - On the plus side, process loopback is **not tied to an endpoint**, so it survives headphone/speaker switches. System (endpoint) loopback does not. **[High]**
4. **Targeting is by PID and process tree, so picking the right process is the hard part.**
   - Each activation takes **one** PID, either "include tree" or "exclude tree".
   - Audio played by a process outside the tree is silent. A user report says Valorant's voice chat runs in the windowless `RiotClientServices.exe`.
   - OBS chooses the process from a *window*, so it cannot pick windowless audio processes. Kairo should pick by process and resolve the root of the process tree.
   - No code is injected into the game, so anti-cheat conflicts are not expected. **[High]** for the mechanism; **[Low]** for the specific game examples.
5. **Multi-track MP4 works, but only the first or "default" track is reliably played.**
   - Chromium's `audioTracks` API is still behind a flag, and Chromium plays only the first audio track by default.
   - Medal reports that Windows Media Player, File Explorer, Movies & TV and Instagram "do not pick the proper default track".
   - DaVinci Resolve imports the stems. Premiere Pro is version-dependent: in 2026 it imports them but shifts every track after the first by one frame.
   - **Use track 1 = the mixed track, marked default (enabled), with the stems after it.** FFmpeg's MP4 muxer does exactly this by default. **[High]** for the mechanism; **[Med]** for per-app behaviour.
6. **Use AAC-LC at 48 kHz.**
   - Windows' in-box Media Foundation AAC encoder takes **16-bit PCM only**, at 44.1 or 48 kHz, with 1, 2 or 6 channels. Stereo bitrates are 96, 128, 160 or 192 kbps.
   - FFmpeg's native `aac` encoder (LGPL) is the main alternative. `libfdk_aac` is "non-free" and cannot ship with GPL builds.
   - Opus-in-MP4 is not an option for v1: **Premiere does not support Opus** (Adobe community manager, 2025-09-11), and Opus is not in Media Foundation's documented codec list.
   - Windows N editions lack Media Foundation unless the Media Feature Pack is installed. **[High]**
7. **Modern USB capture cards are plug-and-play UVC devices, and each brings practical constraints.**
   - Elgato HD60 X, 4K X, 4K S and Neo are all UVC with no driver needed. Their audio arrives as a separate stereo PCM device (up to 24-bit/48 kHz; no Dolby, DTS or 5.1).
   - Elgato says these models have **no multi-app support**, so Kairo would compete with OBS or Elgato Studio for the device. Microsoft's `MediaCapture` *SharedReadOnly* mode may get around this, but that is untested.
   - **PS5 has HDCP on by default.** It must be turned off under Settings → System → HDMI. Xbox and Switch turn HDCP on only inside media apps.
   - Launch MSRPs: 4K S $159.99 (July 2025), HD60 X $179.99, 4K X about $229.99 (Feb 2024). **[High]** for specs and HDCP; **[Med]** for prices.
8. **Cloud gaming should be capturable.**
   - None of the services shows any documented DRM on the game stream. xCloud is WebRTC, according to an open-source client.
   - Windows blanks captures only for `SetWindowDisplayAffinity`, DXGI `DISPLAY_ONLY` swap chains, or hardware-protected DRM video.
   - The NVIDIA GeForce NOW terms of use (version of 2025-08-18) do not mention recording; they prohibit reverse engineering.
   - PS Remote Play blocks some content at the source: video services are blocked, and "depending on the game, some images may not be displayed".
   - All of this needs testing on the real PC. **[Med]**
9. **No console offers a public API for clip import.**
   - **PS5:** The PS App keeps auto-uploaded captures for **14 days**, only clips of 3 minutes or less at 1080p or lower, and only on the mobile app.
   - **Switch 2:** The Nintendo Switch App keeps uploads for **30 days**, up to **100 files** per account, also mobile-only.
   - **Xbox:** Captures uploaded to the Xbox network are deleted after **90 days** and capped at **10 GB**. Xbox has official **OneDrive auto-upload**, and the unofficial **OpenXBL** API (free tier 150 requests per hour; paid plans $5, $15 and $35 per month).
   - The realistic import paths are USB stick, MTP over USB (Switch), OneDrive (Xbox), or a generic "import files" option. **[High]** for limits; **[Med]** for "no API", since that rests on absence of evidence.
10. **Recommendation.** For v1, build:
    - game, mic and voice-chat audio, using process loopback with a fallback to endpoint loopback;
    - an AAC multi-track MP4 with a mixed default track;
    - cloud gaming treated as ordinary window/screen capture plus app audio;
    - a cheap generic file importer.

    Defer capture-card sources until the beta phase (v1.x), but only if the user owns a card. Defer console-specific import automation (OneDrive or Graph for Xbox, MTP auto-detect for Switch) and any unofficial APIs to later stages.

---

## 1. Game and system audio: endpoint (system) loopback vs per-process loopback

### 1.1 Side-by-side comparison

| Aspect | Endpoint ("system") loopback | Per-process loopback |
|---|---|---|
| How to open | `IMMDevice` (render endpoint) → `IAudioClient::Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK …)` [S17][S18] | `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, IID_IAudioClient, PROPVARIANT{VT_BLOB → AUDIOCLIENT_ACTIVATION_PARAMS{AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, {TargetProcessId, PROCESS_LOOPBACK_MODE_INCLUDE/EXCLUDE_TARGET_PROCESS_TREE}}})` [S1][S2][S5] |
| Documented minimum OS | Vista or later. Event-driven loopback works from Windows 10 1703 ("as of Windows 10" in the `Initialize` docs) [S17][S18] | **"Windows 10 Build 20348"**, which is effectively Windows 11 for consumers (Windows 11 21H2 is build 22000) [S1][S2][S3] **[High]** |
| Working minimum in practice | Any target OS | **Windows 10 2004 (19041), fully updated.** OBS gates on 19041 [S4]; the win-capture-audio README says the same [S9]. One developer on Microsoft Q&A (May 2023) said it only seemed to work on Windows 11 [S10], so treat Windows 10 as best-effort. **[Med-High]** |
| What it captures | The mix rendered to **one endpoint**, from every app [S17] | Audio rendered by one PID and its children (include), or by everything except that tree (exclude) [S2][S3] |
| Tied to an endpoint? | Yes. If the user switches from headphones to speakers you must re-open the new default; OBS does this through `IMMNotificationClient` [S5] | **No.** Microsoft: "the capture is not tied to a specific audio endpoint, eliminating the need to create a separate IAudioClient to capture from each physical audio endpoint" [S3] **[High]** |
| Format | Engine mix format from `GetMixFormat` (float) [S18] | `GetMixFormat` and `IsFormatSupported` return **E_NOTIMPL** (confirmed by Microsoft staff, 2023-03-03) [S10]. You must supply a format; use `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` [S3][S12]. OBS asks for float32 at its own rate and channel layout [S5] |
| Other non-working methods | — | wasapi-rs 0.26.0 lists: `GetDevicePeriod`, `GetCurrentPadding` not implemented; `GetBufferSize` returns "huge values"; no `IAudioClock`, `IAudioSessionControl` or `IAudioRenderClient` [S12]. GitHub issue #196 reports the same [S11] **[High]** |
| Timestamps | QPC position from `GetBuffer` (in 100 ns units) [S19]. OBS uses device timestamps for desktop loopback by default [S5] | Same QPC from `GetBuffer`. OBS switched to the OS-provided timestamp in 2023 (PR #9672), which "seems to resolve audio issues" [S14][S5] |
| Silence | When nothing plays, the stream can stall. OBS plays a silent buffer on the endpoint to keep it running ("Silent loopback fix") [S5] | If the target has no render streams, "the capturing process receives silence" [S3] |
| Exclusive mode | "An attempt to create a shared-mode stream fails if the device is already operating in exclusive mode" (`AUDCLNT_E_DEVICE_IN_USE`) [S18] | Not documented. Exclusive streams bypass the shared mixer, so expect them to be missing (**[Low]**, inference) |
| DRM audio | "A trusted audio driver does not permit a loopback device to capture digital streams that contain protected content" [S17] | Not documented; assume the same |
| Known long-run bugs | — | Noise or crackling after long sessions (OBS #8064, 2023-01 to 2025-10). OBS maintainers said it is a Microsoft-side issue fixed on Windows 11 only [S13] **[Med]** |

### 1.2 Limitations that matter for a game clipper

- **One PID per activation, and the tree is defined by parent/child relationships** [S2].
  - "Game, but not Discord, not Spotify and not Kairo" cannot be expressed with a single exclude. You need separate include captures, or one "exclude Kairo" capture for "everything".
  - Make Kairo's UI and capture service a single process tree, so one exclude covers all of Kairo's own sounds.
- **Window-to-PID mapping fails in some cases.**
  - OBS resolves the PID with `GetWindowThreadProcessId` on the chosen window, and its window-match modes are title, class and exe [S5].
  - It drops the capture when the window disappears ("window disappeared") [S5].
  - Failure cases:
    - (a) **Audio in a windowless sibling or launcher process.** OBS forum: Valorant voice chat in `RiotClientServices.exe`, which OBS's window list can't select [S24] **[Low]**.
    - (b) **Several audio sessions per game.** OBS forum: Sea of Thieves crew chat missing from the capture [S25] **[Low]**.
    - (c) **Packaged (UWP-style) apps**, whose top-level window belongs to `ApplicationFrameHost.exe` rather than the app process (inference from how Windows hosts UWP apps; **[Low]**). Most PC games are Win32 or GDK, so this is rare.
    - (d) **Browsers.** The whole browser is one tree, so every tab is captured together (OBS KB, via search summary) **[Low]**.
    - (e) **Steam.** Steam generally launches games as children of `steam.exe` (inference), so include-tree on `steam.exe` would also capture the game. Target the game's PID instead.
- **Anti-cheat.**
  - Capture takes only a PID; nothing is injected and no handle to the game process is required. The audio engine does the work.
  - I found no reports of anti-cheat blocking process loopback. The OBS forum user in [S24] captured Valorant's *game* audio fine; only voice chat was missing (**[Low]**).
  - Compare this with OBS's video "Game Capture" hook, which does inject.
  - Kairo should still resolve executable names with least-privilege calls such as `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and degrade gracefully if access is denied.
- **Long-session degradation.**
  - **#8064** (opened 2023-01-14): Discord audio captured with Application Audio Capture turns noisy after about 30 minutes; re-selecting the source fixes it temporarily [S13].
  - **#8086:** application audio crackles, then drifts out of sync after 10 to 30 minutes [S16].
  - Reports continued on Windows 11 24H2 through 2025 (noise after about 15 to 19 minutes, or after 10+ hours) [S13].
  - OBS maintainer, 2025-10-22: *"All of our testing so far has confirmed this to be an issue on Microsoft's end, which was fixed for a time, but seems to be affecting users again. Fixes will only be made to Windows 11."* The issue was closed 2025-10-25 after users confirmed the fix on Windows 11 25H2 (26200.6899) [S13].
  - The OBS mitigation PR #9807 (letting WASAPI resample, `hnsBufferDuration = 0`) was **closed unmerged** in 2025-08 [S15].
  - **Implication:** an always-on buffer running for hours must detect and recover from this, especially on Windows 10.
- **The Microsoft sample's own bugs.**
  - Issue #196 (2021): the sample passed `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` in the wrong parameter. It was fixed in commit `0b4e48a` (comment dated 2026-03-11) [S11].
  - OBS PR #9807 also notes that the sample uses a non-zero `hnsBufferDuration` with event-driven shared mode, which contradicts the SDK [S15].
  - An AI agent copying the sample must not copy these mistakes.
- **A wasapi-rs pitfall (Rust).**
  - `AudioClient::new_application_loopback_client(pid, include_tree)` maps `include_tree == false` to **`PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`**.
  - Its doc comment, however, says false means "only audio from the target process is captured" [S12]. The code and doc disagree; trust the code.

### 1.3 How OBS "Application Audio Capture" does it

- **Release history:** added in OBS **28.0.0** (2022-09-01), "allow capturing the audio output from a single process" [S6]. OBS **30.1.0** (2024-03-12) added a **"Capture Audio" checkbox to Window/Game Capture** [S7]. That checkbox creates a private child source of type `wasapi_process_output_capture` and reroutes its audio into the parent [S8].
- **OS gate:** the source type is registered only when the build is 19041 or later [S4].
- **Init sequence** [S5]:
  1. Find the window with `ms_find_window` and get its PID with `GetWindowThreadProcessId`.
  2. Build `WAVEFORMATEXTENSIBLE` as float32, using OBS's output sample rate and channel mask.
  3. Set `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` + `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`.
  4. Call `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, …)` and wait on a completion handler.
  5. Call `Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_LOOPBACK, …)`.
  6. Run an event-driven capture loop on RTWQ work queues where available.
  7. Use timestamps of `ts * 100` ns, taken from `GetBuffer`.
  8. Replace `AUDCLNT_BUFFERFLAGS_SILENT` packets with zeros.
  9. Log `TIMESTAMP_ERROR` (PR #9672 treats it as a failure and resets).
  10. Run a reconnect thread every 3,000 ms.
- **Window match priority:** "Window title must match", "Match title, otherwise find window of same type", or "Match title, otherwise find window of same executable" [S5 locale].
- **Default-device handling for *endpoint* sources:** a shared `WASAPINotify` (built on `IMMNotificationClient`) triggers `SetDefaultDevice`, which re-initializes the source [S5]. Desktop-audio sources default to "use device timestamps = true". Mic sources default to false (arrival time minus buffer duration) [S5].

### 1.4 Recommended audio-source design for Kairo (v1)

1. **Game stem:** process loopback with *include tree* on the **root** PID of the game. Resolve the foreground fullscreen or borderless window's PID, then walk up the parent PIDs while the image name is the same. Let users pin an exe and add extra processes for side-car audio, such as a voice-chat process.
2. **Voice-chat stem:** include tree on the root `Discord.exe`, plus a user-editable list (TeamSpeak, Mumble, and so on). Medal ships exactly this: game, mic and Discord as separate tracks, with Discord detected as its own source [S29].
3. **"Everything I heard" source for the mixed track.** Pick one of these, decided by Windows-version testing:
   - (a) process loopback with *exclude tree* on Kairo's own PID. It is endpoint-agnostic and leaves out Kairo's sounds, but is exposed to the Windows 10 long-session bug; or
   - (b) endpoint loopback on the default render device, with `IMMNotificationClient` re-open logic and the silent-render keep-alive.
4. **Mic:** WASAPI shared-mode, event-driven capture (section 2.1).
5. **Watchdog:**
   - Re-create a process-loopback client when it reports `TIMESTAMP_ERROR`, when the target process exits, or when packets stop while the target is still alive.
   - Consider re-creating it proactively at quiet moments, for example every 20 to 30 minutes while the buffer is idle. Users report that toggling the source clears the #8064 noise [S13].
   - Expose level meters per source so users can see a silent game track.
6. **Encode audio continuously into the ring buffer as AAC packets, not PCM.**
   - Float32 stereo PCM at 48 kHz is 384 kB/s, about **23 MB per minute per track**.
   - AAC at 160 kbps is about **1.2 MB per minute per track**.

---

## 2. Microphone, multi-track files, encoders, drift, device changes

### 2.1 Microphone capture specifics

- **Capture model:** standard WASAPI capture in shared mode, event-driven. Timestamps come from `GetBuffer`'s QPC position, in 100 ns units [S19].
- **Unplugging:** handle `AUDCLNT_E_DEVICE_INVALIDATED` (device unplugged or reconfigured) by reconnecting [S18][S19], as OBS's reconnect thread does every 3 s [S5].
- **Default device follow:**
  - Either track the default via `IMMNotificationClient`, as OBS does (its mic sources follow the *eCommunications* default) [S5],
  - or use **automatic stream routing**: activate via `DEVINTERFACE_AUDIO_CAPTURE` with `ActivateAudioInterfaceAsync`, Windows 10 1607+ [S20].
  - Microsoft documents automatic routing only for normal render and capture streams, not for loopback. **Don't rely on it for loopback.**
- **Ducking risk (important for an always-on mic):**
  - Windows treats "the stream opened on a communication device" as a *communication stream*. By default the system then ducks "all non-communication streams" by **80%**, a user setting on the Communications tab [S21][S22].
  - An always-on Kairo mic capture on the default communications mic could keep the game audio ducked both in the user's ears and in the clips.
  - **Mitigation:** call `IAudioSessionControl2::SetDuckingPreference(TRUE)` on Kairo's mic session. This opts out of the default ducking experience; Microsoft recommends calling it during stream creation [S23].
  - Verify on the test PC with the Communications tab at its default setting.
- **Device quirks:** Elgato advises turning off Windows "Audio enhancements" when an HD60 X's audio crackles on Windows 11 [S54]. Expect similar device-specific problems with mics.
- **Hardware encoder separate-mic options for comparison:**
  - AMD Adrenalin's "Separate Microphone Track" saves the mic to its own file. (AMD support article, seen via search summary; the page timed out when I re-checked.) **[Low]**
  - NVIDIA's "Separate both tracks" writes the mic to a second track. (2017-era article.) **[Low]**

### 2.2 Multi-track AAC in one MP4: who plays or imports what

| Consumer | Behaviour with track 1 = mixed (default), tracks 2–4 = stems | Evidence | Confidence |
|---|---|---|---|
| Chrome / Edge (and Chromium-based apps) | Plays only the first or default audio track. The `HTMLMediaElement.audioTracks` API sits behind `#enable-experimental-web-platform-features` in Chrome 37+ and Edge 79+. Safari supports it natively | MDN browser-compat-data [S26]; Chromium MP4 parser review: "Only the first audio and video track present in a file are…" [S27] | High |
| Discord (desktop Electron client and web) | Expect track 1 only, since it is Chromium-based. Not tested | Inference from [S26][S27] | Low |
| Windows Media Player (new), Movies & TV, File Explorer preview, Instagram | Medal: "Some applications do not pick the proper default track", naming Windows Media Player, Windows File Explorer, Windows Movies & TV and Instagram | Medal support article, modified "24 Feb", year not shown [S28] | Med |
| VLC | Lists and plays every track; usable for verifying files | OBS/Adobe forum reports [S30] | Med |
| DaVinci Resolve | Imports separate tracks "out of the box". The July 2026 Adobe bug reporter says the 1-frame offset seen in Premiere does not happen in Resolve | Adobe threads [S30][S31] | Med |
| Premiere Pro | **Version-dependent.** In 2025 users saw only track 1 from OBS multitrack files [S30]. In **Premiere 26.3.0 (July 2026)** multiple tracks import, but every track after the first is **offset by exactly one frame** in MP4/MOV (bug report status "Investigating"). MKV with H.264 is unaffected [S31] | Adobe bug report 2026-07-11, community manager reply 2026-07-13 [S31] | Med |
| CapCut (desktop) | **Unknown.** I found no documentation either way | — | Low; test |

**Recommended file layout**, the common pattern used by Medal and OBS users:

| Track | Content | Flags |
|---|---|---|
| 1 | "Mixed": what the user heard plus their mic, balanced | **default** (tkhd *enabled*) |
| 2 | Game | non-default |
| 3 | Voice chat | non-default |
| 4 | Mic (mono is fine) | non-default |

- **How FFmpeg handles this:**
  - FFmpeg "will automatically set the 'default' disposition flag on the first stream of each type" when no disposition is given [S38].
  - The MP4 muxer marks only default tracks as `tkhd`-*enabled*, and puts all audio tracks in one *alternate group* unless more than one is enabled [S39].
  - That is exactly the "play track 1, others are alternates" signalling players expect.
  - If an editor turns out to ignore *disabled* tracks, that would be one candidate explanation for Premiere's behaviour. This is an untested hypothesis: test with and without the enabled flags on the stems.
- **Offer a "Share copy" export** containing only the mixed track for Discord, Instagram and TikTok uploads. That sidesteps every player quirk above.
- **Muxer choice:** Microsoft's MPEG-4 file sink "does not support edit lists" and, as of its Windows 8 notes, "only supports single video track" [S35]. Use FFmpeg's libavformat for multi-track MP4 with proper `disposition` control and AAC priming handling.

### 2.3 Audio encoders available on Windows

| Encoder | Facts | Notes |
|---|---|---|
| **Media Foundation AAC encoder** (`CLSID_AACMFTEncoder`, `Mfaacenc.dll`) | AAC-**LC only**. Input must be **16-bit PCM** at **44.1 or 48 kHz**, with **1, 2 or 6 channels** (6 channels from Windows 10 onward). Output bitrates 12,000 / 16,000 / 20,000 / 24,000 bytes/s, i.e. **96 / 128 / 160 / 192 kbps** (×6 for 5.1). Default 96 kbps. One output sample is one 1,024-sample frame (21.33 ms at 48 kHz). Every input sample needs a time and a non-zero duration, or `ProcessOutput` divides by zero [S33] | In-box, so Kairo ships no encoder. Missing on **Windows N/KN** unless the Media Feature Pack is installed [S41]. FFmpeg wraps it as `aac_mf` (`libavcodec/mfenc.c`) [S37] |
| **FFmpeg native `aac`** | "The default AAC encoder, natively implemented into FFmpeg." Default 128 kbps CBR [S36] | LGPL. **AAC patent-pool licensing for shipping an encoder was not researched**; flag it for a licensing check |
| `libfdk_aac` | Supports HE-AAC; "incompatible with GPL", needs `--enable-nonfree` [S36] | Not redistributable in GPL builds; avoid |
| **Opus in MP4** | Not in Media Foundation's documented native codec list (page updated 2025-04-15) [S34]. **Premiere Pro 25.5: "the Opus audio track isn't currently supported in Premiere"** (Adobe community manager, 2025-09-11) [S32] | Good for web, bad for editors. Avoid for v1 clips |

**Recommendation:** AAC-LC, 48 kHz, 16-bit input. Use 160 to 192 kbps for stereo tracks and 96 to 128 kbps for a mono mic. Storage cost: four tracks at 160 kbps come to about **4.8 MB per minute**, which is negligible next to the video.

### 2.4 Clock drift and resampling

- **Every audio device runs on its own crystal.** The mic (USB headset), the render endpoint (motherboard or GPU HDMI), and a capture card's audio all drift relative to each other and to QPC.
- **Typical consumer tolerance is about ±50 to 100 ppm** (secondary sources [S42]; **[Med]**). At 48 kHz:
  - 100 ppm means 4.8 samples per second, or **about 360 ms per hour**;
  - 50 ppm means about 180 ms per hour (my own arithmetic).
- **Why this matters for a rolling buffer:** an always-on buffer runs for hours. If audio PTS were derived by *counting samples* since start, A/V sync would decay across the session even though each clip is short.
- **Required design:**
  1. Stamp every packet with its QPC time from `GetBuffer` [S19].
  2. Resample every source to one 48 kHz master timeline, using *timestamp-driven* drift compensation. Examples: libswresample's `async` option ("stretching, squeezing, filling and trimming"), `min_comp` and `max_soft_comp` [S40].
  3. Fill gaps with silence: loopback delivers no data during silence, and OBS uses a silent-render keep-alive [S5].
  4. Watch the `DATA_DISCONTINUITY` and `TIMESTAMP_ERROR` flags [S19][S14].
- **Precedent:** OBS uses OS timestamps for process capture since PR #9672 (2023) [S14]. Its proposed asynchronous resampling for drift (PR #6351) is still open [S13 timeline].

### 2.5 Default-device changes and exclusive mode

- **Headphones ↔ speakers:**
  - Endpoint loopback keeps capturing the *old* endpoint until you re-open it on `OnDefaultDeviceChanged`, which is what OBS does [S5].
  - Process loopback is unaffected, because it is not endpoint-bound [S3].
  - Automatic stream routing (Windows 10 1607+) is documented for regular streams via `DEVINTERFACE_AUDIO_RENDER/CAPTURE`, not for loopback [S20].
- **Exclusive mode:**
  - If any app holds the render device exclusively, shared-mode streams (including loopback) fail to open [S18]. The user can untick "Allow applications to take exclusive control of this device" (`AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED` exists for exactly this [S18]).
  - Show a clear diagnostic rather than recording silence.
  - Exclusive streams are presumably absent from process loopback too (**[Low]**, inference).

---

## 3. Console capture cards

### 3.1 How they appear to Windows

- **USB cards (current Elgato line and most AVerMedia)** are **UVC** video plus **UAC** audio, using in-box drivers.
  - Elgato's driver page lists as "UVC Device (no download)": HD60 S+, **HD60 X**, **4K X**, **Neo**, **4K S**, Cam Link 4K [S49].
  - The PCIe cards (4K60 Pro MK.2, 4K Pro, HD60 Pro) and the older HD60 S and 4K60 S+ need Elgato drivers. The 4K60 Pro "uses DirectShow" [S49].
- **Access APIs:**
  - Media Foundation (`IMFSourceReader`), WinRT `Windows.Media.Capture` / `MediaFrameReader`, or DirectShow.
  - OBS's Video Capture Device source is DirectShow (`win-dshow` + libdshowcapture), with FFmpeg decoding for MJPEG [S64].
- **Audio:** a separate Windows capture endpoint; Elgato refers to the card as a "Capture Device" under Windows Sound settings [S54].
  - Pair the video and audio functions automatically by matching **container IDs**. All devnodes of one physical device share a container ID [S67]. Applying this to capture cards is my inference: **[Med]**.
- **Sharing with other apps:**
  - Elgato lists the HD60 X, 4K X, 4K S and Neo as **without Multi App support**, so only one program can use them at a time [S50].
  - Microsoft documents `MediaCaptureSharingMode.SharedReadOnly`: "you can receive frames from the frame sources even if they are in use by another app, but you can't change the settings" (Windows 10 1607+; doc dated 2026-08-28) [S65][S66].
  - Whether SharedReadOnly coexists with OBS's DirectShow use of these cards is **untested**.
- **Privacy settings:** Windows' camera privacy controls (from Windows 10 1803) can block capture-card access [S57]. Kairo's onboarding must check them.

### 3.2 Popular models (specs from vendor pages; prices dated)

| Model | Interface | Capture (max) | Pass-through | HDR capture | UVC formats | Price (source, date) |
|---|---|---|---|---|---|---|
| **Elgato 4K S** | USB-C (5 Gbps) | 2160p60, 1440p60, 1080p120 | 2160p60, 1440p120, 1080p240, VRR, HDR | up to 1080p60, Windows only | UVC | **$159.99** launch (PCWorld 2025-07-23) [S45][S58] |
| **Elgato HD60 X** | USB 3.0 (5 Gbps) | 2160p30, 1440p60, 1080p60 HDR | 2160p60, 1440p120, 1080p240, VRR, HDR | up to 1080p60, Windows only | UVC | **$179.99** MSRP (cited by PCWorld 2025) [S43][S58] |
| **Elgato 4K X** | USB 3.2 Gen 2 (10 Gbps); Gen 1 works at reduced resolution | 2160p144 SDR; 4K30 / 1440p60 / 1080p120 **HDR** | 2160p144, 1440p240, 1080p240, VRR, HDR | yes (P010), Windows only | MJPG (to 4K144), **P010** (4K30, 1440p60, 1080p120), NV12/I420 (4K60 on Gen 2), YUY2, RGB [S47] | about **$229.99** MSRP, launched 2024-02-01 (Dexerto, PCGamesN, Corsair press release) [S44][S59] |
| **Elgato Neo** | USB 3.0 | 1080p60 | 2160p60 HDR | no; tone-maps HDR to SDR on the card | YUY2 / NV12 at 1080p60 [S48] | not verified [S46] |
| **AVerMedia Live Gamer ULTRA 2.1 (GC553G2)** | USB 3.2 Gen 2 | 2160p144 (Windows), 2160p60 (macOS) | 2160p144 HDR/VRR, 1440p240, 1080p360 | P010 | YUY2, NV12, RGB24, P010, MJPEG | **$209.99** sale / $299.99 regular (AVerMedia US store, read 2026-10-21) [S60]. UVC status not stated |
| **Cheap MacroSilicon dongles (MS2130 / MS2109)** | MS2130: USB 3.x; MS2109: USB 2.0 | MS2130: 1080p60 YUV422/MJPEG out, 4K30 in [S62] | Usually none (check each listing) | no | MS2109 needs MJPEG for 1080p: YUY2 1080p60 is about 1.99 Gbit/s against USB 2.0's 480 Mbit/s (arithmetic) | roughly $10–30 (forum reports) **[Low]** |

- **Latency:**
  - Elgato markets "lag-free passthrough" without giving a number [S43–S45].
  - The *preview* latency into software is not the same as passthrough. Elgato's comparison table shows "~250 ms preview latency" for the 4K60 S+, against "Instant Gameview" (low latency) for the others [S51].
  - For a cheap MS2130, one buyer reported 6 to 11 frames (about 100–180 ms at 60 fps) [S63] **[Low]**.
  - **For instant replay, passthrough is what matters for playability. Capture latency only shifts A/V offset**, which needs a per-source offset setting.

### 3.3 HDCP by console

| Console | HDCP behaviour | Source |
|---|---|---|
| **PS5 / PS5 Pro** | **On by default.** Turn it off under Settings → System → HDMI → *Enable HDCP*, or there is no picture | Elgato (edited 2026-10-16) [S52]; HD60 X PS5 guide [S55] |
| PS4 | On by default; can be disabled since firmware 1.70; always on for movies and video apps | [S52][S61] |
| Xbox Series X\|S / One | Off by default for games; turns on in Netflix and similar apps. Closing them (or restarting) disables it again. There is no manual toggle | AVerMedia FAQ 2025-04 [S61]; Elgato [S52] |
| Switch / Switch 2 | Off by default; on in media apps. Elgato lists Switch 2 as a supported (unencrypted) source for the 4K S | [S52][S61][S45]. No Nintendo statement found for Switch 2 **[Med]** |
| Streaming services (Netflix, Disney+ and others) | Always protected; cannot be captured | [S52][S61] |

### 3.4 Capture-card audio

- **Format support:** PCM/LPCM stereo **up to 24-bit/48 kHz** only. No Dolby (DD, DD+, TrueHD, Atmos), DTS, 2.1, 5.1 or 7.1 [S53].
- **Switch pitfall:** if the Switch outputs surround, only front left and right are captured, so **dialogue on the centre channel goes missing**. Set the console to stereo PCM [S53].
- **Line-in:** cards have a 3.5 mm *line-in*, which is not a mic input. It is for mixing party chat into the capture [S55].
- **Treat card audio as one more WASAPI capture source** with its own clock: apply drift compensation (section 2.4) and a user-adjustable A/V offset.

### 3.5 Running an instant-replay buffer on a capture-card source

- **Architecture:** the pipeline is the same as for screen capture.
  1. Open the UVC source in **NV12** (or YUY2), or **P010** for HDR. Avoid MJPEG unless needed, since it costs a decode.
  2. Feed AMF, Quick Sync or NVENC.
  3. Keep encoded packets in the ring buffer, with the card's UAC audio as a separate track.
- **Bandwidth:** 4K60 NV12 is about 5.97 Gbit/s (arithmetic). That fits USB 3.2 Gen 2 but not Gen 1, which matches Elgato's table: 4K60 NV12 on Gen 2, 4K30 on Gen 1 [S47].
- **HDR:** Elgato records HDR as HEVC in its own software [S47]. Shareable clips need **HDR→SDR tone-mapping**, or the cards' own on-card tone-mapping in SDR modes (the Neo, and the 4K X's "captured in SDR" tiles) [S47][S48].
- **Competitor reality:**
  - Elgato's 4K Capture Utility already offers **"Flashback Recording"** ("slide back in time to save gameplay retroactively") on most models.
  - The **4K S** column shows **no** Flashback; that card is supported by the newer "Elgato Studio" app instead [S51][S45].
  - Kairo's value is a unified PC+console library and an instant hotkey workflow, not the feature itself.
- **Pitfalls:**
  - exclusive device access (section 3.1);
  - the camera privacy toggle;
  - USB bandwidth and controller quality;
  - HDCP black frames on PS5;
  - Switch surround audio;
  - the A/V offset differing per card.

---

## 4. Cloud gaming on a Windows PC

### 4.1 What actually makes a capture black on Windows (mechanisms)

- **`SetWindowDisplayAffinity`:**
  - `WDA_MONITOR` means "everywhere else, the window appears with no content".
  - `WDA_EXCLUDEFROMCAPTURE` (Windows 10 2004+) means the window "does not appear at all" in captures [S68].
  - `GetWindowDisplayAffinity` works "from any process", but "succeeds only when the window is layered" [S69]. As a detector it is therefore only partial.
- **DXGI swap-chain flags** [S70]:
  - `DXGI_SWAP_CHAIN_FLAG_DISPLAY_ONLY`: content is "not accessible via remote accessing or through the desktop duplication APIs".
  - `HW_PROTECTED` / `RESTRICTED_CONTENT`: hardware-protected (DRM) content.
- **Browser DRM:** protected video through EME, such as Netflix. **WebRTC game streams are not EME-protected.**
- **HDCP** applies to HDMI capture cards, not to on-PC window capture.

### 4.2 Per-service assessment

| Service on a Windows PC | Delivery | Can WGC or Desktop Duplication capture it? | Audio for Kairo | Confidence |
|---|---|---|---|---|
| **GeForce NOW**, native app | NVIDIA client; needs 64-bit Windows 10+ and DirectX 11 [S73] | No DRM or display affinity documented. Expect capture to work with WGC. GFN has its own overlay capture (Highlights and manual recording, saved locally; blog 2021-09-13) [S75] | Process loopback on the GFN app | Med (test) |
| **GeForce NOW**, browser | Chrome 77+, Edge 91+, Firefox, Opera GX [S73] | Same as any browser window. "Black screen" reports in OBS usually come from legacy BitBlt window capture with hardware acceleration on; WGC composes correctly (**[Low]**) | Browser process tree (all tabs mixed) | Med |
| **Xbox Cloud Gaming**, browser (and xHome remote play) | **WebRTC**, according to the open-source `xbox-xcloud-player`: "connect to an xCloud / xHome stream using WebRTC" [S71][S72] | No EME DRM expected, so capturable | Browser tree | Med |
| **Xbox Cloud Gaming**, Xbox PC app | Not verified | Expected to behave like the browser; test | Xbox app process | Low |
| **Boosteroid** | Windows app plus browser; the site advertises streaming your sessions to YouTube [S77] | No information found; test | App or browser | Low |
| **Amazon Luna** | Could not verify current PC support (site showed an outage on 2026-10-21) [S78] | Unknown; test | Browser | Low |
| **PS Remote Play** (Windows app v9.5.0) | Sony app [S76] | Sony blocks *at the source*: video services can't be used, and "Depending on the game, some images may not be displayed while using Remote Play" [S76]. OBS users report black *window* capture but working display capture [S24 search] **[Low]** | Remote Play process | Med/Low |
| **PlayStation Portal** | A handheld device, not a Windows app | PC capture not possible. Since 2025-04-09, cloud-streaming captures from the Portal are auto-uploaded to Sony's cloud and kept **14 days** (≤1080p, ≤3 min) [S83] | — | High |

**Kairo guidance:**

- Treat cloud games as ordinary window or monitor capture using WGC, plus process audio of the client.
- Ship presets that map known client executables or browsers to "cloud game" profiles.
- At runtime, check display affinity where possible and detect all-black frames, then tell the user.
- Re-test whenever services update, because any of them could add `WDA_*` protection later.

### 4.3 Terms-of-service notes

- **NVIDIA GeForce NOW Terms of Use (v. 2025-08-18):** no clause on recording or broadcasting. They prohibit reverse engineering (§3.3) and botting or cheating (§3.2) [S74].
- **Microsoft Game Content Usage Rules (last updated January 2015):**
  - Personal, **noncommercial** use; shared items must carry a notice and a link to the Rules.
  - "You can't sell or otherwise earn any compensation from your Item", except YouTube or Twitch partner revenue and donations [S79].
  - This matters if Kairo later hosts or monetizes clips.
- **PSN Terms of Service (last revised April 2026):** no bypassing DRM (§10.6); no "automated methods, such as bots or scripts to interact with Content" (§10.7). Remote Play isn't mentioned [S80].
  - Reverse-engineering PSN capture endpoints conflicts with this.
- **Nintendo Game Content Guidelines (2024-09-02):** sharing videos and images is allowed with creative input. Footage from the Switch Capture Button may be posted as-is. Monetization only via listed platform programs [S81].
- **Bottom line:** capturing your own screen for personal clips is consistent with all of these. Never circumvent HDCP or DRM, and never automate scraping of PSN.

---

## 5. Importing console-native clips into a PC library

### 5.1 What exists today

| Console | Official paths | Retention and limits | Public API? | Unofficial |
|---|---|---|---|---|
| **Xbox Series X\|S / One** | (1) External USB drive. (2) **OneDrive auto-upload**: Settings → Preferences → Capture & share → Automatic uploads (Xbox Wire, 2023-09-27) [S85]. (3) Xbox mobile app | Captures uploaded to the Xbox network can be deleted after **90 days**; **10 GB** cap (policy from Oct 2023, deletions from 2024; secondary reports) [S86][S87] | No public consumer API for third-party apps; the Xbox app's MediaHub endpoints are **undocumented** (den.dev, 2021-12-10) [S87]. A Microsoft Community answer says there is no official PC download path without OneDrive or an external drive [S89] | **OpenXBL** (not affiliated with Microsoft): Free 150 req/h, Small $5/mo for 500, Medium $15/mo for 2,500, Large $35/mo for 5,000, Enterprise custom; endpoints for clips and screenshots [S88] |
| **PS5** | (1) Media Gallery → Copy to a USB drive (FAT32 or exFAT) [S84]. (2) **Auto-upload to the PS App** (mobile): clips ≤3 min at ≤1920×1080 plus screenshots; no PS Plus needed [S82] | **14 days** in the app; 4K clips are not uploaded [S82] | **None found.** Sony documents only the iOS and Android app [S82] | Reverse engineering would breach PSN ToS §10.7 [S80] |
| **PS Portal** (cloud streaming) | Auto-upload to the PS App [S83] | 14 days; ≤1080p, ≤3 min [S83] | None | — |
| **Switch** | **"Copy to a Computer via USB Connection"** over MTP (Windows supported; must use the console's USB-C port, not the dock); microSD card [S91] | Local | None | — |
| **Switch 2** | **"Copy to PC over USB"** over MTP [S92]. **Nintendo Switch App** upload (mobile), manual or **Automatic Uploads** [S93] | App: **30 days**, **100 files per Nintendo Account**; some games block uploads [S93]. In-console clips were reported capped at **30 s** at launch (2025-05) [S94] **[Med]** | None | — |

PS5 file formats, from third-party guides: video as WebM (needed for 4K) or MP4; screenshots as JPG or PNG, with HDR screenshots forced to JPG [S84] **[Med]**. Kairo's importer should accept WebM/VP9 as well as MP4/H.264/HEVC.

### 5.2 Realistic Kairo options

1. **v1-cheap: a generic importer.** Drag and drop, or "Import folder", that scans recursively for MP4, WebM, MOV, JPG and PNG, deduplicates by hash, and reads creation time and duration. This covers PS5 and Xbox via USB stick and any file the user already has.
2. **v1.x: Switch over MTP.** Detect a connected Switch or Switch 2 through **Windows Portable Devices (WPD)** [S95] and offer "Import new captures".
3. **v2: Xbox through OneDrive.** Either watch the local OneDrive sync folder (reported targets: OneDrive *Videos* and *Pictures* roots [S86]), or call **Microsoft Graph `driveItem: delta`** (delegated `Files.Read` works for personal accounts) [S90]. Fully official.
4. **Later, optional: OpenXBL.** It works around OneDrive, but it is unofficial, rate-limited, and breaks if Microsoft changes MediaHub. Captures also expire after 90 days.
5. **Not feasible:** automated PS5, PS Portal or Switch 2 cloud pulls, because the apps are mobile-only and there is no public API. Users can share files from phone to PC themselves.

---

## 6. Recommendations: v1 vs later

### Audio

| Stage | Build |
|---|---|
| **v1 (private beta)** | <ul><li>Process loopback for the **game stem**: auto-detected root PID, with user override and extra processes.</li><li>Process loopback for the **voice stem**: Discord by default, editable list.</li><li>WASAPI **mic**: user-selected device, `SetDuckingPreference(TRUE)`, reconnect on invalidation.</li><li>A **mixed default track** built from what the user heard plus the mic, using exclude-Kairo process loopback or endpoint loopback (choose after Win10/Win11 soak tests).</li><li>AAC-LC 48 kHz, 160–192 kbps.</li><li>FFmpeg (libavformat) MP4 with track 1 default.</li><li>QPC timestamps, libswresample drift compensation, silence fill.</li><li>Default-device change handling for the endpoint loopback and mic.</li><li>Exclusive-mode and empty-source diagnostics with level meters.</li><li>A **watchdog** that re-creates loopback clients (Windows 10 long-session bug).</li><li>A **"Share copy"** export with a single mixed track.</li></ul> |
| **v1.x** | <ul><li>Per-source volume and offset.</li><li>Per-game profiles that remember process sets.</li><li>Loudness normalization on export (EBU R128).</li><li>Mono or stereo mic option.</li><li>Optional HE-AAC is not recommended: Media Foundation supports it only via the profile-level attribute, and editors vary.</li></ul> |
| **Later** | <ul><li>Noise suppression on the mic (RNNoise, vendor SDKs, or Windows voice processing).</li><li>Automatic "voice ducking" in the mixed track.</li><li>Opus/WebM for web sharing in the cloud stage.</li><li>Spatial-audio handling (untested).</li></ul> |

### Console and cloud

| Stage | Build |
|---|---|
| **v1** | <ul><li>**Cloud gaming:** window or monitor capture through WGC (the video team's pipeline) plus the client process's audio, with presets for GFN app or browser, Xbox Cloud (browser or app) and PS Remote Play.</li><li>Black-frame and display-affinity warning.</li><li>Generic **file import** (section 5.2 item 1).</li></ul> |
| **v1.x (beta phase 2, only if the user owns a card)** | <ul><li>**UVC capture-card source**: Media Foundation or `MediaCapture`, NV12 1080p60 first.</li><li>Paired UAC audio via container ID.</li><li>Per-card A/V offset.</li><li>HDCP and privacy onboarding.</li><li>Then 4K and P010 HDR with tone-mapping.</li></ul> |
| **v2** | <ul><li>Switch MTP auto-import.</li><li>Xbox OneDrive/Graph import.</li><li>Capture-card instant replay at 4K/HDR.</li><li>Possibly `SharedReadOnly` coexistence with OBS.</li></ul> |
| **Not planned** | Reverse-engineered PSN downloads; anything that bypasses HDCP or DRM. OpenXBL only if users demand it (unofficial). |

---

## 7. Risks (planning-level)

1. **Process loopback depends on undocumented behaviour on Windows 10.** It is documented only from build 20348. Microsoft's long-session noise fix landed only on Windows 11, and Windows 10 consumer ESU runs to **2027-10-12** [S96][S97]. Expect Windows 10 users to report crackling.
2. **Wrong-process capture produces silent game tracks.** Launchers, windowless audio processes and multi-session games all cause this. The UX must make it visible.
3. **Multi-track incompatibility:** players and social apps choose a stem, or editors misalign tracks (Premiere 26.3). Mitigate with a mixed default track and a single-track share export.
4. **Ducking:** an always-open mic on the communications device can duck game audio by 80% unless Kairo opts out.
5. **Exclusive-mode apps** silently break endpoint loopback.
6. **Capture-card exclusivity, HDCP and surround** make console capture fragile (users run OBS or Elgato Studio at the same time; PS5 HDCP; Switch surround).
7. **Cloud clients could add capture protection** (`WDA_EXCLUDEFROMCAPTURE`) at any time, and PS Remote Play blocks some scenes at the source.
8. **Console clip import relies on short retention windows** (14, 30 and 90 days) and unofficial APIs if automated. Treat it as best-effort.
9. **Licensing:** AAC patent-pool licensing was not researched for shipping FFmpeg's encoder (the in-box Media Foundation encoder avoids shipping one). Windows N editions need the Media Feature Pack.
10. **Recording friends' voices** (the Discord stem) raises consent and privacy questions in some jurisdictions. This is a product and legal decision.

---

## 8. Test plan for the Windows gaming PC (before committing the architecture)

1. **Soak test, 4 to 8 hours:** process loopback (include game, include Discord, exclude Kairo) and endpoint loopback, on Windows 10 22H2 if available and on Windows 11 24H2/25H2. Listen for #8064-style noise and measure A/V drift.
2. **Default-device switching:** move between headphones, speakers and a USB headset during capture. Then open an exclusive-mode app (for example a WASAPI-exclusive music player).
3. **Ducking:** run an always-on mic with the Communications tab at its default, with and without `SetDuckingPreference(TRUE)`.
4. **Process coverage:** the user's own games (launcher-based, anti-cheat-protected), Discord desktop vs browser, Steam games (parent `steam.exe`), browser cloud gaming.
5. **Multi-track MP4 matrix:** Windows Media Player, Movies & TV, File Explorer, Chrome, Edge, Discord upload and inline player, Premiere (current version), Resolve, CapCut, VLC. Each with stems marked enabled and disabled.
6. **Cloud capture through WGC:** GFN app and browser, Xbox Cloud in Edge and Chrome and the Xbox app, Boosteroid, PS Remote Play. Check for black frames and call `GetWindowDisplayAffinity`.
7. **Capture card, if owned:** UVC formats, `SharedReadOnly` coexistence with OBS, container-ID audio pairing, the A/V offset, and the PS5 HDCP toggle.

---

## Sources

Dates are publication, update or "last revised" dates where the page shows one; otherwise the date I accessed it (2026-10-21).

- [S1] Microsoft Learn: AUDIOCLIENT_ACTIVATION_TYPE (min client "Windows 10 Build 20348"; updated 2024-02-22). https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-audioclient_activation_type
- [S2] Microsoft Learn: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS (updated 2024-02-22). https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params
- [S3] Microsoft Learn: Application loopback audio capture sample (2026-02-19). https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
- [S4] OBS Studio source: plugins/win-wasapi/plugin-main.cpp (19041 gate; read 2026-10-21). https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/plugin-main.cpp
- [S5] OBS Studio source: plugins/win-wasapi/win-wasapi.cpp and data/locale/en-US.ini (read 2026-10-21). https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/win-wasapi.cpp
- [S6] OBS Studio 28.0.0 release notes (2022-09-01). https://github.com/obsproject/obs-studio/releases/tag/28.0.0
- [S7] OBS Studio 30.1.0 release notes (2024-03-12). https://github.com/obsproject/obs-studio/releases/tag/30.1.0
- [S8] OBS Studio source: plugins/win-capture/audio-helpers.c/.h. https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/audio-helpers.c
- [S9] bozbez/win-capture-audio README. https://github.com/bozbez/win-capture-audio
- [S10] Microsoft Q&A: process loopback GetMixFormat E_NOTIMPL (accepted answer 2023-03-03). https://learn.microsoft.com/en-us/answers/a/1183153
- [S11] microsoft/Windows-classic-samples issue #196 (2021-08-26; comments through 2026-03-11). https://github.com/microsoft/Windows-classic-samples/issues/196
- [S12] HEnquist/wasapi-rs src/api.rs (crate v0.26.0; read 2026-10-21). https://github.com/HEnquist/wasapi-rs/blob/master/src/api.rs
- [S13] OBS issue #8064, "Sound becomes noisy after a long time" (2023-01-14 to 2025-10-25). https://github.com/obsproject/obs-studio/issues/8064
- [S14] OBS PR #9672, "Use provided timestamp for process capture" (merged 2023-10-08). https://github.com/obsproject/obs-studio/pull/9672
- [S15] OBS PR #9807 (closed unmerged 2025-08-22). https://github.com/obsproject/obs-studio/pull/9807
- [S16] OBS issue #8086 (2023-01-18). https://github.com/obsproject/obs-studio/issues/8086
- [S17] Microsoft Learn: Loopback Recording (2025-04-16). https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
- [S18] Microsoft Learn: IAudioClient::Initialize (updated 2025-09-05). https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize
- [S19] Microsoft Learn: IAudioCaptureClient::GetBuffer (updated 2025-09-05). https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer
- [S20] Microsoft Learn: Automatic Stream Routing (updated 2025-03-11). https://learn.microsoft.com/en-us/windows/win32/coreaudio/automatic-stream-routing
- [S21] Microsoft Learn: Using a Communication Device. https://learn.microsoft.com/en-us/windows/win32/coreaudio/using-the-communication-device
- [S22] Microsoft Learn: Default Ducking Experience. https://learn.microsoft.com/en-us/windows/win32/coreaudio/stream-attenuation
- [S23] Microsoft Learn: IAudioSessionControl2::SetDuckingPreference. https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-setduckingpreference
- [S24] OBS forum (via search summary; site returned HTTP 502 on direct fetch): Valorant voice in RiotClientServices.exe; also PS5 Remote Play black-screen threads. https://obsproject.com/forum/threads/obs-not-recording-player-character-audio-tried-several-ways-to-make-it-work.172940/ and https://obsproject.com/forum/threads/ps-remote-play-with-ps5-is-a-black-screen-ps4-completely-fine-help.146085/
- [S25] OBS forum (via search summary): Application Audio Capture missing a second mixer entry. https://obsproject.com/forum/threads/application-audio-capture-not-capturing-all-audio-when-process-has-multiple-mixer-entries.160169/
- [S26] MDN: HTMLMediaElement.audioTracks (modified 2025-05-27) and mdn/browser-compat-data api/HTMLMediaElement.json. https://developer.mozilla.org/en-US/docs/Web/API/HTMLMediaElement/audioTracks and https://github.com/mdn/browser-compat-data/blob/main/api/HTMLMediaElement.json
- [S27] Chromium code review 2254733006, "Allow MP4 parser to handle multiple audio and video tracks" (2016). https://codereview.chromium.org/2254733006
- [S28] Medal support: Multi-Track Recording (modified "24 Feb", year not shown). https://support.medal.tv/support/solutions/articles/48001220541-multi-track-recording
- [S29] Medal: Separate audio tracks feature page. https://medal.tv/features/separate-audio-tracks
- [S30] Adobe Community: "How to import OBS MP4 multitrack audio clip" (2025-03-14). https://community.adobe.com/t5/premiere-pro-discussions/how-to-import-obs-mp4-multitrack-audio-clip/m-p/15212992
- [S31] Adobe bug report: OBS multi-track audio offset by 1 frame, Premiere 26.3.0 (2026-07-11). https://community.adobe.com/bug-reports-728/imported-media-from-obs-which-contains-multiple-audio-tracks-will-have-all-audio-tracks-besides-the-first-one-off-by-1-frame-1631656
- [S32] Adobe Community: Premiere 25.5 MKV thread, "Opus audio track isn't currently supported" (2025-09-11). https://community.adobe.com/questions-729/pr-25-5-the-rewritten-mkv-support-is-off-1419561
- [S33] Microsoft Learn: AAC Encoder (updated 2024-07-03). https://learn.microsoft.com/en-us/windows/win32/medfound/aac-encoder
- [S34] Microsoft Learn: Supported Media Formats in Media Foundation (updated 2025-04-15). https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation
- [S35] Microsoft Learn: MPEG-4 File Sink (updated 2025-11-25). https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink
- [S36] FFmpeg Codecs documentation (aac, libfdk_aac). https://ffmpeg.org/ffmpeg-codecs.html
- [S37] FFmpeg source: libavcodec/mfenc.c (aac_mf). https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/mfenc.c
- [S38] FFmpeg documentation: -disposition. https://ffmpeg.org/ffmpeg.html
- [S39] FFmpeg source: libavformat/movenc.c (enable_tracks, tkhd alternate group). https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/movenc.c
- [S40] FFmpeg Resampler documentation (async, min_comp, max_soft_comp). https://ffmpeg.org/ffmpeg-resampler.html
- [S41] Microsoft Support: Media Feature Pack for Windows N. https://support.microsoft.com/windows/media-feature-pack-for-windows-n-8622b390-4ce6-43c9-9b42-549e5328e407
- [S42] Clock-drift background (secondary): Forasoft drift correction article and comp.dsp thread. https://www.forasoft.com/learn/audio-for-video/articles-audio/drift-correction-resampling-frame-drop and https://www.dsprelated.com/showthread/comp.dsp/32386-1.php
- [S43] Elgato Help: Game Capture HD60 X Technical Specifications (edited 2024-07-10). https://help.elgato.com/hc/en-us/articles/5293216945805
- [S44] Elgato Help: Game Capture 4K X Technical Specifications (edited 2024-09-03). https://help.elgato.com/hc/en-us/articles/23658118721421
- [S45] Elgato Help: Game Capture 4K S Technical Specifications (edited 2025-07-24). https://help.elgato.com/hc/en-us/articles/37196014096017
- [S46] Elgato Help: Game Capture Neo Technical Specifications (edited 2025-12-01). https://help.elgato.com/hc/en-us/articles/26022609172749
- [S47] Elgato Help: 4K X Supported Resolutions and Frame Rates / codec table (edited 2026-05-21). https://help.elgato.com/hc/en-us/articles/23479175821069
- [S48] Elgato Help: Neo supported passthrough and capture resolutions (edited 2024-05-03). https://help.elgato.com/hc/en-us/articles/26438781217933
- [S49] Elgato Help: Elgato Hardware Drivers (UVC list; edited 2026-10-02). https://help.elgato.com/hc/en-us/articles/360027961152
- [S50] Elgato Help: Capture Devices and Multi App Support (edited 2026-05-11). https://help.elgato.com/hc/en-us/articles/360042392672
- [S51] Elgato Help: Capture Devices device comparison (edited 2026-10-02). https://help.elgato.com/hc/en-us/articles/360041842611
- [S52] Elgato Help: HDCP and Elgato Capture devices (edited 2026-10-16). https://help.elgato.com/hc/en-us/articles/360040482032
- [S53] Elgato Help: multichannel digital audio not supported (edited 2026-06-01). https://help.elgato.com/hc/en-us/articles/360027951892
- [S54] Elgato Help: Windows 11 audio crackling with HD60 X. https://help.elgato.com/hc/en-us/articles/13113377366797
- [S55] Elgato Help: HD60 X PlayStation 5 setup. https://help.elgato.com/hc/en-us/articles/5329003032973
- [S56] Elgato Help: 4K X System Requirements. https://help.elgato.com/hc/en-us/articles/23635913547021
- [S57] Elgato Help: Video Capture and Windows 10/11 privacy settings. https://help.elgato.com/hc/en-us/articles/360027963692
- [S58] PCWorld: Elgato launches 4K S at $159.99 (2025-07-23). https://www.pcworld.com/article/2856673/elgato-launches-4k-60fps-capture-card-thats-cheaper-than-its-predecessor.html
- [S59] 4K X pricing and launch: Dexerto review (MSRP $229.99), PCGamesN ($230), Corsair press release (2024-02-01). https://www.dexerto.com/tech/elgato-game-capture-4k-x-review-2509698/ ; https://www.pcgamesn.com/elgato/4k-x-pro-capture-cards ; https://corsair.com/newsroom/press-release/elgato-unveils-its-most-powerful-game-capture-solutions-yet
- [S60] AVerMedia US store: Live Gamer ULTRA 2.1 GC553G2 (read 2026-10-21). https://shop-us.avermedia.com/collections/creators/products/avermedia-live-gamer-ultra2-1-gc553g2
- [S61] AVerMedia FAQ: HDCP by console (2025-04). https://www.avermedia.com/support/faq/Z71K716p
- [S62] CNX Software: MS2130 HDMI-to-USB board (2022-10-09). https://www.cnx-software.com/2022/10/09/10-hdmi-to-usb-3-2-video-acquisition-board-ms2130/
- [S63] Cinelerra-GG mailing list: MS2130/MS2109 dongles (2023-02; via search summary). https://lists.cinelerra-gg.org/pipermail/cin/2023-February/006338.html
- [S64] OBS Studio source: plugins/win-dshow (DirectShow capture). https://github.com/obsproject/obs-studio/tree/master/plugins/win-dshow
- [S65] Microsoft Learn: MediaCaptureSharingMode (Windows 10 1607+). https://learn.microsoft.com/en-us/uwp/api/windows.media.capture.mediacapturesharingmode
- [S66] Microsoft Learn: Process media frames with MediaFrameReader (2026-08-28). https://learn.microsoft.com/en-us/windows/apps/develop/camera/process-media-frames-with-mediaframereader
- [S67] Microsoft Learn: Overview of Container IDs. https://learn.microsoft.com/en-us/windows-hardware/drivers/install/overview-of-container-ids
- [S68] Microsoft Learn: SetWindowDisplayAffinity (updated 2025-07-01). https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- [S69] Microsoft Learn: GetWindowDisplayAffinity. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity
- [S70] Microsoft Learn: DXGI_SWAP_CHAIN_FLAG (updated 2026-06-17). https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ne-dxgi-dxgi_swap_chain_flag
- [S71] unknownskl/xbox-xcloud-player README ("using WebRTC"). https://github.com/unknownskl/xbox-xcloud-player
- [S72] unknownskl/greenlight (open-source xCloud/xHome client). https://github.com/unknownskl/greenlight
- [S73] NVIDIA: GeForce NOW system requirements (read 2026-10-21). https://www.nvidia.com/en-us/geforce-now/system-reqs/
- [S74] NVIDIA: GeForce NOW Terms of Use (v. 2025-08-18). https://www.nvidia.com/en-us/geforce-now/terms-of-use/
- [S75] NVIDIA Blog: Highlights, Freestyle and Montage in GeForce NOW (2021-09-13). https://blogs.nvidia.com/?p=52518
- [S76] Sony: PS Remote Play page (Windows app v9.5.0; read 2026-10-21). https://remoteplay.dl.playstation.net/remoteplay/lang/en/index.html
- [S77] Boosteroid homepage (read 2026-10-21). https://boosteroid.com/
- [S78] Amazon Luna (outage page on 2026-10-21). https://luna.amazon.com/
- [S79] Microsoft Game Content Usage Rules (last updated January 2015). https://www.xbox.com/en-US/developers/rules
- [S80] PlayStation Network Terms of Service (last revised April 2026). https://www.playstation.com/en-us/legal/psn-terms-of-service/
- [S81] Nintendo Game Content Guidelines (2024-09-02). https://www.nintendo.co.jp/networkservice_guideline/en/index.html
- [S82] PlayStation Support: How to access PS5 game captures on PS App (read 2026-10-21). https://www.playstation.com/en-us/support/games/ps5-game-captures-ps-app/
- [S83] PlayStation Blog: PS Portal update with cloud-streaming capture (2025-04-09). https://blog.playstation.com/2025/04/09/playstation-portal-remote-player-system-update-releases-today-with-cloud-game-streaming-beta-ux-enhancements/
- [S84] PS5 capture format and USB guides (secondary): Android Central, GamesRadar, The Sixth Axis (2020-11-11). https://www.androidcentral.com/how-change-ps5-video-capture-formats ; https://www.gamesradar.com/ps5-capture-settings ; https://www.thesixthaxis.com/2020/11/11/ps5-copy-videos-screenshots-usb-how-to/
- [S85] Xbox Wire: Bonus Xbox update for September, OneDrive captures (2023-09-27). https://news.xbox.com/2023/09/27/bonus-xbox-update-for-september/
- [S86] Xbox capture retention and OneDrive folders (secondary): Pure Xbox (2023-10), Thurrott. https://www.purexbox.com/news/2023/10/xbox-confirms-game-captures-will-be-deleted-beginning-january-2024 ; https://www.thurrott.com/games/289908/xbox-update-game-dvr-onedrive-integration
- [S87] den.dev: "Hidden Xbox APIs: Backing Up Media Captures" (2021-12-10). https://den.dev/blog/xbox-live-download-captures/
- [S88] OpenXBL homepage and pricing (read 2026-10-21). https://xbl.io/ and https://xbl.io/pricing
- [S89] Microsoft Community: downloading uploaded Xbox captures on PC without OneDrive. https://answers.microsoft.com/en-us/xbox/forum/all/how-do-i-download-my-uploaded-xbox-game-captures/1d4848c2-5fe3-49ca-83ec-5b3ba08ccc5b
- [S90] Microsoft Graph: driveItem delta (updated 2026-06-06). https://learn.microsoft.com/en-us/graph/api/driveitem-delta
- [S91] Nintendo Support: Transfer Switch captures to a computer via USB. https://en-americas-support.nintendo.com/app/answers/detail/a_id/53664
- [S92] Nintendo Support: Transfer Switch 2 captures to a computer via USB. https://en-americas-support.nintendo.com/app/answers/detail/a_id/68343
- [S93] Nintendo Support: Upload Switch 2 screenshots and videos to the Nintendo Switch App (30 days, 100 files). https://en-americas-support.nintendo.com/app/answers/detail/a_id/68386
- [S94] Nintendo Wire: Switch 2 recordings still capped at 30 seconds (2025-05-20). https://nintendowire.com/news/2025/05/20/recordings-on-the-switch-2-seem-to-still-be-restricted-to-30-seconds/
- [S95] Microsoft Learn: Windows Portable Devices. https://learn.microsoft.com/en-us/windows/win32/windows-portable-devices
- [S96] Microsoft: Windows 10 Extended Security Updates (ends 2027-10-12; read 2026-10-21). https://www.microsoft.com/en-us/windows/extended-security-updates
- [S97] Help Net Security: Windows 10 consumer ESU extended (2026-06-26). https://www.helpnetsecurity.com/2026/06/26/microsoft-windows-10-free-security-updates-esu-program/
- [S98] Microsoft Learn: Desktop Duplication API (updated 2025-04-15). https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- [S99] OBS Knowledge Base: Application Audio Capture guide (HTTP 502 on direct fetch; content via search summary: OBS 28 on Windows 10 2004+/Windows 11, OBS 30.1 "Capture Audio (BETA)"). https://obsproject.com/kb/application-audio-capture-guide
