// SPDX-License-Identifier: GPL-3.0-or-later
//! The command line (docs/research/16-verify-testbed/README.md §6.1, §6.6), behind `main`
//! so tests can run it whole.
//!
//! Exit codes for `check`: 0 pass (warnings allowed), 1 fail, 2 usage, 3 cannot evaluate (not
//! an MP4, no H.264 track, unreadable, a non-UTF-8 file name, or a required check this build
//! can't run), 4 internal (the report couldn't be written). For `mutate`: 0 written, 1 the change
//! doesn't apply to this clip, 2 usage (an output that names no file included), 3 the input can't
//! be read or rebuilt, 4 the output can't be written (it never replaces an existing file, and an
//! interrupted run leaves at most `OUT.partial`). For `synth`: 0 written, 2 usage (settings that
//! can't be rendered, a terminal as stdout, or an expect path that names a folder), 4 the frames
//! or the expect file couldn't be written (a closed pipe included; it never replaces an existing
//! expect file, and writes it only once every frame is out). Messages and reports name the file
//! only.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use kairo_core::testsig::Curve;

use crate::expect::Expect;
use crate::mutate::{MutateError, Op};
use crate::profile::{Plan, Profile};
use crate::synth::{Codec, Defect, Format, HdrSource, Synth};
use crate::{Clip, boxes};

/// The usage text.
#[cfg(feature = "dev-tools")]
pub const USAGE: &str =
    "usage: kairo-verify check <clip.mp4> --profile PROFILE [--gate m3] [--expect FILE]
                    [--cue FILE.wav] [--av-tight] [--voice-included]
                    [--decoder none|cli] [--ffmpeg PATH] [--json FILE|-] [--summary FILE|-]
       kairo-verify boxes <clip.mp4>
       kairo-verify mutate undo-faststart|add-elst|set-colr P T M R|remove-colr
                           |set-default TRACK|perturb-stts S D  IN OUT
       kairo-verify synth --preset 1080p60|1440p60|WxH@fps [--source-hz 180] [--seconds 30]
                          [--defect D] [--hdr natural|match --lw NITS --sdr-white NITS]
                          [--codec nv12|ipcm [--gop N]] --expect FILE    (frames on stdout)
       kairo-verify --version | --help
profiles: test capture smoke engine share discord whatsapp reels original
defects: drop[@K] dup[@K] judder corrupt-bars bars-sparse unreadable
The cli decoder (--ffmpeg PATH) decodes for the content checks; the linked decoder comes with
research 16's PR 6.";

/// The usage text.
#[cfg(not(feature = "dev-tools"))]
pub const USAGE: &str =
    "usage: kairo-verify check <clip.mp4> --profile PROFILE [--gate m3] [--expect FILE]
                    [--decoder none] [--json FILE|-] [--summary FILE|-]
       kairo-verify boxes <clip.mp4>
       kairo-verify mutate undo-faststart|add-elst|set-colr P T M R|remove-colr
                           |set-default TRACK|perturb-stts S D  IN OUT
       kairo-verify --version | --help
profiles: test capture smoke engine share discord whatsapp reels original
This build checks structure only: the content checks need a decoder, and the linked one comes
with research 16's PR 6.";

/// Where a report goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Output {
    Stdout,
    File(PathBuf),
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Check {
        clip: PathBuf,
        profile: Profile,
        gate_m3: bool,
        expect: Option<PathBuf>,
        ffmpeg: Option<PathBuf>,
        json: Option<Output>,
        summary: Option<Output>,
        cue: Option<PathBuf>,
        av_tight: bool,
        voice_included: bool,
    },
    Boxes(PathBuf),
    Mutate {
        op: Op,
        input: PathBuf,
        output: PathBuf,
    },
    Synth {
        synth: Synth,
        expect: PathBuf,
    },
    Version,
    Help,
}

fn output(value: &OsString) -> Output {
    if value == "-" {
        Output::Stdout
    } else {
        Output::File(PathBuf::from(value))
    }
}

