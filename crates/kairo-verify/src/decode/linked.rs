// SPDX-License-Identifier: GPL-3.0-or-later
//! The linked decoder (docs/research/16-verify-testbed/README.md §2 item 3, §3.3): LGPL FFmpeg's
//! avcodec and avformat through `ffmpeg-next`'s safe API, on Windows, where Kairo ships them.
//!
//! It opens a clip only after Kairo's parser has accepted it, as `file:` with
//! `ignore_editlist=1`, `protocol_whitelist=file` and `format_whitelist=mov,mp4,m4a,3gp,3g2,mj2`,
//! so linked avformat can't open a URL: only `kairo-net` may open sockets (CLAUDE.md). The
//! decoder gets `err_detect=+crccheck+bitstream+buffer` and frame threading. A picture FFmpeg
//! flags as corrupt or with decode errors counts as one error. Pictures arrive in presentation
//! order, borrowed in place; audio as f32 at the track's own channel count, AAC priming included.
//! It refuses to run unless avutil, avcodec and avformat all report an LGPL licence (README §2
//! item 18), and the clip's path must be UTF-8, since `ffmpeg-next` would panic on another.
//! The `kairo-verify` binary keeps FFmpeg's log quiet, so no message names a path (CLAUDE.md).

use std::ops::ControlFlow;
use std::path::PathBuf;

use ffmpeg::format::sample::Type as Layout;
use ffmpeg::format::{Pixel, Sample};
use ffmpeg::{Dictionary, Error, Packet, codec, frame, media};
use ffmpeg_next as ffmpeg;

use super::{DecodeError, DecodeStats, Decoder, Planes};

/// avutil's, avcodec's and avformat's licences, as the linked libraries report them.
pub fn licences() -> [&'static str; 3] {
    [
        ffmpeg::util::license(),
        ffmpeg::codec::license(),
        ffmpeg::format::license(),
    ]
}

/// The first of [`licences`] that isn't LGPL, if any: a GPL or non-free build must never be
/// linked into Kairo (CLAUDE.md).
pub fn not_lgpl() -> Option<&'static str> {
    licences().into_iter().find(|l| !l.starts_with("LGPL"))
}

/// FFmpeg's error, as a message for a report: its code's text only, never a path.
fn text(error: Error) -> String {
    error.to_string()
}

/// The linked decoder for one clip.
#[derive(Debug, Clone)]
pub struct Linked {
    clip: PathBuf,
}

impl Linked {
    /// The decoder for `clip`.
    pub fn new(clip: impl Into<PathBuf>) -> Self {
        Self { clip: clip.into() }
    }

    /// The clip, opened as only a local MP4 may be.
    fn open(&self) -> Result<ffmpeg::format::context::Input, DecodeError> {
        if let Some(licence) = not_lgpl() {
            return Err(DecodeError::Start(format!(
                "the linked FFmpeg is {licence}, not LGPL"
            )));
        }
        let path = self
            .clip
            .to_str()
            .filter(|p| !p.contains('\0'))
            .ok_or_else(|| DecodeError::Start("the clip's path isn't UTF-8".to_owned()))?;
        let mut options = Dictionary::new();
        options.set("ignore_editlist", "1");
        options.set("protocol_whitelist", "file");
        options.set("format_whitelist", "mov,mp4,m4a,3gp,3g2,mj2");
        ffmpeg::format::input_with_dictionary(&format!("file:{path}"), options)
            .map_err(|e| DecodeError::Start(format!("FFmpeg couldn't open the clip: {}", text(e))))
    }

    /// An opened decoder for `stream`, with error detection and frame threading.
    fn decoder(stream: &ffmpeg::Stream<'_>) -> Result<codec::decoder::Opened, DecodeError> {
        let start = |e: Error| DecodeError::Start(format!("FFmpeg's decoder: {}", text(e)));
        let mut context =
            codec::context::Context::from_parameters(stream.parameters()).map_err(start)?;
        context.set_threading(codec::threading::Config {
            kind: codec::threading::Type::Frame,
            count: 0,
        });
        let mut options = Dictionary::new();
        options.set("err_detect", "+crccheck+bitstream+buffer");
        // A context made from parameters knows its codec's ID but holds no codec, and `open_with`
        // passes none, which avcodec_open2 refuses as an invalid argument: so name it by the ID.
        let id = context.id();
        context.decoder().open_as_with(id, options).map_err(start)
    }
}

