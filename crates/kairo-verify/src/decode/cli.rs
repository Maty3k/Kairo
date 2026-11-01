// SPDX-License-Identifier: GPL-3.0-or-later
//! The CLI decoder (feature `dev-tools`): it spawns the FFmpeg it is given and reads the
//! pictures from its pipe (docs/research/16-verify-testbed/README.md §3.3). Development and
//! tests only, never shipped: the FFmpeg is spawned, never linked, so its licence (Homebrew's is
//! GPL) doesn't touch Kairo's (README §2 item 18).
//!
//! It runs, in the clip's folder so that FFmpeg's messages name the file only:
//! `ffmpeg -v warning -nostdin -protocol_whitelist file,pipe -err_detect
//! +crccheck+bitstream+buffer -ignore_editlist 1 -c:v h264 -i file:<file name> -map 0:v:0
//! -fps_mode passthrough -enc_time_base:v demux -pix_fmt yuv420p -f yuv4mpegpipe -`, and for
//! audio track N `… -c:a aac -i file:<file name> -map 0:a:N -f f32le -` at the track's own
//! channel count. YUV4MPEG2 states the picture size in its header, and its frames are packed
//! yuv420p. No filter runs: it only decodes. Any line FFmpeg prints counts as a decode error,
//! warnings included: FFmpeg reports a frame it had to conceal (the flags the linked backend
//! reads) only as the warning `corrupt decoded frame`. The pipe keeps the clip's time base, so
//! pictures less than a frame apart don't trip FFmpeg's warnings about its own output.
//! A relative `--ffmpeg` path is taken from the caller's folder, not the clip's.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};

use super::{DecodeError, DecodeStats, Decoder, Picture, Planes};

/// The longest YUV4MPEG2 header or frame line read.
const MAX_LINE: u64 = 1_024;

/// The largest picture accepted: 16,384 px a side, as pattern v1 allows.
const MAX_SIDE: usize = 16_384;

/// Sample frames handed over at a time.
const AUDIO_CHUNK: usize = 4_096;

/// The YUV4MPEG2 chroma tags of 8-bit 4:2:0.
const CHROMA_420: [&str; 4] = ["420", "420jpeg", "420mpeg2", "420paldv"];

/// What reads FFmpeg's stdout: it returns how many frames it handed over and whether it
/// stopped early.
type ReadOutput<'a> =
    dyn FnMut(&mut BufReader<ChildStdout>) -> Result<(usize, bool), DecodeError> + 'a;

/// A decoder that spawns FFmpeg.
#[derive(Debug, Clone)]
pub struct Cli {
    ffmpeg: PathBuf,
    clip: PathBuf,
}

impl Cli {
    /// Decodes `clip` with the FFmpeg program at `ffmpeg`: a bare name is looked up on the
    /// `PATH`, and a relative path is taken from the current folder.
    pub fn new(ffmpeg: impl Into<PathBuf>, clip: impl Into<PathBuf>) -> Self {
        let ffmpeg = ffmpeg.into();
        let ffmpeg = if ffmpeg.is_relative() && ffmpeg.components().count() > 1 {
            std::path::absolute(&ffmpeg).unwrap_or(ffmpeg)
        } else {
            ffmpeg
        };
        Self {
            ffmpeg,
            clip: clip.into(),
        }
    }

    /// The clip's folder, where FFmpeg runs.
    fn folder(&self) -> PathBuf {
        self.clip
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
    }

    /// The command: the decoder `codec` forces (`-c:v h264`), then `output`.
    fn command(&self, codec: [&str; 2], output: &[&str]) -> Result<Command, DecodeError> {
        let name = self
            .clip
            .file_name()
            .ok_or_else(|| DecodeError::Start("the clip has no file name".to_owned()))?;
        // `file:` so that a name like `take:2.mp4` isn't read as a protocol.
        let mut input = OsString::from("file:");
        input.push(name);
        let mut command = Command::new(&self.ffmpeg);
        command
            .current_dir(self.folder())
            .args([
                "-v",
                "warning",
                "-nostdin",
                "-protocol_whitelist",
                "file,pipe",
                "-err_detect",
                "+crccheck+bitstream+buffer",
                "-ignore_editlist",
                "1",
            ])
            .args(codec)
            .arg("-i")
            .arg(input)
            .args(output)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(command)
    }