/// A command-line word that is safe to echo in a message: a short token, never a path or a
/// piece of one, since output must not name folders (CLAUDE.md).
fn shown(word: &OsStr) -> String {
    match word.to_str() {
        Some(w) if safe_word(w) => format!("{w:?}"),
        _ => "(a word that may be a path, not shown)".to_owned(),
    }
}

/// Whether a word can't be a path or a piece of one: at most 32 bytes, with no `/`, `\\`, `:` or
/// control character.
fn safe_word(word: &str) -> bool {
    word.len() <= 32 && !word.contains(['/', '\\', ':']) && !word.chars().any(char::is_control)
}

/// Options README §6.1 documents that this build doesn't take yet, with what brings them.
const LATER_OPTIONS: [(&str, &str); 2] = [
    (
        "--testbed-log",
        "source.stability and the testbed and tone logs (research 16's PR 8)",
    ),
    (
        "--voice-log",
        "source.stability and the testbed and tone logs (research 16's PR 8)",
    ),
];

/// Parses the arguments, without the program name.
fn parse_args(args: &[OsString]) -> Result<Command, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(USAGE.to_owned());
    };
    match verb.to_str() {
        Some("--version" | "-V") if rest.is_empty() => return Ok(Command::Version),
        Some("--help" | "-h") if rest.is_empty() => return Ok(Command::Help),
        Some("boxes") => {
            return match rest {
                [clip] if !clip.is_empty() => Ok(Command::Boxes(PathBuf::from(clip))),
                _ => Err(format!("boxes takes one clip\n{USAGE}")),
            };
        }
        Some("mutate") => return parse_mutate(rest),
        Some("synth") => return parse_synth(rest),
        Some("check") => {}
        _ => return Err(format!("unknown command {}\n{USAGE}", shown(verb))),
    }
    let mut clip = None;
    let mut profile = None;
    let mut gate_m3 = false;
    let mut expect = None;
    let mut ffmpeg = None;
    // An explicit --decoder wins over --ffmpeg, in either order.
    let mut chosen: Option<bool> = None;
    let mut json = None;
    let mut summary = None;
    let mut cue = None;
    let mut av_tight = false;
    let mut voice_included = false;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let flag = word.to_str().unwrap_or_default();
        let mut value = |name: &str| {
            words
                .next()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match flag {
            "--profile" => {
                let name = value("--profile")?;
                profile = Some(
                    name.to_str()
                        .and_then(Profile::parse)
                        .ok_or_else(|| format!("unknown profile {}\n{USAGE}", shown(name)))?,
                );
            }
            "--gate" => match value("--gate")?.to_str() {
                Some("m3") => gate_m3 = true,
                _ => return Err(format!("--gate takes m3\n{USAGE}")),
            },
            "--expect" => expect = Some(PathBuf::from(value("--expect")?)),
            "--decoder" => match value("--decoder")?.to_str() {
                Some("none") => chosen = Some(false),
                Some("cli") if cfg!(feature = "dev-tools") => chosen = Some(true),
                Some("cli") => {
                    return Err("this build has no cli decoder (feature dev-tools)".to_owned());
                }
                Some("linked") => {
                    return Err("the linked decoder comes with research 16's PR 6; use --decoder cli --ffmpeg PATH".to_owned());
                }
                _ => return Err(format!("--decoder takes linked, cli or none\n{USAGE}")),
            },
            "--ffmpeg" if cfg!(feature = "dev-tools") => {
                ffmpeg = Some(PathBuf::from(value("--ffmpeg")?));
            }
            "--json" => json = Some(output(value("--json")?)),
            "--cue" => cue = Some(PathBuf::from(value("--cue")?)),
            "--av-tight" => av_tight = true,
            "--voice-included" => voice_included = true,
            "--summary" => summary = Some(output(value("--summary")?)),
            _ if let Some((_, with)) = LATER_OPTIONS.iter().find(|(f, _)| *f == flag) => {
                return Err(format!(
                    "{flag} comes with {with}, which this build doesn't have yet"
                ));
            }
            _ if flag.starts_with('-') && flag.len() > 1 => {
                // Only the part before any `=`: what follows may be a path.
                let name = OsStr::new(flag.split('=').next().unwrap_or_default());
                return Err(format!("unknown option {}\n{USAGE}", shown(name)));
            }
            _ if clip.is_none() && !word.is_empty() => clip = Some(PathBuf::from(word)),
            _ => {
                return Err(format!(
                    "check takes one clip; an extra argument was given\n{USAGE}"
                ));
            }
        }
    }
    let clip = clip.ok_or_else(|| format!("check needs a clip\n{USAGE}"))?;
    let cli_decoder = chosen.unwrap_or(ffmpeg.is_some());
    if cli_decoder && ffmpeg.is_none() {
        return Err(format!("--decoder cli needs --ffmpeg PATH\n{USAGE}"));
    }
    let profile = profile.ok_or_else(|| format!("check needs --profile\n{USAGE}"))?;
    if json == Some(Output::Stdout) && summary == Some(Output::Stdout) {
        return Err("--json - and --summary - can't both use stdout".to_owned());
    }
    Ok(Command::Check {
        clip,
        profile,
        gate_m3,
        expect,
        ffmpeg: ffmpeg.filter(|_| cli_decoder),
        json,
        summary,
        cue,
        av_tight,
        voice_included,
    })
}

