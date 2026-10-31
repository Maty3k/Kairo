// SPDX-License-Identifier: GPL-3.0-or-later
//! The audio checks (docs/research/16-verify-testbed/README.md §6.3–6.5, signals.md §3.3–3.7):
//! `av.offset`, `av.offset_tight`, `av.inter_track`, `audio.tone_snr`, `audio.routing` and
//! `audio.cue`, from each audio track's (L+R)/2 and the pictures' barcode line.
//!
//! Audio sample s of a track plays at s / 48 kHz on the clip's timeline, AAC priming included,
//! and a picture at its own sample's presentation time: both from Kairo's sample tables, with no
//! edit list (`container.no_editlist` owns that). The game beep of flash m is expected where the
//! barcode line puts testbed frame F·m; the voice and mic beeps are found by scanning the tracks
//! that should carry them, and every track is then measured at those times.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::Outcome;
use crate::analyze::Line;
use crate::analyze::audio::{self, Beep, CueMatch, Detector, MS, RATE};
use crate::expect::Tone;
use crate::profile::{self, Plan, Profile};

/// The tones the clips carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    /// The testbed's 1 kHz beep, at each flash.
    Game,
    /// The voice-tone process's 2 kHz beep, at k s + 0.5 s on its own clock.
    Voice,
    /// The 3 kHz mic marker, at k s + 0.25 s in synthetic clips.
    Mic,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Game, Kind::Voice, Kind::Mic];

    pub fn hz(self) -> f64 {
        match self {
            Self::Game => 1_000.0,
            Self::Voice => 2_000.0,
            Self::Mic => 3_000.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Voice => "voice",
            Self::Mic => "mic",
        }
    }
}

/// What a track should carry (README §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Expected {
    pub game: Tone,
    pub voice: Tone,
    pub mic: Tone,
    pub cue: Tone,
}

impl Expected {
    fn tone(&self, kind: Kind) -> Tone {
        match kind {
            Kind::Game => self.game,
            Kind::Voice => self.voice,
            Kind::Mic => self.mic,
        }
    }

    /// Track `index`, titled `title`: the expect file's tones when it lists that track with
    /// tones (a tone it leaves out is `any`, the cue `absent`); otherwise the engine's matrix
    /// (README §6.4, voice present in the Mix when a Voice chat track shows consent) or an
    /// export's single track.
    pub fn of(plan: &Plan, index: usize, title: &str, has_voice_track: bool) -> Self {
        if let Some(track) = plan
            .expect
            .as_ref()
            .and_then(|e| e.tracks.get(index))
            .filter(|t| t.title == title && !t.tones.is_empty())
        {
            let get = |name: &str, default| track.tones.get(name).copied().unwrap_or(default);
            return Self {
                game: get("game", Tone::Any),
                voice: get("voice", Tone::Any),
                mic: get("mic", Tone::Any),
                cue: get("cue", Tone::Absent),
            };
        }
        let (present, absent, any) = (Tone::Present, Tone::Absent, Tone::Any);
        if plan.profile.is_share_export() {
            let voice = if plan.voice_included { present } else { absent };
            return Self {
                game: present,
                voice,
                mic: any,
                cue: absent,
            };
        }
        let (game, voice) = match title {
            "Mix" => (present, if has_voice_track { present } else { absent }),
            "Game" => (present, absent),
            "Voice chat" => (absent, present),
            "Everything else" => (absent, absent),
            _ => (any, any),
        };
        Self {
            game,
            voice,
            mic: any,
            cue: absent,
        }
    }
}

/// One audio track as the checks see it.
pub(crate) struct Track<'a> {
    pub title: String,
    pub expected: Expected,
    /// Its (L+R)/2 at 48 kHz.
    pub samples: &'a [f32],
}