/// What a step of the decode loop says to do next.
enum Next {
    /// Feed it more.
    More,
    /// The callback asked to stop.
    Stop,
    /// The decoder has given everything.
    Done,
}

/// Records a decode error.
fn note(stats: &mut DecodeStats, what: String) {
    stats.errors += 1;
    stats.last_error = Some(what);
}

/// A picture's planes, borrowed: 8-bit 4:2:0 only (`yuv420p`, or `yuvj420p` for full range).
fn planes(picture: &frame::Video) -> Result<Planes<'_>, DecodeError> {
    if !matches!(picture.format(), Pixel::YUV420P | Pixel::YUVJ420P)
        || picture.planes() < 3
        || picture.stride(1) != picture.stride(2)
    {
        return Err(DecodeError::Unsupported(format!(
            "pictures in {:?}, not 8-bit 4:2:0",
            picture.format()
        )));
    }
    Planes::new(
        (picture.width() as usize, picture.height() as usize),
        (picture.data(0), picture.stride(0)),
        (picture.data(1), picture.data(2), picture.stride(1)),
    )
}

/// Takes every picture the decoder has ready, handing each to `each`.
fn pictures(
    decoder: &mut codec::decoder::Video,
    picture: &mut frame::Video,
    stats: &mut DecodeStats,
    each: &mut dyn FnMut(usize, &Planes<'_>) -> ControlFlow<()>,
) -> Result<Next, DecodeError> {
    loop {
        match decoder.receive_frame(picture) {
            Ok(()) => {
                if picture.is_corrupt() || picture.has_decode_errors() {
                    note(stats, "a picture decoded with errors".to_owned());
                }
                let index = stats.frames;
                stats.frames += 1;
                if each(index, &planes(picture)?).is_break() {
                    return Ok(Next::Stop);
                }
            }
            Err(Error::Eof) => return Ok(Next::Done),
            Err(Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                return Ok(Next::More);
            }
            Err(e) => {
                note(stats, text(e));
                return Ok(Next::More);
            }
        }
    }
}

/// Appends a decoded audio frame's samples to `out`, interleaved at its own channel count.
fn interleave(
    sound: &frame::Audio,
    channels: usize,
    out: &mut Vec<f32>,
) -> Result<(), DecodeError> {
    let count = usize::from(sound.channels());
    if count != channels {
        return Err(DecodeError::Unsupported(format!(
            "{count} channels decoded; the track says {channels}"
        )));
    }
    let samples = sound.samples();
    match sound.format() {
        Sample::F32(Layout::Planar) if sound.planes() >= count => {
            let planes: Vec<&[f32]> = (0..count).map(|c| sound.plane::<f32>(c)).collect();
            for i in 0..samples {
                out.extend(planes.iter().map(|p| p.get(i).copied().unwrap_or(0.0)));
            }
        }
        Sample::F32(Layout::Packed) if sound.planes() >= 1 => {
            let bytes = sound.data(0);
            let (words, _) = bytes.as_chunks::<4>();
            out.extend(
                words
                    .iter()
                    .take(samples * count)
                    .map(|&w| f32::from_le_bytes(w)),
            );
        }
        other => {
            return Err(DecodeError::Unsupported(format!(
                "audio in {other:?}, not 32-bit float"
            )));
        }
    }
    Ok(())
}

/// Takes every audio frame the decoder has ready, handing each to `each`.
fn sounds(
    decoder: &mut codec::decoder::Audio,
    sound: &mut frame::Audio,
    channels: usize,
    chunk: &mut Vec<f32>,
    stats: &mut DecodeStats,
    each: &mut dyn FnMut(&[f32]) -> ControlFlow<()>,
) -> Result<Next, DecodeError> {
    loop {
        match decoder.receive_frame(sound) {
            Ok(()) => {
                if sound.is_corrupt() || sound.has_decode_errors() {
                    note(stats, "audio decoded with errors".to_owned());
                }
                chunk.clear();
                interleave(sound, channels, chunk)?;
                stats.frames += sound.samples();
                if each(chunk).is_break() {
                    return Ok(Next::Stop);
                }
            }
            Err(Error::Eof) => return Ok(Next::Done),
            Err(Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                return Ok(Next::More);
            }
            Err(e) => {
                note(stats, text(e));
                return Ok(Next::More);
            }
        }
    }
}

impl Decoder for Linked {
    fn kind(&self) -> &'static str {
        "linked"
    }

    fn licence(&mut self) -> Option<&'static str> {
        Some(not_lgpl().unwrap_or(licences()[1]))
    }

    fn video(
        &mut self,
        each: &mut dyn FnMut(usize, &Planes<'_>) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError> {
        let mut input = self.open()?;
        let (index, opened) = {
            let stream = input
                .streams()
                .best(media::Type::Video)
                .ok_or_else(|| DecodeError::Start("the clip has no video stream".to_owned()))?;
            (stream.index(), Self::decoder(&stream)?)
        };
        let mut decoder = opened
            .video()
            .map_err(|e| DecodeError::Start(format!("FFmpeg's decoder: {}", text(e))))?;
        let mut stats = DecodeStats::default();
        let mut picture = frame::Video::empty();
        loop {
            // A new packet each time: `read` doesn't release the last one's data.
            let mut packet = Packet::empty();
            match packet.read(&mut input) {
                Ok(()) => {}
                Err(Error::Eof) => break,
                Err(e) => {
                    note(&mut stats, text(e));
                    break;
                }
            }
            if packet.stream() != index {
                continue;
            }
            if let Err(e) = decoder.send_packet(&packet) {
                note(&mut stats, text(e));
                continue;
            }
            if let Next::Stop = pictures(&mut decoder, &mut picture, &mut stats, each)? {
                return Ok(stats);
            }
        }
        if decoder.send_eof().is_ok() {
            pictures(&mut decoder, &mut picture, &mut stats, each)?;
        }
        Ok(stats)
    }

    fn audio(
        &mut self,
        track: usize,
        channels: usize,
        each: &mut dyn FnMut(&[f32]) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError> {
        let mut input = self.open()?;
        let (index, opened) = {
            let stream = input
                .streams()
                .filter(|s| s.parameters().medium() == media::Type::Audio)
                .nth(track)
                .ok_or_else(|| DecodeError::Start(format!("no audio track {}", track + 1)))?;
            (stream.index(), Self::decoder(&stream)?)
        };
        let mut decoder = opened
            .audio()
            .map_err(|e| DecodeError::Start(format!("FFmpeg's decoder: {}", text(e))))?;
        let channels = channels.max(1);
        let mut stats = DecodeStats::default();
        let mut sound = frame::Audio::empty();
        let mut chunk = Vec::with_capacity(1_024 * channels);
        loop {
            let mut packet = Packet::empty();
            match packet.read(&mut input) {
                Ok(()) => {}
                Err(Error::Eof) => break,
                Err(e) => {
                    note(&mut stats, text(e));
                    break;
                }
            }
            if packet.stream() != index {
                continue;
            }
            if let Err(e) = decoder.send_packet(&packet) {
                note(&mut stats, text(e));
                continue;
            }
            let next = sounds(
                &mut decoder,
                &mut sound,
                channels,
                &mut chunk,
                &mut stats,
                each,
            )?;
            if let Next::Stop = next {
                return Ok(stats);
            }
        }
        if decoder.send_eof().is_ok() {
            sounds(
                &mut decoder,
                &mut sound,
                channels,
                &mut chunk,
                &mut stats,
                each,
            )?;
        }
        Ok(stats)
    }
}
