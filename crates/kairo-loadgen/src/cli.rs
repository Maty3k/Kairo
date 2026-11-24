// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-loadgen`'s command line (docs/research/17-loadgen-bench/README.md §3.2, §3.9), in the
//! testbed's idiom: the command first, the monitor by its device path with no default screen,
//! each flag at most once, unknown arguments refused, and messages that never repeat a value, so
//! stderr names no path (CLAUDE.md).
//!
//! `run` takes its count explicitly (`--iters` for `alu`, `--reads` for `bw`) and has no default
//! one, so a run can't quietly differ from the last; `calibrate` finds the count instead. A flag
//! that would do nothing is refused rather than ignored: the texture's flags without the
//! bandwidth workload, `run`'s flags on `calibrate` and the other way round.

use std::path::PathBuf;

use crate::workload::{
    self, DEFAULT_SEED, DEFAULT_TARGET_MS, DEFAULT_TEXTURE_MIB, DEFAULT_WARMUP_SECONDS, Texture,
    Workload,
};

/// The usage text.
pub const USAGE: &str = "usage: kairo-loadgen run --monitor <monitorDevicePath>
                     (--workload alu --iters N | --workload bw --reads N)
                     [--texture-mib 256] [--seed N] [--bands 16] [--buffers 3] [--latency 2]
                     [--detail on|off] [--seconds 60] [--activate] [--log FILE] [--ready FILE]
                     [--stop-on-stdin-eof]
       kairo-loadgen calibrate --monitor <monitorDevicePath> --workload alu|bw [--target-ms 8]
                     [--warmup 120] [--texture-mib 256] [--seed N] [--bands 16] [--buffers 3]
                     [--latency 2] [--detail on|off] [--activate] [--out FILE] [--log FILE]
                     [--stop-on-stdin-eof]
       kairo-loadgen --version | --help
--monitor takes the monitor's device path (pc harness displays lists them); there is no default
screen. run draws a fixed load until --seconds after its first frame at the full count, and
never changes the count: --iters 1 to 65536 for alu, --reads a multiple of 4 from 4 to 4096 for
bw. calibrate finds the count whose frames take --target-ms (1 to 50) of GPU time, holding the
load --warmup seconds (0 to 3600) before it measures. --texture-mib (a power of two from 1 to
1024) and --seed (decimal or 0x hex) are for bw. --bands 1 to 64, --buffers 2 to 4, --latency 1
to 3. --detail off draws the picture without its grain. --activate waits until the foreground
lock time-out has passed since the last input, then takes the foreground. The log, ready and
--out files must not exist yet; the ready file's folder must be on NTFS.
Exit codes: 0 ok, 2 usage or a file that can't be made, 3 refused (monitor, HDR, no tearing,
a window that doesn't cover the monitor, a TDR guard, a display change, a calibration that
can't reach its target), 4 device, swap chain, shader or timestamps failed, or the device lost,
5 the foreground not obtained.";

/// Exit codes (README §3.9, the testbed's numbering). `kairo-devrun`'s own stop gives a job 1,
/// so loadgen never uses it.
pub mod exit {
    /// Ran to `--seconds`, or stopped by `WM_CLOSE` or stdin EOF; `calibrate` wrote its result.
    pub const OK: u8 = 0;
    /// A bad command line, or a log, ready or `--out` file that exists or can't be made.
    pub const USAGE: u8 = 2;
    /// Refused: the monitor, HDR, tearing, the client area, a TDR guard, a display change, or a
    /// calibration that can't reach its target within the caps.
    pub const REFUSED: u8 = 3;
    /// The device, swap chain, shaders or timestamps failed, or the device was lost.
    pub const DEVICE: u8 = 4;
    /// `--activate` didn't get the foreground.
    pub const FOCUS: u8 = 5;
}