/// Where the game beeps belong: each flash m with its expected onset, in audio samples.
pub(crate) fn game_expected(
    line: &Line,
    numbers: &[Option<i64>],
    pts: &[i64],
    timescale: u64,
    flash_fps: u32,
) -> Vec<(i64, f64)> {
    // (testbed frame, clip time) of the readable frames after the held frame 0.
    let points: Vec<(i64, f64)> = numbers
        .iter()
        .zip(pts)
        .skip(1)
        .filter_map(|(n, &t)| Some(((*n)?, t as f64)))
        .collect();
    let (Some(first), Some(last)) = (
        points.iter().map(|p| p.0).min(),
        points.iter().map(|p| p.0).max(),
    ) else {
        return Vec::new();
    };
    let f = i64::from(flash_fps.max(1));
    let window = 0.5 * timescale as f64;
    let mut out = Vec::new();
    for m in first.div_euclid(f)..=last.div_euclid(f) + 1 {
        let n_f = f * m;
        if n_f < first || n_f > last {
            continue;
        }
        let predicted = line.alpha + line.beta * n_f as f64;
        // α_m: the 5th percentile of t − β·N over the frames within 0.5 s, so the line goes
        // through the freshest frames there (README §6.3 av.offset).
        let mut offsets: Vec<f64> = points
            .iter()
            .filter(|&&(_, t)| (t - predicted).abs() <= window)
            .map(|&(n, t)| t - line.beta * n as f64)
            .collect();
        if offsets.is_empty() {
            continue;
        }
        offsets.sort_by(f64::total_cmp);
        let alpha = offsets[((offsets.len() - 1) as f64 * 0.05).floor() as usize];
        let ticks = alpha + line.beta * n_f as f64;
        out.push((m, ticks / timescale as f64 * RATE));
    }
    out
}

/// The beeps and levels each track shows at every event, worked out once for all six checks.
pub(crate) struct Audio<'a> {
    tracks: Vec<Track<'a>>,
    /// Each flash m with its expected game onset (samples).
    game: Vec<(i64, f64)>,
    /// Per track, per tone, the beep measured at each event (the game's at each flash).
    beeps: Vec<BTreeMap<Kind, Vec<Option<Beep>>>>,
    /// Per track, per tone, the narrowband level in each event's steady part.
    levels: Vec<BTreeMap<Kind, Vec<f64>>>,
    /// Per track, per tone without events, the loudest 80 ms window of a sliding scan.
    scans: Vec<BTreeMap<Kind, f64>>,
    /// Per track, the cue's best match.
    cues: Vec<Option<CueMatch>>,
    /// Why there are no game windows, when there aren't.
    no_game: Option<String>,
}

/// How far a beep is searched for around where it should be: ±250 ms around a flash (A/V up to
/// that), ±100 ms around another track's beep.
const FLASH_RADIUS: f64 = 250.0 * MS;
const EVENT_RADIUS: f64 = 100.0 * MS;

/// Prefers the track titled `first`, then the Mix, then the others in order.
fn preference(tracks: &[Track<'_>], first: &str) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    order.sort_by_key(|&i| match tracks[i].title.as_str() {
        t if t == first => 0,
        "Mix" => 1,
        _ => 2,
    });
    order
}