/// A whole number from the command line.
fn number<T: FromStr>(word: &OsStr, what: &str) -> Result<T, String> {
    word.to_str()
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| format!("{what}, not {}\n{USAGE}", shown(word)))
}

/// `mutate OP [VALUES] IN OUT`.
fn parse_mutate(rest: &[OsString]) -> Result<Command, String> {
    let usage = || format!("mutate takes a change, its values, the input and the output\n{USAGE}");
    let Some((name, rest)) = rest.split_first() else {
        return Err(usage());
    };
    // How many values each change takes before IN and OUT.
    let count = match name.to_str() {
        Some("undo-faststart" | "add-elst" | "remove-colr") => 0,
        Some("set-default") => 1,
        Some("perturb-stts") => 2,
        Some("set-colr") => 4,
        _ => return Err(format!("unknown change {}\n{USAGE}", shown(name))),
    };
    let [values @ .., input, output] = rest else {
        return Err(usage());
    };
    if values.len() != count || input.is_empty() || output.is_empty() {
        return Err(usage());
    }
    let op = match (name.to_str(), values) {
        (Some("set-colr"), [p, t, m, r]) => {
            let code = "set-colr's P, T and M are codes from 0 to 65535";
            let full_range = match r.to_str() {
                Some("0") => false,
                Some("1") => true,
                _ => return Err(format!("set-colr's R is 0 or 1\n{USAGE}")),
            };
            Op::SetColr {
                primaries: number(p, code)?,
                transfer: number(t, code)?,
                matrix: number(m, code)?,
                full_range,
            }
        }
        (Some("set-default"), [title]) => {
            // Messages repeat the title, so it may not look like a path (CLAUDE.md).
            let title = title
                .to_str()
                .filter(|t| !t.is_empty() && safe_word(t))
                .ok_or_else(|| {
                    format!(
                        "set-default takes a track title (up to 32 bytes, no / \\ or :)\n{USAGE}"
                    )
                })?;
            Op::SetDefault(title.to_owned())
        }
        (Some("perturb-stts"), [sample, ticks]) => {
            let sample = number(sample, "perturb-stts's S is a sample number from 1")?;
            let ticks = number(ticks, "perturb-stts's D is a number of ticks from 1")?;
            if sample == 0 || ticks == 0 {
                return Err(format!("perturb-stts's S and D start at 1\n{USAGE}"));
            }
            Op::PerturbStts { sample, ticks }
        }
        (Some("undo-faststart"), []) => Op::UndoFaststart,
        (Some("add-elst"), []) => Op::AddElst,
        _ => Op::RemoveColr,
    };
    Ok(Command::Mutate {
        op,
        input: PathBuf::from(input),
        output: PathBuf::from(output),
    })
}