    /// Runs FFmpeg with `codec` and `output`, letting `read` take its stdout: `read` returns how
    /// many frames it handed over and whether it stopped early. Every message line is a decode
    /// error, and so is a failed exit; an FFmpeg that fails before giving anything didn't start.
    fn run(
        &self,
        codec: [&str; 2],
        output: &[&str],
        read: &mut ReadOutput<'_>,
    ) -> Result<DecodeStats, DecodeError> {
        let mut child = self
            .command(codec, output)?
            .spawn()
            .map_err(|e| DecodeError::Start(format!("FFmpeg: {e}")))?;
        let (Some(stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DecodeError::Start("FFmpeg's pipes".to_owned()));
        };
        // FFmpeg's messages are read on their own thread, so a full stderr pipe can't stall it.
        let messages = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        });
        let mut reader = BufReader::new(stdout);
        let read = read(&mut reader);
        // Killed while its stdout is still open, so a stop doesn't make it print broken-pipe
        // errors; the exit status of a kill asked for here says nothing about the clip.
        let stopped = matches!(read, Ok((_, true)));
        if stopped || read.is_err() {
            let _ = child.kill();
        }
        let status = child.wait();
        drop(reader);
        let text = messages.join().unwrap_or_default();
        let lines: Vec<String> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| self.sanitize(l))
            .collect();
        let last = lines.last().cloned();
        let (frames, _) = read.map_err(|e| match (e, &last) {
            (DecodeError::Read(what), Some(line)) => {
                DecodeError::Read(format!("{what}; FFmpeg: {line}"))
            }
            (e, _) => e,
        })?;
        let failed = !stopped && status.as_ref().is_ok_and(|s| !s.success());
        if frames == 0 && failed {
            return Err(DecodeError::Start(format!(
                "FFmpeg failed before decoding anything: {}",
                last.as_deref().unwrap_or("no message")
            )));
        }
        let mut stats = DecodeStats {
            frames,
            errors: lines.len(),
            last_error: last,
        };
        if let Ok(status) = status
            && failed
            && stats.errors == 0
        {
            stats.errors = 1;
            stats.last_error = Some(format!("FFmpeg ended with {status}"));
        }
        Ok(stats)
    }

    /// A message line without the clip's folder, the FFmpeg program's path or any other path:
    /// reports name files only (README §3.3, CLAUDE.md).
    fn sanitize(&self, line: &str) -> String {
        let mut text = line.trim().to_owned();
        for path in [absolute(&self.folder()), absolute(&self.ffmpeg)] {
            if let Some(path) = path.to_str().filter(|p| p.len() > 1) {
                text = text.replace(path, "…");
            }
        }
        text.split(' ')
            .map(|word| if looks_like_path(word) { "…" } else { word })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The licence of the FFmpeg it spawns, from `ffmpeg -L`: for the report, since a spawned
    /// program's licence doesn't bind Kairo (README §2 item 18).
    pub fn licence(&self) -> Option<&'static str> {
        let output = Command::new(&self.ffmpeg)
            .args(["-hide_banner", "-L"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("Lesser General Public License") {
            Some("LGPL")
        } else if text.contains("General Public License") {
            Some("GPL")
        } else {
            None
        }
    }
}

/// `path` made absolute, or as it is.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether a word of a message looks like an absolute path: `/…`, `X:\…`, `X:/…` or `\\…`,
/// perhaps quoted or followed by punctuation.
fn looks_like_path(word: &str) -> bool {
    let word = word.trim_start_matches(['\'', '"', '(', '[']);
    let bytes = word.as_bytes();
    word.starts_with('/') && word.len() > 1
        || word.starts_with("\\\\")
        || (bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

impl Decoder for Cli {
    fn kind(&self) -> &'static str {
        "cli"
    }

    fn licence(&mut self) -> Option<&'static str> {
        Cli::licence(self)
    }

    fn video(
        &mut self,
        each: &mut dyn FnMut(usize, &Planes<'_>) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError> {
        // The clip's own time base on the pipe: in the pipe's 1/fps, two pictures less than a
        // frame apart (a VFR clip) would share a timestamp, and FFmpeg's warnings about it
        // would read as decode errors.
        let output = [
            "-map",
            "0:v:0",
            "-fps_mode",
            "passthrough",
            "-enc_time_base:v",
            "demux",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "yuv4mpegpipe",
            "-",
        ];
        self.run(["-c:v", "h264"], &output, &mut |reader| {
            read_y4m(reader, each)
        })
    }

    fn audio(
        &mut self,
        track: usize,
        channels: usize,
        each: &mut dyn FnMut(&[f32]) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError> {
        let map = format!("0:a:{track}");
        let output = ["-map", map.as_str(), "-f", "f32le", "-"];
        self.run(["-c:a", "aac"], &output, &mut |reader| {
            read_f32(reader, channels, each)
        })
    }
}

/// Hands little-endian f32 samples, interleaved at `channels`, to `each` in chunks of whole
/// frames; returns the frames read and whether `each` stopped early.
fn read_f32(
    reader: &mut impl Read,
    channels: usize,
    each: &mut dyn FnMut(&[f32]) -> ControlFlow<()>,
) -> Result<(usize, bool), DecodeError> {
    let frame = 4 * channels.max(1);
    let mut bytes = vec![0_u8; AUDIO_CHUNK * frame];
    let mut samples = Vec::with_capacity(AUDIO_CHUNK * channels.max(1));
    let (mut filled, mut frames) = (0, 0);
    loop {
        let n = match reader.read(&mut bytes[filled..]) {
            Ok(n) => n,
            // A signal during the read: read again, as read_exact does for the pictures.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(DecodeError::Read(e.to_string())),
        };
        filled += n;
        let whole = filled / frame * frame;
        if whole > 0 && (n == 0 || filled == bytes.len()) {
            samples.clear();
            let (words, _) = bytes[..whole].as_chunks::<4>();
            samples.extend(words.iter().map(|&w| f32::from_le_bytes(w)));
            frames += whole / frame;
            bytes.copy_within(whole..filled, 0);
            filled -= whole;
            if each(&samples).is_break() {
                return Ok((frames, true));
            }
        }
        if n == 0 {
            break;
        }
    }
    if filled > 0 {
        return Err(DecodeError::Read(
            "the audio ends in the middle of a sample frame".to_owned(),
        ));
    }
    Ok((frames, false))
}

/// Reads one line, up to and without its newline, or `None` at the end of the stream.
fn line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, DecodeError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| DecodeError::Read(e.to_string()))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.pop() != Some(b'\n') {
        return Err(DecodeError::Read(
            "a YUV4MPEG2 line is cut short or too long".to_owned(),
        ));
    }
    Ok(Some(bytes))
}

/// The width and height a YUV4MPEG2 header gives, if its chroma is 4:2:0.
fn header(bytes: &[u8]) -> Result<(usize, usize), DecodeError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| DecodeError::Read("the YUV4MPEG2 header isn't text".to_owned()))?;
    let mut fields = text.split(' ');
    if fields.next() != Some("YUV4MPEG2") {
        return Err(DecodeError::Read("not a YUV4MPEG2 stream".to_owned()));
    }
    let (mut width, mut height, mut chroma) = (None, None, "420jpeg");
    for field in fields {
        match field.split_at_checked(1) {
            Some(("W", v)) => width = v.parse::<usize>().ok(),
            Some(("H", v)) => height = v.parse::<usize>().ok(),
            Some(("C", v)) => chroma = v,
            _ => {}
        }
    }
    if !CHROMA_420.contains(&chroma) {
        return Err(DecodeError::Unsupported(format!(
            "YUV4MPEG2 chroma {chroma}, not 4:2:0"
        )));
    }
    match (width, height) {
        (Some(w), Some(h)) if (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h) => {
            Ok((w, h))
        }
        _ => Err(DecodeError::Read(
            "the YUV4MPEG2 header has no usable size".to_owned(),
        )),
    }
}

