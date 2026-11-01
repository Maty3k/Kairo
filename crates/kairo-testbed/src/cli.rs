// SPDX-License-Identifier: GPL-3.0-or-later
//! `testbed.exe`'s command line (docs/research/16-verify-testbed/README.md §4.10).

use std::path::PathBuf;

use kairo_core::testbed_log::{Mode, Pace};
use kairo_core::testsig::{Load, MAX_SIDE, Pattern};

/// The usage text.
pub const USAGE: &str =
    "usage: testbed --monitor <monitorDevicePath> [--mode borderless|windowed|exclusive]
               [--size WxH] [--activate] [--fps N] [--pace vsync|timer] [--seconds S]
               [--pattern full|motion] [--load scroll|churn|none] [--hdr auto|off] [--hdr-patches]
               [--run-tag 0..127] [--log FILE] [--ready FILE] [--stop-on-stdin-eof]
       testbed --version | --help
--monitor takes the monitor's device path (pc harness displays lists them); there is no
default screen. --size is for windowed mode (default 1280x720). --fps defaults to the
monitor's rounded refresh rate. --seconds defaults to 60. Exclusive mode needs focus, so it
implies --activate, which waits until the foreground lock time-out has passed since the last
input. The log and the ready file must not exist yet.
Exit codes: 0 ok, 2 usage, 3 monitor not found or rate refused, 4 device or swap chain,
5 focus or exclusive fullscreen not obtained.";

/// Exit codes (README §4.10).
pub mod exit {
    /// Ran to the end, or was closed or stopped.
    pub const OK: u8 = 0;
    /// A bad command line, or a log or ready file that can't be made.
    pub const USAGE: u8 = 2;
    /// The monitor isn't there, or it can't show the rate asked for.
    pub const MONITOR: u8 = 3;
    /// The D3D11 device or the swap chain failed.
    pub const DEVICE: u8 = 4;
    /// Focus or exclusive fullscreen wasn't obtained.
    pub const FOCUS: u8 = 5;
}

/// Whether to draw in HDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hdr {
    /// FP16 scRGB when the monitor's output is in HDR (PQ), else BGRA8.
    Auto,
    /// Always BGRA8.
    Off,
}

/// What `testbed` was asked to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The monitor's `monitorDevicePath`.
    pub monitor: String,
    /// The window mode.
    pub mode: Mode,
    /// The client size in windowed mode.
    pub size: (u32, u32),
    /// Take the foreground (exclusive mode always does).
    pub activate: bool,
    /// Frames per second; `None`: the monitor's rounded refresh rate.
    pub fps: Option<u32>,
    /// Vsync or timer pacing.
    pub pace: Pace,
    /// How long to run.
    pub seconds: u32,
    /// The scene.
    pub pattern: Pattern,
    /// The load band.
    pub load: Load,
    /// HDR or not.
    pub hdr: Hdr,
    /// HDR patches above SDR white in the motion pattern too.
    pub hdr_patches: bool,
    /// The run's tag; `None`: random.
    pub run_tag: Option<u8>,
    /// The QPC log.
    pub log: Option<PathBuf>,
    /// The ready file.
    pub ready: Option<PathBuf>,
    /// Stop at EOF on standard input.
    pub stop_on_stdin_eof: bool,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Run the testbed.
    Run(Box<Config>),
    /// `--version`.
    Version,
    /// `--help`.
    Help,
}