impl<'a> Audio<'a> {
    /// Measures `tracks` against the game beeps' expected onsets, or `no_game` saying why there
    /// are none. `cue` is the cue's PCM at 48 kHz.
    pub fn new(tracks: Vec<Track<'a>>, game: Result<Vec<(i64, f64)>, String>, cue: &[f32]) -> Self {
        let detectors: BTreeMap<Kind, Detector> = Kind::ALL
            .iter()
            .map(|&k| (k, Detector::new(k.hz())))
            .collect();
        let (game, no_game) = match game {
            Ok(game) => (game, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        // Each tone's reference onsets (samples): the game's from the beeps found nearest each
        // flash, the others from a scan.
        let mut events: BTreeMap<Kind, Vec<f64>> = BTreeMap::new();
        let mut beeps: Vec<BTreeMap<Kind, Vec<Option<Beep>>>> =
            tracks.iter().map(|_| BTreeMap::new()).collect();
        // The game: every track measured at each flash; the reference onset is the first
        // carrying track's beep that counts, else the expected time.
        let game_detector = &detectors[&Kind::Game];
        for (i, track) in tracks.iter().enumerate() {
            beeps[i].insert(
                Kind::Game,
                game.iter()
                    .map(|&(_, e)| game_detector.measure(track.samples, e, FLASH_RADIUS))
                    .collect(),
            );
        }
        let carriers = preference(&tracks, "Game");
        events.insert(
            Kind::Game,
            game.iter()
                .enumerate()
                .map(|(j, &(_, e))| {
                    carriers
                        .iter()
                        .filter(|&&i| tracks[i].expected.game == Tone::Present)
                        .find_map(|&i| beeps[i][&Kind::Game][j].filter(|b| game_detector.counts(b)))
                        .map_or(e, |b| b.at)
                })
                .collect(),
        );
        // Voice and mic: scanned in the tracks that should carry them, the dedicated one
        // first; every track then measured near those onsets.
        for (kind, first) in [(Kind::Voice, "Voice chat"), (Kind::Mic, "Mic")] {
            let detector = &detectors[&kind];
            let found = preference(&tracks, first)
                .into_iter()
                .filter(|&i| tracks[i].expected.tone(kind) == Tone::Present)
                .map(|i| detector.scan(tracks[i].samples))
                .find(|onsets| !onsets.is_empty())
                .unwrap_or_default();
            let onsets: Vec<f64> = found.iter().map(|b| b.at).collect();
            for (i, track) in tracks.iter().enumerate() {
                beeps[i].insert(
                    kind,
                    onsets
                        .iter()
                        .map(|&at| detector.measure(track.samples, at, EVENT_RADIUS))
                        .collect(),
                );
            }
            events.insert(kind, onsets);
        }
        // Levels in each event's steady part; a tone with no events is scanned instead, outside
        // the game windows (± 10 ms).
        let game_windows: Vec<(f64, f64)> = events[&Kind::Game]
            .iter()
            .map(|&at| (at - 10.0 * MS, at + 110.0 * MS))
            .collect();
        let mut levels = Vec::with_capacity(tracks.len());
        let mut scans = Vec::with_capacity(tracks.len());
        for track in &tracks {
            let mut per_kind = BTreeMap::new();
            let mut scanned = BTreeMap::new();
            for kind in Kind::ALL {
                let onsets = &events[&kind];
                if onsets.is_empty() {
                    let skip: &[(f64, f64)] = if kind == Kind::Game {
                        &[]
                    } else {
                        &game_windows
                    };
                    scanned.insert(kind, scan_level(track.samples, kind.hz(), skip));
                    continue;
                }
                per_kind.insert(
                    kind,
                    onsets
                        .iter()
                        .filter_map(|&at| {
                            let (start, len) = audio::plateau(at)?;
                            audio::level_db(track.samples, kind.hz(), start, len)
                        })
                        .collect(),
                );
            }
            levels.push(per_kind);
            scans.push(scanned);
        }
        let cues = tracks
            .iter()
            .map(|t| audio::match_cue(t.samples, cue))
            .collect();
        Self {
            tracks,
            game,
            beeps,
            levels,
            scans,
            cues,
            no_game,
        }
    }

    /// Each track's A/V offsets in ms, game-carrying tracks from their own beeps and the others
    /// through the Mix (README §6.3 av.offset): (offsets, how).
    fn offsets(&self) -> Vec<Option<(Vec<f64>, &'static str)>> {
        let game_detector = Detector::new(Kind::Game.hz());
        let direct: Vec<Option<Vec<f64>>> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, track)| {
                (track.expected.game == Tone::Present).then(|| {
                    self.beeps[i][&Kind::Game]
                        .iter()
                        .zip(&self.game)
                        .filter_map(|(b, &(_, e))| {
                            b.filter(|b| game_detector.counts(b))
                                .map(|b| (b.at - e) / MS)
                        })
                        .collect()
                })
            })
            .collect();
        // The anchor: the Mix if it times the game beep, else the first track that does.
        let anchor = preference(&self.tracks, "Mix")
            .into_iter()
            .find(|&i| direct[i].as_ref().is_some_and(|o| !o.is_empty()));
        (0..self.tracks.len())
            .map(|i| {
                if let Some(o) = direct[i].as_ref().filter(|o| !o.is_empty()) {
                    return Some((o.clone(), "game beeps"));
                }
                let a = anchor.filter(|&a| a != i)?;
                let base = median(direct[a].as_deref()?)?;
                // A voice-only or mic-only track: the anchor's offset plus its own beeps'
                // distance from the anchor's.
                [Kind::Voice, Kind::Mic].into_iter().find_map(|kind| {
                    if self.tracks[i].expected.tone(kind) != Tone::Present {
                        return None;
                    }
                    let d = Detector::new(kind.hz());
                    let gaps: Vec<f64> = self.beeps[i][&kind]
                        .iter()
                        .zip(&self.beeps[a][&kind])
                        .filter_map(|(x, y)| {
                            let (x, y) = (x.filter(|b| d.counts(b))?, y.filter(|b| d.counts(b))?);
                            ((x.at - y.at).abs() <= 5.0 * MS).then_some((x.at - y.at) / MS)
                        })
                        .collect();
                    let gap = median(&gaps)?;
                    Some((
                        vec![base + gap],
                        if kind == Kind::Voice {
                            "voice beeps, through the Mix"
                        } else {
                            "mic markers, through the Mix"
                        },
                    ))
                })
            })
            .collect()
    }
}

