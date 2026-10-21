# Kairo research 10: WASAPI audio capture, sync and mixing in Rust

- **Prepared:** 2026-10-21, as a code-level cookbook chapter for the Kairo engine (`kairo-engine.exe`, Rust).
- **Scope:** the `wasapi` crate (process loopback, endpoint loopback, mic), flags and QPC timestamps, device notifications, ducking, MMCSS, process-tree resolution (game, Discord), one 48 kHz timeline (drift, resampling, silence, mixing, limiter, meters), aligning audio to the video keyframe on save, a process-loopback watchdog, the Windows 10 long-session noise bug, crates, code skeletons, gotchas and a soak-test design.
- **Method.** I read the published source tarballs from crates.io: `wasapi` 0.26.0 (plus 0.20–0.25 for version history), `rubato` 5.0.1, `audioadapter` 5.0.0, `audioadapter-buffers` 5.2.0, `cpal` 0.18.2, `ffmpeg-next` 9.0.0, `ffmpeg-sys-next` 9.0.1, `windows` 0.62.2, `windows-core` 0.62.2, `windows-implement` 0.60.2 and `rtrb` 0.3.5. I also read:
  - FFmpeg `release/9.0` sources;
  - the Microsoft SDK and Win32 doc sources (the Markdown that learn.microsoft.com renders);
  - OBS `master` (`plugins/win-wasapi`), the OBS #8064 thread (all 150+ comments) and the related PRs and commits;
  - Microsoft's ApplicationLoopback sample;
  - Chromium's `AudioShifter`.
- **Local experiments on the Mac (no Windows run):**
  - (a) FFmpeg 8.0.1: decoding an AAC stream from the middle;
  - (b) a Python simulation of the drift loop, to choose its gains.

  Scripts: `prototypes/scripts/`.
- **Tags:**
  - **[Src]**: verified in the source code of the stated version.
  - **[Doc]**: verified in vendor documentation.
  - **[Exp]**: from a local experiment.
  - **[Estimate]**: my arithmetic or judgement.
  - **[Uncertain]**: one secondary source, or inference.
  - **[Unverified]**: I could not check it; test it on the PC.

---

## 0. Read first: what changes versus the brief and the plan draft

| # | Brief / plan says | Finding | Action |
|---|---|---|---|
| 1 | `wasapi` 0.26.0; `include_tree=false` means exclude | **Confirmed [Src].** In 0.26.0 `new_application_loopback_client(pid, include_tree)` maps `false` to `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`. The doc comment says "if false only audio from the target process is captured". Windows has **no** "target process only" mode; there are only include-tree and exclude-tree [Doc]. The crate's own doc example passes `false` with `std::process::id()`, which captures everything *except* the caller. | Wrap it in a Kairo `enum LoopbackMode { IncludeTree, ExcludeTree }` (§2.1). Pin `wasapi = "=0.26.0"`. |
| 2 | "Follow the default device via `IMMNotificationClient`" (implies the windows crate) | The crate has it **since 0.24.0**: `DeviceEnumerator::register_notification_callback(DeviceEventCallbacks)`. Session ducking has been there **since 0.21.0** (`AudioSessionControl::set_ducking_preference`). Raw escape hatches arrived **in 0.26.0** (`AudioClient::as_iaudioclient`, `Device::as_immdevice`, `Handle::as_handle`) [Src]. | No hand-written `IMMNotificationClient` (§2.6). |
| 3 | Mic: "`SetDuckingPreference(TRUE)`, otherwise Windows can turn other audio down by 80%" | **Misdirected.** Microsoft documents `SetDuckingPreference` as a way for a *media app's session to opt out of being ducked*, not as a way to stop a communications stream from ducking others [Doc]. The control for "don't duck others because of my stream" is `IAudioClientDuckingControl::SetDuckingOptionsForCurrentStream(AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS)`. It is documented for **render** streams from build 20348 [Doc]. One developer's test (2026-10-10, build 26100) found that `GetService` for it succeeds on a capture stream, and that a communications *capture* stream did not duck at all [Uncertain]. | Call both (cheap), open the mic on the **console** role by default, and test ducking on the PC (§3.1). |
| 4 | "Re-create a process-loopback client on `TIMESTAMP_ERROR`" | **Don't.** OBS did exactly that in PR #9672 (merged 2023-10-08), then reverted to "log only" in commit `0f498133a` (2023-11-27: "Only log packets with timestamp errors rather than resetting the device") [Src]. | Treat `TIMESTAMP_ERROR` as "don't trust this packet's timestamp" (§5.6). |
| 5 | Drift via libswresample `async` | Feasible, but neither `ffmpeg-next` 9.0.0 nor `rsmpeg` 0.18 wraps `swr_next_pts` or `swr_set_compensation` (you need raw FFI) [Src]. Its built-in controller reacts to raw timestamp jitter (with `async`, `min_comp` defaults to 1 ms) and corrects in bursts [Src]. You need your own timestamp smoothing anyway. | **Recommend `rubato` 5.0.1 `Async` (polynomial, Septic) plus a small PI loop on QPC error.** It is pure Rust, so it is unit-testable on the Mac. Keep swr as the documented fallback (§5.3–5.5). |
| 6 | Win10: "re-create every 20–30 min at quiet moments" | #8064 reports onset at 2 min–3 h. The community workaround script restarts **every 10 min** [Uncertain]. A hard restart leaves an audible gap. | Make it a **seamless A/B swap** (overlap two clients on the QPC timeline, crossfade 20 ms, drop the old one). Default every 10 min on Win10 and Win11 below 24H2, 60 min on 24H2+ (§6.2). |
| 7 | "Drop audio before the first video keyframe; ≤21.3 ms offset; no edit lists" | **[Exp]** Decoding an AAC stream from packet N gives a *wrong first frame* (1,024 samples; max error 0.12 on a 0.125-peak sine). From N+1 on it is bit-identical. | Default for library clips: include **one pre-roll AAC packet** and write true negative PTS. FFmpeg's MP4 muxer then emits an edit list that trims exactly to the keyframe, so start is sample-exact with no artefact. Share exports re-encode anyway (§5.8). |
| 8 | – | Missing gotchas, all in §7: <ul><li>Bluetooth headset mics force HFP (mono, low quality) while the mic is open.</li><li>The UI must be a child of the engine for "exclude Kairo's tree" to cover it.</li><li>Process-loopback activation has no timeout in the crate.</li><li>All `wasapi` objects are `!Send`.</li><li>`read_from_device_to_deque` panics on a failed `ReleaseBuffer`.</li><li>The `SILENT` flag requires zeroing the buffer.</li><li>`DATA_DISCONTINUITY` on the first packet.</li></ul> | See §7. |

---

## 1. Crates and versions (checked on crates.io 2026-10-21)

