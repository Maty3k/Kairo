// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-harness`'s command line (docs/research/16-verify-testbed/README.md §8.2, §8.7).

use std::path::PathBuf;

/// The usage text.
pub const USAGE: &str =
    "usage: kairo-harness run --run-id harness-<yyyyMMdd-HHmmss> --monitor <monitorDevicePath>
           --seconds 5..60 --profile capture|smoke [--testbed-fps N] [--mode borderless]
           [--output 1920x1080@60|2560x1440@60] [--capture gfxcapture|ddagrab] [--capture-rate 90]
           [--loglevel debug|verbose] [--finishing h264_metadata] [--voice] [--oled]
       kairo-harness displays --out FILE
       kairo-harness --version | --help
run starts the testbed on the monitor, records it with FFmpeg into AMF and has kairo-verify judge
the clip, all in the folder <logs>\\<run id>. --run-id is the job's own id. Only borderless runs
in this version. --oled is refused: the OLED shows moving content only, never a capture's.
Exit codes: 0 a report exists (whatever its verdict), 1 a step failed or timed out, 2 usage,
3 the PC refused (monitor, OLED, HDR, rate, disk, another FFmpeg, display off, a missing
program), 4 the encoder's read-back failed, 5 internal.";

/// The verify profile the clip is judged by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Gate 2's: research 16 §6.5's `capture`.
    Capture,
    /// The 10 s smoke (§8.7): cadence only warns.
    Smoke,
}

impl Profile {
    /// Its name on kairo-verify's command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Smoke => "smoke",
        }
    }
}

/// The clip's size: PLAN's Recommended (1080p60) or High (1440p60) preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// 1920×1080 at 60 fps.
    P1080,
    /// 2560×1440 at 60 fps.
    P1440,
}

impl Output {
    /// Width and height.
    pub fn size(self) -> (u32, u32) {
        match self {
            Self::P1080 => (1920, 1080),
            Self::P1440 => (2560, 1440),
        }
    }

    /// The output frame rate.
    pub fn fps(self) -> u32 {
        60
    }
}

/// How FFmpeg takes the testbed's pictures (house rule 2: WGC or DDA only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// `gfxcapture` by the testbed's window handle: Windows.Graphics.Capture.
    Gfx,
    /// `ddagrab` cropped to the testbed's client area: Desktop Duplication (risk 14's fallback).
    Dda,
}

/// FFmpeg's log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// The least verbose level that prints AMF's trace (README §8.5 step 3).
    Debug,
    /// Quieter, if debug logging is found to change the capture's cadence.
    Verbose,
}

impl LogLevel {
    /// Its name on FFmpeg's command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Verbose => "verbose",
        }
    }
}

/// What `run` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `harness-<yyyyMMdd-HHmmss>`: the job's id and the run folder's name.
    pub run_id: String,
    /// The monitor's `monitorDevicePath`.
    pub monitor: String,
    /// How long to capture.
    pub seconds: u32,
    /// The verify profile.
    pub profile: Profile,
    /// The testbed's rate; `None`: the monitor's rounded refresh rate.
    pub testbed_fps: Option<u32>,
    /// The clip's size.
    pub output: Output,
    /// How FFmpeg captures.
    pub capture: Capture,
    /// `gfxcapture`'s `max_framerate`: 360 by default, 120 with `--capture-rate 90`.
    pub max_framerate: u32,
    /// FFmpeg's log level.
    pub loglevel: LogLevel,
    /// Apply `h264_metadata`'s BT.709 colour fix after the encoder.
    pub finishing: bool,
    /// Start `kairo-tone` beside the testbed, and have the testbed beep.
    pub voice: bool,
    /// The monitor was given as the OLED: refused before anything runs.
    pub oled: bool,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Gate 2's capture.
    Run(Request),
    /// The displays and adapters, written to a file.
    Displays {
        /// Where.
        out: PathBuf,
    },
    /// The version.
    Version,
    /// The usage text.
    Help,
}

/// The run id's shape: `harness-` and a UTC stamp, `yyyyMMdd-HHmmss`.
pub fn valid_run_id(id: &str) -> bool {
    let Some(stamp) = id.strip_prefix("harness-") else {
        return false;
    };
    let bytes = stamp.as_bytes();
    bytes.len() == 15
        && bytes[8] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 8 || b.is_ascii_digit())
}

/// The value after flag `name` at `i`: a missing, empty or `--` value is an error.
fn value<'a>(args: &'a [String], i: usize, name: &str) -> Result<&'a str, String> {
    match args.get(i + 1) {
        Some(v) if !v.is_empty() && !v.starts_with("--") => Ok(v),
        _ => Err(format!("{name} needs a value")),
    }
}

/// Sets `slot` once.
fn once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("{name} given twice"));
    }
    Ok(())
}