/// The loudest narrowband level of `hz` over 80 ms windows every 40 ms, skipping those that
/// touch `skip`; −240 dBFS for a track too short for one.
fn scan_level(x: &[f32], hz: f64, skip: &[(f64, f64)]) -> f64 {
    let len = (80.0 * MS) as usize;
    let hop = (40.0 * MS) as usize;
    let mut loudest = audio::db(0.0);
    let mut start = 0;
    while start + len <= x.len() {
        let (a, b) = (start as f64, (start + len) as f64);
        if !skip.iter().any(|&(s, e)| a < e && b > s)
            && let Some(level) = audio::level_db(x, hz, start, len)
        {
            loudest = loudest.max(level);
        }
        start += hop;
    }
    loudest
}

/// The median of `values`, if any.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(sorted[sorted.len() / 2])
}

fn fmt_ms(v: f64) -> String {
    format!("{v:+.2} ms")
}

/// `av.offset`: each track's beeps against the pictures, every event within 40 ms.
pub(crate) fn av_offset(audio: &Audio<'_>) -> Outcome {
    if let Some(why) = &audio.no_game {
        return Outcome::cannot(why.clone());
    }
    let limit = profile::AV_MAX_MS;
    let mut outcome = Outcome {
        limit: json!({"max_abs_ms": limit, "every_event": true, "source": "PLAN:351, README §6.5"}),
        ..Outcome::default()
    };
    let mut values = Vec::new();
    let mut medians = Vec::new();
    for (track, found) in audio.tracks.iter().zip(audio.offsets()) {
        let Some((offsets, how)) = found else {
            values.push(json!({"track": track.title, "measured": false}));
            continue;
        };
        let (min, max) = offsets
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &o| {
                (a.min(o), b.max(o))
            });
        let mid = median(&offsets).unwrap_or(0.0);
        medians.push(format!("{} {}", track.title, fmt_ms(mid)));
        values.push(json!({"track": track.title, "measured": true, "via": how,
                           "events": offsets.len(), "median_ms": mid, "min_ms": min, "max_ms": max}));
        for (j, &o) in offsets.iter().enumerate() {
            outcome.require(o.abs() <= limit, || {
                format!(
                    "{}: beep {j} is {} off the pictures",
                    track.title,
                    fmt_ms(o)
                )
            });
        }
    }
    if medians.is_empty() {
        return Outcome::cannot("no track carries a beep to time");
    }
    outcome.value = json!({"tracks": values});
    outcome.detail = medians.join(", ");
    outcome
}

