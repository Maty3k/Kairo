// SPDX-License-Identifier: GPL-3.0-or-later
//! FFmpeg's command line for gate 2's capture (docs/research/16-verify-testbed/README.md §8.4):
//! the testbed's window through Windows.Graphics.Capture (`gfxcapture`, by handle, so FFmpeg
//! lists no other window's title), or the monitor through Desktop Duplication (`ddagrab`),
//! cropped to the testbed's client area so nothing else on the screen is recorded. One d3d11va
//! device on the adapter that drives the monitor serves both the capture and AMF, which gets
//! NV12 BT.709 limited and the options `kairo-verify corpus build` gives it. One word per
//! argument: no shell ever reads it.

use kairo_core::ffmpeg_cli::{self, H264_METADATA, HIGH_RATES, RECOMMENDED_RATES};

use crate::cli::{LogLevel, Output};

/// The clip FFmpeg writes, in the run folder.
pub const CLIP: &str = "capture.mp4";

/// Where the pictures come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The testbed's window, by handle.
    Window {
        /// The handle, as the ready file gives it.
        hwnd: isize,
    },
    /// A part of the monitor.
    Desktop {
        /// The monitor's DXGI output index on its adapter.
        output: u32,
        /// The crop, in the monitor's pixels: left, top, width, height.
        crop: (u32, u32, u32, u32),
    },
}

/// The capture's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// The pictures' source.
    pub source: Source,
    /// The DXGI index of the adapter that drives the monitor, in FFmpeg's process.
    pub adapter_index: u32,
    /// `gfxcapture`'s `max_framerate` and `ddagrab`'s `framerate`.
    pub max_framerate: u32,
    /// The clip's size and rate.
    pub output: Output,
    /// How long to record.
    pub seconds: u32,
    /// FFmpeg's log level.
    pub loglevel: LogLevel,
    /// Apply `h264_metadata`'s colour fix after the encoder.
    pub finishing: bool,
}

/// The crop of the client area `client` (left, top, width, height in desktop pixels) on the
/// monitor `monitor` (left, top, right, bottom): `None` unless it lies on the monitor and has a
/// size, so `ddagrab` never records the rest of the screen.
pub fn desktop_crop(
    monitor: (i32, i32, i32, i32),
    client: (i32, i32, i32, i32),
) -> Option<(u32, u32, u32, u32)> {
    let (left, top, right, bottom) = monitor;
    let (x, y, width, height) = client;
    let fits = width > 0
        && height > 0
        && x >= left
        && y >= top
        && i64::from(x) + i64::from(width) <= i64::from(right)
        && i64::from(y) + i64::from(height) <= i64::from(bottom);
    fits.then(|| {
        (
            (x - left) as u32,
            (y - top) as u32,
            width as u32,
            height as u32,
        )
    })
}

/// The source filter.
fn source(settings: &Settings) -> String {
    match settings.source {
        Source::Window { hwnd } => format!(
            "gfxcapture=hwnd={hwnd}:max_framerate={}:capture_cursor=0:display_border=0",
            settings.max_framerate
        ),
        Source::Desktop {
            output,
            crop: (x, y, w, h),
        } => format!(
            "ddagrab=output_idx={output}:framerate={}:draw_mouse=0:offset_x={x}:offset_y={y}:video_size={w}x{h}",
            settings.max_framerate
        ),
    }
}