/// Parses the arguments, without the program name.
pub fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [flag] if flag == "--version" || flag == "-V" => return Ok(Command::Version),
        [flag] if flag == "--help" || flag == "-h" => return Ok(Command::Help),
        _ => {}
    }
    let mut monitor = None;
    let mut mode = None;
    let mut size = None;
    let mut activate = false;
    let mut fps = None;
    let mut pace = None;
    let mut seconds = None;
    let mut pattern = None;
    let mut load = None;
    let mut hdr = None;
    let mut hdr_patches = false;
    let mut run_tag = None;
    let mut log = None;
    let mut ready = None;
    let mut stop_on_stdin_eof = false;
    let mut words = args.iter();
    while let Some(flag) = words.next() {
        let mut value = |name: &str| {
            words
                .next()
                .filter(|v| !v.is_empty() && !v.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        let once = |seen: bool, name: &str| {
            if seen {
                Err(format!("{name} is given twice\n{USAGE}"))
            } else {
                Ok(())
            }
        };
        match flag.as_str() {
            "--monitor" => {
                once(monitor.is_some(), flag)?;
                monitor = Some(value(flag)?);
            }
            "--mode" => {
                once(mode.is_some(), flag)?;
                mode = Some(match value(flag)?.as_str() {
                    "borderless" => Mode::Borderless,
                    "windowed" => Mode::Windowed,
                    "exclusive" => Mode::Exclusive,
                    _ => {
                        return Err(format!(
                            "--mode takes borderless, windowed or exclusive\n{USAGE}"
                        ));
                    }
                });
            }
            "--size" => {
                once(size.is_some(), flag)?;
                size = Some(parse_size(&value(flag)?)?);
            }
            "--activate" => {
                once(activate, flag)?;
                activate = true;
            }
            "--fps" => {
                once(fps.is_some(), flag)?;
                fps = Some(number(&value(flag)?, 1, 1_000, flag)?);
            }
            "--pace" => {
                once(pace.is_some(), flag)?;
                pace = Some(match value(flag)?.as_str() {
                    "vsync" => Pace::Vsync,
                    "timer" => Pace::Timer,
                    _ => return Err(format!("--pace takes vsync or timer\n{USAGE}")),
                });
            }
            "--seconds" => {
                once(seconds.is_some(), flag)?;
                seconds = Some(number(&value(flag)?, 1, u32::MAX, flag)?);
            }
            "--pattern" => {
                once(pattern.is_some(), flag)?;
                pattern = Some(match value(flag)?.as_str() {
                    "full" => Pattern::Full,
                    "motion" => Pattern::Motion,
                    _ => return Err(format!("--pattern takes full or motion\n{USAGE}")),
                });
            }
            "--load" => {
                once(load.is_some(), flag)?;
                load = Some(match value(flag)?.as_str() {
                    "scroll" => Load::Scroll,
                    "churn" => Load::Churn,
                    "none" => Load::None,
                    _ => return Err(format!("--load takes scroll, churn or none\n{USAGE}")),
                });
            }
            "--hdr" => {
                once(hdr.is_some(), flag)?;
                hdr = Some(match value(flag)?.as_str() {
                    "auto" => Hdr::Auto,
                    "off" => Hdr::Off,
                    _ => return Err(format!("--hdr takes auto or off\n{USAGE}")),
                });
            }
            "--hdr-patches" => {
                once(hdr_patches, flag)?;
                hdr_patches = true;
            }
            "--run-tag" => {
                once(run_tag.is_some(), flag)?;
                run_tag = Some(number(&value(flag)?, 0, 127, flag)? as u8);
            }
            "--log" => {
                once(log.is_some(), flag)?;
                log = Some(PathBuf::from(value(flag)?));
            }
            "--ready" => {
                once(ready.is_some(), flag)?;
                ready = Some(PathBuf::from(value(flag)?));
            }
            "--stop-on-stdin-eof" => {
                once(stop_on_stdin_eof, flag)?;
                stop_on_stdin_eof = true;
            }
            _ => return Err(format!("unknown argument\n{USAGE}")),
        }
    }
    let Some(monitor) = monitor else {
        return Err(format!(
            "--monitor is required: there is no default screen\n{USAGE}"
        ));
    };
    let mode = mode.unwrap_or(Mode::Borderless);
    let pace = pace.unwrap_or(Pace::Vsync);
    if mode == Mode::Exclusive && pace == Pace::Timer {
        return Err(format!("exclusive mode paces by vsync only\n{USAGE}"));
    }
    if size.is_some() && mode != Mode::Windowed {
        return Err(format!("--size is for windowed mode\n{USAGE}"));
    }
    let hdr = hdr.unwrap_or(Hdr::Auto);
    if hdr == Hdr::Off && hdr_patches {
        return Err(format!("--hdr-patches needs HDR\n{USAGE}"));
    }
    if log.is_some() && log == ready {
        return Err(format!(
            "--log and --ready must be different files\n{USAGE}"
        ));
    }
    Ok(Command::Run(Box::new(Config {
        monitor,
        mode,
        size: size.unwrap_or((1_280, 720)),
        activate: activate || mode == Mode::Exclusive,
        fps,
        pace,
        seconds: seconds.unwrap_or(60),
        pattern: pattern.unwrap_or(Pattern::Full),
        load: load.unwrap_or(Load::Scroll),
        hdr,
        hdr_patches,
        run_tag,
        log,
        ready,
        stop_on_stdin_eof,
    })))
}

/// A whole number from `low` to `high`.
fn number(text: &str, low: u32, high: u32, name: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|n| (low..=high).contains(n))
        .ok_or_else(|| format!("{name} takes a whole number from {low} to {high}\n{USAGE}"))
}

/// `WxH`, each side 1 to [`MAX_SIDE`].
fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("--size takes WxH, such as 1280x720\n{USAGE}"))?;
    Ok((
        number(w, 1, MAX_SIDE, "--size")?,
        number(h, 1, MAX_SIDE, "--size")?,
    ))
}

