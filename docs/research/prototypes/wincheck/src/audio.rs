//! Capture thread skeleton: wasapi =0.26.0 + windows =0.62.2. Every COM object lives and dies on this thread.
use wasapi::{AudioClient, DeviceEnumerator, Direction, Role, SampleType, StreamMode, WaveFormat};
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{IAudioClientDuckingControl, AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS};
use windows::Win32::System::Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, WaitForMultipleObjects};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopbackMode { IncludeTree, ExcludeTree }
/// wasapi 0.26.0: include_tree=false means EXCLUDE_TARGET_PROCESS_TREE (its doc comment is wrong).
/// Blocks on a Condvar with no timeout: the watchdog abandons the thread after 3 s.
pub fn open_process_loopback(pid: u32, mode: LoopbackMode) -> Result<AudioClient, wasapi::WasapiError> {
    AudioClient::new_application_loopback_client(pid, matches!(mode, LoopbackMode::IncludeTree))
}

pub enum SourceSpec { Process { pid: u32, mode: LoopbackMode }, Endpoint { device_id: Option<String>, is_mic: bool } }
#[derive(Clone, Copy, Default)]
pub struct PacketMeta { pub ts_hns: u64, pub arrival_hns: i64, pub device_pos: u64, pub frames: u32, pub flags: u8 }
pub const F_DISC: u8 = 1; pub const F_SILENT: u8 = 2; pub const F_TSERR: u8 = 4;

struct Mmcss(HANDLE);
impl Drop for Mmcss { fn drop(&mut self) { unsafe { let _ = AvRevertMmThreadCharacteristics(self.0); } } }

fn mic_no_ducking(client: &AudioClient) {
    // Process-loopback clients have no session control (E_NOINTERFACE); this is for the mic only.
    if let Ok(sc) = client.get_audiosessioncontrol() { let _ = sc.set_ducking_preference(true); }
    unsafe {
        if let Ok(dc) = client.as_iaudioclient().GetService::<IAudioClientDuckingControl>() {
            let _ = dc.SetDuckingOptionsForCurrentStream(AUDIO_DUCKING_OPTIONS_DO_NOT_DUCK_OTHER_STREAMS);
        }
    }
}

pub fn capture_main(spec: SourceSpec, ch: usize, pcm: &mut rtrb::Producer<f32>, meta: &mut rtrb::Producer<PacketMeta>,
                    shutdown: HANDLE, ready: &crossbeam_channel::Sender<Result<(), String>>) -> anyhow::Result<()> {
    wasapi::initialize_mta().ok()?;   // S_FALSE is fine; call wasapi::deinitialize() after this fn returns
    let mut task = 0u32;
    let _mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task) }.ok().map(Mmcss); // never "Pro Audio"
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, 48_000, ch, None); // float32 48 kHz; mic: ch = 1
    let is_mic = matches!(spec, SourceSpec::Endpoint { is_mic: true, .. });
    let mut client = match &spec {
        SourceSpec::Process { pid, mode } => open_process_loopback(*pid, *mode)?,
        SourceSpec::Endpoint { device_id: Some(id), .. } => DeviceEnumerator::new()?.get_device(id)?.get_iaudioclient()?,
        SourceSpec::Endpoint { device_id: None, is_mic } => {
            let dir = if *is_mic { Direction::Capture } else { Direction::Render }; // Render + Capture below = loopback
            DeviceEnumerator::new()?.get_default_device_for_role(&dir, &Role::Console)?.get_iaudioclient()?
        }
    };
    // Event-driven shared mode: Microsoft requires BOTH durations = 0. Direction::Capture on a render
    // (or process-loopback) client adds AUDCLNT_STREAMFLAGS_LOOPBACK.
    client.initialize_client(&fmt, &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })?;
    if is_mic { mic_no_ducking(&client); } // before start: the opt-out applies only after a stop/start
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    let mut buf = vec![0f32; 48_000 * ch]; // 1 s: a too-small slice makes read_from_device drop the packet
    client.start_stream()?;
    let _ = ready.send(Ok(()));
    let handles = [event.as_handle(), shutdown];
    let mut first = true;
    loop {
        let r = unsafe { WaitForMultipleObjects(&handles, false, 20) }; // 20 ms backstop poll
        if r.0 == WAIT_OBJECT_0.0 + 1 { break; }
        while let Some(n) = capture.get_next_packet_size()? {
            if n == 0 { break; }
            let (frames, info) = capture.read_from_device(bytemuck::cast_slice_mut::<f32, u8>(&mut buf))?;
            if frames == 0 { break; }
            let samples = frames as usize * ch;
            if info.flags.silent { buf[..samples].fill(0.0); } // SILENT: the crate copies garbage; zero it
            let mut flags = 0u8;
            if info.flags.data_discontinuity && !(first && info.index == 0) { flags |= F_DISC; }
            if info.flags.silent { flags |= F_SILENT; }
            if info.flags.timestamp_error { flags |= F_TSERR; } // count it; never restart (OBS 0f498133a)
            first = false;
            let m = PacketMeta { ts_hns: info.timestamp, arrival_hns: crate::qpc::now_hns(), device_pos: info.index, frames, flags };
            if pcm.push_entire_slice(&buf[..samples]).is_ok() { let _ = meta.push(m); } // else overrun -> aligner sees a gap
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

pub enum DevEvent { Default { dir: Direction, role: Role, id: Option<String> }, Removed(String), State(String, wasapi::DeviceState) }
/// Long-lived MTA thread. Callbacks must not block: try_send only. Never drop the registration from a
/// thread a callback might wait on. Expect 3 callbacks (one per role) per user change; debounce 300 ms.
pub fn watch_devices(en: &DeviceEnumerator, tx: crossbeam_channel::Sender<DevEvent>) -> Result<wasapi::DeviceEventRegistration, wasapi::WasapiError> {
    let mut cb = wasapi::DeviceEventCallbacks::new();
    let t = tx.clone(); cb.set_default_device_callback(move |dir, role, id| { let _ = t.try_send(DevEvent::Default { dir, role, id }); });
    let t = tx.clone(); cb.set_device_removed_callback(move |id| { let _ = t.try_send(DevEvent::Removed(id)); });
    cb.set_device_state_callback(move |id, st| { let _ = tx.try_send(DevEvent::State(id, st)); });
    en.register_notification_callback(cb)
}