/// `synth`'s options (README §6.1).
fn parse_synth(rest: &[OsString]) -> Result<Command, String> {
    let mut format: Option<Format> = None;
    let mut source_hz = 180;
    let mut seconds = 30;
    let mut defect = None;
    let mut curve = None;
    let mut lw_nits = None;
    let mut sdr_white_nits = None;
    let mut ipcm = false;
    let mut gop = None;
    let mut expect = None;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let flag = word.to_str().unwrap_or_default();
        let mut value = |name: &str| {
            words
                .next()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        // A value that isn't UTF-8 parses as nothing; errors never repeat the value, which may
        // be a path.
        let text = |word: &OsStr| word.to_str().unwrap_or_default().to_owned();
        match flag {
            "--preset" => {
                format = Some(
                    text(value("--preset")?)
                        .parse()
                        .map_err(|e| format!("{e}\n{USAGE}"))?,
                );
            }
            "--source-hz" => {
                source_hz = number(value("--source-hz")?, "--source-hz takes a rate in Hz")?
            }
            "--seconds" => seconds = number(value("--seconds")?, "--seconds takes whole seconds")?,
            "--defect" => {
                defect = Some(
                    text(value("--defect")?)
                        .parse::<Defect>()
                        .map_err(|e| format!("{e}\n{USAGE}"))?,
                );
            }
            "--hdr" => match value("--hdr")?.to_str() {
                Some("natural") => curve = Some(Curve::Natural),
                Some("match") => curve = Some(Curve::MatchSdrApps),
                _ => return Err(format!("--hdr takes natural or match\n{USAGE}")),
            },
            "--lw" => lw_nits = Some(number(value("--lw")?, "--lw takes the peak in nits")?),
            "--sdr-white" => {
                sdr_white_nits = Some(number(value("--sdr-white")?, "--sdr-white takes nits")?);
            }
            "--codec" => match value("--codec")?.to_str() {
                Some("nv12") => ipcm = false,
                Some("ipcm") => ipcm = true,
                _ => return Err(format!("--codec takes nv12 or ipcm\n{USAGE}")),
            },
            "--gop" => gop = Some(number(value("--gop")?, "--gop takes a number of frames")?),
            "--expect" => expect = Some(PathBuf::from(value("--expect")?)),
            "--wav-dir" => {
                return Err(
                    "--wav-dir and the audio tracks come with research 16's PR 5; this build writes video only"
                        .to_owned(),
                );
            }
            _ if flag.starts_with('-') && flag.len() > 1 => {
                let name = OsStr::new(flag.split('=').next().unwrap_or_default());
                return Err(format!("unknown option {}\n{USAGE}", shown(name)));
            }
            _ => {
                return Err(format!(
                    "synth takes no file; the frames go to stdout\n{USAGE}"
                ));
            }
        }
    }
    let format = format.ok_or_else(|| format!("synth needs --preset\n{USAGE}"))?;
    let expect = expect.ok_or_else(|| format!("synth needs --expect FILE\n{USAGE}"))?;
    let hdr = match (curve, lw_nits, sdr_white_nits) {
        (None, None, None) => None,
        (Some(curve), Some(lw_nits), Some(sdr_white_nits)) => Some(HdrSource {
            curve,
            lw_nits,
            sdr_white_nits,
        }),
        _ => return Err(format!("--hdr, --lw and --sdr-white go together\n{USAGE}")),
    };
    let codec = match (ipcm, gop) {
        (true, gop) => Codec::Ipcm {
            gop: gop.unwrap_or(format.fps),
        },
        (false, None) => Codec::Nv12,
        (false, Some(_)) => return Err(format!("--gop goes with --codec ipcm\n{USAGE}")),
    };
    let synth = Synth {
        format,
        source_hz,
        seconds,
        defect,
        hdr,
        codec,
    };
    synth.check().map_err(|e| e.to_string())?;
    Ok(Command::Synth { synth, expect })
}

/// The file's name for messages and reports, or `None` if it isn't UTF-8.
fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

fn say(err: &mut dyn Write, text: &str) {
    let _ = writeln!(err, "kairo-verify: {text}");
}

fn write_to(output: &Output, text: &str, out: &mut dyn Write) -> std::io::Result<()> {
    match output {
        Output::Stdout => out.write_all(text.as_bytes()),
        Output::File(path) => std::fs::write(path, text),
    }
}