/// A random run tag, 0 to 127: the standard library's hasher keys are random per process.
pub fn random_run_tag() -> u8 {
    use std::hash::{BuildHasher, RandomState};
    (RandomState::new().hash_one(std::process::id()) & 0x7F) as u8
}

/// The frame rate when `--fps` isn't given: the refresh rate in mHz, rounded to whole Hz.
pub fn default_fps(refresh_mhz: u64) -> u32 {
    u32::try_from((refresh_mhz + 500) / 1_000)
        .unwrap_or(u32::MAX)
        .max(1)
}

/// Why a rate can't be paced, if it can't (exit 3): vsync needs a whole number of vblanks per
/// frame, 1 to 4 (`Refresh::fits_vsync`); timer pacing runs 60 to 360 fps (README §4.3).
pub fn refuse_rate(pace: Pace, fps: u32, vsync_k: Option<u32>) -> Option<String> {
    match pace {
        Pace::Vsync if vsync_k.is_none() => Some(format!(
            "{fps} fps isn't the refresh rate divided by 1 to 4 within 0.5%: use --pace timer"
        )),
        Pace::Timer if !(60..=360).contains(&fps) => {
            Some(format!("timer pacing runs 60 to 360 fps, not {fps}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn run(line: &str) -> Config {
        match parse(&args(line)) {
            Ok(Command::Run(config)) => *config,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn defaults_and_flags() {
        let c = run("--monitor \\\\?\\DISPLAY#X#1");
        assert_eq!(c.monitor, "\\\\?\\DISPLAY#X#1");
        assert_eq!(c.mode, Mode::Borderless);
        assert_eq!(c.size, (1_280, 720));
        assert!(!c.activate);
        assert_eq!(c.fps, None);
        assert_eq!(c.pace, Pace::Vsync);
        assert_eq!(c.seconds, 60);
        assert_eq!(c.pattern, Pattern::Full);
        assert_eq!(c.load, Load::Scroll);
        assert_eq!(c.hdr, Hdr::Auto);
        assert_eq!(c.run_tag, None);
        let c = run(
            "--monitor m --mode windowed --size 640x360 --fps 90 --pace timer --seconds 20 \
             --pattern motion --load churn --hdr auto --hdr-patches --run-tag 127 --log a.log \
             --ready r.json --stop-on-stdin-eof",
        );
        assert_eq!(c.mode, Mode::Windowed);
        assert_eq!(c.size, (640, 360));
        assert_eq!(c.fps, Some(90));
        assert_eq!(c.pace, Pace::Timer);
        assert_eq!(c.seconds, 20);
        assert_eq!(c.pattern, Pattern::Motion);
        assert_eq!(c.load, Load::Churn);
        assert!(c.hdr_patches);
        assert_eq!(c.run_tag, Some(127));
        assert_eq!(c.log, Some(PathBuf::from("a.log")));
        assert_eq!(c.ready, Some(PathBuf::from("r.json")));
        assert!(c.stop_on_stdin_eof);
        // Exclusive mode needs focus.
        assert!(run("--monitor m --mode exclusive").activate);
        assert_eq!(parse(&args("--version")), Ok(Command::Version));
        assert_eq!(parse(&args("-h")), Ok(Command::Help));
    }

    #[test]
    fn refuses_what_it_cant_do() {
        for line in [
            "",
            "--mode windowed",
            "--monitor",
            "--monitor m --monitor n",
            "--monitor --mode windowed",
            "--monitor m --mode fullscreen",
            "--monitor m --size 1280x720",
            "--monitor m --mode windowed --size 1280",
            "--monitor m --mode windowed --size 0x720",
            "--monitor m --mode windowed --size 16385x720",
            "--monitor m --fps 0",
            "--monitor m --fps sixty",
            "--monitor m --seconds 0",
            "--monitor m --run-tag 128",
            "--monitor m --mode exclusive --pace timer",
            "--monitor m --hdr off --hdr-patches",
            "--monitor m --log x --ready x",
            "--monitor m --beep on",
            "--monitor m extra",
        ] {
            let message = parse(&args(line)).unwrap_err();
            assert!(message.contains("usage: testbed"), "{line}: {message}");
        }
    }

    #[test]
    fn rates_and_tags() {
        assert_eq!(default_fps(179_960), 180);
        assert_eq!(default_fps(59_940), 60);
        assert_eq!(default_fps(0), 1);
        assert!(refuse_rate(Pace::Vsync, 180, Some(1)).is_none());
        assert!(refuse_rate(Pace::Vsync, 144, None).is_some());
        assert!(refuse_rate(Pace::Timer, 240, None).is_none());
        assert!(refuse_rate(Pace::Timer, 59, None).is_some());
        assert!(refuse_rate(Pace::Timer, 361, None).is_some());
        assert!(random_run_tag() <= 127);
    }
}