/// `--bands` when it isn't given: about 0.5 ms a draw at the 8 ms target (README §3.5).
pub const DEFAULT_BANDS: u32 = 16;
/// The most bands.
pub const MAX_BANDS: u32 = 64;
/// `--buffers` when it isn't given: one on screen, one drawn, one queued (README §3.4).
pub const DEFAULT_BUFFERS: u32 = 3;
/// `--latency` when it isn't given: one frame always queued, so the GPU never waits for the CPU.
pub const DEFAULT_LATENCY: u32 = 2;
/// `--seconds` when it isn't given.
pub const DEFAULT_SECONDS: u32 = 60;
/// The range `--target-ms` takes, in ms \[proposed\]: well inside the 100 ms frame guard.
pub const TARGET_MS_RANGE: (f64, f64) = (1.0, 50.0);
/// The longest `--warmup`, in seconds \[proposed\]: a cap that catches a typo, beside the job's own
/// time limit.
pub const MAX_WARMUP_SECONDS: u32 = 3_600;

/// What `run` and `calibrate` share.
#[derive(Debug, Clone, PartialEq)]
pub struct Common {
    /// The monitor's `monitorDevicePath`.
    pub monitor: String,
    /// The workload.
    pub workload: Workload,
    /// The bandwidth workload's texture; `None` for `alu`.
    pub texture: Option<Texture>,
    /// Bands per frame, one draw each.
    pub bands: u32,
    /// Swap-chain buffers.
    pub buffers: u32,
    /// The swap chain's maximum frame latency.
    pub latency: u32,
    /// The grain (`--detail on`); `off` compiles the shaders with `KAIRO_DETAIL=0`.
    pub detail: bool,
    /// Take the foreground.
    pub activate: bool,
    /// The log.
    pub log: Option<PathBuf>,
    /// Stop at EOF on standard input.
    pub stop_on_stdin_eof: bool,
}

/// What `run` was asked to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct RunConfig {
    /// The shared flags.
    pub common: Common,
    /// `--iters` or `--reads`.
    pub count: u32,
    /// How long to run, from the first frame at the full count.
    pub seconds: u32,
    /// The ready file.
    pub ready: Option<PathBuf>,
}

/// What `calibrate` was asked to find.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrateConfig {
    /// The shared flags.
    pub common: Common,
    /// The GPU time per frame to reach, in ms.
    pub target_ms: f64,
    /// How long to hold the load before the fit, in seconds.
    pub warmup_seconds: u32,
    /// The file the calibration line is also written to.
    pub out: Option<PathBuf>,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `run`.
    Run(Box<RunConfig>),
    /// `calibrate`.
    Calibrate(Box<CalibrateConfig>),
    /// `--version`.
    Version,
    /// `--help`.
    Help,
}

impl Command {
    /// The flags `run` and `calibrate` share; `None` for `--version` and `--help`.
    pub fn common(&self) -> Option<&Common> {
        match self {
            Self::Run(run) => Some(&run.common),
            Self::Calibrate(calibrate) => Some(&calibrate.common),
            Self::Version | Self::Help => None,
        }
    }
}

/// A refusal: the reason, then the usage text.
fn refuse(why: &str) -> String {
    format!("{why}\n{USAGE}")
}

