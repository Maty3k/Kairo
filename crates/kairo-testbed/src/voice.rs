// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-tone.exe beeps` (docs/research/16-verify-testbed/README.md §5): the "voice chat"
//! beep, 2 kHz by default, at every whole QPC second + 500 ms on the default playback device,
//! from its own process. The harness starts it beside the testbed, never as its child, so an
//! include-tree capture of the testbed's process doesn't take it as game sound. Its log,
//! `kairo_core::testbed_log`'s tone format, says when each beep was meant to play and when it
//! did; it names no device and no path (CLAUDE.md).

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kairo_core::testbed_log::{Event, TONE_LOG_HEADER, ToneHeader, ToneRecord, VoiceRecord};
use kairo_core::timeline::HNS_PER_SECOND;

use crate::audio::{Audio, Report};
use crate::beeps::{self, Beep, InOrder, LEAD_HNS, Played};
use crate::cli::{self, ToneConfig, exit};
use crate::{pace, platform};

/// Plays the beeps; returns the exit code.
pub fn run(config: &ToneConfig) -> u8 {
    let mut file = match &config.log {
        Some(path) => match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => Some(file),
            Err(e) => {
                eprintln!("kairo-tone: can't make the log: {}", e.kind());
                return exit::USAGE;
            }
        },
        None => None,
    };
    let mut write = |line: Option<String>| {
        if let (Some(file), Some(line)) = (file.as_mut(), line) {
            // One write per line, so a run stopped midway leaves whole lines.
            let _ = file.write_all(line.as_bytes());
        }
    };
    let event = |name: &str, fields: &[(&str, &str)]| {
        Event::new(name, fields).and_then(|e| ToneRecord::Event(e).line())
    };
    let header = ToneHeader {
        run_tag: config.run_tag.unwrap_or_else(cli::random_run_tag),
        hz: config.hz,
        offset_ms: config.offset_ms,
        dbfs: config.dbfs,
    };
    write(Some(format!("{TONE_LOG_HEADER}\n")));
    write(header.line());
    let stdin_eof = Arc::new(AtomicBool::new(false));
    if config.stop_on_stdin_eof {
        let eof = Arc::clone(&stdin_eof);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buffer = [0_u8; 256];
            while let Ok(n) = std::io::Read::read(&mut stdin, &mut buffer) {
                if n == 0 {
                    break;
                }
            }
            eof.store(true, Ordering::Relaxed);
        });
    }
    platform::opt_out_of_power_throttling();
    let audio = Audio::start(config.hz, config.dbfs);
    // The first beep a whole second after the stream has had time to start.
    let first_second = pace::next_whole_second(platform::now_hns() + 2 * LEAD_HNS) / HNS_PER_SECOND;
    let end = (first_second + i64::from(config.seconds)) * HNS_PER_SECOND;
    let mut order = InOrder::default();
    let mut next = 0_u64;
    let mut reason = "seconds";
    let mut failed = false;
    let voice = |played: Played| {
        ToneRecord::Voice(VoiceRecord {
            beep: played.beep.index,
            target_hns: played.beep.target_hns,
            played_hns: played.played_hns,
        })
        .line()
    };
    loop {
        if stdin_eof.load(Ordering::Relaxed) {
            reason = "stdin";
            break;
        }
        let now = platform::now_hns();
        if now >= end {
            break;
        }
        loop {
            let target = beeps::voice_target(first_second, next, config.offset_ms);
            if target >= end || target > now + LEAD_HNS {
                break;
            }
            let beep = Beep {
                index: next,
                frame: 0,
                target_hns: target,
            };
            audio.beep(beep);
            order.sent(beep);
            next += 1;
        }
        for report in audio.reports() {
            match report {
                Report::Played(played) => {
                    for p in order.settle(played) {
                        write(voice(p));
                    }
                }
                Report::DeviceChanged => write(event("audio_device_changed", &[])),
                Report::Failed(message) => {
                    write(event("audio_failed", &[]));
                    eprintln!("kairo-tone: audio: {message}");
                    failed = true;
                }
            }
        }
        if failed {
            reason = "error";
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for report in audio.stop() {
        if let Report::Played(played) = report {
            for p in order.settle(played) {
                write(voice(p));
            }
        }
    }
    for p in order.rest() {
        write(voice(p));
    }
    write(event("end", &[("reason", reason)]));
    if failed { exit::DEVICE } else { exit::OK }
}
