// SPDX-License-Identifier: GPL-3.0-or-later
//! The FFmpeg command lines M1's instruments share, and what FFmpeg's log must say about them
//! (docs/research/16-verify-testbed/README.md §7.2, §8.4, §8.5). `kairo-verify corpus build`
//! and the harness's capture give AMF the same options, convert to BT.709 limited the same way
//! and read the same log lines, so the PC corpus and gate 2's clips come from one encoder setup.
//! Plain text in and out, so neither needs the other, nor FFmpeg's libraries, to share it.

/// Tags frames BT.709 limited (README §7.2's `SETPARAMS`): the only way the 9.0 CLI hands the
/// primaries and transfer to the encoder and `colr`.
pub const SETPARAMS_FILTER: &str =
    "setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv";

/// README §7.2's colour fix for an encoder whose VUI lacks BT.709 limited (`--finishing
/// h264_metadata`): an LGPL bitstream filter, applied after the encoder's options.
pub const H264_METADATA: &str = "h264_metadata=video_full_range_flag=0:colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1";

/// What FFmpeg prints for an option no stream used (EXP-8: a warning, exit 0).
pub const UNUSED_OPTION: &str = "has not been used for any stream";

/// The average and peak rates of PLAN's Recommended preset (1080p60).
pub const RECOMMENDED_RATES: (&str, &str) = ("25M", "40M");

/// The average and peak rates of PLAN's High preset (1440p60); the peak stays at the 50 Mbps
/// limit (CLAUDE.md).
pub const HIGH_RATES: (&str, &str) = ("36M", "50M");

/// AMF's H.264 options (README §7.2), every one set: NV12 in, `rate` average and `peak` maximum
/// and buffer, an IDR every `gop` frames with SPS and PPS before each, and `bframes` B-frames.
pub fn amf_h264(rate: &str, peak: &str, gop: u32, bframes: u32) -> Vec<String> {
    let (gop, bframes) = (gop.to_string(), bframes.to_string());
    [
        "-pix_fmt",
        "nv12",
        "-c:v",
        "h264_amf",
        "-usage",
        "transcoding",
        "-quality",
        "balanced",
        "-rc",
        "vbr_peak",
        "-b:v",
        rate,
        "-maxrate",
        peak,
        "-bufsize",
        peak,
        "-g",
        &gop,
        "-vbaq",
        "1",
        "-preanalysis",
        "0",
        "-high_motion_quality_boost_enable",
        "0",
        "-preencode",
        "0",
        "-bf",
        &bframes,
        "-max_b_frames",
        &bframes,
        "-frame_skipping",
        "0",
        "-filler_data",
        "0",
        "-coder",
        "cabac",
        "-profile:v",
        "high",
        "-header_spacing",
        &gop,
        "-async_depth",
        "4",
    ]
    .map(String::from)
    .to_vec()
}

/// swscale's conversion of a BGRA picture to NV12 BT.709 limited at `width`×`height` with
/// left-sited chroma, the reference's siting (README §8.4), then the BT.709 tags.
pub fn nv12_bt709_chain(width: u32, height: u32) -> String {
    format!(
        "scale=w={width}:h={height}:flags=bicubic+accurate_rnd+full_chroma_int+full_chroma_inp\
         :out_color_matrix=bt709:out_range=tv:out_chroma_loc=left,format=nv12,{SETPARAMS_FILTER}"
    )
}

/// The vendor:device of the d3d11va adapter FFmpeg opened, from its `Using device vvvv:dddd (…)`
/// line (hwcontext_d3d11va.c), lowercase. The encoder's own `Using device kd (type d3d11va)`
/// line doesn't match: only four hex digits, a colon and four more do.
pub fn adapter_device(log: &str) -> Option<String> {
    log.match_indices("Using device ").find_map(|(at, word)| {
        let rest = log.get(at + word.len()..)?;
        let pair = rest.get(..9)?;
        let hex = |s: &str| s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit());
        let (vendor, device) = pair.split_once(':')?;
        (hex(vendor) && hex(device) && rest.get(9..11) == Some(" ("))
            .then(|| pair.to_ascii_lowercase())
    })
}

/// Whether the log's stream mapping sends a stream to `encoder`: `-> h264 (h264_amf)` from a
/// file or a pipe, `-> Stream #0:0 (h264_amf)` from a filtergraph. Only the indented lines of the
/// `Stream mapping:` block count; another component's `[…]` line printed among them is skipped.
pub fn mapped_to(log: &str, encoder: &str) -> bool {
    let Some((_, mapping)) = log.split_once("Stream mapping:") else {
        return false;
    };
    let tag = format!("({encoder})");
    mapping
        .lines()
        .skip(1)
        .filter(|line| !line.starts_with('['))
        .take_while(|line| line.starts_with(' '))
        .any(|line| line.contains("-> ") && line.contains(&tag))
}

/// The output's video stream line, under `Output #0`.
pub fn output_video_line(log: &str) -> Option<&str> {
    log.split("Output #0")
        .nth(1)?
        .lines()
        .find(|line| line.contains("Video: h264"))
}