/// Hands each frame of a YUV4MPEG2 stream to `each`; returns the frames read and whether
/// `each` stopped early.
fn read_y4m(
    reader: &mut impl BufRead,
    each: &mut dyn FnMut(usize, &Planes<'_>) -> ControlFlow<()>,
) -> Result<(usize, bool), DecodeError> {
    let Some(first) = line(reader)? else {
        // FFmpeg printed nothing: its messages say why.
        return Ok((0, false));
    };
    let (width, height) = header(&first)?;
    let mut bytes = vec![0_u8; Picture::packed_len(width, height)];
    let mut index = 0;
    while let Some(marker) = line(reader)? {
        if !marker.starts_with(b"FRAME") {
            return Err(DecodeError::Read(
                "a YUV4MPEG2 frame without its FRAME line".to_owned(),
            ));
        }
        reader
            .read_exact(&mut bytes)
            .map_err(|e| DecodeError::Read(format!("frame {index}: {e}")))?;
        let (y, chroma) = bytes.split_at(width * height);
        let (cb, cr) = chroma.split_at(chroma.len() / 2);
        let (cw, _) = (width.div_ceil(2), height.div_ceil(2));
        let planes = Planes::new((width, height), (y, width), (cb, cr, cw))?;
        if each(index, &planes).is_break() {
            return Ok((index + 1, true));
        }
        index += 1;
    }
    Ok((index, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(frames: usize) -> Vec<u8> {
        let mut bytes = b"YUV4MPEG2 W4 H2 F60:1 Ip A1:1 C420jpeg XYSCSS=420JPEG\n".to_vec();
        for k in 0..frames {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend(std::iter::repeat_n(k as u8, 8));
            bytes.extend([128; 4]);
        }
        bytes
    }

    #[test]
    fn reads_yuv4mpeg2_frames() {
        let mut lumas = Vec::new();
        let (frames, stopped) = read_y4m(&mut &stream(3)[..], &mut |i, p| {
            lumas.push((i, p.y_row(1)[3]));
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!((frames, stopped), (3, false));
        assert_eq!(lumas, [(0, 0), (1, 1), (2, 2)]);
        // The callback may stop it.
        let (frames, stopped) =
            read_y4m(&mut &stream(3)[..], &mut |_, _| ControlFlow::Break(())).unwrap();
        assert_eq!((frames, stopped), (1, true));
        // No output at all is no frames.
        assert_eq!(
            read_y4m(&mut &b""[..], &mut |_, _| ControlFlow::Continue(())).unwrap(),
            (0, false)
        );
    }

    #[test]
    fn refuses_damaged_or_foreign_streams() {
        let go = &mut |_: usize, _: &Planes<'_>| ControlFlow::Continue(());
        let mut cut = stream(2);
        cut.truncate(cut.len() - 3);
        assert!(read_y4m(&mut &cut[..], go).is_err());
        for header in [
            &b"YUV4MPEG2 W4 H2 C444\n"[..],
            b"YUV4MPEG2 W0 H2\n",
            b"YUV4MPEG2 H2\n",
            b"RIFF\n",
            b"YUV4MPEG2 W4 H2",
        ] {
            assert!(read_y4m(&mut &header[..], go).is_err(), "{header:?}");
        }
        let mut no_marker = stream(1);
        let at = no_marker.windows(5).position(|w| w == b"FRAME").unwrap();
        no_marker.splice(at..at + 5, *b"FRAMX");
        assert!(read_y4m(&mut &no_marker[..], go).is_err());
    }

    /// A folder with a fake `ffmpeg` that runs `body`, removed when dropped.
    #[cfg(unix)]
    struct Fake(PathBuf);

    #[cfg(unix)]
    impl Fake {
        fn new(name: &str, body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir()
                .join(format!("kairo-verify-fake-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let program = dir.join("ffmpeg");
            std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(dir)
        }

        fn cli(&self) -> Cli {
            Cli::new(self.0.join("ffmpeg"), self.0.join("clip.mp4"))
        }
    }

    #[cfg(unix)]
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn messages_and_exit_codes_count_as_decode_errors() {
        // Two 4x2 frames, two messages (one naming folders), then a failed exit.
        let fake = Fake::new(
            "errors",
            r"printf 'YUV4MPEG2 W4 H2 F60:1 Ip C420jpeg\nFRAME\n\020\020\020\020\020\020\020\020\200\200\200\200FRAME\n\020\020\020\020\020\020\020\020\200\200\200\200'
echo 'corrupt decoded frame' >&2
echo '[h264 @ 0x1] error while decoding /Users/someone/clips/a.mp4 in C:\Users\someone' >&2
exit 1",
        );
        let stats = fake
            .cli()
            .video(&mut |_, _| ControlFlow::Continue(()))
            .unwrap();
        assert_eq!((stats.frames, stats.errors), (2, 2));
        let last = stats.last_error.unwrap();
        assert!(!last.contains("someone"), "{last}");
        // A failed exit with no message still counts.
        let quiet = Fake::new(
            "quiet",
            r"printf 'YUV4MPEG2 W4 H2 C420jpeg\nFRAME\n\020\020\020\020\020\020\020\020\200\200\200\200'
exit 3",
        );
        let stats = quiet
            .cli()
            .video(&mut |_, _| ControlFlow::Continue(()))
            .unwrap();
        assert_eq!((stats.frames, stats.errors), (1, 1));
        // Failing before any frame means it didn't start: an FFmpeg too old for -fps_mode.
        let old = Fake::new(
            "old",
            "echo \"Unrecognized option 'fps_mode'.\" >&2\nexit 1",
        );
        let result = old.cli().video(&mut |_, _| ControlFlow::Continue(()));
        assert!(
            matches!(&result, Err(DecodeError::Start(m)) if m.contains("fps_mode")),
            "{result:?}"
        );
        // The callback's stop kills it without broken-pipe noise.
        let endless = Fake::new(
            "endless",
            r"printf 'YUV4MPEG2 W4 H2 C420jpeg\n'
while true; do printf 'FRAME\n\020\020\020\020\020\020\020\020\200\200\200\200' || exit 1; done",
        );
        let stats = endless.cli().video(&mut |i, _| {
            if i == 9 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert_eq!(stats.map(|s| (s.frames, s.errors)).ok(), Some((10, 0)));
    }

    #[test]
    fn relative_ffmpeg_paths_come_from_the_current_folder() {
        let cli = Cli::new("tools/ffmpeg", "clips/a.mp4");
        assert!(cli.ffmpeg.is_absolute() && cli.ffmpeg.ends_with("tools/ffmpeg"));
        // A bare name is left to the PATH lookup.
        assert_eq!(Cli::new("ffmpeg", "a.mp4").ffmpeg, PathBuf::from("ffmpeg"));
        assert!(looks_like_path("/Users/x/a.mp4") && looks_like_path("'C:\\x"));
        assert!(looks_like_path("\\\\server\\share") && !looks_like_path("a.mp4"));
        assert!(!looks_like_path("/"));
    }

    #[test]
    fn reads_raw_f32_audio_in_whole_frames() {
        // 10,000 stereo frames: L = i, R = -i.
        let bytes: Vec<u8> = (0..10_000)
            .flat_map(|i| [i as f32, -(i as f32)])
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut seen = Vec::new();
        let mut chunks = 0;
        let (frames, stopped) = read_f32(&mut &bytes[..], 2, &mut |chunk| {
            chunks += 1;
            seen.extend_from_slice(chunk);
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!((frames, stopped, chunks), (10_000, false, 3));
        assert_eq!(seen.len(), 20_000);
        assert_eq!((seen[9_998], seen[9_999]), (4_999.0, -4_999.0));
        // The callback may stop it; a cut frame is an error; nothing is no frames.
        let (frames, stopped) =
            read_f32(&mut &bytes[..], 2, &mut |_| ControlFlow::Break(())).unwrap();
        assert_eq!((frames, stopped), (4_096, true));
        assert!(
            read_f32(&mut &bytes[..bytes.len() - 2], 2, &mut |_| {
                ControlFlow::Continue(())
            })
            .is_err()
        );
        assert_eq!(
            read_f32(&mut &b""[..], 1, &mut |_| ControlFlow::Continue(())).unwrap(),
            (0, false)
        );
    }

    #[test]
    fn a_missing_ffmpeg_fails_to_start() {
        let mut cli = Cli::new("/nonexistent/ffmpeg", "clip.mp4");
        let result = cli.video(&mut |_, _| ControlFlow::Continue(()));
        assert!(matches!(result, Err(DecodeError::Start(_))), "{result:?}");
        let result = cli.audio(0, 2, &mut |_| ControlFlow::Continue(()));
        assert!(matches!(result, Err(DecodeError::Start(_))), "{result:?}");
    }
}