fn check(command: Command, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Command::Check {
        clip,
        profile,
        gate_m3,
        expect,
        ffmpeg,
        json,
        summary,
        cue,
        av_tight,
        voice_included,
    } = command
    else {
        return 4;
    };
    let Some(name) = file_name(&clip).map(str::to_owned) else {
        say(err, "the clip's file name must be UTF-8");
        return 3;
    };
    let expect = match expect {
        None => None,
        Some(path) => match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| Expect::parse(&t).map_err(|e| e.to_string()))
        {
            Ok(expect) => Some(expect),
            Err(error) => {
                say(
                    err,
                    &format!("{}: {error}", file_name(&path).unwrap_or("expect file")),
                );
                return 2;
            }
        },
    };
    let mut plan = match Plan::new(profile, gate_m3, expect) {
        Ok(plan) => plan,
        Err(error) => {
            say(err, &error.to_string());
            return 2;
        }
    };
    plan.av_tight = av_tight;
    plan.voice_included = voice_included;
    if let Some(path) = cue {
        let name = file_name(&path).unwrap_or("the cue file");
        match read_cue(&path) {
            Ok(samples) => plan.cue = Some(samples),
            Err(error) => {
                say(err, &format!("{name}: {error}"));
                return 2;
            }
        }
    }
    let mut decoder = make_decoder(ffmpeg, &clip);
    let report = match File::open(&clip) {
        Ok(mut file) => crate::check_with(
            &mut file,
            &name,
            &plan,
            decoder
                .as_mut()
                .map(|d| d as &mut dyn crate::decode::Decoder),
        ),
        Err(error) => {
            say(err, &format!("{name}: {error}"));
            return 3;
        }
    };
    // Without options the summary goes to stdout, unless the JSON does.
    let summary = summary.or(match json {
        Some(Output::Stdout) => None,
        _ => Some(Output::Stdout),
    });
    for (target, text) in [(json, report.to_json() + "\n"), (summary, report.to_text())] {
        if let Some(target) = target
            && let Err(error) = write_to(&target, &text, out)
        {
            say(err, &format!("can't write the report: {error}"));
            return 4;
        }
    }
    report.verdict.exit_code()
}

/// `--cue`'s PCM as (L+R)/2: a 48 kHz WAV of at most 5 s.
fn read_cue(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let wav = crate::wav::read(&bytes)?;
    if wav.rate != 48_000 || !(1..=2).contains(&wav.channels) {
        return Err(format!(
            "{} Hz, {} channels; the cue must be 48 kHz mono or stereo",
            wav.rate, wav.channels
        ));
    }
    let mut mono = Vec::new();
    crate::analyze::audio::downmix(&wav.samples, usize::from(wav.channels), &mut mono);
    if mono.is_empty() || mono.len() > 5 * 48_000 {
        return Err("the cue must last between one sample and 5 s".to_owned());
    }
    Ok(mono)
}

/// The decoder `--ffmpeg` asks for.
#[cfg(feature = "dev-tools")]
fn make_decoder(ffmpeg: Option<PathBuf>, clip: &Path) -> Option<crate::decode::cli::Cli> {
    ffmpeg.map(|program| crate::decode::cli::Cli::new(program, clip))
}

#[cfg(not(feature = "dev-tools"))]
fn make_decoder(_: Option<PathBuf>, _: &Path) -> Option<NoDecoder> {
    None
}

/// The decoder a build without `dev-tools` never has.
#[cfg(not(feature = "dev-tools"))]
enum NoDecoder {}

#[cfg(not(feature = "dev-tools"))]
impl crate::decode::Decoder for NoDecoder {
    fn kind(&self) -> &'static str {
        match *self {}
    }

    fn video(
        &mut self,
        _: &mut dyn FnMut(usize, &crate::decode::Planes<'_>) -> std::ops::ControlFlow<()>,
    ) -> Result<crate::decode::DecodeStats, crate::decode::DecodeError> {
        match *self {}
    }
}