/// What's wrong with an AMF session's device lines (README §8.5): the d3d11va adapter asked
/// for (`hw_index`), the vendor:device FFmpeg opened, which must be `hw_device` when given, and
/// the encoder's own lines. FFmpeg names the index before it looks the adapter up, and falls back
/// to the default one without naming a vendor:device if the index doesn't exist.
pub fn amf_device_problems(
    log: &str,
    hw_index: Option<u32>,
    hw_device: Option<&str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for line in [
        "Using device kd (type d3d11va) with h264_amf encoder.",
        "AMF via D3D11.",
    ] {
        if !log.contains(line) {
            problems.push(format!("no \"{line}\""));
        }
    }
    match hw_index {
        Some(n) if !log.contains(&format!("Selecting d3d11va adapter {n}")) => {
            problems.push(format!("no \"Selecting d3d11va adapter {n}\""));
        }
        Some(_) => {}
        None => problems.push("no adapter asked for (--hw-index)".to_owned()),
    }
    match (adapter_device(log), hw_device) {
        (None, _) => problems.push(
            "no \"Using device vvvv:dddd\": the adapter wasn't found, and FFmpeg fell back to the default one".to_owned(),
        ),
        (Some(found), Some(want)) if !found.eq_ignore_ascii_case(want) => {
            problems.push(format!("the adapter is {found}, not {want}"));
        }
        _ => {}
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_amf_options_are_set_in_full() {
        let args = amf_h264("25M", "40M", 60, 0).join(" ");
        assert_eq!(
            args,
            "-pix_fmt nv12 -c:v h264_amf -usage transcoding -quality balanced -rc vbr_peak \
             -b:v 25M -maxrate 40M -bufsize 40M -g 60 -vbaq 1 -preanalysis 0 \
             -high_motion_quality_boost_enable 0 -preencode 0 -bf 0 -max_b_frames 0 \
             -frame_skipping 0 -filler_data 0 -coder cabac -profile:v high -header_spacing 60 \
             -async_depth 4"
        );
        assert!(
            amf_h264(HIGH_RATES.0, HIGH_RATES.1, 120, 2)
                .join(" ")
                .contains(
                    "-b:v 36M -maxrate 50M -bufsize 50M -g 120 -vbaq 1 -preanalysis 0 \
             -high_motion_quality_boost_enable 0 -preencode 0 -bf 2 -max_b_frames 2"
                )
        );
    }

    #[test]
    fn the_conversion_ends_tagged() {
        assert_eq!(
            nv12_bt709_chain(1920, 1080),
            "scale=w=1920:h=1080:flags=bicubic+accurate_rnd+full_chroma_int+full_chroma_inp\
             :out_color_matrix=bt709:out_range=tv:out_chroma_loc=left,format=nv12,\
             setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv"
        );
    }

    #[test]
    fn mappings_from_a_file_or_a_filtergraph() {
        let pipe = "Stream mapping:\n  Stream #0:0 -> #0:0 (rawvideo (native) -> h264 (h264_amf))\nPress [q]";
        let graph = "Stream mapping:\n  setparams:default -> Stream #0:0 (h264_amf)\nPress [q]";
        assert!(mapped_to(pipe, "h264_amf") && mapped_to(graph, "h264_amf"));
        assert!(!mapped_to(pipe, "h264_mf"));
        // Only the mapping block counts, past another component's line in it.
        let elsewhere =
            "[h264_amf] (h264_amf) -> x\nStream mapping:\n  Stream #0:0 -> #0:0 (copy)\n";
        assert!(!mapped_to(elsewhere, "h264_amf"));
        let interleaved =
            "Stream mapping:\n[graph 0 @ 0x1] x\n  setparams:default -> Stream #0:0 (h264_amf)\n";
        assert!(mapped_to(interleaved, "h264_amf"));
        assert!(!mapped_to("no mapping here", "h264_amf"));
    }

    #[test]
    fn the_adapter_and_device_lines() {
        let log = "[AVHWDeviceContext @ 0x1] Selecting d3d11va adapter 1\n\
                   [AVHWDeviceContext @ 0x1] Using device ABCD:1234 (Test GPU).\n\
                   [h264_amf @ 0x2] Using device kd (type d3d11va) with h264_amf encoder.\n\
                   [h264_amf @ 0x2] AMF via D3D11.\n";
        assert_eq!(adapter_device(log).as_deref(), Some("abcd:1234"));
        assert!(amf_device_problems(log, Some(1), Some("abcd:1234")).is_empty());
        assert_eq!(
            amf_device_problems(log, Some(1), Some("abcd:9999")),
            ["the adapter is abcd:1234, not abcd:9999"]
        );
        assert_eq!(
            amf_device_problems(log, Some(0), None),
            ["no \"Selecting d3d11va adapter 0\""]
        );
        let fallback = log.replace("Using device ABCD:1234 (Test GPU).", "");
        assert_eq!(adapter_device(&fallback), None);
        assert_eq!(amf_device_problems(&fallback, Some(1), None).len(), 1);
        let output = "Output #0, mp4, to 'capture.mp4':\n  Stream #0:0: Video: h264 (High), nv12(tv, bt709, progressive), 1920x1080\n";
        assert!(output_video_line(output).is_some_and(|l| l.contains("nv12(tv, bt709")));
        assert_eq!(
            output_video_line("Input #0\n  Stream #0:0: Video: h264"),
            None
        );
    }
}