/// `av.offset_tight`: M3 task 9's pair on the game-carrying tracks, median ≤10 ms and maximum
/// ≤21.3 ms.
pub(crate) fn av_offset_tight(audio: &Audio<'_>) -> Outcome {
    if let Some(why) = &audio.no_game {
        return Outcome::cannot(why.clone());
    }
    let (median_limit, max_limit) = profile::AV_TIGHT_MS;
    let mut outcome = Outcome {
        limit: json!({"median_abs_ms": median_limit, "max_abs_ms": max_limit, "source": "CB:1193 (M3 task 9)"}),
        ..Outcome::default()
    };
    let mut measured = 0;
    for (track, found) in audio.tracks.iter().zip(audio.offsets()) {
        let Some((offsets, "game beeps")) = found else {
            continue;
        };
        measured += 1;
        let mid = median(&offsets).unwrap_or(0.0);
        let worst = offsets.iter().fold(0.0_f64, |a, &o| a.max(o.abs()));
        outcome.require(mid.abs() <= median_limit, || {
            format!(
                "{}: median {}, over {median_limit} ms",
                track.title,
                fmt_ms(mid)
            )
        });
        outcome.require(worst <= max_limit, || {
            format!(
                "{}: a beep {worst:.2} ms off, over {max_limit} ms",
                track.title
            )
        });
    }
    if measured == 0 {
        return Outcome::cannot("no track carries the game beep to time");
    }
    outcome.detail = format!("{measured} tracks within {median_limit}/{max_limit} ms");
    outcome
}

/// `av.inter_track`: the same beep on the Mix and on each other track that carries it agrees
/// within ±0.5 ms.
pub(crate) fn av_inter_track(audio: &Audio<'_>) -> Outcome {
    let limit = profile::AV_INTER_TRACK_MS;
    let mut outcome = Outcome {
        limit: json!({"max_abs_ms": limit, "source": "README §6.3 (proposed), PLAN:191"}),
        ..Outcome::default()
    };
    let Some(mix) = audio.tracks.iter().position(|t| t.title == "Mix") else {
        return Outcome::skip("no Mix track to compare the others with");
    };
    let mut pairs = Vec::new();
    for kind in Kind::ALL {
        if audio.tracks[mix].expected.tone(kind) != Tone::Present {
            continue;
        }
        let d = Detector::new(kind.hz());
        for (i, track) in audio.tracks.iter().enumerate() {
            if i == mix || track.expected.tone(kind) != Tone::Present {
                continue;
            }
            let gaps: Vec<f64> = audio.beeps[i][&kind]
                .iter()
                .zip(&audio.beeps[mix][&kind])
                .filter_map(|(x, y)| {
                    let (x, y) = (x.filter(|b| d.counts(b))?, y.filter(|b| d.counts(b))?);
                    ((x.at - y.at).abs() <= 5.0 * MS).then_some((x.at - y.at) / MS)
                })
                .collect();
            if gaps.is_empty() {
                continue;
            }
            let worst = gaps.iter().fold(0.0_f64, |a, &g| a.max(g.abs()));
            pairs.push(json!({"tone": kind.name(), "track": track.title, "beeps": gaps.len(), "max_abs_ms": worst}));
            outcome.require(worst <= limit, || {
                format!(
                    "{} beeps on {} and the Mix are up to {worst:.3} ms apart",
                    kind.name(),
                    track.title
                )
            });
        }
    }
    if pairs.is_empty() {
        return Outcome::skip("no beep is on both the Mix and another track");
    }
    outcome.detail = format!("{} track pairs within ±{limit} ms", pairs.len());
    outcome.value = json!({"pairs": pairs});
    outcome
}