fn show_boxes(path: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Some(name) = file_name(path).map(str::to_owned) else {
        say(err, "the clip's file name must be UTF-8");
        return 3;
    };
    let clip = match File::open(path)
        .map_err(|e| e.to_string())
        .and_then(|mut f| Clip::read(&mut f).map_err(|e| e.to_string()))
    {
        Ok(clip) => clip,
        Err(error) => {
            say(err, &format!("{name}: {error}"));
            return 3;
        }
    };
    let text = format!(
        "{name}: {} bytes\n{}",
        clip.mp4.file_len,
        boxes::tree(&clip)
    );
    match out.write_all(text.as_bytes()) {
        Ok(()) => 0,
        Err(_) => 4,
    }
}

fn mutate_file(
    op: &Op,
    input: &Path,
    output: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let Some(in_name) = file_name(input) else {
        say(err, "the input must be a file with a UTF-8 name");
        return 3;
    };
    let Some(out_name) = file_name(output) else {
        say(err, "the output must be a file with a UTF-8 name");
        return 2;
    };
    let mut source = match File::open(input) {
        Ok(file) => file,
        Err(error) => {
            say(err, &format!("{in_name}: {error}"));
            return 3;
        }
    };
    // Never over an existing file, the input included.
    if output.symlink_metadata().is_ok() {
        say(
            err,
            &format!("{out_name}: already exists; mutate never replaces a file"),
        );
        return 4;
    }
    // Written under a temporary name first, then linked to the output's name, which fails if
    // something took it meanwhile: an interrupted run never leaves a cut clip under that name.
    let mut partial = output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let file = match File::create_new(&partial) {
        Ok(file) => file,
        Err(error) => {
            say(err, &format!("{out_name}.partial: {error}"));
            return 4;
        }
    };
    let mut writer = BufWriter::new(file);
    let result = crate::mutate::mutate(&mut source, op, &mut writer)
        .and_then(|()| writer.flush().map_err(MutateError::Write));
    drop(writer);
    let result =
        result.and_then(|()| std::fs::hard_link(&partial, output).map_err(MutateError::Write));
    let _ = std::fs::remove_file(&partial);
    match result {
        Ok(()) => {
            let _ = writeln!(out, "{out_name}: {in_name} with {op}");
            0
        }
        Err(error) => {
            let name = if matches!(error, MutateError::Write(_)) {
                out_name
            } else {
                in_name
            };
            say(err, &format!("{name}: {error}"));
            error.exit_code()
        }
    }
}

/// `synth`: the frames to `out`, then the expect file, which is written under a temporary name
/// and linked to its own only once every frame is out.
fn synth_file(
    synth: &Synth,
    expect: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
    stdout_is_terminal: bool,
) -> u8 {
    if stdout_is_terminal {
        say(
            err,
            "synth writes binary frames to stdout: pipe them to an encoder or redirect them to a file",
        );
        return 2;
    }
    // A file, not a folder: a trailing separator, `.`, `..` or an existing folder is refused
    // without repeating it, since its name would be a folder's (CLAUDE.md).
    let names_folder =
        expect.as_os_str().to_string_lossy().ends_with(['/', '\\']) || expect.is_dir();
    let Some(name) = file_name(expect).filter(|_| !names_folder) else {
        say(err, "the expect file must name a file, with a UTF-8 name");
        return 2;
    };
    if expect.symlink_metadata().is_ok() {
        say(
            err,
            &format!("{name}: already exists; synth never replaces a file"),
        );
        return 4;
    }
    let folder = expect
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !folder.is_dir() {
        say(err, &format!("{name}: its folder doesn't exist"));
        return 4;
    }
    // The frames first; then the expect file, written under a temporary name and linked to its
    // own, so an interrupted run leaves nothing behind but, for the moment it takes to write a
    // few hundred bytes, the temporary file.
    if let Err(error) = synth.write(out) {
        say(err, &error.to_string());
        return error.exit_code();
    }
    let mut partial = expect.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let mut file = match File::create_new(&partial) {
        Ok(file) => file,
        Err(error) => {
            say(err, &format!("{name}.partial: {error}"));
            return 4;
        }
    };
    let result = serde_json::to_string_pretty(&synth.expect())
        .map_err(|e| (4, format!("{name}: {e}")))
        .and_then(|text| {
            file.write_all(format!("{text}\n").as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| (4, format!("{name}: {e}")))
        });
    drop(file);
    let result = result.and_then(|()| {
        std::fs::hard_link(&partial, expect).map_err(|e| (4, format!("{name}: {e}")))
    });
    let _ = std::fs::remove_file(&partial);
    match result {
        Ok(()) => {
            say(err, &format!("synth: {synth}; the expect file is {name}"));
            0
        }
        Err((code, message)) => {
            say(err, &message);
            code
        }
    }
}

