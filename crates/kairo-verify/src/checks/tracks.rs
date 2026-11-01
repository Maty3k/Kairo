// SPDX-License-Identifier: GPL-3.0-or-later
//! The track checks (container.md §6 C6, C10–C12): start alignment, the track layout, the AAC
//! format and the durations.

use serde_json::json;

use super::{Ctx, Outcome};
use crate::aac;
use crate::mp4::tracks::{SampleEntry, Track};
use crate::profile::{self, Layout, Length, Role};

/// The audio rate every Kairo track uses (PLAN:191).
const RATE: u64 = 48_000;
/// AAC-LC samples per packet.
const AAC_FRAME: u64 = 1024;

/// ⌈a / b⌉ for b > 0.
fn ceil_div(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}

/// C6: the held first video frame equals an audio lead of 1,024–2,047 samples, and every
/// audio track's first packet is a whole AAC frame; video alone starts at once.
pub(crate) fn start_alignment(ctx: &Ctx<'_>) -> Outcome {
    let t = ctx.timing.timescale;
    let (lo, hi) = profile::LEAD_SAMPLES;
    let window = (ceil_div(lo * t, RATE), ceil_div(hi * t, RATE));
    let mut outcome = Outcome {
        limit: json!({"lead_samples": [lo, hi], "extension_ticks": [window.0, window.1],
                      "source": "PLAN:179, CB:49 (P8), core/cut.rs:7–12"}),
        ..Outcome::default()
    };
    let Some(extension) = ctx.timing.extension else {
        return Outcome::skip("no video timing (see video.cfr)");
    };
    let audio: Vec<(usize, &Track)> = ctx.audio().collect();
    let firsts: Vec<Option<u32>> = audio.iter().map(|(_, t)| t.first_delta()).collect();
    let lead = u64::try_from(extension).ok().map(|e| e * RATE / t.max(1));
    // How long the first frame is held beyond one frame (at most 42.7 ms, PLAN:179).
    let hold_ms = extension as f64 * 1_000.0 / t.max(1) as f64;
    outcome.value = json!({"extension_ticks": extension, "hold_ms": hold_ms, "lead_samples": lead,
                           "audio_first_deltas": firsts});
    if audio.is_empty() {
        outcome.require(extension == 0, || {
            format!("video only, but frame 1 is held {extension} ticks")
        });
        outcome.detail = "video only: no hold".to_owned();
        return outcome;
    }
    let e = u64::try_from(extension).unwrap_or(0);
    if extension < 0 || e > window.1 {
        outcome.fails.push(format!(
            "frame 1 is held {extension} ticks; want {}–{} (an audio lead of {lo}–{hi} samples)",
            window.0, window.1
        ));
    } else if e < window.0 {
        // A lead under one AAC frame is a cut at the track's very first packet (core/cut.rs).
        let fail = !ctx.plan.profile.is_share_export();
        outcome.violation(
            fail,
            format!(
                "frame 1 is held {e} ticks: an audio lead under {lo} samples (a truncated start)"
            ),
        );
    }
    for ((index, _), first) in audio.iter().zip(&firsts) {
        outcome.require(*first == Some(AAC_FRAME as u32), || {
            format!(
                "audio track {}'s first packet lasts {}, not 1024",
                index + 1,
                first.unwrap_or(0)
            )
        });
    }
    outcome.detail = format!(
        "frame 1 held {e} ticks: audio leads by {} samples",
        lead.unwrap_or(0)
    );
    outcome
}

/// A track's title: its `udta/name`, else its `hdlr` name.
pub(crate) fn title(track: &Track) -> Option<&str> {
    let (hdlr, udta) = titles(track);
    udta.or(hdlr)
}

/// The title of a track, as `hdlr` and `udta/name` give it.
fn titles(track: &Track) -> (Option<&str>, Option<&str>) {
    (
        track.hdlr.as_ref().map(|h| h.name.as_str()),
        track.udta_name.as_deref(),
    )
}