/// Parses the arguments, without the program name. An error is the message for standard error.
pub fn parse(args: &[String]) -> Result<Command, String> {
    match args.first().map(String::as_str) {
        Some("run") => parse_run(&args[1..]).map(Command::Run),
        Some("displays") => match &args[1..] {
            [flag, out] if flag == "--out" && !out.is_empty() => Ok(Command::Displays {
                out: PathBuf::from(out),
            }),
            _ => Err(USAGE.to_owned()),
        },
        Some("--version" | "-V") if args.len() == 1 => Ok(Command::Version),
        Some("--help" | "-h") if args.len() == 1 => Ok(Command::Help),
        _ => Err(USAGE.to_owned()),
    }
}

fn parse_run(args: &[String]) -> Result<Request, String> {
    let (mut run_id, mut monitor, mut seconds, mut profile) = (None, None, None, None);
    let (mut fps, mut mode, mut output, mut capture) = (None, None, None, None);
    let (mut rate, mut loglevel, mut finishing) = (None, None, None);
    let (mut voice, mut oled) = (false, false);
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let mut takes = true;
        match flag {
            "--run-id" => once(&mut run_id, value(args, i, flag)?.to_owned(), flag)?,
            "--monitor" => once(&mut monitor, value(args, i, flag)?.to_owned(), flag)?,
            "--seconds" => {
                let s: u32 = value(args, i, flag)?
                    .parse()
                    .map_err(|_| "--seconds takes a whole number".to_owned())?;
                once(&mut seconds, s, flag)?;
            }
            "--profile" => {
                let p = match value(args, i, flag)? {
                    "capture" => Profile::Capture,
                    "smoke" => Profile::Smoke,
                    _ => return Err("--profile takes capture or smoke".to_owned()),
                };
                once(&mut profile, p, flag)?;
            }
            "--testbed-fps" => {
                let f: u32 = value(args, i, flag)?
                    .parse()
                    .ok()
                    .filter(|f| (1..=1_000).contains(f))
                    .ok_or("--testbed-fps takes a rate of 1 to 1000")?;
                once(&mut fps, f, flag)?;
            }
            "--mode" => once(&mut mode, value(args, i, flag)?.to_owned(), flag)?,
            "--output" => {
                let o = match value(args, i, flag)? {
                    "1920x1080@60" => Output::P1080,
                    "2560x1440@60" => Output::P1440,
                    _ => return Err("--output takes 1920x1080@60 or 2560x1440@60".to_owned()),
                };
                once(&mut output, o, flag)?;
            }
            "--capture" => {
                let c = match value(args, i, flag)? {
                    "gfxcapture" => Capture::Gfx,
                    "ddagrab" => Capture::Dda,
                    _ => return Err("--capture takes gfxcapture or ddagrab".to_owned()),
                };
                once(&mut capture, c, flag)?;
            }
            "--capture-rate" => {
                if value(args, i, flag)? != "90" {
                    return Err("--capture-rate takes 90 (max_framerate=120)".to_owned());
                }
                once(&mut rate, 120, flag)?;
            }
            "--loglevel" => {
                let l = match value(args, i, flag)? {
                    "debug" => LogLevel::Debug,
                    "verbose" => LogLevel::Verbose,
                    _ => return Err("--loglevel takes debug or verbose".to_owned()),
                };
                once(&mut loglevel, l, flag)?;
            }
            "--finishing" => {
                if value(args, i, flag)? != "h264_metadata" {
                    return Err("--finishing takes h264_metadata".to_owned());
                }
                once(&mut finishing, true, flag)?;
            }
            "--voice" | "--oled" => {
                let set = if flag == "--voice" {
                    &mut voice
                } else {
                    &mut oled
                };
                if std::mem::replace(set, true) {
                    return Err(format!("{flag} given twice"));
                }
                takes = false;
            }
            _ => return Err(format!("unknown argument: {flag}\n{USAGE}")),
        }
        i += if takes { 2 } else { 1 };
    }
    let run_id = run_id.ok_or("--run-id is required")?;
    if !valid_run_id(&run_id) {
        return Err("--run-id takes harness-<yyyyMMdd-HHmmss>, the job's id".to_owned());
    }
    let seconds = seconds.ok_or("--seconds is required")?;
    if !(5..=60).contains(&seconds) {
        return Err("--seconds takes 5 to 60".to_owned());
    }
    match mode.as_deref() {
        None | Some("borderless") => {}
        Some("windowed" | "exclusive") => {
            return Err(
                "--mode: only borderless runs in this version (exclusive needs a foreground wait of minutes; a windowed capture of the frame is unverified)"
                    .to_owned(),
            );
        }
        Some(_) => return Err("--mode takes borderless".to_owned()),
    }
    let capture = capture.unwrap_or(Capture::Gfx);
    if rate.is_some() && capture == Capture::Dda {
        return Err("--capture-rate is gfxcapture's max_framerate: not with ddagrab".to_owned());
    }
    Ok(Request {
        run_id,
        monitor: monitor.ok_or("--monitor is required")?,
        seconds,
        profile: profile.ok_or("--profile is required")?,
        testbed_fps: fps,
        output: output.unwrap_or(Output::P1080),
        capture,
        max_framerate: rate.unwrap_or(360),
        loglevel: loglevel.unwrap_or(LogLevel::Debug),
        finishing: finishing.unwrap_or(false),
        voice,
        oled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    const BASE: &str = "run --run-id harness-20261111-203000 --monitor \\\\?\\DISPLAY#X --seconds 30 --profile capture";

    #[test]
    fn a_run_with_its_defaults() {
        let Ok(Command::Run(r)) = parse(&args(BASE)) else {
            panic!("{:?}", parse(&args(BASE)));
        };
        assert_eq!(r.run_id, "harness-20261111-203000");
        assert_eq!(r.monitor, "\\\\?\\DISPLAY#X");
        assert_eq!((r.seconds, r.profile), (30, Profile::Capture));
        assert_eq!(
            (r.testbed_fps, r.output, r.capture),
            (None, Output::P1080, Capture::Gfx)
        );
        assert_eq!((r.max_framerate, r.loglevel), (360, LogLevel::Debug));
        assert!(!r.finishing && !r.voice && !r.oled);
    }

    #[test]
    fn every_flag() {
        let line = format!(
            "{BASE} --testbed-fps 180 --mode borderless --output 2560x1440@60 --capture-rate 90 \
             --loglevel verbose --finishing h264_metadata --voice --oled"
        );
        let Ok(Command::Run(r)) = parse(&args(&line)) else {
            panic!("{line}");
        };
        assert_eq!((r.testbed_fps, r.output), (Some(180), Output::P1440));
        assert_eq!((r.max_framerate, r.loglevel), (120, LogLevel::Verbose));
        assert!(r.finishing && r.voice && r.oled);
        let dda = format!("{BASE} --capture ddagrab --profile smoke");
        assert!(parse(&args(&dda)).is_err(), "--profile twice");
        let dda = BASE.replace("capture", "smoke") + " --capture ddagrab";
        let Ok(Command::Run(r)) = parse(&args(&dda)) else {
            panic!("{dda}");
        };
        assert_eq!((r.capture, r.profile), (Capture::Dda, Profile::Smoke));
        assert_eq!(
            parse(&args("displays --out d.json")),
            Ok(Command::Displays {
                out: PathBuf::from("d.json")
            })
        );
        assert_eq!(parse(&args("--version")), Ok(Command::Version));
        assert_eq!(parse(&args("-h")), Ok(Command::Help));
    }

    #[test]
    fn refusals() {
        let missing = |flag: &str| {
            let words: Vec<&str> = BASE.split_whitespace().collect();
            let at = words.iter().position(|w| *w == flag).unwrap();
            let mut kept = words.clone();
            kept.drain(at..at + 2);
            kept.join(" ")
        };
        let mut bad = vec![
            missing("--run-id"),
            missing("--monitor"),
            missing("--seconds"),
            missing("--profile"),
        ];
        for extra in [
            "--seconds 30",
            "--voice --voice",
            "--testbed-fps 0",
            "--output 1280x720@60",
            "--mode windowed",
            "--mode exclusive",
            "--mode fullscreen",
            "--capture dshow",
            "--capture-rate 60",
            "--capture ddagrab --capture-rate 90",
            "--finishing h264_mp4toannexb",
            "--loglevel trace",
            "--monitor",
            "--wat",
        ] {
            bad.push(format!("{BASE} {extra}"));
        }
        for seconds in ["4", "61", "x"] {
            bad.push(BASE.replace("--seconds 30", &format!("--seconds {seconds}")));
        }
        for id in [
            "harness-displays",
            "harness-2026111-203000",
            "build-20261111-203000",
        ] {
            bad.push(BASE.replace("harness-20261111-203000", id));
        }
        bad.push(BASE.replace("--monitor \\\\?\\DISPLAY#X", "--monitor --seconds"));
        bad.extend(["", "displays", "displays --out", "capture", "run"].map(str::to_owned));
        for line in bad {
            assert!(parse(&args(&line)).is_err(), "{line}");
        }
    }

    #[test]
    fn run_ids_are_stamps() {
        assert!(valid_run_id("harness-20261111-203000"));
        for bad in [
            "harness-20261111203000",
            "harness-2026111a-203000",
            "harness-20261111-20300",
            "harness-20261111-2030001",
            "harness--20261111-20300",
            "harness-displays",
        ] {
            assert!(!valid_run_id(bad), "{bad}");
        }
    }
}
