// SPDX-License-Identifier: GPL-3.0-or-later
//! The export checks that need no decoding (README §6.5; PLAN:243–249): `export.size`,
//! `export.shape` and `export.tags`.

use serde_json::json;

use super::{Ctx, Outcome};
use crate::mp4::tracks::SampleEntry;
use crate::profile::{self, Profile};

/// The video's width and height from its sample entry.
fn dimensions(ctx: &Ctx<'_>) -> Option<(u32, u32)> {
    match ctx.entry() {
        Some(SampleEntry::Visual { width, height, .. }) => {
            Some((u32::from(*width), u32::from(*height)))
        }
        _ => None,
    }
}

/// `export.size`: Copy for Discord ≤19,000,000 bytes; Reels ≤300 MB.
pub(crate) fn size(ctx: &Ctx<'_>) -> Outcome {
    let bytes = ctx.clip.mp4.file_len;
    let (limit, source) = match ctx.plan.profile {
        Profile::Discord => (profile::DISCORD_MAX_BYTES, "PLAN:243"),
        Profile::Reels => (profile::REELS_MAX_BYTES, "PLAN:245"),
        _ => return Outcome::skip("no size limit"),
    };
    let mut outcome = Outcome {
        value: json!({"bytes": bytes}),
        limit: json!({"max_bytes": limit, "source": source}),
        detail: format!("{bytes} bytes"),
        ..Outcome::default()
    };
    outcome.require(bytes <= limit, || format!("{bytes} bytes; at most {limit}"));
    outcome
}

/// `export.shape`: the picture size, rate and length each export allows.
pub(crate) fn shape(ctx: &Ctx<'_>) -> Outcome {
    let profile = ctx.plan.profile;
    let mut outcome = Outcome::default();
    let Some((w, h)) = dimensions(ctx) else {
        outcome.fails.push("no visual sample entry".to_owned());
        return outcome;
    };
    // The clip's own rate: an export is judged as it is, whatever was expected.
    let Some((num, den)) = ctx.timing.measured.map(|m| m.fps).or(ctx.timing.fps) else {
        outcome
            .fails
            .push("no frame rate (see video.cfr)".to_owned());
        return outcome;
    };
    let (short, long) = (w.min(h), w.max(h));
    // fps ≤ n ⟺ num ≤ n · den.
    let at_most = |n: u64| num <= n * den;
    let fps = if den == 1 {
        num.to_string()
    } else {
        format!("{num}/{den}")
    };
    outcome.value = json!({"size": [w, h], "fps": fps});
    outcome.detail = format!("{w}x{h} at {fps} fps");
    match profile {
        Profile::Reels => {
            let (rw, rh) = profile::REELS_SIZE;
            outcome.limit = json!({"size": [rw, rh], "fps": [profile::REELS_FPS.0, profile::REELS_FPS.1],
                                   "seconds": [profile::REELS_SECONDS.0, profile::REELS_SECONDS.1],
                                   "source": "PLAN:245, research 06:156"});
            outcome.require((w, h) == (rw, rh), || {
                format!("{w}x{h}; Reels want {rw}x{rh}")
            });
            let (lo, hi) = profile::REELS_FPS;
            outcome.require(num >= lo * den && at_most(hi), || {
                format!("{fps} fps; Reels want {lo}–{hi}")
            });
            let t = ctx.timing.timescale;
            let ticks = ctx.video.stts_duration();
            let (min, max) = profile::REELS_SECONDS;
            outcome.require(t > 0 && ticks >= min * t && ticks <= max * t, || {
                format!(
                    "{:.1} s; Reels want {min} s to {} min",
                    ticks as f64 / t.max(1) as f64,
                    max / 60
                )
            });
        }
        _ => {
            // Share, Discord and WhatsApp: H.264 up to 1080p60.
            let (max_long, max_short, max_fps) = profile::SHARE_MAX;
            outcome.limit = json!({"max_size": [max_long, max_short], "either_orientation": true,
                                   "fps": max_fps, "source": "PLAN:246"});
            outcome.require(short <= max_short && long <= max_long && at_most(max_fps), || {
                format!("{w}x{h} at {fps} fps; at most {max_long}x{max_short} (either orientation) at {max_fps} fps")
            });
            if profile == Profile::Discord {
                let rung = profile::DISCORD_LADDER
                    .iter()
                    .any(|&(s, f)| short == s && den == 1 && num == u64::from(f));
                outcome.limit = json!({"ladder": profile::DISCORD_LADDER, "source": "PLAN:243"});
                outcome.require(rung, || {
                    format!("{short}p{fps} isn't on the Discord ladder")
                });
            }
            if profile == Profile::Whatsapp {
                let (lines, rate) = profile::WHATSAPP_MAX;
                outcome.limit = json!({"max_lines": lines, "fps": rate,
                                       "audio_bps": profile::WHATSAPP_AUDIO_BPS,
                                       "audio_bps_allowed": profile::WHATSAPP_AUDIO_LIMIT_BPS,
                                       "source": "PLAN:244 [proposed]; the allowance from Mac encodes"});
                outcome.require(short <= lines && at_most(rate), || {
                    format!("{w}x{h} at {fps} fps; at most {lines} lines at {rate} fps")
                });
                for (index, track) in ctx.audio() {
                    let (bytes, ticks) = (
                        u128::from(track.total_bytes()),
                        u128::from(track.stts_duration()),
                    );
                    let bps = (bytes * 8 * 48_000)
                        .checked_div(ticks)
                        .map_or(0, |b| u64::try_from(b).unwrap_or(u64::MAX));
                    outcome.require(bps <= profile::WHATSAPP_AUDIO_LIMIT_BPS, || {
                        format!(
                            "audio track {} averages {bps} bps; WhatsApp wants 128 kbps (at most {})",
                            index + 1,
                            profile::WHATSAPP_AUDIO_LIMIT_BPS
                        )
                    });
                }
            }
        }
    }
    outcome
}

/// `export.tags`: a share copy keeps only the title and the encoder tag (PLAN:249).
pub(crate) fn tags(ctx: &Ctx<'_>) -> Outcome {
    let keys: Vec<String> = ctx
        .movie
        .ilst_keys
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut outcome = Outcome {
        value: json!({"ilst_keys": keys}),
        limit: json!({"ilst_keys": ["\\xa9nam", "\\xa9too"], "source": "PLAN:249"}),
        detail: format!(
            "tags {}",
            if keys.is_empty() {
                "none".to_owned()
            } else {
                keys.join(", ")
            }
        ),
        ..Outcome::default()
    };
    for key in &ctx.movie.ilst_keys {
        outcome.require(profile::SHARE_TAGS.contains(&&key.0), || {
            format!("a {key} tag; a share copy keeps only ©nam and ©too")
        });
    }
    outcome
}