fn channels(track: &Track) -> Option<u16> {
    match track.entries.first() {
        Some(SampleEntry::Audio { channels, .. }) => Some(*channels),
        _ => None,
    }
}

fn enabled(track: &Track) -> bool {
    track.tkhd.is_some_and(|t| t.flags & 1 == 1)
}

/// C10: the video track first, then the expected audio tracks in role order, titled, with
/// only the first enabled and all in one alternate group.
pub(crate) fn layout(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome::default();
    let tracks = &ctx.movie.tracks;
    // 1. One video track, first, enabled, group 0, avc1.
    let videos: Vec<usize> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.handler().is_some_and(|h| h.0 == *b"vide"))
        .map(|(i, _)| i + 1)
        .collect();
    outcome.require(videos == [1], || {
        format!("video tracks at {videos:?}; want one, first")
    });
    outcome.require(enabled(ctx.video), || {
        "the video track isn't enabled".to_owned()
    });
    let group = ctx.video.tkhd.map_or(0, |t| t.alternate_group);
    outcome.require(group == 0, || {
        format!("the video track is in alternate group {group}")
    });
    // 6. Nothing but video and sound.
    for (i, track) in tracks.iter().enumerate() {
        let kind = track.handler().map(|h| h.to_string()).unwrap_or_default();
        outcome.require(matches!(kind.as_str(), "vide" | "soun"), || {
            format!("track {} is a {kind:?} track", i + 1)
        });
    }
    // 7. Ids unique, rising, below next_track_id.
    let ids: Vec<u32> = tracks
        .iter()
        .map(|t| t.tkhd.map_or(0, |t| t.track_id))
        .collect();
    let next = ctx.movie.mvhd.map_or(0, |m| m.next_track_id);
    outcome.require(
        ids.windows(2).all(|w| w[0] < w[1]) && ids.iter().all(|&id| id > 0 && id < next),
        || format!("track ids {ids:?} aren't rising and below next_track_id {next}"),
    );
    let audio: Vec<&Track> = ctx.audio().map(|(_, t)| t).collect();
    let found: Vec<String> = audio
        .iter()
        .map(|t| {
            let (hdlr, _) = titles(t);
            format!(
                "{} {}ch{}",
                hdlr.unwrap_or("?"),
                channels(t).unwrap_or(0),
                if enabled(t) { " default" } else { "" }
            )
        })
        .collect();
    outcome.value = json!({"video": videos, "audio": found,
                           "languages": tracks.iter().map(|t| t.mdhd.map(|m| String::from_utf8_lossy(&m.language).into_owned())).collect::<Vec<_>>()});
    match &ctx.plan.layout {
        Layout::OneStereo => {
            outcome.limit =
                json!({"audio": "one stereo track, enabled", "source": "PLAN:249, README §6.5"});
            outcome.require(audio.len() == 1, || {
                format!("{} audio tracks; a share copy has one", audio.len())
            });
            if let Some(track) = audio.first() {
                outcome.require(channels(track) == Some(2), || {
                    "the audio track isn't stereo".to_owned()
                });
                outcome.require(enabled(track), || {
                    "the audio track isn't enabled".to_owned()
                });
            }
        }
        Layout::Exact(roles) => {
            outcome.limit = json!({"audio": roles.iter().map(|r| &r.title).collect::<Vec<_>>(),
                                   "source": "PLAN:185–191, CB:573, CB:598; the expect file"});
            check_roles(&mut outcome, &audio, roles);
        }
        Layout::Engine => {
            // Mix, Game and Mic, then Voice chat and Everything else, either of which may be
            // missing (plan B and Windows 10 have no Voice chat track, PLAN:188).
            let [mix, game, mic, voice, rest] = profile::engine_roles();
            let mut roles = vec![mix, game, mic];
            match audio.get(3..) {
                Some([fourth]) if titles(fourth).0 == Some(rest.title.as_str()) => roles.push(rest),
                Some([_]) => roles.push(voice),
                Some([_, _, ..]) => roles.extend([voice, rest]),
                _ => {}
            }
            outcome.limit = json!({"audio": ["Mix", "Game", "Mic", "[Voice chat]", "[Everything else]"],
                                   "source": "PLAN:185–191, CB:598"});
            check_roles(&mut outcome, &audio, &roles);
        }
    }
    outcome.detail = format!(
        "video + {}",
        if found.is_empty() {
            "no audio".to_owned()
        } else {
            found.join(", ")
        }
    );
    outcome
}

