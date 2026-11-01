// SPDX-License-Identifier: GPL-3.0-or-later
//! The render stream (docs/research/16-verify-testbed/README.md §4.7, §5): WASAPI shared mode,
//! event-driven, 48 kHz f32 stereo, on the console role's default render device, through
//! `wasapi` =0.26.0's safe API. It runs on its own thread with COM in the multithreaded
//! apartment: the caller hands it beeps and reads back what played. It never switches devices
//! (M1:41): when the default device changes it reports that and plays on, and when its device
//! goes away it stops and reports the failure. It never opens a capture device or the
//! microphone (M1:15), and reports name no device.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

use wasapi::{
    DeviceEnumerator, DeviceEventCallbacks, Direction, Role, SampleType, StreamMode, WaveFormat,
};

use crate::beeps::{Beep, Beeper, CHANNELS, ClockPoint, Played, SAMPLE_RATE};

/// What the stream reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    /// A beep's fate.
    Played(Played),
    /// The default render device changed; the stream stays on its own.
    DeviceChanged,
    /// The stream couldn't start or stopped: no device, or it went away.
    Failed(String),
}

enum Command {
    Beep(Beep),
    Stop,
}

/// The render stream's thread.
pub struct Audio {
    commands: Sender<Command>,
    reports: Receiver<Report>,
    thread: Option<JoinHandle<()>>,
}

impl Audio {
    /// Starts a stream of beeps at `hz` and `dbfs` peak.
    pub fn start(hz: u32, dbfs: f64) -> Audio {
        let (commands, command_rx) = mpsc::channel();
        let (report_tx, reports) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("audio".to_owned())
            .spawn(move || {
                if let Err(message) = stream(hz, dbfs, &command_rx, &report_tx) {
                    let _ = report_tx.send(Report::Failed(message));
                }
            })
            .ok();
        Audio {
            commands,
            reports,
            thread,
        }
    }

    /// Hands the stream a beep, at least [`crate::beeps::LEAD_HNS`] ahead of its target.
    pub fn beep(&self, beep: Beep) {
        let _ = self.commands.send(Command::Beep(beep));
    }

    /// What the stream has reported since the last call.
    pub fn reports(&self) -> Vec<Report> {
        self.reports.try_iter().collect()
    }

    /// Stops the stream; returns what it reported last, its unplayed beeps included.
    pub fn stop(mut self) -> Vec<Report> {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.reports.try_iter().collect()
    }
}

impl Drop for Audio {
    /// A run that ends on an error still stops the stream and its thread.
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The thread's body: open the device, then fill its buffer on each event until told to stop.
fn stream(
    hz: u32,
    dbfs: f64,
    commands: &Receiver<Command>,
    reports: &Sender<Report>,
) -> Result<(), String> {
    let text = |e: wasapi::WasapiError| e.to_string();
    // S_FALSE (already initialised on this thread) is fine too.
    wasapi::initialize_mta()
        .ok()
        .map_err(|e| format!("COM: {e}"))?;
    let enumerator = DeviceEnumerator::new().map_err(text)?;
    let device = enumerator
        .get_default_device_for_role(&Direction::Render, &Role::Console)
        .map_err(|e| format!("no default render device: {e}"))?;
    let changed = reports.clone();
    let mut callbacks = DeviceEventCallbacks::new();
    callbacks.set_default_device_callback(move |direction, role, _id| {
        if direction == Direction::Render && role == Role::Console {
            let _ = changed.send(Report::DeviceChanged);
        }
    });
    // Notifications are a help, not a need: the stream plays without them.
    let _registration = enumerator.register_notification_callback(callbacks).ok();
    let mut client = device.get_iaudioclient().map_err(text)?;
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS,
        None,
    );
    // Event-driven shared mode wants both durations 0 (Microsoft); the engine converts the
    // format to the device's mix format.
    client
        .initialize_client(
            &format,
            &Direction::Render,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: 0,
            },
        )
        .map_err(text)?;
    let event = client.set_get_eventhandle().map_err(text)?;
    let render = client.get_audiorenderclient().map_err(text)?;
    let clock = client.get_audioclock().map_err(text)?;
    let frequency = clock.get_frequency().map_err(text)?;
    let mut beeper = Beeper::new(hz, dbfs);
    let mut samples = Vec::new();
    let mut bytes = Vec::new();
    let mut fill = |beeper: &mut Beeper| -> Result<(), String> {
        let frames = client.get_available_space_in_frames().map_err(text)? as usize;
        if frames == 0 {
            return Ok(());
        }
        samples.resize(frames * CHANNELS, 0.0_f32);
        beeper.fill(&mut samples);
        bytes.clear();
        bytes.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        render.write_to_device(frames, &bytes, None).map_err(text)
    };
    // Silence first, so the stream starts with a full buffer (Microsoft's rendering example).
    fill(&mut beeper)?;
    client.start_stream().map_err(text)?;
    let mut waiting: Vec<Beep> = Vec::new();
    let result = loop {
        let mut stop = false;
        loop {
            match commands.try_recv() {
                Ok(Command::Beep(beep)) => waiting.push(beep),
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                    stop = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if stop {
            break Ok(());
        }
        // A missed event isn't an error: the buffer is checked again either way.
        let _ = event.wait_for_event(100);
        let (position, qpc) = match clock.get_position() {
            Ok(reading) => reading,
            Err(e) => break Err(format!("the audio clock: {e}")),
        };
        if let Some(point) = ClockPoint::from_position(position, frequency, qpc) {
            for beep in waiting.drain(..) {
                beeper.schedule(beep, &point);
            }
            for played in beeper.retire(&point) {
                let _ = reports.send(Report::Played(played));
            }
        }
        if let Err(e) = fill(&mut beeper) {
            break Err(e);
        }
    };
    let _ = client.stop_stream();
    // Scheduled beeps come before the ones still waiting, so they stay in order.
    for played in beeper.finish() {
        let _ = reports.send(Report::Played(played));
    }
    for beep in waiting {
        let _ = reports.send(Report::Played(Played {
            beep,
            played_hns: None,
        }));
    }
    result
}