/// `audio.tone_snr`: each beep a track should carry, fitted over its steady part, jointly with
/// any other tone there: SNR ≥35 dB and within 1 Hz. In engine clips with a Mic track, the Mix's
/// SNR only warns (room noise, README §6.5).
pub(crate) fn tone_snr(audio: &Audio<'_>, plan: &Plan) -> Outcome {
    let (min_db, hz_tolerance) = (profile::TONE_SNR_MIN_DB, profile::TONE_HZ_TOLERANCE);
    let mut outcome = Outcome {
        limit: json!({"min_snr_db": min_db, "hz": hz_tolerance, "source": "README §6.5 (proposed), signals.md §3.5"}),
        ..Outcome::default()
    };
    let has_mic = audio.tracks.iter().any(|t| t.title == "Mic");
    let mut worst: Option<f64> = None;
    let mut fitted = 0;
    for (i, track) in audio.tracks.iter().enumerate() {
        let soft = plan.profile == Profile::Engine && track.title == "Mix" && has_mic;
        for kind in Kind::ALL {
            if track.expected.tone(kind) != Tone::Present {
                continue;
            }
            for beep in audio.beeps[i][&kind].iter().flatten() {
                if beep.level_db < audio::BEEP_MIN_DB {
                    continue;
                }
                let Some((start, len)) = audio::plateau(beep.at) else {
                    continue;
                };
                // Other tones whose beeps overlap this steady part fit jointly.
                let mut freqs = vec![kind.hz()];
                for other in Kind::ALL {
                    if other != kind
                        && audio.beeps[i][&other].iter().flatten().any(|b| {
                            b.level_db >= audio::BEEP_MIN_DB
                                && b.at < (start + len) as f64
                                && b.at + 100.0 * MS > start as f64
                        })
                    {
                        freqs.push(other.hz());
                    }
                }
                let Some(fit) = audio::fit(track.samples, &freqs, start, len) else {
                    continue;
                };
                fitted += 1;
                worst = Some(worst.map_or(fit.snr_db, |w: f64| w.min(fit.snr_db)));
                let at_s = beep.at / RATE;
                if fit.snr_db < min_db {
                    outcome.violation(!soft, format!(
                        "{}: the {} beep at {at_s:.3} s is {:.1} dB over its noise, under {min_db}",
                        track.title,
                        kind.name(),
                        fit.snr_db
                    ));
                }
                if (beep.hz - kind.hz()).abs() > hz_tolerance {
                    outcome.violation(!soft, format!(
                        "{}: the {} beep at {at_s:.3} s is at {:.2} Hz, not {} ± {hz_tolerance}",
                        track.title,
                        kind.name(),
                        beep.hz,
                        kind.hz()
                    ));
                }
            }
        }
    }
    if fitted == 0 {
        return Outcome::skip("no beep a track should carry was found (audio.routing)");
    }
    outcome.value = json!({"beeps": fitted, "min_snr_db": worst});
    outcome.detail = format!("{fitted} beeps, worst {:.1} dB", worst.unwrap_or(0.0));
    outcome
}

/// `audio.routing`: each tone's narrowband level in each track against what the track should
/// carry: present at ≥−60 dBFS in ≥90% of its windows, absent at ≤−70 dBFS in every one, and a
/// leak in between, which fails an absent route and warns on a present one (README §2 item 16).
pub(crate) fn routing(audio: &Audio<'_>) -> Outcome {
    let (present_db, share, absent_db) = (
        profile::ROUTING_PRESENT_DB,
        profile::ROUTING_PRESENT_SHARE,
        profile::ROUTING_ABSENT_DB,
    );
    let mut outcome = Outcome {
        limit: json!({"present_db": present_db, "present_share": share, "absent_db": absent_db,
                      "source": "README §2 item 16 (proposed)"}),
        ..Outcome::default()
    };
    let mut table = Vec::new();
    for (i, track) in audio.tracks.iter().enumerate() {
        for kind in Kind::ALL {
            let want = track.expected.tone(kind);
            let (heard, loudest) = match audio.levels[i].get(&kind) {
                Some(levels) if !levels.is_empty() => {
                    let loud = levels.iter().filter(|&&l| l >= present_db).count();
                    let loudest = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let heard = if loud as f64 >= share * levels.len() as f64 {
                        Tone::Present
                    } else if loudest <= absent_db {
                        Tone::Absent
                    } else {
                        Tone::Any
                    };
                    (heard, loudest)
                }
                _ => {
                    // No beep of this tone anywhere: present can't hold; absent is the scan's.
                    let loudest = audio.scans[i].get(&kind).copied().unwrap_or(audio::db(0.0));
                    let heard = if loudest <= absent_db {
                        Tone::Absent
                    } else {
                        Tone::Any
                    };
                    (heard, loudest)
                }
            };
            table.push(json!({"track": track.title, "tone": kind.name(), "want": want,
                              "heard": match heard { Tone::Any => "leak", Tone::Present => "present", Tone::Absent => "absent" },
                              "loudest_db": loudest}));
            let what = format!(
                "{} carries the {} beep at up to {loudest:.1} dBFS",
                track.title,
                kind.name()
            );
            match (want, heard) {
                (Tone::Any, _) | (Tone::Present, Tone::Present) | (Tone::Absent, Tone::Absent) => {}
                (Tone::Present, Tone::Any) => outcome
                    .warns
                    .push(format!("{what}: only some of its beeps, or faint")),
                (Tone::Present, Tone::Absent) => outcome.fails.push(format!(
                    "{}: no {} beep, which it should carry",
                    track.title,
                    kind.name()
                )),
                (Tone::Absent, _) => outcome.fails.push(format!("{what}, which it shouldn't")),
            }
        }
    }
    outcome.detail = format!("{} tracks match their routes", audio.tracks.len());
    outcome.value = Value::from(table);
    outcome
}