/// Parses the arguments, without the program name.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let (command, rest) = match args {
        [flag] if flag == "--version" || flag == "-V" => return Ok(Command::Version),
        [flag] if flag == "--help" || flag == "-h" => return Ok(Command::Help),
        [command, rest @ ..] if command == "run" || command == "calibrate" => {
            (command.as_str(), rest)
        }
        _ => return Err(refuse("the command comes first: run or calibrate")),
    };
    let mut seen: Vec<&str> = Vec::new();
    let mut monitor = None;
    let mut workload = None;
    let mut iters = None;
    let mut reads = None;
    let mut texture_mib = None;
    let mut seed = None;
    let mut bands = None;
    let mut buffers = None;
    let mut latency = None;
    let mut detail = None;
    let mut seconds = None;
    let mut target_ms = None;
    let mut warmup = None;
    let mut activate = false;
    let mut log = None;
    let mut ready = None;
    let mut out = None;
    let mut stop_on_stdin_eof = false;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let flag = word.as_str();
        let only = match flag {
            "--iters" | "--reads" | "--seconds" | "--ready" => Some("run"),
            "--target-ms" | "--warmup" | "--out" => Some("calibrate"),
            "--monitor"
            | "--workload"
            | "--texture-mib"
            | "--seed"
            | "--bands"
            | "--buffers"
            | "--latency"
            | "--detail"
            | "--activate"
            | "--log"
            | "--stop-on-stdin-eof" => None,
            _ => return Err(refuse("unknown argument")),
        };
        if let Some(only) = only
            && only != command
        {
            return Err(refuse(&format!("{flag} is for {only} only")));
        }
        if seen.contains(&flag) {
            return Err(refuse(&format!("{flag} is given twice")));
        }
        seen.push(flag);
        match flag {
            "--activate" => activate = true,
            "--stop-on-stdin-eof" => stop_on_stdin_eof = true,
            _ => {
                let value = words
                    .next()
                    .filter(|v| !v.is_empty() && !v.starts_with("--"))
                    .ok_or_else(|| refuse(&format!("{flag} needs a value")))?;
                match flag {
                    "--monitor" => monitor = Some(value.clone()),
                    "--workload" => {
                        workload = Some(
                            Workload::from_name(value)
                                .ok_or_else(|| refuse("--workload takes alu or bw"))?,
                        );
                    }
                    "--iters" => {
                        let alu = Workload::Alu;
                        iters = Some(parse_count(value, alu).ok_or_else(|| {
                            refuse(&format!(
                                "--iters takes a whole number from 1 to {}",
                                alu.max_count()
                            ))
                        })?);
                    }
                    "--reads" => {
                        let bw = Workload::Bw;
                        reads = Some(parse_count(value, bw).ok_or_else(|| {
                            refuse(&format!(
                                "--reads takes a multiple of 4 from 4 to {}",
                                bw.max_count()
                            ))
                        })?);
                    }
                    "--texture-mib" => {
                        texture_mib = Some(
                            number(value, 1, workload::MAX_TEXTURE_MIB)
                                .filter(|mib| mib.is_power_of_two())
                                .ok_or_else(|| {
                                    refuse("--texture-mib takes a power of two from 1 to 1024")
                                })?,
                        );
                    }
                    "--seed" => {
                        seed = Some(parse_seed(value).ok_or_else(|| {
                            refuse("--seed takes a whole number below 2^32, decimal or 0x hex")
                        })?);
                    }
                    "--bands" => bands = Some(whole(value, 1, MAX_BANDS, flag)?),
                    "--buffers" => buffers = Some(whole(value, 2, 4, flag)?),
                    "--latency" => latency = Some(whole(value, 1, 3, flag)?),
                    "--detail" => {
                        detail = Some(match value.as_str() {
                            "on" => true,
                            "off" => false,
                            _ => return Err(refuse("--detail takes on or off")),
                        });
                    }
                    "--seconds" => seconds = Some(whole(value, 1, u32::MAX, flag)?),
                    "--target-ms" => {
                        let (low, high) = TARGET_MS_RANGE;
                        target_ms = Some(
                            value
                                .parse::<f64>()
                                .ok()
                                .filter(|ms| ms.is_finite() && (low..=high).contains(ms))
                                .ok_or_else(|| {
                                    refuse(&format!("--target-ms takes {low} to {high} ms"))
                                })?,
                        );
                    }
                    "--warmup" => warmup = Some(whole(value, 0, MAX_WARMUP_SECONDS, flag)?),
                    "--log" => log = Some(PathBuf::from(value)),
                    "--ready" => ready = Some(PathBuf::from(value)),
                    "--out" => out = Some(PathBuf::from(value)),
                    _ => return Err(refuse("unknown argument")),
                }
            }
        }
    }
    let Some(monitor) = monitor else {
        return Err(refuse("--monitor is required: there is no default screen"));
    };
    let Some(workload) = workload else {
        return Err(refuse("--workload is required: alu or bw"));
    };
    let texture = match workload {
        Workload::Bw => Texture::new(
            texture_mib.unwrap_or(DEFAULT_TEXTURE_MIB),
            seed.unwrap_or(DEFAULT_SEED),
        ),
        Workload::Alu if texture_mib.is_some() || seed.is_some() => {
            return Err(refuse("--texture-mib and --seed are for --workload bw"));
        }
        Workload::Alu => None,
    };
    let files: Vec<&PathBuf> = [&log, &ready, &out].into_iter().flatten().collect();
    if files
        .iter()
        .enumerate()
        .any(|(i, a)| files[..i].contains(a))
    {
        return Err(refuse("--log, --ready and --out must be different files"));
    }
    let common = Common {
        monitor,
        workload,
        texture,
        bands: bands.unwrap_or(DEFAULT_BANDS),
        buffers: buffers.unwrap_or(DEFAULT_BUFFERS),
        latency: latency.unwrap_or(DEFAULT_LATENCY),
        detail: detail.unwrap_or(true),
        activate,
        log,
        stop_on_stdin_eof,
    };
    if command == "calibrate" {
        return Ok(Command::Calibrate(Box::new(CalibrateConfig {
            common,
            target_ms: target_ms.unwrap_or(DEFAULT_TARGET_MS),
            warmup_seconds: warmup.unwrap_or(DEFAULT_WARMUP_SECONDS),
            out,
        })));
    }
    let count = match (workload, iters, reads) {
        (Workload::Alu, Some(n), None) | (Workload::Bw, None, Some(n)) => n,
        (_, None, None) => {
            return Err(refuse(
                "run needs its count, never assumed: --iters N for alu or --reads N for bw",
            ));
        }
        _ => return Err(refuse("--iters goes with --workload alu, --reads with bw")),
    };
    Ok(Command::Run(Box::new(RunConfig {
        common,
        count,
        seconds: seconds.unwrap_or(DEFAULT_SECONDS),
        ready,
    })))
}