/// Rules 2–5 of C10 for an exact list of roles.
fn check_roles(outcome: &mut Outcome, audio: &[&Track], roles: &[Role]) {
    outcome.require(audio.len() == roles.len(), || {
        format!(
            "{} audio tracks; want {} ({})",
            audio.len(),
            roles.len(),
            roles
                .iter()
                .map(|r| r.title.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    for (k, (track, role)) in audio.iter().zip(roles).enumerate() {
        let (hdlr, udta) = titles(track);
        outcome.require(hdlr == Some(role.title.as_str()), || {
            format!(
                "audio track {} is titled {:?} in hdlr; want {:?}",
                k + 1,
                hdlr.unwrap_or(""),
                role.title
            )
        });
        outcome.require(udta == Some(role.title.as_str()), || match udta {
            Some(udta) => format!(
                "audio track {} is titled {udta:?} in udta/name; want {:?}",
                k + 1,
                role.title
            ),
            None => format!(
                "audio track {} has no udta/name title; want {:?}",
                k + 1,
                role.title
            ),
        });
        outcome.require(enabled(track) == role.default, || {
            format!(
                "audio track {} ({}) is {}",
                k + 1,
                role.title,
                if enabled(track) {
                    "enabled; only the default track may be"
                } else {
                    "not enabled, but it is the default"
                }
            )
        });
        outcome.require(channels(track) == Some(role.channels), || {
            format!(
                "audio track {} ({}) has {} channels; want {}",
                k + 1,
                role.title,
                channels(track).unwrap_or(0),
                role.channels
            )
        });
    }
    let groups: Vec<i16> = audio
        .iter()
        .map(|t| t.tkhd.map_or(0, |t| t.alternate_group))
        .collect();
    if let Some(&first) = groups.first() {
        outcome.require(first != 0 && groups.iter().all(|&g| g == first), || {
            format!("audio alternate groups {groups:?}; want one shared non-zero group")
        });
    }
}

/// C11: AAC LC at 48 kHz, mono or stereo as the entry says, in 1,024-sample packets.
pub(crate) fn audio_format(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome {
        limit: json!({"codec": "AAC LC", "rate": RATE, "channels": [1, 2], "packet": AAC_FRAME,
                      "source": "PLAN:191, research 09:702–714"}),
        ..Outcome::default()
    };
    let audio: Vec<(usize, &Track)> = ctx.audio().collect();
    if audio.is_empty() {
        return Outcome::skip("no audio tracks");
    }
    let mut bitrates = Vec::new();
    for &(index, track) in &audio {
        let n = index + 1;
        let Some(SampleEntry::Audio {
            kind,
            channels,
            sample_rate,
            esds,
        }) = track.entries.first()
        else {
            outcome
                .fails
                .push(format!("track {n} has no audio sample entry"));
            continue;
        };
        outcome.require(kind.0 == *b"mp4a", || {
            format!("track {n} is {kind}, not mp4a")
        });
        outcome.require(u64::from(*sample_rate) == RATE, || {
            format!("track {n}'s entry says {sample_rate} Hz")
        });
        let timescale = track.mdhd.map_or(0, |m| u64::from(m.timescale));
        outcome.require(timescale == RATE, || {
            format!("track {n}'s timescale is {timescale}")
        });
        let parsed = esds.first().and_then(|e| aac::parse_esds(e));
        match parsed {
            None => outcome
                .fails
                .push(format!("track {n}'s esds is missing or doesn't parse")),
            Some(e) => {
                outcome.require(
                    (e.object_type_indication, e.stream_type) == (0x40, 5),
                    || {
                        format!(
                            "track {n}'s esds says OTI {:#x} stream type {}",
                            e.object_type_indication, e.stream_type
                        )
                    },
                );
                match e.config {
                    None => outcome
                        .fails
                        .push(format!("track {n} has no AudioSpecificConfig")),
                    Some(c) => {
                        outcome.require(c.object_type == 2, || {
                            format!("track {n} is AOT {}, not AAC LC (2)", c.object_type)
                        });
                        outcome.require(u64::from(c.sample_rate) == RATE, || {
                            format!("track {n}'s config says {} Hz", c.sample_rate)
                        });
                        outcome.require(
                            matches!(c.channels, 1 | 2) && u16::from(c.channels) == *channels,
                            || {
                                format!(
                                    "track {n}'s config has {} channels, its entry {channels}",
                                    c.channels
                                )
                            },
                        );
                        outcome.require(!c.frame_length_flag, || {
                            format!("track {n} uses 960-sample frames")
                        });
                        outcome.require(!c.depends_on_core_coder, || {
                            format!("track {n} depends on a core coder")
                        });
                        if let Some((aot, sbr)) = c.sync_extension {
                            outcome.require(aot == 5 && !sbr, || {
                                format!(
                                    "track {n} signals SBR (extension AOT {aot}, present {sbr})"
                                )
                            });
                        }
                    }
                }
            }
        }
        // Every packet lasts 1,024 samples but the last, which may be shorter: read from the
        // stts runs, never sample by sample.
        let runs: Vec<(u32, u32)> = track.stts.iter().copied().filter(|&(c, _)| c > 0).collect();
        let (mut odd, mut start) = (None, 0_u64);
        for (i, &(count, delta)) in runs.iter().enumerate() {
            // The run's packets that aren't the track's last one.
            let body = if i + 1 == runs.len() {
                count - 1
            } else {
                count
            };
            if odd.is_none() && body > 0 && u64::from(delta) != AAC_FRAME {
                odd = Some((start + 1, delta));
            }
            start += u64::from(count);
        }
        if let Some((packet, delta)) = odd {
            outcome.fails.push(format!(
                "track {n}'s packet {packet} lasts {delta}, not 1024"
            ));
        }
        if let Some(&(_, last)) = runs.last() {
            outcome.require((1..=AAC_FRAME).contains(&u64::from(last)), || {
                format!("track {n}'s last packet lasts {last}")
            });
        }
        let (bytes, ticks) = (track.total_bytes(), track.stts_duration());
        bitrates.push((ticks > 0).then(|| {
            u64::try_from(u128::from(bytes) * 8 * u128::from(RATE) / u128::from(ticks))
                .unwrap_or(u64::MAX)
        }));
    }
    outcome.value = json!({"tracks": audio.len(), "average_bps": bitrates});
    outcome.detail = format!("{} tracks of AAC LC 48 kHz", audio.len());
    outcome
}

/// C12: the durations agree with the samples and each other, the video is as long as the
/// profile says, and every audio track ends with the video.
pub(crate) fn duration(ctx: &Ctx<'_>) -> Outcome {
    let mut outcome = Outcome::default();
    let movie_scale = ctx.movie.mvhd.map_or(0, |m| u64::from(m.timescale));
    let mut longest = 0_u64;
    for (index, track) in ctx.movie.tracks.iter().enumerate() {
        let n = index + 1;
        let (Some(mdhd), Some(tkhd)) = (track.mdhd, track.tkhd) else {
            continue;
        };
        let sum = track.stts_duration();
        outcome.require(mdhd.duration == sum, || {
            format!(
                "track {n}'s mdhd duration {} isn't the stts sum {sum}",
                mdhd.duration
            )
        });
        if track.has_edts {
            // An edit list sets the track's length (no_editlist reports the list itself).
            let edits: u64 = track.edits.iter().map(|e| e.segment_duration).sum();
            outcome.require(tkhd.duration.abs_diff(edits) <= 1, || {
                format!(
                    "track {n}'s tkhd duration {} isn't its edits' {edits} ± 1",
                    tkhd.duration
                )
            });
        } else if mdhd.timescale > 0 {
            // u128: a long track times a large movie timescale passes 2^64.
            let want = u64::try_from(
                (u128::from(sum) * u128::from(movie_scale)).div_ceil(u128::from(mdhd.timescale)),
            )
            .unwrap_or(u64::MAX);
            outcome.require(tkhd.duration.abs_diff(want) <= 1, || {
                format!(
                    "track {n}'s tkhd duration {} isn't ⌈{sum} × {movie_scale} / {}⌉ = {want} ± 1",
                    tkhd.duration, mdhd.timescale
                )
            });
        }
        longest = longest.max(tkhd.duration);
    }
    if let Some(mvhd) = ctx.movie.mvhd {
        outcome.require(mvhd.duration.abs_diff(longest) <= 1, || {
            format!(
                "mvhd duration {} isn't the longest track's {longest} ± 1",
                mvhd.duration
            )
        });
    }
    // The video's length.
    let frames = ctx.samples.len() as u64;
    let gop = ctx.timing.gop_frames();
    match ctx.plan.length {
        Length::Frames {
            expected,
            tolerance,
            source,
        } => {
            outcome.limit = json!({"frames": expected, "tolerance": tolerance, "source": source});
            outcome.require(frames.abs_diff(expected) <= tolerance, || {
                format!("{frames} frames; want {expected} ± {tolerance}")
            });
        }
        Length::WithinGop { seconds } => {
            if let (Some(fps), Some(g)) = (ctx.timing.whole_fps(), gop) {
                let want = seconds * fps;
                outcome.limit = json!({"frames": want, "tolerance": g, "source": "PLAN:446 (± 1 GOP), PLAN:162"});
                if frames.abs_diff(want) > g {
                    outcome
                        .fails
                        .push(format!("{frames} frames; want {want} ± {g} (± 1 GOP)"));
                } else if frames < want {
                    outcome.warns.push(format!(
                        "{frames} frames: shorter than the {want} asked for"
                    ));
                }
            } else {
                outcome
                    .fails
                    .push("the frame rate isn't whole, so the length can't be judged".to_owned());
            }
        }
        Length::Any => {
            outcome.limit =
                json!({"frames": null, "source": "no length rule without an expect file"});
        }
    }
    // Every audio track ends within [−1 tick, +1,024 samples + 1 tick] of the video's end.
    let t = ctx.timing.timescale;
    let video_end = ctx.video.stts_duration();
    let mut ends = Vec::new();
    for (index, track) in ctx.audio() {
        let a = track.mdhd.map_or(0, |m| u64::from(m.timescale));
        if a == 0 || t == 0 {
            continue;
        }
        // In units of 1 / (a · t) seconds.
        let gap = i128::from(track.stts_duration()) * i128::from(t)
            - i128::from(video_end) * i128::from(a);
        let (low, high) = (-i128::from(a), i128::from(AAC_FRAME * t + a));
        let samples = gap * i128::from(RATE) / (i128::from(a) * i128::from(t));
        ends.push(samples);
        outcome.require((low..=high).contains(&gap), || {
            format!("audio track {} ends {samples:+} samples from the video's end; want −1 tick to +1024 samples", index + 1)
        });
    }
    let seconds = if t > 0 {
        video_end as f64 / t as f64
    } else {
        0.0
    };
    outcome.value =
        json!({"frames": frames, "seconds": seconds, "audio_end_offsets_samples": ends});
    outcome.detail = format!("{frames} frames, {seconds:.3} s; durations agree");
    outcome
}