/// FFmpeg's arguments, without the program.
pub fn args(settings: &Settings) -> Vec<String> {
    let (width, height) = settings.output.size();
    let fps = settings.output.fps();
    let (rate, peak) = match settings.output {
        Output::P1080 => RECOMMENDED_RATES,
        Output::P1440 => HIGH_RATES,
    };
    let graph = format!(
        "{},fps={fps},hwdownload,format=bgra,{}[v]",
        source(settings),
        ffmpeg_cli::nv12_bt709_chain(width, height)
    );
    let mut args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        settings.loglevel.name(),
        "-n",
        "-init_hw_device",
        &format!("d3d11va=kd:{}", settings.adapter_index),
        "-filter_hw_device",
        "kd",
        "-filter_complex",
        &graph,
        "-map",
        "[v]",
        "-t",
        &settings.seconds.to_string(),
        "-fps_mode",
        "cfr",
        "-r",
        &fps.to_string(),
    ]
    .map(String::from)
    .to_vec();
    args.extend(ffmpeg_cli::amf_h264(rate, peak, fps, 0));
    if settings.finishing {
        args.extend(["-bsf:v", H264_METADATA].map(String::from));
    }
    args.extend(["-movflags", "+faststart", "-use_editlist", "0", CLIP].map(String::from));
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate2() -> Settings {
        Settings {
            source: Source::Window { hwnd: 263_330 },
            adapter_index: 1,
            max_framerate: 360,
            output: Output::P1080,
            seconds: 30,
            loglevel: LogLevel::Debug,
            finishing: false,
        }
    }

    #[test]
    fn gate_2s_command_word_for_word() {
        assert_eq!(
            args(&gate2()).join(" "),
            "-hide_banner -nostdin -loglevel debug -n -init_hw_device d3d11va=kd:1 \
             -filter_hw_device kd -filter_complex \
             gfxcapture=hwnd=263330:max_framerate=360:capture_cursor=0:display_border=0,fps=60,\
             hwdownload,format=bgra,scale=w=1920:h=1080:flags=bicubic+accurate_rnd+\
             full_chroma_int+full_chroma_inp:out_color_matrix=bt709:out_range=tv:\
             out_chroma_loc=left,format=nv12,setparams=color_primaries=bt709:color_trc=bt709:\
             colorspace=bt709:range=tv[v] -map [v] -t 30 -fps_mode cfr -r 60 -pix_fmt nv12 \
             -c:v h264_amf -usage transcoding -quality balanced -rc vbr_peak -b:v 25M \
             -maxrate 40M -bufsize 40M -g 60 -vbaq 1 -preanalysis 0 \
             -high_motion_quality_boost_enable 0 -preencode 0 -bf 0 -max_b_frames 0 \
             -frame_skipping 0 -filler_data 0 -coder cabac -profile:v high -header_spacing 60 \
             -async_depth 4 -movflags +faststart -use_editlist 0 capture.mp4"
        );
        // The AMF part is kairo-core's, as corpus build's is.
        let all = args(&gate2()).join(" ");
        assert!(all.contains(&ffmpeg_cli::amf_h264("25M", "40M", 60, 0).join(" ")));
    }

    #[test]
    fn variants() {
        let mut s = gate2();
        s.output = Output::P1440;
        s.max_framerate = 120;
        s.loglevel = LogLevel::Verbose;
        s.finishing = true;
        let line = args(&s).join(" ");
        assert!(line.contains("-loglevel verbose"));
        assert!(line.contains(":max_framerate=120:"));
        assert!(line.contains("scale=w=2560:h=1440:"));
        assert!(line.contains("-b:v 36M -maxrate 50M -bufsize 50M"));
        assert!(line.contains(&format!("-async_depth 4 -bsf:v {H264_METADATA} -movflags")));
        s.source = Source::Desktop {
            output: 2,
            crop: (0, 0, 2560, 1440),
        };
        let line = args(&s).join(" ");
        assert!(line.contains(
            "-filter_complex ddagrab=output_idx=2:framerate=120:draw_mouse=0:offset_x=0:offset_y=0:video_size=2560x1440,fps=60,"
        ));
    }

    #[test]
    fn crops_lie_on_the_monitor() {
        // A 1920×1080 monitor right of the primary one, the testbed's window filling it.
        let monitor = (2560, 0, 4480, 1080);
        assert_eq!(
            desktop_crop(monitor, (2560, 0, 1920, 1080)),
            Some((0, 0, 1920, 1080))
        );
        assert_eq!(
            desktop_crop(monitor, (2600, 40, 1280, 720)),
            Some((40, 40, 1280, 720))
        );
        for client in [
            (2559, 0, 1920, 1080),
            (2560, 0, 1921, 1080),
            (2560, 1, 1920, 1080),
            (2560, 0, 0, 1080),
            (2560, 0, 1920, -1),
        ] {
            assert_eq!(desktop_crop(monitor, client), None, "{client:?}");
        }
    }
}