/// Runs the command line `args` (without the program name), writing to `out` and `err`, and
/// returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    run_with(args, out, err, false)
}

/// [`run`], told whether stdout is a terminal, which `synth` refuses to write frames to.
pub fn run_with(
    args: &[OsString],
    out: &mut dyn Write,
    err: &mut dyn Write,
    stdout_is_terminal: bool,
) -> u8 {
    match parse_args(args) {
        Ok(Command::Version) => {
            let _ = writeln!(out, concat!("kairo-verify ", env!("CARGO_PKG_VERSION")));
            0
        }
        Ok(Command::Help) => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        Ok(Command::Boxes(path)) => show_boxes(&path, out, err),
        Ok(Command::Mutate { op, input, output }) => mutate_file(&op, &input, &output, out, err),
        Ok(Command::Synth { synth, expect }) => {
            synth_file(&synth, &expect, out, err, stdout_is_terminal)
        }
        Ok(command) => check(command, out, err),
        Err(message) => {
            let _ = writeln!(err, "{message}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<OsString> {
        line.split_whitespace().map(OsString::from).collect()
    }

    #[test]
    fn parses_the_command_line() {
        assert_eq!(parse_args(&args("--version")), Ok(Command::Version));
        assert_eq!(parse_args(&args("-h")), Ok(Command::Help));
        assert_eq!(
            parse_args(&args("boxes a.mp4")),
            Ok(Command::Boxes(PathBuf::from("a.mp4")))
        );
        assert_eq!(
            parse_args(&args(
                "check a.mp4 --profile engine --gate m3 --json - --summary s.txt"
            )),
            Ok(Command::Check {
                clip: PathBuf::from("a.mp4"),
                profile: Profile::Engine,
                gate_m3: true,
                expect: None,
                ffmpeg: None,
                json: Some(Output::Stdout),
                summary: Some(Output::File(PathBuf::from("s.txt"))),
                cue: None,
                av_tight: false,
                voice_included: false,
            })
        );
        assert!(matches!(
            parse_args(&args(
                "check a.mp4 --profile engine --cue c.wav --av-tight --voice-included"
            )),
            Ok(Command::Check {
                cue: Some(_),
                av_tight: true,
                voice_included: true,
                ..
            })
        ));
        let mutate = |line: &str, op: Op| {
            assert_eq!(
                parse_args(&args(line)),
                Ok(Command::Mutate {
                    op,
                    input: PathBuf::from("in.mp4"),
                    output: PathBuf::from("out.mp4"),
                }),
                "{line}"
            );
        };
        // The count of values is checked first, so a missing one says so.
        for line in [
            "mutate set-colr 1 1 1 a.mp4 b.mp4",
            "mutate perturb-stts 100 a.mp4 b.mp4",
        ] {
            let message = parse_args(&args(line)).unwrap_err();
            assert!(
                message.starts_with("mutate takes a change, its values"),
                "{line}: {message}"
            );
        }
        let message = parse_args(&args("mutate set-default /x/in.mp4 a.mp4 b.mp4")).unwrap_err();
        assert!(
            message.starts_with("set-default takes a track title") && !message.contains("/x/"),
            "{message}"
        );
        mutate("mutate undo-faststart in.mp4 out.mp4", Op::UndoFaststart);
        mutate("mutate add-elst in.mp4 out.mp4", Op::AddElst);
        mutate("mutate remove-colr in.mp4 out.mp4", Op::RemoveColr);
        mutate(
            "mutate set-colr 6 6 6 1 in.mp4 out.mp4",
            Op::SetColr {
                primaries: 6,
                transfer: 6,
                matrix: 6,
                full_range: true,
            },
        );
        mutate(
            "mutate set-default Game in.mp4 out.mp4",
            Op::SetDefault("Game".to_owned()),
        );
        mutate(
            "mutate perturb-stts 100 128 in.mp4 out.mp4",
            Op::PerturbStts {
                sample: 100,
                ticks: 128,
            },
        );
        assert_eq!(
            parse_args(&args(
                "synth --preset 480x270@60 --seconds 4 --codec ipcm --expect e.json"
            )),
            Ok(Command::Synth {
                synth: Synth {
                    format: Format {
                        width: 480,
                        height: 270,
                        fps: 60
                    },
                    source_hz: 180,
                    seconds: 4,
                    defect: None,
                    hdr: None,
                    codec: Codec::Ipcm { gop: 60 },
                },
                expect: PathBuf::from("e.json"),
            })
        );
        assert!(matches!(
            parse_args(&args(
                "synth --preset 1080p60 --defect drop@930 --hdr natural --lw 1000 --sdr-white 240 --source-hz 360 --expect e.json"
            )),
            Ok(Command::Synth {
                synth: Synth {
                    defect: Some(Defect::Drop(Some(930))),
                    hdr: Some(HdrSource {
                        curve: Curve::Natural,
                        lw_nits: 1000,
                        sdr_white_nits: 240
                    }),
                    source_hz: 360,
                    seconds: 30,
                    codec: Codec::Nv12,
                    ..
                },
                ..
            })
        ));
        assert!(matches!(
            parse_args(&args(
                "check --profile test --expect e.json --decoder none c.mp4"
            )),
            Ok(Command::Check {
                expect: Some(_),
                ..
            })
        ));
        for bad in [
            "",
            "check",
            "check a.mp4",
            "check a.mp4 --profile clip",
            "check a.mp4 --profile engine --gate m4",
            "check a.mp4 --profile engine --decoder cli",
            "check a.mp4 --profile engine --decoder linked --ffmpeg f",
            "check a.mp4 --profile engine --json - --summary -",
            "check a.mp4 b.mp4 --profile engine",
            "check a.mp4 --profile engine --frobnicate",
            "check a.mp4 --profile",
            "boxes",
            "boxes a b",
            "mutate",
            "mutate add-elst",
            "mutate add-elst a.mp4",
            "mutate add-elst a.mp4 b.mp4 c.mp4",
            "mutate frobnicate a.mp4 b.mp4",
            "mutate set-colr 1 1 1 a.mp4 b.mp4",
            "mutate set-colr 1 1 1 2 a.mp4 b.mp4",
            "mutate set-colr 1 1 70000 0 a.mp4 b.mp4",
            "mutate set-colr -1 1 1 0 a.mp4 b.mp4",
            "mutate set-default a.mp4 b.mp4",
            "mutate perturb-stts 0 128 a.mp4 b.mp4",
            "mutate perturb-stts 100 0 a.mp4 b.mp4",
            "mutate perturb-stts 100 a.mp4 b.mp4",
            "mutate set-default /x/in.mp4 a.mp4 b.mp4",
            "mutate set-default C:\\x\\in.mp4 a.mp4 b.mp4",
            "mutate set-default a-very-long-title-of-more-than-32-bytes a.mp4 b.mp4",
            "mutate add-elst a.mp4",
            "synth --preset 1080p60",
            "synth --expect e.json",
            "synth --preset 1080p60 --expect e.json --gop 60",
            "synth --preset 1080p60 --expect e.json --hdr natural",
            "synth --preset 1080p60 --expect e.json --lw 1000 --sdr-white 240",
            "synth --preset 1080p60 --expect e.json --hdr pq --lw 1000 --sdr-white 240",
            "synth --preset 1080p60 --expect e.json --defect frobnicate",
            "synth --preset 1080p60 --expect e.json --defect judder --source-hz 165",
            "synth --preset 1080p60 --expect e.json --wav-dir .",
            "synth --preset 1080p60 --expect e.json --codec h265",
            "synth --preset 1080p60 --expect e.json --seconds 0",
            "synth --preset 1081x1080@60 --expect e.json",
            "synth --preset 1080p60 --expect e.json extra",
            "--version extra",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