/// A whole number from `low` to `high`.
fn number(text: &str, low: u32, high: u32) -> Option<u32> {
    text.parse::<u32>()
        .ok()
        .filter(|n| (low..=high).contains(n))
}

/// [`number`], or a refusal that names the flag and the range.
fn whole(text: &str, low: u32, high: u32, flag: &str) -> Result<u32, String> {
    number(text, low, high)
        .ok_or_else(|| refuse(&format!("{flag} takes a whole number from {low} to {high}")))
}

/// A count `workload` takes (`Workload::valid_count`).
fn parse_count(text: &str, workload: Workload) -> Option<u32> {
    text.parse::<u32>()
        .ok()
        .filter(|&n| workload.valid_count(n))
}

/// A seed: decimal, or hex after `0x`.
fn parse_seed(text: &str) -> Option<u32> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) => {
            u32::from_str_radix(hex, 16).ok()
        }
        Some(_) => None,
        None if text.chars().all(|c| c.is_ascii_digit()) => text.parse::<u32>().ok(),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn run(line: &str) -> RunConfig {
        match parse(&args(line)) {
            Ok(Command::Run(config)) => *config,
            other => panic!("{line}: {other:?}"),
        }
    }

    fn calibrate(line: &str) -> CalibrateConfig {
        match parse(&args(line)) {
            Ok(Command::Calibrate(config)) => *config,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn run_takes_its_count_and_defaults_the_rest() {
        let r = run("run --monitor \\\\?\\DISPLAY#X#1 --workload alu --iters 3072");
        assert_eq!(r.common.monitor, "\\\\?\\DISPLAY#X#1");
        assert_eq!(r.common.workload, Workload::Alu);
        assert_eq!(r.count, 3_072);
        assert_eq!(r.common.texture, None);
        assert_eq!(
            (r.common.bands, r.common.buffers, r.common.latency),
            (16, 3, 2)
        );
        assert!(r.common.detail);
        assert!(!r.common.activate && !r.common.stop_on_stdin_eof);
        assert_eq!(
            (r.seconds, r.common.log.clone(), r.ready.clone()),
            (60, None, None)
        );
        // The bandwidth workload's texture defaults to 256 MiB from the default seed.
        let r = run("run --workload bw --reads 24 --monitor m");
        assert_eq!(r.count, 24);
        assert_eq!(r.common.texture, Texture::new(256, 0x4B4C_4731));
        // Every flag.
        let r = run(
            "run --monitor m --workload bw --reads 4096 --texture-mib 1 --seed 0xFF --bands 64 \
             --buffers 4 --latency 3 --detail off --seconds 90 --activate --log a.log \
             --ready r.json --stop-on-stdin-eof",
        );
        assert_eq!(r.count, 4_096);
        assert_eq!(r.common.texture, Texture::new(1, 255));
        assert_eq!(
            (r.common.bands, r.common.buffers, r.common.latency),
            (64, 4, 3)
        );
        assert!(!r.common.detail && r.common.activate && r.common.stop_on_stdin_eof);
        assert_eq!(r.seconds, 90);
        assert_eq!(r.common.log, Some(PathBuf::from("a.log")));
        assert_eq!(r.ready, Some(PathBuf::from("r.json")));
        // The smallest counts and settings.
        let r = run("run --monitor m --workload alu --iters 1 --bands 1 --buffers 2 --latency 1");
        assert_eq!((r.count, r.common.bands, r.common.buffers), (1, 1, 2));
        assert_eq!(
            run("run --monitor m --workload alu --iters 65536").count,
            65_536
        );
        assert_eq!(run("run --monitor m --workload bw --reads 4").count, 4);
    }

    #[test]
    fn seeds_are_decimal_or_hex() {
        let seed = |text: &str| {
            run(&format!(
                "run --monitor m --workload bw --reads 8 --seed {text}"
            ))
            .common
            .texture
            .unwrap()
            .seed()
        };
        assert_eq!(seed("0"), 0);
        assert_eq!(seed("1263290161"), 0x4B4C_4731);
        assert_eq!(seed("0x4b4c4731"), 0x4B4C_4731);
        assert_eq!(seed("0XFFFFFFFF"), u32::MAX);
        assert_eq!(seed("4294967295"), u32::MAX);
        for text in [
            "0x",
            "0x1g",
            "-1",
            "+5",
            "0x+5",
            "4294967296",
            "0x100000000",
            "1e3",
        ] {
            assert_eq!(parse_seed(text), None, "{text}");
        }
    }

    #[test]
    fn calibrate_finds_the_count_itself() {
        let c = calibrate("calibrate --monitor m --workload alu");
        assert_eq!(c.common.workload, Workload::Alu);
        assert_eq!(
            (c.target_ms, c.warmup_seconds, c.out.clone()),
            (8.0, 120, None)
        );
        assert_eq!(c.common.texture, None);
        let c = calibrate(
            "calibrate --monitor m --workload bw --target-ms 8.5 --warmup 0 --texture-mib 1024 \
             --seed 7 --bands 4 --buffers 2 --latency 1 --detail off --activate --out c.json \
             --log c.log --stop-on-stdin-eof",
        );
        assert_eq!((c.target_ms, c.warmup_seconds), (8.5, 0));
        assert_eq!(c.common.texture, Texture::new(1_024, 7));
        assert_eq!(c.out, Some(PathBuf::from("c.json")));
        assert_eq!(c.common.log, Some(PathBuf::from("c.log")));
        assert!(c.common.activate && c.common.stop_on_stdin_eof && !c.common.detail);
        assert_eq!(
            calibrate("calibrate --monitor m --workload alu --target-ms 1 --warmup 3600")
                .warmup_seconds,
            3_600
        );
        assert_eq!(
            calibrate("calibrate --monitor m --workload alu --target-ms 50").target_ms,
            50.0
        );
    }

    #[test]
    fn version_and_help_stand_alone() {
        assert_eq!(parse(&args("--version")), Ok(Command::Version));
        assert_eq!(parse(&args("-V")), Ok(Command::Version));
        assert_eq!(parse(&args("--help")), Ok(Command::Help));
        assert_eq!(parse(&args("-h")), Ok(Command::Help));
        assert_eq!(Command::Help.common(), None);
        let command = parse(&args("run --monitor m --workload alu --iters 4")).unwrap();
        assert_eq!(command.common().map(|c| c.workload), Some(Workload::Alu));
    }

    #[test]
    fn refuses_what_it_cant_do() {
        for line in [
            "",
            "--monitor m",
            "bench --monitor m",
            "--version extra",
            "run",
            "run --workload alu --iters 100",
            "run --monitor m --iters 100",
            "run --monitor m --workload alu",
            "run --monitor m --workload bw",
            "run --monitor m --workload alu --reads 4",
            "run --monitor m --workload bw --iters 4",
            "run --monitor m --workload alu --iters 10 --reads 4",
            "run --monitor m --workload gpu --iters 10",
            "run --monitor m --workload alu --iters 0",
            "run --monitor m --workload alu --iters 65537",
            "run --monitor m --workload alu --iters ten",
            "run --monitor m --workload alu --iters -4",
            "run --monitor m --workload bw --reads 6",
            "run --monitor m --workload bw --reads 0",
            "run --monitor m --workload bw --reads 4100",
            "run --monitor m --workload bw --reads 4 --texture-mib 0",
            "run --monitor m --workload bw --reads 4 --texture-mib 3",
            "run --monitor m --workload bw --reads 4 --texture-mib 2048",
            "run --monitor m --workload alu --iters 4 --texture-mib 256",
            "run --monitor m --workload alu --iters 4 --seed 7",
            "run --monitor m --workload bw --reads 4 --seed 0x",
            "run --monitor m --workload bw --reads 4 --seed 4294967296",
            "run --monitor m --workload alu --iters 4 --bands 0",
            "run --monitor m --workload alu --iters 4 --bands 65",
            "run --monitor m --workload alu --iters 4 --buffers 1",
            "run --monitor m --workload alu --iters 4 --buffers 5",
            "run --monitor m --workload alu --iters 4 --latency 0",
            "run --monitor m --workload alu --iters 4 --latency 4",
            "run --monitor m --workload alu --iters 4 --detail maybe",
            "run --monitor m --workload alu --iters 4 --seconds 0",
            "run --monitor m --workload alu --iters 4 --seconds 4294967296",
            "run --monitor m --workload alu --iters 4 --target-ms 8",
            "run --monitor m --workload alu --iters 4 --warmup 10",
            "run --monitor m --workload alu --iters 4 --out c.json",
            "run --monitor m --workload alu --iters 4 --log x --ready x",
            "run --monitor m --monitor n --workload alu --iters 4",
            "run --monitor --workload alu --iters 4",
            "run --monitor m --workload alu --iters 4 --activate --activate",
            "run --monitor m --workload alu --iters 4 --log",
            "run --monitor m --workload alu --iters 4 extra",
            "run --monitor m --workload alu --iters 4 --help",
            "run --monitor m --workload alu --iters 4 --oled",
            "calibrate --monitor m",
            "calibrate --workload alu",
            "calibrate --monitor m --workload alu --iters 4",
            "calibrate --monitor m --workload bw --reads 4",
            "calibrate --monitor m --workload alu --seconds 10",
            "calibrate --monitor m --workload alu --ready r.json",
            "calibrate --monitor m --workload alu --target-ms 0.5",
            "calibrate --monitor m --workload alu --target-ms 51",
            "calibrate --monitor m --workload alu --target-ms nan",
            "calibrate --monitor m --workload alu --target-ms inf",
            "calibrate --monitor m --workload alu --warmup 3601",
            "calibrate --monitor m --workload alu --log x --out x",
            "calibrate --monitor m --workload alu --texture-mib 256",
        ] {
            let message = parse(&args(line)).unwrap_err();
            assert!(
                message.contains("usage: kairo-loadgen"),
                "{line}: {message}"
            );
        }
        // A refusal never repeats a value given: a path stays off stderr.
        let message = parse(&args("run --monitor C:\\secret --workload alu")).unwrap_err();
        assert!(!message.contains("secret"), "{message}");
        let message = parse(&args("run --monitor m --workload alu --iters 4 C:\\x")).unwrap_err();
        assert!(!message.contains("C:\\x"), "{message}");
    }

    #[test]
    fn the_usage_text_names_every_flag_and_exit_code() {
        for flag in [
            "--monitor",
            "--workload",
            "--iters",
            "--reads",
            "--texture-mib",
            "--seed",
            "--bands",
            "--buffers",
            "--latency",
            "--detail",
            "--seconds",
            "--activate",
            "--log",
            "--ready",
            "--stop-on-stdin-eof",
            "--target-ms",
            "--warmup",
            "--out",
            "--version",
            "--help",
        ] {
            assert!(USAGE.contains(flag), "{flag}");
        }
        assert!(USAGE.contains("Exit codes: 0 ok, 2 usage"));
        assert!(USAGE.lines().all(|line| line.chars().count() <= 100));
    }
}