/// `audio.cue`: Kairo's cue by matched filter in every track: present at NCC ≥0.5 and
/// ≥−70 dBFS, possible from 0.2 (a warning), against where it should be (absent everywhere
/// unless an expect file says otherwise; README §6.3).
pub(crate) fn cue(audio: &Audio<'_>) -> Outcome {
    let (present_ncc, possible_ncc, min_db) = (
        profile::CUE_PRESENT_NCC,
        profile::CUE_POSSIBLE_NCC,
        profile::CUE_MIN_DB,
    );
    let mut outcome = Outcome {
        limit: json!({"present_ncc": present_ncc, "possible_ncc": possible_ncc, "min_db": min_db,
                      "source": "README §6.3 (proposed), signals.md §3.7"}),
        ..Outcome::default()
    };
    let mut table = Vec::new();
    for (track, found) in audio.tracks.iter().zip(&audio.cues) {
        let (ncc, level, at) = found.map_or((0.0, audio::db(0.0), 0.0), |c| {
            (c.ncc, c.level_db, c.at as f64 / RATE)
        });
        let present = ncc >= present_ncc && level >= min_db;
        let possible = !present && ncc >= possible_ncc;
        table.push(json!({"track": track.title, "ncc": ncc, "level_db": level, "at_s": at}));
        match track.expected.cue {
            Tone::Absent if present => outcome.fails.push(format!(
                "{}: the cue at {at:.3} s (NCC {ncc:.3}, {level:.1} dBFS), which it shouldn't carry",
                track.title
            )),
            Tone::Absent if possible => outcome.warns.push(format!(
                "{}: something like the cue at {at:.3} s (NCC {ncc:.3})",
                track.title
            )),
            Tone::Present if !present => outcome.fails.push(format!(
                "{}: no cue (best NCC {ncc:.3}), which it should carry",
                track.title
            )),
            _ => {}
        }
    }
    outcome.detail = format!("{} tracks checked for the cue", audio.tracks.len());
    outcome.value = Value::from(table);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_and_export_tracks_get_the_matrix() {
        let engine = Plan::new(Profile::Engine, false, None).unwrap();
        let mix = Expected::of(&engine, 0, "Mix", true);
        assert_eq!(
            (mix.game, mix.voice, mix.cue),
            (Tone::Present, Tone::Present, Tone::Absent)
        );
        assert_eq!(Expected::of(&engine, 0, "Mix", false).voice, Tone::Absent);
        let voice = Expected::of(&engine, 3, "Voice chat", true);
        assert_eq!((voice.game, voice.voice), (Tone::Absent, Tone::Present));
        let share = Plan::new(Profile::Share, false, None).unwrap();
        assert_eq!(Expected::of(&share, 0, "Mix", false).voice, Tone::Absent);
    }

    #[test]
    fn the_scan_skips_the_game_windows() {
        let mut x = vec![0.0_f32; 96_000];
        for (n, v) in x.iter_mut().enumerate().skip(24_000).take(4_800) {
            *v =
                kairo_core::testsig::tone_burst_at(2_000.0, 100, (n - 24_000) as f64 / RATE) as f32;
        }
        assert!(scan_level(&x, 2_000.0, &[]) > -31.0);
        assert!(scan_level(&x, 2_000.0, &[(23_000.0, 30_000.0)]) < -200.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
    }
}