| Crate | Version (date) | Licence / MSRV | Role | Notes |
|---|---|---|---|---|
| `wasapi` | **0.26.0** (2026-10-11) | MIT / 1.85, edition 2024 | All WASAPI capture | Depends on `windows` and `windows-core` **^0.62.2**. Logs through the `log` crate. 0.27 will probably drop the `windows` dependency (open PR #74: "use windows-core [0.100] and generated bindings", MSRV 1.95). **Pin `=0.26.0`.** [Src] |
| `windows` | **0.62.2** (2025-10-06) | MIT/Apache / 1.82 | Ducking control, MMCSS, Toolhelp, process handles, events, QPC | Must be the **same** 0.62.x as `wasapi`'s, so that `wasapi::IAudioClient::GetService::<IAudioClientDuckingControl>()` type-checks. `windows-core` 0.100.0 exists (2026-09-03, MSRV 1.95), but no `windows` 0.100 umbrella crate does. [Src] |
| `rubato` | **5.0.1** (2026-10-01) | MIT/Apache / 1.87 | Drift-compensating resampler | Uses `audioadapter` **5.0.0** and `audioadapter-buffers` **5.2.0**, and re-exports both. The API changed a lot since 0.16; see §5.4. [Src] |
| `rtrb` | **0.3.5** (2026-08-18) | MIT/Apache | Wait-free SPSC ring from capture thread to aligner | `RingBuffer::<f32>::new(n)`, `push_entire_slice`, `pop_entire_slice`. [Src] |
| `crossbeam-channel` | **0.5.17** (2026-09-05) | MIT/Apache | Control and notification messages | `try_send` from COM callbacks (non-blocking). |
| `bytemuck` | **1.25.2** (2026-07-19) | Zlib/Apache/MIT | View an aligned `Vec<f32>` as `&mut [u8]` for `read_from_device` | Avoids unaligned casts. |
| `ffmpeg-next` | **9.0.0** (2026-08-05) | WTFPL | AAC encode and mux (other report). Optional swr | `ffmpeg_next::ffi` = `ffmpeg-sys-next` **9.0.1** (2026-10-09). Bindings are generated at build time. |
| `cpal` | 0.18.2 (2026-08-16) | Apache / 1.85 | **Not used** on Windows | See §2.8. |

The licences are all compatible with GPL-3.0.

---

## 2. `wasapi` 0.26.0: the parts Kairo uses

### 2.1 Process loopback (include/exclude tree), verified in source

`src/api.rs`, `AudioClient::new_application_loopback_client` [Src]:

```rust
ProcessLoopbackMode: if include_tree {
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
} else {
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE   // <- false = EXCLUDE
},
```

The doc comment on the same function reads: *"If true, the loopback capture client will capture audio from the target process and all its child processes, if false only audio from the target process is captured."* That is wrong. Microsoft defines only `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` ("Render streams from the specified process and its child processes are included") and `..._EXCLUDE_TARGET_PROCESS_TREE` ("…are excluded") [Doc].

```rust
// kairo-engine/src/audio/loopback.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopbackMode { IncludeTree, ExcludeTree }

/// wasapi 0.26.0: include_tree=true -> INCLUDE_TARGET_PROCESS_TREE, false -> EXCLUDE_TARGET_PROCESS_TREE.
/// Blocks until ActivateAudioInterfaceAsync completes (no timeout in the crate). Call on the capture thread.
pub fn open_process_loopback(pid: u32, mode: LoopbackMode) -> wasapi::WasapiRes<wasapi::AudioClient> {
    wasapi::AudioClient::new_application_loopback_client(pid, matches!(mode, LoopbackMode::IncludeTree))
}
```

**What the function does [Src]:**
- It builds `AUDIOCLIENT_ACTIVATION_PARAMS { ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, ProcessLoopbackParams { TargetProcessId, ProcessLoopbackMode } }` inside a `PROPVARIANT` of type `VT_BLOB`.
- It calls `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, …)` and then **waits on a `Condvar` with no timeout** until `ActivateCompleted` fires.
- The completion handler is a `#[implement]` object. `windows-implement` 0.60.2 makes such objects agile by default (`agile: true`, so `QueryInterface(IAgileObject)` succeeds). That satisfies the documented requirement that the handler implement `IAgileObject` [Src][Doc].
- It returns an `AudioClient` with `direction: Render, sharemode: Shared`. Calling `initialize_client(.., &Direction::Capture, ..)` then adds `AUDCLNT_STREAMFLAGS_LOOPBACK`. Passing `Direction::Render` would try to open a render stream and fail.

**Non-functional methods on a process-loopback client** (crate docs, matching Microsoft Q&A) [Src]:
- `get_mixformat`, `is_supported`, `get_device_period`, `get_current_padding` → "Not implemented".
- `get_buffer_size` returns huge garbage values.
- `get_audiosessioncontrol`, `get_audioclock`, `get_audiorenderclient` → `E_NOINTERFACE`.
- `get_next_packet_size` **works**: the crate's `record_application` example uses it.

**OS support:**
- Documented minimum: "Windows 10 Build 20348" [Doc].
- OBS registers it from 19041 [Src, OBS `plugin-main.cpp`].
- `win-capture-audio`'s README warns that on outdated Win10 builds "the source [is] not picking up any audio". So activation success is not proof that audio flows [Uncertain]. The capability probe must check for real samples (§8.2).

### 2.2 `initialize_client`: exactly which flags reach `IAudioClient::Initialize` [Src]

| `StreamMode` | ShareMode | StreamFlags (`+LOOPBACK` when the client is Render and you pass `Direction::Capture`) | hnsBufferDuration | hnsPeriodicity |
|---|---|---|---|---|
| `EventsShared { autoconvert: true, buffer_duration_hns: 0 }` **← use this** | SHARED | `EVENTCALLBACK \| AUTOCONVERTPCM \| SRC_DEFAULT_QUALITY` | 0 | 0 |
| `PollingShared { autoconvert, buffer_duration_hns }` | SHARED | `AUTOCONVERTPCM \| SRC_DEFAULT_QUALITY` if autoconvert | as given | 0 |
| `EventsExclusive { period_hns }` | EXCLUSIVE | `EVENTCALLBACK` | period | period |
| `PollingExclusive { .. }` | EXCLUSIVE | 0 | as given | as given |

- Microsoft: *"For a shared-mode stream that uses event-driven buffering, the caller must set both hnsPeriodicity and hnsBufferDuration to 0"* [Doc]. The crate passes whatever you give it, so **pass 0**:
  - the crate's `record.rs` example passes `min_time` (contrary to the docs);
  - `record_application.rs` passes 0;
  - OBS passes 5 s;
  - OBS PR #9807, which switched to 0 for #8064, was closed unmerged.
- `AUTOCONVERTPCM` inserts "a channel matrixer and a sample rate converter … between the uncompressed format supplied to `Initialize` and the audio engine mix format". `SRC_DEFAULT_QUALITY` chooses the better SRC "if the audio is ultimately intended to be heard by humans" [Doc]. The crate sets both when `autoconvert: true`.
- Process loopback requires a format you choose, because `GetMixFormat` is `E_NOTIMPL`. **Ask every source for `WAVEFORMATEXTENSIBLE` float32, 48,000 Hz, stereo** (the mic: mono): `WaveFormat::new(32, 32, &SampleType::Float, 48_000, channels, None)` [Src].
- Microsoft's sample, after the fix for samples issue #196, uses `LOOPBACK | EVENTCALLBACK | AUTOCONVERTPCM` with `0, 0` [Src].
- Crate errors to expect: `WasapiError::LoopbackWithExclusiveMode`, `WasapiError::RenderToCaptureDevice`, and `WasapiError::Windows(windows_core::Error)` for everything else [Src].

### 2.3 Event-driven capture, QPC timestamps and flags

**What the crate returns [Src]:**
- `AudioCaptureClient::read_from_device(&self, data: &mut [u8]) -> WasapiRes<(u32 /*frames*/, BufferInfo)>`, where `BufferInfo { flags: BufferFlags { data_discontinuity, silent, timestamp_error }, index: u64 /*device position, frames*/, timestamp: u64 /*QPC, 100 ns*/ }`.
- Each call is exactly one `GetBuffer`/`ReleaseBuffer` pair. Drain until `get_next_packet_size()? == Some(0)`.
- `AUDCLNT_S_BUFFER_EMPTY` is a success code, so it comes back as `Ok((0, _))`.

**Timestamp semantics [Doc]:**
- `pu64QPCPosition` is "the value of the performance counter at the time that the audio endpoint device recorded the device position of the first audio frame in the data packet". It is converted as `10,000,000 · t / f` (100 ns units).
- That is the same unit and epoch as WGC's `SystemRelativeTime`, so audio and video share one clock with no translation.

**Flags (`_AUDCLNT_BUFFERFLAGS`) [Doc] and Kairo's handling:**

| Flag | Microsoft | Kairo |
|---|---|---|
| `SILENT` (0x2) | "Treat all of the data in the packet as silence and ignore the actual data values." | **Zero the samples yourself.** The crate copies whatever bytes are in the buffer, and OBS zero-fills [Src]. |
| `DATA_DISCONTINUITY` (0x1) | "not correlated with the previous packet's device position … stream state transition or timing glitch". Win7+: usable for glitch detection. | Hard re-anchor of this source to the packet's timestamp (§5.6). Ignore it on the **first packet after `Start`**: `cpal` 0.18.2 skips it when `device_position == 0`, because it is "undefined on the first GetBuffer after Start" [Src]. |
| `TIMESTAMP_ERROR` (0x4) | "The time at which the device's stream position was recorded is uncertain." | Don't feed this packet's timestamp to the drift loop; place it by prediction. Count it. **Do not reset** (OBS reverted that) [Src]. |

**`GetBuffer` rules [Doc]:**
- `GetBuffer` and `ReleaseBuffer` must alternate on the same thread; otherwise you get `AUDCLNT_E_OUT_OF_ORDER`.
- Release within one engine period, or you risk losing data.

**Error codes to classify:**
- `AUDCLNT_E_DEVICE_INVALIDATED` (0x88890004), `AUDCLNT_E_RESOURCES_INVALIDATED` (0x88890026), `AUDCLNT_E_SERVICE_NOT_RUNNING` (0x88890010): reconnect with backoff.
- `AUDCLNT_E_DEVICE_IN_USE` (0x8889000A, exclusive mode): diagnostic.
- `E_ACCESSDENIED`: mic privacy [Unverified that blocked desktop mic access shows up as this error rather than as silence; test it].

All of these constants exist in `windows::Win32::Media::Audio` / `Foundation` 0.62.2 [Src].

**Crate-specific traps [Src]:**
- `read_from_device` checks the slice size *after* `GetBuffer`. If the slice is too short it **releases the packet and returns `DataLengthTooShort`**, so the data is lost. Allocate one second up front.
- `read_from_device_to_deque` calls `ReleaseBuffer(..).unwrap()`, which **panics** if the device dies mid-read, and it grows a `VecDeque` on the audio thread. **Never use it.**
- Wrapped COM objects are **`!Send`**: `windows-core` 0.62 implements no `Send` for Win32 interfaces [Src]. Create, use and drop every `AudioClient`, `AudioCaptureClient` and `Handle` on the one capture thread that owns them.

**Capture-thread skeleton** (wasapi 0.26.0 + windows 0.62.2 + rtrb 0.3.5 + bytemuck):

```rust
use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, WaitForMultipleObjects};

pub enum SourceSpec { Process { pid: u32, mode: LoopbackMode }, Endpoint { device_id: String, is_mic: bool } }
#[derive(Clone, Copy)] pub struct PacketMeta { pub ts_100ns: u64, pub arrival_100ns: u64, pub device_pos: u64, pub frames: u32, pub flags: u8 }
const F_DISC: u8 = 1; const F_SILENT: u8 = 2; const F_TSERR: u8 = 4;

struct Mmcss(HANDLE);
impl Drop for Mmcss { fn drop(&mut self) { unsafe { let _ = AvRevertMmThreadCharacteristics(self.0); } } }

/// Runs on its own std::thread. Every COM object lives and dies here.
fn capture_main(spec: SourceSpec, ch: usize, pcm: &mut rtrb::Producer<f32>, meta: &mut rtrb::Producer<PacketMeta>,
                shutdown: HANDLE, ready: &crossbeam_channel::Sender<Result<(), String>>) -> anyhow::Result<()> {
    wasapi::initialize_mta().ok()?;                       // HRESULT -> Result; S_FALSE is fine
    let mut task = 0u32;
    let _mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task) }.ok().map(Mmcss);
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, 48_000, ch, None);
    let mut client = match &spec {
        SourceSpec::Process { pid, mode } => open_process_loopback(*pid, *mode)?,      // may block, see watchdog
        SourceSpec::Endpoint { device_id, .. } => DeviceEnumerator::new()?.get_device(device_id)?.get_iaudioclient()?,
    };
    // Render endpoint + Direction::Capture => endpoint loopback; capture endpoint => mic.
    client.initialize_client(&fmt, &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })?;
    if let SourceSpec::Endpoint { is_mic: true, .. } = spec { mic_no_ducking(&client); }  // §3.1, before Start
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    let mut buf = vec![0f32; 48_000 * ch];                // 1 s, f32-aligned
    client.start_stream()?;
    let _ = ready.send(Ok(()));
    let handles = [event.as_handle(), shutdown];
    let mut first = true;
    loop {
        let r = unsafe { WaitForMultipleObjects(&handles, false, 20) }; // 20 ms poll as a backstop (OBS: 10 ms for loopback)
        if r.0 == WAIT_OBJECT_0.0 + 1 { break; }                        // shutdown event
        while let Some(n) = capture.get_next_packet_size()? {            // drain everything, also on timeout
            if n == 0 { break; }
            let (frames, info) = capture.read_from_device(bytemuck::cast_slice_mut::<f32, u8>(&mut buf))?;
            if frames == 0 { break; }
            let samples = frames as usize * ch;
            if info.flags.silent { buf[..samples].fill(0.0); }
            let mut flags = 0u8;
            if info.flags.data_discontinuity && !(first && info.index == 0) { flags |= F_DISC; }
            if info.flags.silent { flags |= F_SILENT; }
            if info.flags.timestamp_error { flags |= F_TSERR; }
            first = false;
            let m = PacketMeta { ts_100ns: info.timestamp, arrival_100ns: qpc_100ns(), device_pos: info.index, frames, flags };
            if pcm.push_entire_slice(&buf[..samples]).is_ok() { let _ = meta.push(m); } // else: overrun, the aligner sees a gap
        }
    }
    let _ = client.stop_stream();
    Ok(())  // locals drop here (COM releases); the caller then calls wasapi::deinitialize()
}
```

`qpc_100ns()` uses `QueryPerformanceCounter` and `QueryPerformanceFrequency` (`windows::Win32::System::Performance`). Cache the frequency, and compute in `i128`: `c * 10_000_000 / f`.

### 2.4 Mic in shared mode

**Device:**
- **Default:** the console-role default capture device: `DeviceEnumerator::get_default_device_for_role(&Direction::Capture, &Role::Console)`.
- **Optional:** the user's pinned device ID via `get_device(id)`.
- OBS follows the *eCommunications* default for mics [Src], but the OS treats "the stream opened on a communication device" as a communication stream [Doc]. Console is the safer default for ducking; it is the same device for most headsets.

**Stream setup:**
- **Format:** float32, 48 kHz, **mono**. `AUTOCONVERTPCM` downmixes stereo mics.
- **Stream category:** leave it at the default (`Other`). `StreamCategory::Communications` through `set_properties` is what enables Windows AEC (the crate's `aec` example) [Src]. Keep it for a later "speakers user" option.
- **Session events:** `client.get_audiosessioncontrol()?.register_session_notification(EventCallbacks)`, with `set_disconnected_callback(|reason: DisconnectReason| …)`. That reports `DeviceRemoval`, `FormatChanged`, `ExclusiveModeOverride`, `ServerShutdown` and others [Src]. Keep the returned `EventRegistration` alive on the capture thread.

**Reconnect triggers:** `AUDCLNT_E_DEVICE_INVALIDATED`, a session disconnect, or a default-device change (§2.6). Use OBS's model: retry every 3 s [Src].

### 2.5 Endpoint ("system") loopback: the fallback source

- Get the device with `get_default_device_for_role(&Direction::Render, &Role::Console)`, then `get_iaudioclient()`, then `initialize_client(&fmt48k, &Direction::Capture, EventsShared{autoconvert: true, 0})`. The crate adds `LOOPBACK` [Src].
  - Microsoft says loopback data is "in the device format" [Doc]. Whether `AUTOCONVERTPCM` converts loopback data on every build is **[Unverified]**; `cpal` never uses autoconvert for capture [Src].
  - Fallback: `get_mixformat()`, then let rubato do the rate conversion. The `Async` ratio becomes `48000 / mix_rate`; use `new_sinc` when the ratio is not ≈1.
- Event-driven loopback works from Win10 1703 [Doc].
- With nothing playing, endpoint loopback may deliver nothing. Kairo's timeline fills silence by QPC, so OBS's "silent render keep-alive" is not needed.
- Re-open the source on `OnDefaultDeviceChanged(Render, Console)`, as OBS does [Src].
- An app holding the device in exclusive mode makes `Initialize` fail with `AUDCLNT_E_DEVICE_IN_USE` [Doc]. Report it in diagnostics instead of recording silence.

### 2.6 Default-device and endpoint notifications (built into the crate since 0.24.0)

```rust
pub enum DevEvent { Default { dir: wasapi::Direction, role: wasapi::Role, id: Option<String> }, Removed(String), State(String, wasapi::DeviceState) }

/// Call on a long-lived MTA "device-monitor" thread; keep the returned registration on that thread.
pub fn watch_devices(en: &wasapi::DeviceEnumerator, tx: crossbeam_channel::Sender<DevEvent>)
    -> wasapi::WasapiRes<wasapi::DeviceEventRegistration> {
    let mut cb = wasapi::DeviceEventCallbacks::new();
    let t = tx.clone(); cb.set_default_device_callback(move |dir, role, id| { let _ = t.try_send(DevEvent::Default { dir, role, id }); });
    let t = tx.clone(); cb.set_device_removed_callback(move |id| { let _ = t.try_send(DevEvent::Removed(id)); });
    cb.set_device_state_callback(move |id, st| { let _ = tx.try_send(DevEvent::State(id, st)); });
    en.register_notification_callback(cb)   // Drop unregisters (synchronously)
}
```

Rules from Microsoft's `IMMNotificationClient` page [Doc]:
- Callbacks must be **non-blocking**.
- Never call `Register`/`UnregisterEndpointNotificationCallback` inside a callback.
- Never release the final reference on an MMDevice object inside a callback.

What that means for Kairo:
- **Only `try_send` from the closures.** Do the re-open work on your own thread.
- **Never drop the `DeviceEventRegistration` from a thread that a callback might be waiting on.** `cpal`'s source notes that Unregister "waits for any in-progress `IMMNotificationClient` callback to finish", so it can deadlock [Src].
- **Expect three callbacks per user change** (eConsole, eMultimedia, eCommunications). Filter on the role you follow, de-duplicate by ID as OBS does [Src], and debounce by 300 ms [Estimate].
- **`id == None`** means there is no default device left for that role [Doc]. `cpal` claims Windows sometimes sends no callback in that case [Src, unconfirmed], so also act on `Removed`/`State` for the device you are using.
- **Process loopback is not tied to an endpoint** (Microsoft's sample: "the capture is not tied to a specific audio endpoint") [Doc]. Whether it continues seamlessly across a headphones↔speakers switch is **[Unverified]**; test it (§8.2).

### 2.7 Version traps for AI-written code [Src]

- 0.26.0 **removed** `get_next_nbr_frames` (now `get_next_packet_size`), `get_periods` (now `get_device_period`) and `set_audio_stream_category` (now `set_properties(AudioClientProperties::new().set_category(..))`).
- 0.18+ uses the `StreamMode` enum. Older snippets that call `initialize_client(&fmt, period, &Direction, &ShareMode, bool)` will not compile.
- `Direction` and `Role` are passed **by reference** (`&Direction::Capture`).
- Logging uses `log`. Bridge it into `tracing` with `tracing-log`, and filter `wasapi=warn`: `read_from_device` emits a `trace!` per packet.

### 2.8 `cpal` 0.18.2, briefly [Src]

| | `cpal` 0.18.2 | `wasapi` 0.26.0 |
|---|---|---|
| Process loopback | **No** | Yes |
| Endpoint loopback | Yes (an output device used as input turns on `LOOPBACK`) | Yes |
| Buffer flags | Only `DATA_DISCONTINUITY` (reported as an Xrun error). **`SILENT` ignored.** | All three, plus device position and QPC |
| Capture formats | Native formats only; no `AUTOCONVERTPCM` for capture | Any format, with autoconvert |
| Default-device change | Stream reports `StreamInvalidated` or `DeviceNotAvailable` (no re-route) | You get callbacks and choose |
| MMCSS | Optional `realtime` feature (`audio_thread_priority`) | Do it yourself (§3.3) |
| Verdict | Not suitable for Kairo's Windows engine. Only useful for a future macOS port. | **Use** |

---

## 3. Ducking, Bluetooth and MMCSS

### 3.1 Ducking: what the APIs actually do

**What Microsoft documents [Doc]:**
- The OS "considers the stream opened on a communication device to be a communication stream". "The audio system generates ducking events when a communication stream is opened or closed for rendering or capturing streams."
- The default experience reduces other streams by 80% (user setting, Sound → Communications).
- `IAudioSessionControl2::SetDuckingPreference(optOut)` "enables or disables the default stream attenuation experience". The how-to page *Disabling the Default Ducking Experience* uses it in a **media app**, on "the session manager for the audio session that provides volume control for the **non-communication** streams". In other words, it opts the caller's own playback out of *being ducked*.
  - "The opt-out change does not take effect until the stream is stopped and started again."
  - "We recommend that the application call SetDuckingPreference during stream creation."
- `IAudioClientDuckingControl::SetDuckingOptionsForCurrentStream(AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS)`: "allows an app to specify that the system shouldn't duck the audio of other streams when the app's audio **render** stream is active". Minimum client "Windows 10 Build 20348". It is obtained via `IAudioClient::GetService`.

**Evidence that capture streams behave differently [Uncertain]:** the speecher PR #270 (2026-10-10, Win11 26100 VM, loopback RMS):
- a communications **capture** stream on the default mic: no ducking (0.3535 → 0.3535);
- a communications **render** stream: ducked (→ 0.0445);
- the same render stream with `DO_NOT_DUCK_OTHER_STREAMS`: not ducked;
- `GetService` for the ducking control "succeeds on a capture stream".

The author concludes that `SetDuckingPreference` "opts the caller's session out of being ducked, so it doesn't help here".

**Kairo recipe:**
- Mic on the console role.
- After `initialize_client`, before `start_stream`: call both APIs below.
- Kairo's own sound cue is a render stream: give it `DO_NOT_DUCK_OTHER_STREAMS` and a non-communications category.

```rust
use windows::Win32::Media::Audio::{IAudioClientDuckingControl, AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS};

fn mic_no_ducking(client: &wasapi::AudioClient) {
    // (a) Documented meaning: don't duck *our* session. Harmless; available since wasapi 0.21.0.
    if let Ok(sc) = client.get_audiosessioncontrol() { let _ = sc.set_ducking_preference(true); }
    // (b) Don't let *this stream* duck others. Documented for render streams on build 20348+; E_NOINTERFACE elsewhere.
    unsafe {
        if let Ok(dc) = client.as_iaudioclient().GetService::<IAudioClientDuckingControl>() {
            let _ = dc.SetDuckingOptionsForCurrentStream(AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS);
        }
    }
}
```

**Diagnostics only:** read (never write) `HKCU\Software\Microsoft\Multimedia\Audio\UserDuckingPreference`. The DWORD values are 0 = mute others, 1 = reduce by 80% (default when absent), 2 = 50%, 3 = do nothing. This mapping comes from community sources plus one Microsoft-hosted blog post [Uncertain], so put it in the capability report.

### 3.2 Bluetooth headset mics (new gotcha)

**The problem:** a classic Bluetooth headset can play A2DP (high-quality stereo, no mic) or HFP (mic plus mono, low-quality playback). Opening the headset mic switches it to HFP, so an **always-on** Kairo mic would degrade the user's game audio for the whole session (Microsoft Q&A answers; Windows 11 LE Audio "super wideband" fixes it only on supported hardware) [Uncertain].

**Kairo:**
- Detect Bluetooth capture endpoints and do **not** auto-enable the mic on them. Explain why in onboarding.
- Detection: the endpoint's enumerator or bus via `Device::as_immdevice().OpenPropertyStore(..)`, for example `PKEY_Device_EnumeratorName` = `BTHENUM`/`BTHHFENUM` [Unverified property values; check on the PC with a BT headset].

### 3.3 MMCSS for capture threads [Doc][Src]

- `AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index)` → `windows::core::Result<HANDLE>`. On exit call `AvRevertMmThreadCharacteristics(h)`. Both are in `windows::Win32::System::Threading` and link `avrt.dll`.
- The task name must be a subkey of `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile\Tasks`. The defaults are Audio, Capture, Distribution, Games, Playback, Pro Audio and Window Manager.
- OBS uses `"Audio"` on its capture threads. Microsoft's loopback sample uses an MF work queue with `"Capture"`.
- **Don't use "Pro Audio"** (High category, priority 23–26), so it doesn't compete with the game.
- If it fails (MMCSS disabled, `ERROR_INVALID_TASK_NAME`), log and continue at normal priority.
- The mixer/encoder thread does not need MMCSS: it runs 200 ms behind real time (§5.2).

---

## 4. Process-tree resolution

### 4.1 Primitives (windows 0.62.2) [Src][Doc]

**Snapshot:**
- `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)`, then `Process32FirstW`/`Process32NextW` over `PROCESSENTRY32W { dwSize, th32ProcessID, th32ParentProcessID, szExeFile: [u16; 260], .. }`. The struct implements `Default`.
- Features: `Win32_System_Diagnostics_ToolHelp`, `Win32_Foundation`.

**Parent IDs can lie:** `th32ParentProcessID` is "the process that created this process". It is recorded at creation and can point to an **unrelated process that reused a dead parent's PID** (Old New Thing, 2015-04-03). Guard every hop:
- the parent's creation time (`GetProcessTimes`) must be `<=` the child's;
- open with `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)`;
- if you can't read it, stop walking.

**Pin the target:** holding a process handle keeps the PID from being reused. Open `PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION` (the flags implement `BitOr`) and poll `WaitForSingleObject(h, 0) == WAIT_OBJECT_0` to detect exit. Whether these minimal rights are granted for elevated or anti-cheat-protected games is **[Unverified]**. On failure, fall back to polling the Toolhelp snapshot every 2 s.

**Full path:** `QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)`.

```rust
pub struct Proc { pub pid: u32, pub ppid: u32, pub exe: String /* lower-case file name */ }

pub fn snapshot() -> windows::core::Result<Vec<Proc>> { unsafe {
    let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut out = Vec::with_capacity(512);
    let mut ok = Process32FirstW(snap, &mut e).is_ok();
    while ok {
        let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
        out.push(Proc { pid: e.th32ProcessID, ppid: e.th32ParentProcessID,
                        exe: String::from_utf16_lossy(&e.szExeFile[..n]).to_ascii_lowercase() });
        ok = Process32NextW(snap, &mut e).is_ok();               // Err(ERROR_NO_MORE_FILES) ends the walk
    }
    let _ = CloseHandle(snap);
    Ok(out)
}}

pub fn creation_time(pid: u32) -> Option<u64> { unsafe {
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let (mut c, mut x, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let r = GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u);
    let _ = CloseHandle(h);
    r.ok()?;
    Some(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
}}

/// Walk up while `same_app(parent, child)` holds and the parent is genuinely older than the child.
pub fn resolve_root(pid: u32, by_pid: &HashMap<u32, Proc>, same_app: impl Fn(&Proc, &Proc) -> bool) -> u32 {
    let mut cur = pid;
    for _ in 0..32 {
        let Some(child) = by_pid.get(&cur) else { break };
        let Some(parent) = by_pid.get(&child.ppid) else { break };        // parent already exited
        if parent.pid <= 4 { break; }                                      // Idle / System
        match (creation_time(parent.pid), creation_time(child.pid)) {
            (Some(p), Some(c)) if p <= c => {}
            _ => break,                                                    // PID reused, or no access
        }
        if !same_app(parent, child) { break; }
        cur = parent.pid;
    }
    cur
}
```

### 4.2 The game's root

1. **Start PID:**
   - `GetForegroundWindow()` → `GetWindowThreadProcessId(hwnd, Some(&mut pid))` (feature `Win32_UI_WindowsAndMessaging`), or the WGC capture target's window.
   - If the owner is `applicationframehost.exe` (a UWP frame), find the child window that belongs to a different process [Uncertain; GDK/Game Pass games are Win32 and own their windows].
2. **`same_app` for games:** the parent's image lives in the **same install directory tree** as the child's. Compare `QueryFullProcessImageNameW` paths by common directory prefix. This handles `VALORANT.exe` → `VALORANT-Win64-Shipping.exe`, where the image names differ.
3. **Stop list:** `explorer.exe`, `steam.exe`, `steamwebhelper.exe`, `epicgameslauncher.exe`, `battle.net.exe`, `agent.exe`, `eadesktop.exe`, `galaxyclient.exe`, `upc.exe`, `ubisoftconnect.exe`, `riotclientservices.exe`, `cmd.exe`, `powershell.exe`, `pwsh.exe`, `conhost.exe`, `svchost.exe`, `services.exe`.
   - Steam launches games as its children. Include-tree on `steam.exe` would add Steam's own sounds and friends' chat [Inference, report 02].
4. **Capture:** open **one include-tree client on the root**. Children the game spawns later (CEF overlays, helper processes) are covered by "and its child processes" [Doc].
   - Whether grandchildren stay included after an intermediate parent exits is **[Unverified]**. Test it with a tone-spawning chain (§8.2).
5. **Extra processes:** user-added processes (pinned exes, or sidecars such as Valorant's voice in `RiotClientServices.exe` [Uncertain]) each need **their own include client**, summed into the game stem. Each extra client multiplies the Win10 noise exposure (§6).
6. **In-game voice chat that the game renders itself** (CS2, most EOS/Vivox titles [Uncertain]) lands in the **game** stem. Tell the user in the voice-chat consent text.

### 4.3 Discord

**Root:** the main `Discord.exe` (`DiscordPTB.exe`, `DiscordCanary.exe`) owns the window, since Electron's browser process creates the top-level HWNDs. Its parent is usually the already-exited Squirrel `Update.exe`. Walk up while the image name is unchanged, which stops at the first non-Discord parent or at a dead or reused PID.

**Does include-tree on the root capture voice? Yes, in practice [Uncertain, but strong]:**
- OBS's Application Audio Capture resolves the PID with `GetWindowThreadProcessId` on the chosen window and always activates `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` [Src].
- Guides for capturing Discord voice separately use exactly that ("Choose your Discord app next to **Window**", 2025-01-14) [Uncertain].

**Which child plays voice:** **[Unverified].**
- Users report two Discord volume-mixer entries (voice versus notifications), which suggests two rendering processes.
- No source says whether the `discord_voice` module renders from the browser, renderer or a utility process.
- It doesn't matter for include-tree on the root.
- It matters only if you want "voice without notification pings". The test is to run the `wasapi` example `processes` during a call (`cargo run --example processes`). It prints every active render session's PID and peak level [Src]. Map the PIDs to `--type=` flags with Process Explorer.

**Discord in a browser** is the whole browser tree, so every tab gets mixed into the voice stem. Say so in the UI.

### 4.4 Kairo's own tree (for "system audio" = exclude-tree)

- Exclude-tree takes **one** PID. To exclude both the engine and the UI (WebView2 previews play audio), the **UI must be a child of the engine**.
- If the user launches the UI first, it should ask the engine over the pipe to (re)spawn the UI as its child, then exit.
- WebView2's browser and renderer processes are children of the UI host [Uncertain; verify with the snapshot code].

---

## 5. One 48 kHz timeline

### 5.1 Units

- **Engine epoch:** `E` = `qpc_100ns()` at engine start. Video (WGC `SystemRelativeTime`, pacer ticks) and audio (`BufferInfo.timestamp`) are both 100 ns QPC [Doc].
- **Timeline sample index:** `s = (t_100ns − E) · 48 000 / 10 000 000 = (t − E) · 3 / 625`. Compute in `i128` and floor.
- **AAC:** frame = 1,024 samples = 21.333 ms. FFmpeg's `aac` sets `frame_size = 1024` and `initial_padding = 1024` [Src]. The first packet comes out with `pts = first_input_pts − 1024` [Exp: ffprobe shows `pts=-1024`].

### 5.2 Data flow and threads

```
capture thread / source (MTA, MMCSS "Audio")      rtrb SPSC (f32 + PacketMeta)
  WASAPI event/20 ms poll → read_from_device ───────────────►  mixer thread (normal priority, every 10 ms of QPC)
                                                                for each source: SourceAligner (QPC → timeline, drift, gaps)
                                                                   read_block(pos = now − D, 480 frames)
                                                                mix (gains) → limiter → meters
                                                                per track: 1024-frame FIFO → AAC encoder (pts = timeline)
                                                                   → ReplayRing (packets, same 1024-grid on all tracks)
watchdog thread (250 ms): liveness, stalls, session meters, A/B refresh, reconnect backoff
device-monitor thread (MTA): DeviceEventCallbacks → crossbeam → watchdog
```

- **`D` (jitter-buffer delay) = 200 ms [Estimate].** Latency is irrelevant for a replay buffer, so a generous `D` lets late packets from every source arrive before a block is mixed.
- A save whose end is "now" waits until the mixer has passed `end + 1024` samples, which is ≤ ~250 ms.

### 5.3 Drift compensation: options compared

| | **rubato 5.0.1 `Async` + own PI loop** (recommended) | **libswresample `async`** via `ffmpeg-next` | **rubato `Slip`** |
|---|---|---|---|
| How | Estimate each source's rate and phase against QPC (PI loop on the timestamp error). Adjust the ratio with `set_resample_ratio_relative(r, ramp=true)`. | Feed per-packet input PTS (in units of `1/(in_rate·out_rate)`) through `swr_next_pts`. swr calls `swr_set_compensation`, or injects silence/drops samples above `min_hard_comp` [Src]. | Pass-through; insert or drop one frame under a 128-frame crossfade when the error reaches 1 frame [Src]. |
| Binding cost | Safe Rust | `swr_next_pts` and `swr_set_compensation` are **not wrapped** by `ffmpeg-next` 9.0.0 or `rsmpeg` 0.18. Use `ffmpeg_next::ffi` plus `Context::as_mut_ptr()` [Src]. | Safe Rust |
| Jitter handling | Yours: innovation clip plus low-pass (§5.4) | With `async`, swr sets `min_comp = 0.001` s (1 ms), so ±1–2 ms of timestamp jitter triggers compensation every packet [Src]. Raise `min_comp` and corrections come in bursts of up to `max_soft_comp`. | Yours (same loop) |
| Quality at ratio ≈ 1 | Septic polynomial: no aliasing at ratio≈1; mild HF droop. `new_sinc` if needed. | Default swr filter (high quality) | No filtering at all; a brief comb-like blend at each slip. 4.8 slips/s at 100 ppm. Rubato's README says "a few ppm", but its type docs demo 100 ppm with a ceiling of ~0.8%. |
| Tests on the Mac | `cargo test`, no native libraries | Needs FFmpeg headers and libs plus bindgen | `cargo test` |
| Verdict | **Use** | Fallback | Try it in the listening test; same `Adjustable` trait, so it swaps in one line |

Chromium solves the same problem in `media/base/audio_shifter.{h,cc}` (BSD). It takes noisy push timestamps and pull at the output clock, uses a `ClockSmoother`, and adjusts the ratio over an "adjustment time" of "at least a few seconds", clamped to [0.9, 1.1]. It is a good reference [Src]. The classic theory is Fons Adriaensen's *Using a DLL to filter time* (LAC 2005).

### 5.4 `SourceAligner` (per source), with rubato 5.0.1

**Loop constants**, from my simulation (`pll_sim2.py`: 100 ppm source, ±1.5 ms uniform timestamp jitter, 1% outliers of +8 ms) [Estimate/Exp]:
- packet = 480 frames (10 ms);
- `ALPHA = 0.02` per packet, so a ~0.5 s low-pass;
- τ = 15 s, `KP = 2/(τ·48000)`, `KI = KP/τ`;
- innovation clip ±96 samples (2 ms);
- clamp ±1000 ppm.

**Results:**

| τ | Ratio wobble (std) | p99 placement error | Settle after a 10 ms initial error |
|---|---|---|---|
| 10 s | 18 ppm | 0.10 ms | 34 s |
| 20 s | 9 ppm | 0.08 ms | 61 s |

Hard-resync on start removes the initial error.

```rust
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use std::collections::VecDeque;

const CHUNK: usize = 480;            // 10 ms input chunk
const HARD: f64 = 2400.0;            // >50 ms error => re-anchor (silence or trim)
const CLIP: f64 = 96.0;              // innovation clip, 2 ms
const ALPHA: f64 = 0.02;
const TAU: f64 = 15.0;
const KP: f64 = 2.0 / (TAU * 48_000.0);
const KI: f64 = KP / TAU;
const LIM: f64 = 1.001;              // ±1000 ppm (constructor max_resample_ratio_relative = 1.01)

pub struct SourceAligner {
    ch: usize, rs: Async<f32>, in_acc: Vec<f32>, scratch: Vec<f32>,
    out: VecDeque<f32>, out_pos: i64,          // timeline index of out.front()
    pred: f64, err_lp: f64, integ: f64, ratio: f64, locked: bool,
}

impl SourceAligner {
    pub fn new(ch: usize) -> Self {
        let rs = Async::<f32>::new_poly(1.0, 1.01, PolynomialDegree::Septic, CHUNK, ch, FixedAsync::Input).expect("rubato");
        let scratch = vec![0.0; rs.output_frames_max() * ch];
        Self { ch, rs, in_acc: Vec::with_capacity(CHUNK * ch * 8), scratch, out: VecDeque::with_capacity(48_000 * ch),
               out_pos: 0, pred: 0.0, err_lp: 0.0, integ: 0.0, ratio: 1.0, locked: false }
    }

    /// `ts` = timeline index of the packet's first frame (from BufferInfo.timestamp).
    pub fn push(&mut self, ts: i64, data: &[f32], disc: bool, ts_err: bool) {
        let frames = data.len() / self.ch;
        let e = ts as f64 - self.pred;                      // >0: input belongs later => stretch (ratio up)
        if !self.locked || disc || e.abs() > HARD {
            self.resync(ts);
        } else if !ts_err {
            let innov = (e - self.err_lp).clamp(-CLIP, CLIP);
            self.err_lp += ALPHA * innov;
            self.integ += self.err_lp * frames as f64 / 48_000.0;
            self.ratio = (1.0 + KP * self.err_lp + KI * self.integ).clamp(1.0 / LIM, LIM);
            let _ = self.rs.set_resample_ratio_relative(self.ratio, true);   // ramps over the next chunk
        }
        self.pred += frames as f64 * self.ratio;
        self.in_acc.extend_from_slice(data);
        while self.in_acc.len() >= CHUNK * self.ch {
            let n_in = self.rs.input_frames_next();                           // == CHUNK for FixedAsync::Input
            let input = InterleavedSlice::new(&self.in_acc[..n_in * self.ch], self.ch, n_in).unwrap();
            let cap = self.scratch.len() / self.ch;
            let mut output = InterleavedSlice::new_mut(&mut self.scratch, self.ch, cap).unwrap();
            let (_, written) = self.rs.process_into_buffer(&input, &mut output, None).unwrap();
            self.out.extend(&self.scratch[..written * self.ch]);
            self.in_acc.drain(..n_in * self.ch);
        }
    }

    fn resync(&mut self, ts: i64) {
        self.rs.reset();
        self.in_acc.clear();
        let start = ts - self.rs.output_delay() as i64;        // so that input frame 0 lands exactly at `ts`
        let end = self.out_pos + (self.out.len() / self.ch) as i64;
        if self.out.is_empty() { self.out_pos = start; }
        else if start > end { self.out.extend(std::iter::repeat(0.0).take((start - end) as usize * self.ch)); } // gap
        else { self.out.truncate(((start - self.out_pos).max(0) as usize) * self.ch); }                       // overlap
        // Real code: 2-5 ms fade-out before / fade-in after the splice to avoid clicks.
        self.pred = ts as f64; self.err_lp = 0.0;                // keep `integ` (the rate estimate) across re-anchors
        self.locked = true;
    }

    /// Mixer: fill `dst` with frames [pos, pos+n); zero what is missing; drop what arrived too late.
    pub fn read_block(&mut self, pos: i64, n: usize, dst: &mut [f32]) {
        let ch = self.ch;
        dst[..n * ch].fill(0.0);
        if self.out_pos < pos {
            let drop = (((pos - self.out_pos) as usize) * ch).min(self.out.len());
            self.out.drain(..drop);
            self.out_pos += (drop / ch) as i64;
            if self.out.is_empty() { self.out_pos = self.out_pos.max(pos + n as i64); return; }
        }
        let lead = ((self.out_pos - pos).max(0) as usize).min(n);
        let take = (self.out.len() / ch).min(n - lead);
        for (i, s) in self.out.drain(..take * ch).enumerate() { dst[lead * ch + i] = s; }
        self.out_pos += take as i64;
    }
}
```

**Per-source timestamp policy:**
- Process and endpoint loopback: device QPC (OBS uses the OS timestamp for process capture, PR #9672) [Src].
- Mics: OBS defaults to **arrival time** ("use device timestamps" off) [Src]. Use the device QPC, but compare it with `arrival_100ns − packet_duration`. If they disagree by more than 50 ms persistently, or the timestamps go backwards, switch that source to arrival time and record it in the capability report [Estimate thresholds].

### 5.5 Fallback: libswresample `async` with `ffmpeg-next` 9.0.0 [Src]

```rust
use ffmpeg_next::{ffi, format::{Sample, sample::Type}, software::resampling::Context, ChannelLayout, Dictionary};
let mut o = Dictionary::new();
o.set("async", "100");          // >1 = max stretch in samples/s (100/48000 ≈ 0.2 %); also sets min_comp = 0.001
o.set("min_comp", "0.005");     // ignore errors below 5 ms (jitter)
o.set("min_hard_comp", "0.050");// above 50 ms: swr_inject_silence / swr_drop_output
o.set("first_pts", "0");        // in samples
let mut swr = Context::get_with(Sample::F32(Type::Packed), ChannelLayout::STEREO, 48_000,
                                Sample::F32(Type::Planar), ChannelLayout::STEREO, 48_000, o)?; // opts applied before swr_init
// per packet (pts unit = 1/(in_rate*out_rate) s):
let out_pts = unsafe { ffi::swr_next_pts(swr.as_mut_ptr(), ts_samples * 48_000) } / 48_000;
swr.run(&in_frame, &mut out_frame)?;   // swr_convert_frame; it does NOT handle pts itself
```

`swr_next_pts` semantics [Src, `swresample.c`]:
- When `|Δ| > min_comp`: if it is the first PTS or `|Δ| > min_hard_comp`, it fills or trims (hard).
- Otherwise it calls `swr_set_compensation(comp, out_rate·comp_duration)`, with `comp` clipped to `max_soft_comp`.
- `comp_duration` defaults to 1 s.
- When compensation is active at 48→48 kHz, swr forces its resampler on.

### 5.6 Gaps, flags and silence

| Event | Handling |
|---|---|
| No packets (loopback silence, stall, overrun) | The aligner's `out` runs dry. `read_block` zero-fills, which inserts silence. When packets resume, `e > HARD` re-anchors with a silence fill. |
| `DATA_DISCONTINUITY` (not first packet) | `resync(ts)`. Increment the `glitches` counter. |
| `SILENT` | Zeros, already handled in the capture thread. The timestamp is still used. |
| `TIMESTAMP_ERROR` | Skip the PI update, place by prediction, count it. Escalate (log once, plus capability report) if more than 10% of packets over 10 s carry it [Estimate]. |
| Overlap (timestamps go backwards by more than 50 ms) | `resync` truncates the overlapping tail. Fade the splice. |
| Source re-created (A/B swap, reconnect) | New aligner; it anchors on its first packet; crossfade (§6.2). |

### 5.7 Mixer: gains, limiter, meters

**Mixed track (T1) = Σ gᵢ·stemᵢ** over the enabled stems (game, mic, voice if enabled, other apps in screen mode). Don't add a system-loopback source on top of stems that already contain the same audio (double counting).

**Gains:**
- Per-source value in dB, set in the UI.
- Smooth each change with a linear ramp across one 10 ms block (no zipper noise).
- Defaults: game 0 dB, mic 0 dB, voice −3 dB [Estimate].

**Limiter** (mixed bus only; stereo-linked; latency = look-ahead L):
- Ceiling −1.0 dBFS (0.891), L = 240 frames (5 ms), release 100 ms [Estimate].
- Algorithm:
  1. `g_req[n] = min(1, ceil / max|x[n]|)`.
  2. Sliding-window **minimum** of `g_req` over the next L frames (monotonic deque, O(1)).
  3. Release: if the held minimum rises, approach it with a one-pole (time constant 100 ms); if it falls, follow it immediately.
  4. **Box filter** (moving average, length L) over that envelope. The gain then reaches the minimum exactly as the delayed peak arrives, with no overshoot.
  5. `y[n] = x[n−L] · g[n]`.
- Tag the mixed track's blocks with `pos − L` so it stays aligned with the stems.
- Stems: hard clip at ±1.0 only. WASAPI float can exceed 1.0, and the AAC encoders expect [−1, 1].

**Meters** (per stem post-gain, and mix post-limiter):
- Per 10 ms block: `peak = max|x|`, `sumsq += x²`.
- Publish every 50 ms (20 Hz): `peak_dbfs = 20·log10(max(peak, 1e-9))`, `rms_dbfs = 10·log10(max(sumsq/n, 1e-18))`, plus the limiter's gain reduction.
- Store them in atomics. The IPC thread emits them only while a UI is subscribed, for example:

```json
{"jsonrpc":"2.0","method":"audio.levels","params":{"t_ms":123456,
 "sources":[{"id":"game","peak_dbfs":-9.8,"rms_dbfs":-21.4,"state":"ok"},
            {"id":"mic","peak_dbfs":-31.0,"rms_dbfs":-44.2,"state":"ok"}],
 "mix":{"peak_dbfs":-1.0,"rms_dbfs":-17.9,"gr_db":-2.3}}}
```

- `state`: `ok`, `silent` (below −60 dBFS for more than 10 s while the Windows session meter shows activity, §6.1), `stalled`, `blocked` (privacy or exclusive mode), `reconnecting`.

### 5.8 AAC framing and aligning to the video keyframe on save

**Framing:**
- Start every track's encoder at the same grid origin `G` (a timeline index that is a multiple of 1024 after start).
- Feed it gapless 1,024-frame planar f32 frames with `pts = timeline index`, time base 1/48000. Gaps are already silence.
- Packet `k` then has `pts = G + 1024·(k−1)` on **all** tracks.
- Packets go into the ring with these PTS. A 160 kbps stereo track is about 20 kB/s, so 120 s × 4 tracks ≈ 9.6 MB.

**Why the start needs care [Exp, FFmpeg 8.0.1]:**
- Decoding an AAC stream (native encoder, 160 kbps) from packet N gives a **wrong first output frame** (RMS 0.074 vs 0.088; max diff 0.121 on a 0.125-peak sine).
- Frames N+1 onwards are **bit-identical** to a full decode.
- Cause: the MDCT overlap with the missing packet N−1.

**Save recipe for a video keyframe at timeline `s0`:**
1. `p` = the packet whose output covers `s0` (`pts_p ≤ s0 < pts_p + 1024`). Also include **`p−1` as pre-roll**.
2. Write audio packets with `pts' = pts − s0`. The first is in [−2047, −1024].
3. Video starts at 0.
4. FFmpeg's MP4 muxer has `AVFMT_TS_NEGATIVE`, and `use_editlist` is auto → 1 for non-fragmented MP4. It writes an `elst` whose `media_time` trims the negative start [Src, `movenc.c`]. The decoded audio then starts **exactly at the keyframe**, artefact-free.
5. Check it on the PC with `ffprobe -show_entries packet=pts` plus the box inspector from report 06: `prototypes/scripts/mp4boxes.py`.

**When edit lists are not allowed** (Instagram/YouTube-style uploads), use one of:
- the "Precise" re-encode export, which encodes fresh PCM from the clip's start (FFmpeg's normal priming plus elst, or `-use_editlist 0`);
- the brief's approach: start at the first packet with `pts ≥ s0`, `use_editlist 0`. That accepts ≤21.3 ms early audio plus the first-frame artefact.

---

## 6. Watchdog and the Windows 10 long-session noise bug

### 6.1 Signals and actions (process loopback and endpoint sources)

| Signal | Detection | Action |
|---|---|---|
| Capture error | The capture thread returns `Err(WasapiError::Windows(e))`. Classify by `e.code()`. | `DEVICE_INVALIDATED` / `RESOURCES_INVALIDATED` / `SERVICE_NOT_RUNNING`: restart, backoff 0.25 s → 1 s → 3 s → 10 s cap (OBS: fixed 3 s [Src]). `DEVICE_IN_USE`: state "blocked (exclusive mode)", retry every 30 s. `E_ACCESSDENIED`: "blocked by privacy settings", retry when settings change or every 30 s. |
| Activation hang | `ready` not received within 3 s. `new_application_loopback_client` waits without a timeout [Src]. | Mark failed. Abandon the thread (it is parked forever; log it) and retry with a new thread after backoff. |
| Target exited | The pinned handle is signalled (`WaitForSingleObject(h, 0) == WAIT_OBJECT_0`), or the PID is missing from the snapshot | Stop the source. State "waiting for game". Game detection re-resolves. |
| Stall | No packets for more than 2 s **and** the Windows session meter shows the target tree active (any session whose PID is in the tree with peak > −50 dBFS for at least 1 s) | Restart through the A/B swap. Session meters: `Device::get_iaudiosessionmanager()?.get_audiosessionenumerator()?`, `get_session(i)?.get_process_id()`, `get_state() == SessionState::Active`, `get_audiometerinformation()?.get_peak_value()` [Src]. Poll every 1 s on all active render endpoints. |
| Long silence | Captured stem below −90 dBFS (all-zero) for more than 5 s while the session meter is above −50 dBFS | Same as stall (broken capture). |
| Packets but none expected | – | Unknown: whether process loopback delivers continuous `SILENT` packets while the target is idle is **[Unverified]**. Log the inter-packet gap distribution in the soak and set the thresholds from it. |
| Proactive refresh due | Timer, per the OS policy in §6.3 | A/B swap (§6.2), preferably while the stem has been below −50 dBFS for 500 ms. |
| `TIMESTAMP_ERROR` | Flag | **No restart** (OBS `0f498133a`). Count it only. |

### 6.2 Seamless A/B swap

1. Start client **B** with the same spec while **A** keeps running. Its capture thread gets a fresh `SourceAligner`.
2. Wait until B is locked and has more than 200 ms of data. Give up after 3 s: kill B, keep A, retry later.
3. Pick a swap position `P = mixer_pos + 50 ms`. For blocks in [P, P + 20 ms] the mixer reads both and applies a linear crossfade A→B. Both are QPC-aligned, and the residual misalignment is ~0.1 ms [Estimate from §5.4], so no comb effect is audible over 20 ms.
4. Signal A's shutdown event, then drop it.

This removes the audible gap of OBS users' "restart the source every 10 min" script (#8064 comment, 2023-12-28) [Uncertain].

### 6.3 OS policy (from the #8064 history [Src: issue thread])

**What the #8064 thread says:**
- 2024-03-27, OBS maintainer: "Microsoft has implemented a potential fix … targeted for Windows 11 24H2".
- Later: "It is only fixed in Windows 11 24H2 and newer"; "Fixes will only be made to Windows 11."
- Closed 2025-10-25 after users confirmed the fix on 25H2 26200.6899.
- Some users still saw it on 24H2 26100.2605 [Uncertain].
- One reporter says the buzzing "builds up" faster with loud audio (minutes with loud music, ~2.5 h when quiet) [Uncertain].
- Discord's own app-audio streaming shows the same symptom, which points to the OS.

| Windows build | Game / voice stems | "System / other apps" source | Proactive A/B refresh (default) |
|---|---|---|---|
| ≥ 26100 (Win11 24H2/25H2/26H2) | Process loopback, include-tree | Process loopback, exclude-tree on the engine PID | 60 min (turn it off if the 8 h soak is clean) |
| 22000–22631 (Win11 21H2–23H2) | Process loopback | Exclude-tree or endpoint loopback (decide in the soak) | 10 min |
| 19041–19045 (Win10 2004–22H2) | Process loopback, labelled "best effort" | **Endpoint loopback**: #8064 concerns Application Audio Capture, and testers used device capture as the clean reference [Uncertain] | 10 min |
| < 19041 | None: endpoint loopback only, no per-app stems | Endpoint loopback | – |

**Also:**
- Follow the SDK exactly (`EventsShared { autoconvert: true, buffer_duration_hns: 0 }`). OBS's tests of exactly this were "preliminary positive", then inconclusive (PR #9807, closed unmerged).
- Optional soak experiment: request the **device's own rate** (read `Device::get_device_format()` for the default render endpoint) instead of 48 kHz, and let rubato convert. #8064 test logs show crackle with mismatched OS/OBS/content rates. Whether matched rates avoid it is unknown [Uncertain].

### 6.4 Endpoint and mic reconnect

- Follow the default device (debounced `DevEvent::Default` for the role you track), `Removed`/`State` for the device in use, and session `disconnected` callbacks.
- To reconnect: stop the thread, close the source, then open a new one on the new device ID. It gets a new aligner, which re-anchors by QPC, so there is a short silence and no drift.
- Automatic stream routing (activating through `StringFromIID(DEVINTERFACE_AUDIO_CAPTURE)` and `AudioClient::from_iaudioclient`, Win10 1607+) is documented for normal render/capture streams, not loopback [Doc]. It is an option for the mic later, but explicit re-open is simpler and testable.

---

## 7. Gotchas (checklist for code review)

1. `include_tree=false` **excludes**. Use the `LoopbackMode` enum and never pass a bare `bool` (§2.1).
2. Don't trust the crate doc example that passes `include_tree = false` with its own PID. Don't copy its `record_application` example's PID choice either. It loops over every `firefox.exe` and keeps `process.parent().unwrap_or(pid)` of the *last* one. If that is the main process, the target becomes **explorer.exe's tree** [Src]. Use `resolve_root` (§4.1).
3. Pass `buffer_duration_hns: 0` for event-driven shared mode [Doc]; the crate's `record` example passes `min_time`.
4. `read_from_device` loses the packet if your slice is too small. Size it at 1 s up front.
5. Never use `read_from_device_to_deque`: it calls `unwrap()` on `ReleaseBuffer` and allocates.
6. Zero the buffer on `SILENT`.
7. Ignore `DATA_DISCONTINUITY` on the first packet after `Start` (`device_position == 0`).
8. Don't restart on `TIMESTAMP_ERROR`.
9. All `wasapi` / `windows` COM objects are `!Send`. Keep them on one thread; `AudioClient` can't move into an async task or a pool.
10. Drop order: COM objects first, then `wasapi::deinitialize()` (CoUninitialize) last. Use a guard declared first, or call it after the function that owns the objects has returned.
11. `new_application_loopback_client` can block forever. Use a watchdog plus a disposable thread.
12. `IMMNotificationClient` callbacks: `try_send` only. Never unregister (drop the registration) from a thread a callback might be waiting on. Expect three callbacks per change.
13. `SetDuckingPreference` ≠ "my mic won't duck others". Use `IAudioClientDuckingControl` and the console role, and test.
14. Bluetooth headset mic: the always-on mic forces HFP for the whole session. Don't auto-enable it.
15. One process-loopback client = one tree. Sidecar processes need extra clients, and every client is exposed to the noise bug.
16. Parent PIDs get reused. Check creation times and pin with a handle.
17. The UI must be a child of the engine, or exclude-tree won't remove Kairo's UI sounds.
18. Process loopback captures *before* the user hears it (engine time). The mic captures when the user speaks. Both are on QPC, so sync is right; Bluetooth output latency is real and can't be fixed by Kairo (offer a user offset).
19. With speakers instead of a headset, the mic re-captures game audio (comb filtering in the mix). AEC (`StreamCategory::Communications` + `get_aec_control()`) is a later option and triggers communications behaviour.
20. Exclusive-mode apps: endpoint loopback fails (`DEVICE_IN_USE`). Process loopback of an exclusive stream is presumably silent [Uncertain].
21. Protected (DRM) audio is blocked from loopback [Doc, report 02]. Expect silence from some players and streaming apps.
22. `wasapi` logs per packet at trace level. Filter `wasapi=warn`.
23. Version drift: `wasapi` 0.27 will likely change the raw-COM API (PR #74). Pin `=0.26.0`, and pin `windows = "0.62.2"` with the same features.
24. `windows` feature gates: `CreateEventW` needs `Win32_Security` as well as `Win32_System_Threading`. `ActivateAudioInterfaceAsync` is behind `Win32_System_Com_StructuredStorage` + `Win32_System_Variant` (the crate enables these itself) [Src].
25. FFmpeg AAC: `initial_padding = 1024`. Packet PTS are already shifted by −1024, so don't subtract it again.
26. Windows mic privacy ("Let desktop apps access your microphone"): failure mode **[Unverified]** (error or silence). The meter state and capability report must show it.

---

## 8. Testing on the Windows PC, and the soak-test design

### 8.1 Run audio tests in the interactive session

Audio endpoints and sessions belong to the logged-in user's session. Run every capture binary the same way as the GUI and capture processes: from SSH via a `/IT` scheduled task [Uncertain that SSH-session WASAPI fails; don't rely on it].

```powershell
$exe = "C:\src\kairo\target\release\examples\audio_soak.exe"
schtasks /Create /TN KairoAudioSoak /TR "`"$exe`" --hours 8 --out C:\kairo-soak" /SC ONCE /ST 00:00 /IT /F
schtasks /Run /TN KairoAudioSoak
Get-Content C:\kairo-soak\events.jsonl -Wait -Tail 20     # from the SSH session
```

### 8.2 Per-piece tests (Windows PC)

1. **Crate smoke test:**
   - `cargo run --example devices`
   - `--example device_notifications` (switch outputs and inputs while it runs)
   - `--example processes` during a Discord call: which PIDs render, with peaks [Src: examples exist in 0.26.0]
2. **Include/exclude semantics:** start a tone process (a tiny `wasapi` render example). Include-tree on it: tone present. Exclude-tree on it: tone absent while other audio is present. Exclude-tree on the engine: Kairo's cue absent.
3. **Tree coverage:** chain A→B→C. C plays a tone. Kill B. Does include-tree(A) still capture C? Also test Steam (parent `steam.exe`), Epic, Battle.net, Riot (VALORANT) and Discord (parent `Update.exe`). Log the resolved root and image paths.
4. **Activation reality check:** after activation, play a −30 dBFS tone in Kairo's own process for 300 ms and require it in an include-tree(self) capture. This is the "per-app audio works" capability bit. It also catches outdated Win10 builds that activate but stay silent.
5. **Ducking matrix:** Communications tab at its default (80%). Music playing in another app; measure its level through endpoint loopback RMS. Kairo mic opened on:
   - (a) console role, plain;
   - (b) console + `SetDuckingPreference`;
   - (c) console + `DO_NOT_DUCK_OTHER_STREAMS`;
   - (d) communications role.

   Keep the cheapest variant with no ducking.
6. **Default-device switch** while capturing (AudioDeviceCmdlets `Set-AudioDevice -ID …` in a loop, or by hand):
   - process loopback must have no gap over 50 ms [Unverified expectation];
   - endpoint loopback and mic must re-open within 1 s.
7. **Exclusive mode:** `cargo run --example playnoise_exclusive` while capturing. Expect the `DEVICE_IN_USE` diagnostic for endpoint loopback, and record what process loopback does.
8. **Mic loss and privacy:** unplug and replug a USB headset; toggle "Let desktop apps access your microphone"; restart Audiosrv (`Restart-Service Audiosrv`, admin). Every case must recover or show a clear state.
9. **Bluetooth:** with a BT headset, confirm the HFP switch when the mic is opened, and that detection keeps the mic off.
10. **A/V sync:** a test pattern window that flashes white and beeps (same QPC instant) every second. Save 30 s clips and measure the flash-to-beep offset with ffmpeg (`blackframe`/`signalstats` for video, `silencedetect`/`astats` for audio). Target: |offset| < 10 ms, constant over an 8 h session.

### 8.3 Soak harness (`audio_soak` example, 4 h / 8 h / 24 h per the plan)

**Signal source:** a child process `soak-tone.exe` (wasapi render, console role) plays a continuous 1 kHz sine at −20 dBFS. Every 10 s it adds a 50 ms marker chirp. It logs each marker's QPC render time, from `IAudioClock::GetPosition` QPC plus the position-to-frame mapping, to `markers.jsonl`.

**Captured simultaneously** (each through the production `SourceAligner`):
- (1) include-tree(tone);
- (2) exclude-tree(engine);
- (3) endpoint loopback;
- (4) the mic (optional; drift only).

**Analysis, online, every 1 s per source** (written to `metrics.jsonl`):
- **Noise (#8064 detector):** fit and subtract the known 1 kHz sine (phase-tracked) and compute THD+N. Alarm above −50 dB, or a jump of 20 dB over the run's first-hour median [Estimate thresholds].
- **Timing:** detect each marker by cross-correlation and compare its timeline position with the logged QPC. Track the per-source offset over hours (pass: drift < ±2 ms after the first minute; jitter p99 < 1 ms).
- **Health:** counters for gaps, discontinuities, `TIMESTAMP_ERROR`, overruns, restarts and A/B swaps; inter-packet gap histogram (this answers "does process loopback send packets during silence?"); ratio estimate in ppm.
- **Process:** CPU% of the engine, private bytes, handle count, thread count, sampled every minute. Pass: flat after warm-up.

**Disruption schedule** (scripted, so a run is reproducible):
- t+30 min: kill and relaunch the tone process;
- every 15 min: switch the default output;
- every 60 min: switch the default input;
- t+2 h: `playnoise_exclusive` for 60 s;
- t+3 h: restart Audiosrv;
- manual: sleep/resume, USB unplug, a Discord call, a GPU-heavy game.

**Matrix:**
- OS: Win11 25H2/26H2 on the user's PC. Win10 22H2 if a machine is available (a VM is acceptable for *our* noise detection, though Microsoft did not accept VM traces for #8064).
- Refresh: off / 10 min / 60 min.
- Requested rate: 48 kHz fixed versus the device rate.

**Pass gate for M2/M3:** zero unrecovered failures; zero noise alarms on Win11; recovery under 3 s after every disruption; A/V offset stable within ±10 ms over 8 h.

### 8.4 Mac-side unit tests (pure Rust, no Windows)

- `SourceAligner`: port `pll_sim2.py`:
  - drift ±200 ppm with jitter ±3 ms: converges, p99 error < 0.5 ms, no `resync` after lock;
  - a 300 ms gap: exactly that much silence, then re-lock;
  - an overlap: trimmed;
  - a `TIMESTAMP_ERROR` storm: no drift.
- Limiter: output never exceeds the ceiling on random and impulsive input; gain has no steps above X dB per sample.
- Meters: dBFS of known signals.
- AAC save selection: `p−1`/`p` packet math across grid offsets 0..1023, and negative-PTS writing.
- Process-tree resolver: on a synthetic process table (PID reuse, dead parents, launcher stop list), with `creation_time` mocked behind a trait.

---

## 9. Recommended dependencies (engine audio module)

```toml
[dependencies]
rubato = "=5.0.1"              # Async (poly/sinc) + Slip; re-exports audioadapter 5 / audioadapter-buffers 5.x
rtrb = "0.3.5"                 # SPSC capture -> aligner
crossbeam-channel = "0.5.17"
bytemuck = "1.25.2"
ffmpeg-next = "9.0.0"          # AAC encode + MP4 mux (encode report); ffi = ffmpeg-sys-next 9.0.1

[target.'cfg(windows)'.dependencies]
wasapi = "=0.26.0"             # MIT, MSRV 1.85; windows ^0.62.2
windows = { version = "0.62.2", features = [
  "Win32_Foundation",
  "Win32_Media_Audio",                 # IAudioClientDuckingControl, AUDCLNT_E_* constants
  "Win32_Security",                    # needed by CreateEventW / CreateWaitableTimerExW
  "Win32_System_Com",
  "Win32_System_Threading",            # AvSetMm*, events, WaitFor*, OpenProcess, GetProcessTimes, QueryFullProcessImageNameW
  "Win32_System_Diagnostics_ToolHelp", # CreateToolhelp32Snapshot, PROCESSENTRY32W
  "Win32_System_Performance",          # QueryPerformanceCounter/Frequency
  "Win32_UI_WindowsAndMessaging",      # GetForegroundWindow, GetWindowThreadProcessId
] }
```

- Keep `SourceAligner`, the mixer, limiter, meters and save-selection math in a platform-independent crate (`kairo-core`), so `cargo test` runs on the Mac.
- Only `capture/*.rs`, `ducking.rs`, `proctree.rs` and `watchdog.rs` are `#[cfg(windows)]`. Compile-check those with `cargo xwin check --target x86_64-pc-windows-msvc`.

If the engine switches to C#/.NET (the fallback stack), the design carries over unchanged: NAudio 3.3's `WasapiRecorder` for process loopback, the same aligner/PI loop, and the same watchdog.

---

## Sources

All were accessed on 2026-10-21; dates in parentheses are publication or release dates.

**Crates (source tarballs from static.crates.io, read locally)**
- wasapi 0.26.0 (2026-10-11): `src/api.rs`, `src/events.rs`, `src/errors.rs`, `examples/{record_application,record,device_notifications,processes,aec}.rs`, `README.md`, `AGENTS.md`, `Cargo.toml`. https://crates.io/crates/wasapi · https://docs.rs/wasapi/0.26.0/wasapi/struct.AudioClient.html · https://github.com/HEnquist/wasapi-rs (commit 7afc1c7)
- wasapi 0.20.0, 0.21.0, 0.23.0, 0.24.0, 0.25.0 (version history of ducking, notifications and removed APIs).
- wasapi-rs issue #73 and PRs #74 and #75 (2026-10-11 / 2026-10-21). https://github.com/HEnquist/wasapi-rs/pull/74
- rubato 5.0.1 (2026-10-01): `README.md` (changelog, choosing a resampler), `src/lib.rs`, `src/asynchro.rs`, `src/slip.rs`. https://crates.io/crates/rubato · https://docs.rs/rubato/5.0.1/rubato/
- audioadapter 5.0.0 and audioadapter-buffers 5.2.0 (`src/direct.rs`: `InterleavedSlice`). https://crates.io/crates/audioadapter-buffers
- cpal 0.18.2 (2026-08-16): `src/host/wasapi/{stream,device,mod}.rs`. https://crates.io/crates/cpal
- ffmpeg-next 9.0.0 (2026-08-05): `src/software/resampling/context.rs`, `src/util/channel_layout.rs`, `src/codec/encoder/*`. https://docs.rs/ffmpeg-next/9.0.0/ffmpeg_next/software/resampling/context/struct.Context.html
- ffmpeg-sys-next 9.0.1 (2026-10-09) and rsmpeg 0.18.0+ffmpeg.8.0 (`src/swresample/swresample.rs`). https://crates.io/crates/ffmpeg-sys-next · https://crates.io/crates/rsmpeg
- windows 0.62.2 (2025-10-06): `Win32/Media/Audio`, `Win32/System/Threading`, `Win32/System/Diagnostics/ToolHelp`, `Win32/System/Performance`, `Win32/Foundation`, `Win32/UI/WindowsAndMessaging`. windows-core 0.62.2 and windows-implement 0.60.2 (agile default). https://crates.io/crates/windows · https://crates.io/crates/windows-core
- rtrb 0.3.5, crossbeam-channel 0.5.17 and bytemuck 1.25.2 (crates.io API).

**FFmpeg (release/9.0 branch, raw.githubusercontent.com/FFmpeg/FFmpeg)**
- `libswresample/swresample.h` (`swr_next_pts`, `swr_set_compensation`), `swresample.c` (`swr_init` async handling, `swr_next_pts`), `options.c`, `swresample_frame.c`.
- `libavfilter/af_aresample.c` (PTS scaling).
- `libavcodec/aacenc.c` (`frame_size`, `initial_padding` = 1024).
- `libavformat/movenc.c` (`use_editlist`, `elst`, `AVFMT_TS_NEGATIVE`).
- FFmpeg Resampler documentation. https://ffmpeg.org/ffmpeg-resampler.html

**Microsoft documentation** (Markdown sources of learn.microsoft.com: github.com/MicrosoftDocs/sdk-api and github.com/MicrosoftDocs/win32)
- IAudioCaptureClient::GetBuffer. https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer
- _AUDCLNT_BUFFERFLAGS. https://learn.microsoft.com/en-us/windows/win32/api/audioclient/ne-audioclient-_audclnt_bufferflags
- IAudioClient::Initialize. https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize
- AUDCLNT_STREAMFLAGS_XXX constants. https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants
- PROCESS_LOOPBACK_MODE / AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS / AUDIOCLIENT_ACTIVATION_TYPE (min "Windows 10 Build 20348"). https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-process_loopback_mode
- ActivateAudioInterfaceAsync (IAgileObject requirement). https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-activateaudiointerfaceasync
- IMMNotificationClient (callback rules) and OnDefaultDeviceChanged (NULL ID). https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient
- IAudioSessionControl2::SetDuckingPreference. https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-setduckingpreference
- IAudioClientDuckingControl / SetDuckingOptionsForCurrentStream / AUDIO_DUCKING_OPTIONS. https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudioclientduckingcontrol
- Using a Communication Device; Default Ducking Experience; Disabling the Default Ducking Experience. https://learn.microsoft.com/en-us/windows/win32/coreaudio/using-the-communication-device · https://learn.microsoft.com/en-us/windows/win32/coreaudio/stream-attenuation · https://learn.microsoft.com/en-us/windows/win32/coreaudio/disabling-the-ducking-experience
- Loopback Recording (2025-04-16); Automatic Stream Routing. https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording · https://learn.microsoft.com/en-us/windows/win32/coreaudio/automatic-stream-routing
- AvSetMmThreadCharacteristicsW; Multimedia Class Scheduler Service (2025-07-14). https://learn.microsoft.com/en-us/windows/win32/api/avrt/nf-avrt-avsetmmthreadcharacteristicsw · https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service
- CreateToolhelp32Snapshot; PROCESSENTRY32W; GetProcessTimes; QueryFullProcessImageNameW. https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/ns-tlhelp32-processentry32w
- Application loopback sample: README and `LoopbackCapture.cpp` (fixed flags, `0, 0` durations, "Capture" MF queue). https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/ApplicationLoopback
- Raymond Chen, *The Old New Thing*, 2015-04-03 (parent PID reuse, creation-time check, holding a handle). https://devblogs.microsoft.com/oldnewthing/20150403-00/?p=44313

**OBS Studio**
- `plugins/win-wasapi/win-wasapi.cpp` (master): process-loopback init, 5 s buffer, `EVENTCALLBACK|LOOPBACK`, 10 ms loopback poll, MMCSS "Audio", 3 s reconnect, eCommunications mic default, timestamp policy, SILENT zero-fill. https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/win-wasapi.cpp
- `plugins/win-wasapi/plugin-main.cpp` (19041 gate).
- Issue #8064 (2023-01-14 to 2025-10-25), full comment history via the GitHub API. https://github.com/obsproject/obs-studio/issues/8064
- PR #9672 (merged 2023-10-08); commit 0f498133a "win-wasapi: Ignore timestamp errors" (2023-11-27); PR #9807 (closed unmerged). https://github.com/obsproject/obs-studio/pull/9672 · https://github.com/obsproject/obs-studio/commit/0f498133a

**Other**
- speecher PR #270 (2026-10-10): ducking test on build 26100 [Uncertain]. https://github.com/firemonster612/speecher/pull/270
- Chromium `media/base/audio_shifter.h` / `.cc`. https://chromium.googlesource.com/chromium/src/+/main/media/base/audio_shifter.h
- F. Adriaensen, *Using a DLL to filter time* (LAC 2005). https://kokkinizita.linuxaudio.org/papers/usingdll.pdf
- bozbez/win-capture-audio README (Win10 2004+; outdated builds capture nothing). https://github.com/bozbez/win-capture-audio
- OBS Discord-capture guide (2025-01-14). https://www.nablamind.com/record-screen/how-to-record-discord-audio-with-obs.html
- Discord: two volume-mixer entries (VoiceAttack forum, anecdotal). https://forum.voiceattack.com/smf/index.php?topic=2958.0
- UserDuckingPreference values (community): https://winhance.net/docs/features/optimizations/sound.html · MS-hosted blog: https://learn.microsoft.com/fr-fr/archive/blogs/toml/ducking-music-volume-experience-with-incoming-lync-call-using-zune-player
- Bluetooth HFP versus A2DP (Microsoft Q&A, community): https://learn.microsoft.com/en-us/answers/questions/3946429/the-sound-is-bad-while-using-the-microphone

**Local experiments (Mac, FFmpeg 8.0.1, Python 3 + NumPy 2.4.1)**
- AAC mid-stream decode test: an ADTS stream cut at packet 200; only the first decoded frame differs. Folder: `(local scratch file, not kept)`.
- Drift-loop simulation: `prototypes/scripts/pll_sim2.py` (and `pll_sim.py`).
- Earlier Kairo reports: `00-brief.md`, `02-audio-console-cloud.md`, `05-stack-workflow.md` and `06-post-clip-sharing.md` in ``.
