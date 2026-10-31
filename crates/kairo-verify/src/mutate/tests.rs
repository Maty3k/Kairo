// SPDX-License-Identifier: GPL-3.0-or-later
//! `mutate` against the in-memory builder: rebuilding changes nothing, each change equals the
//! same defect built from scratch on eleven of the builder's file layouts, and each fails the
//! checks its defect names and no other (README §9's U tier: "each operation flips exactly its own
//! check"; `set-colr` flips `video.colr` and `video.params`, README §7.3).

use super::*;
use crate::checks::Status;
use crate::expect::Expect;
use crate::profile::{Plan, Profile};
use crate::testkit::{ColrSpec, Spec};

/// The engine profile with an expect file for the builder's 2 s clip.
fn engine() -> Plan {
    let text = r#"{"kairo_expect":1,"size":[320,180],"fps":60,"seconds":2,"tracks":[
        {"title":"Mix","channels":2,"default":true},
        {"title":"Game","channels":2,"default":false},
        {"title":"Mic","channels":1,"default":false}]}"#;
    Plan::new(Profile::Engine, false, Some(Expect::parse(text).unwrap())).unwrap()
}

fn failing(bytes: &[u8]) -> Vec<&'static str> {
    crate::check(&mut &bytes[..], "clip.mp4", &engine())
        .checks
        .iter()
        .filter(|f| f.status == Status::Fail)
        .map(|f| f.id)
        .collect()
}

fn mutated(input: &[u8], op: &Op) -> Result<Vec<u8>, MutateError> {
    let mut out = Vec::new();
    mutate(&mut &input[..], op, &mut out).map(|()| out)
}

fn colr_666() -> Op {
    Op::SetColr {
        primaries: 6,
        transfer: 6,
        matrix: 6,
        full_range: false,
    }
}

/// A change to the builder's good clip.
type Edit = fn(&mut Spec);

/// Each layout the builder makes, as a change to the good clip.
fn layouts() -> Vec<(&'static str, Edit)> {
    vec![
        ("good", |_| {}),
        ("version-1 boxes", |s| s.version1 = true),
        ("a 64-bit mdat", |s| s.largesize_mdat = true),
        ("co64", |s| s.co64 = true),
        ("a size-0 mdat", |s| s.mdat_to_end = true),
        ("many chunks", |s| s.video_chunk = 7),
        ("video last", |s| s.video_last = true),
        ("QuickTime meta", |s| s.qt_meta = true),
        ("udta filler", |s| s.udta_filler = 2),
        ("no pasp", |s| s.pasp = None),
        ("moov last", |s| s.faststart = false),
    ]
}

/// Each change, the same defect as a builder change, and the checks it must fail.
fn changes() -> Vec<(Op, Edit, &'static [&'static str])> {
    vec![
        (
            Op::UndoFaststart,
            |s| s.faststart = false,
            &["container.faststart"],
        ),
        (Op::AddElst, |s| s.edts = true, &["container.no_editlist"]),
        (
            colr_666(),
            |s| s.colr = vec![ColrSpec::Nclx(6, 6, 6, false, 0)],
            &["video.colr", "video.params"],
        ),
        (
            Op::SetColr {
                primaries: 1,
                transfer: 1,
                matrix: 1,
                full_range: true,
            },
            |s| s.colr = vec![ColrSpec::Nclx(1, 1, 1, true, 0)],
            &["video.colr", "video.params"],
        ),
        (Op::RemoveColr, |s| s.colr = Vec::new(), &["video.colr"]),
        (
            Op::SetDefault("Game".to_owned()),
            |s| s.audio[1].enabled = true,
            &["tracks.layout"],
        ),
        (
            Op::PerturbStts {
                sample: 100,
                ticks: 128,
            },
            |s| {
                s.deltas[99] += 128;
                s.deltas[100] -= 128;
            },
            &["video.cfr"],
        ),
    ]
}

#[test]
fn rebuilding_without_a_change_gives_the_input_back() {
    let mut all = layouts();
    all.push(("not faststart", |s| s.faststart = false));
    all.push(("an edit list", |s| s.edts = true));
    all.push(("no colr", |s| s.colr = Vec::new()));
    for (name, layout) in all {
        let input = Spec::with(layout).build();
        let mut out = Vec::new();
        rebuild(&mut &input[..], None, &mut out).unwrap();
        assert!(out == input, "{name}: the rebuild differs");
    }
}

#[test]
fn each_change_is_its_defect_built_from_scratch_and_fails_only_its_check() {
    for (name, layout) in layouts() {
        let input = Spec::with(layout).build();
        // Video after the audio already fails the track layout and moov last fails faststart;
        // nothing else may change.
        let before = failing(&input);
        let odd = matches!(name, "video last" | "moov last");
        assert_eq!(before.is_empty(), !odd, "{name}: {before:?}");
        for (op, defect, fails) in changes() {
            if name == "moov last" && op == Op::UndoFaststart {
                continue;
            }
            let out = mutated(&input, &op).unwrap_or_else(|e| panic!("{name}, {op}: {e}"));
            let expected = Spec::with(|s| {
                layout(s);
                defect(s);
            })
            .build();
            assert!(
                out == expected,
                "{name}, {op}: differs from the built defect"
            );
            let mut want: Vec<&str> = crate::profile::STRUCTURE_CHECKS
                .into_iter()
                .filter(|id| fails.contains(id) || before.contains(id))
                .collect();
            want.dedup();
            assert_eq!(failing(&out), want, "{name}, {op}");
        }
    }
}

#[test]
fn undoing_faststart_equals_the_built_moov_last_clip() {
    // The builder's layout without faststart is ftyp, free, mdat, moov; undoing faststart on the
    // good clip makes exactly that, so the offsets moved with the media.
    let good = Spec::good().build();
    let slow = mutated(&good, &Op::UndoFaststart).unwrap();
    assert!(slow == Spec::with(|s| s.faststart = false).build());
    assert!(matches!(
        mutated(&slow, &Op::UndoFaststart),
        Err(MutateError::NotApplicable(_))
    ));
    // Changes after moov moved still find the media.
    let both = mutated(&slow, &Op::AddElst).unwrap();
    assert_eq!(
        failing(&both),
        ["container.faststart", "container.no_editlist"]
    );
}

#[test]
fn a_size_0_mdat_that_is_no_longer_last_gets_its_size() {
    let input = Spec::with(|s| s.mdat_to_end = true).build();
    let out = mutated(&input, &Op::UndoFaststart).unwrap();
    let mp4 = crate::mp4::Mp4::read(&mut &out[..]).unwrap();
    let kinds: Vec<String> = mp4.top.iter().map(|b| b.kind.to_string()).collect();
    assert_eq!(kinds, ["ftyp", "free", "mdat", "moov"]);
    assert_eq!(
        &out[mp4.top[2].offset as usize..][..4],
        &(mp4.top[2].size as u32).to_be_bytes()
    );
}

#[test]
fn identity_edits_are_as_long_as_their_tracks() {
    let out = mutated(&Spec::good().build(), &Op::AddElst).unwrap();
    let clip = Clip::read(&mut &out[..]).unwrap();
    let movie = clip.movie.unwrap();
    assert_eq!(movie.tracks.len(), 4);
    for track in &movie.tracks {
        assert!(track.has_edts);
        let [edit] = track.edits.as_slice() else {
            panic!("{} edits", track.edits.len());
        };
        assert_eq!(edit.segment_duration, track.tkhd.unwrap().duration);
        assert_eq!((edit.media_time, edit.rate), (0, 0x0001_0000));
    }
    // A track too long for 32 bits gets a version-1 elst.
    let Body::Parent { children, .. } = identity_edit(1 << 33).body else {
        panic!("edts holds no boxes");
    };
    let Body::Raw(elst) = &children[0].body else {
        panic!("elst isn't raw");
    };
    assert_eq!(elst[0], 1);
    assert_eq!(&elst[8..16], &(1_u64 << 33).to_be_bytes());
}

#[test]
fn perturbing_moves_ticks_between_neighbours_and_keeps_the_total() {
    let runs = [(1, 584), (119, 256)];
    assert_eq!(
        perturb(&runs, 100, 128),
        Ok(vec![(1, 584), (98, 256), (1, 384), (1, 128), (19, 256)])
    );
    // Across two runs, and merged with a neighbour of the same length.
    assert_eq!(
        perturb(&runs, 1, 72),
        Ok(vec![(1, 656), (1, 184), (118, 256)])
    );
    assert_eq!(
        perturb(&[(1, 20), (1, 10), (1, 30), (1, 20)], 2, 10),
        Ok(vec![(4, 20)])
    );
    assert_eq!(
        perturb(&[(3, 10)], 2, 5),
        Ok(vec![(1, 10), (1, 15), (1, 5)])
    );
    // Zero-count runs disappear.
    assert_eq!(
        perturb(&[(0, 7), (3, 10)], 1, 5),
        Ok(vec![(1, 15), (1, 5), (1, 10)])
    );
    for (sample, ticks) in [(0, 1), (120, 1), (121, 1), (100, 0), (100, 256), (100, 300)] {
        assert!(perturb(&runs, sample, ticks).is_err(), "{sample} {ticks}");
    }
    assert!(perturb(&[(2, u32::MAX)], 1, 1).is_err());
}

#[test]
fn a_change_that_does_not_apply_is_refused() {
    let good = Spec::good().build();
    let refused = |input: &[u8], op: Op| {
        assert!(
            matches!(mutated(input, &op), Err(MutateError::NotApplicable(_))),
            "{op}"
        );
    };
    refused(&Spec::with(|s| s.edts = true).build(), Op::AddElst);
    let no_colr = Spec::with(|s| s.colr = Vec::new()).build();
    refused(&no_colr, Op::RemoveColr);
    refused(&no_colr, colr_666());
    // QuickTime's nclc isn't changed: set-colr writes nclx only.
    refused(
        &Spec::with(|s| s.colr = vec![ColrSpec::Other(*b"nclc")]).build(),
        colr_666(),
    );
    refused(&good, Op::SetDefault("Mix".to_owned()));
    refused(&good, Op::SetDefault("Voice chat".to_owned()));
    let two_games = Spec::with(|s| {
        s.audio[2].title = "Game".to_owned();
        s.audio[2].udta_name = Some("Game".to_owned());
    })
    .build();
    refused(&two_games, Op::SetDefault("Game".to_owned()));
    for (sample, ticks) in [(0, 1), (120, 1), (100, 0), (100, 256)] {
        refused(&good, Op::PerturbStts { sample, ticks });
    }
}

#[test]
fn a_damaged_or_cut_clip_is_refused_without_panicking() {
    let good = Spec::good().build();
    for cut in (0..good.len()).step_by(41) {
        let result = mutated(&good[..cut], &Op::AddElst);
        assert!(
            matches!(result, Err(MutateError::Unsound(_) | MutateError::Read(_))),
            "cut at {cut}: {result:?}"
        );
    }
    // A fragmented clip fails container.structure, so it isn't rebuilt.
    assert!(matches!(
        mutated(&Spec::with(|s| s.moof = true).build(), &Op::AddElst),
        Err(MutateError::Unsound(_))
    ));
}

/// A writer that fails once it has taken `room` bytes.
struct Full {
    room: usize,
}

impl Write for Full {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.room == 0 {
            return Err(io::Error::other("disk full"));
        }
        let n = buf.len().min(self.room);
        self.room -= n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_write_is_reported_as_one() {
    let good = Spec::good().build();
    let len = mutated(&good, &Op::RemoveColr).unwrap().len();
    for room in [0, 100, len - 1] {
        let result = mutate(&mut &good[..], &Op::RemoveColr, &mut Full { room });
        assert!(matches!(result, Err(MutateError::Write(_))), "{room}");
    }
    assert_eq!(MutateError::Write(io::Error::other("x")).exit_code(), 4);
    assert_eq!(MutateError::NotApplicable(String::new()).exit_code(), 1);
    assert_eq!(MutateError::Unsound(String::new()).exit_code(), 3);
    assert_eq!(MutateError::Read(io::Error::other("x")).exit_code(), 3);
}

#[test]
fn operations_print_as_the_command_line_writes_them() {
    let ops: Vec<String> = changes().iter().map(|(op, _, _)| op.to_string()).collect();
    assert_eq!(
        ops,
        [
            "undo-faststart",
            "add-elst",
            "set-colr 6 6 6 0",
            "set-colr 1 1 1 1",
            "remove-colr",
            "set-default \"Game\"",
            "perturb-stts 100 128"
        ]
    );
}

#[test]
fn set_colr_writes_nclx_only_and_refuses_a_change_to_nothing() {
    // A profile colr (ICC, 36 bytes here) is never touched: alone it is refused, beside nclx
    // only the nclx changes.
    let mut profile = b"prof".to_vec();
    profile.extend([0x5A; 32]);
    let alone = Spec::with(|s| s.colr = vec![ColrSpec::Payload(profile.clone())]).build();
    assert!(matches!(
        mutated(&alone, &colr_666()),
        Err(MutateError::NotApplicable(_))
    ));
    let both = Spec::with(|s| {
        s.colr = vec![
            ColrSpec::Nclx(1, 1, 1, false, 0),
            ColrSpec::Payload(profile.clone()),
        ]
    })
    .build();
    let expected = Spec::with(|s| {
        s.colr = vec![
            ColrSpec::Nclx(6, 6, 6, false, 0),
            ColrSpec::Payload(profile.clone()),
        ]
    })
    .build();
    assert!(mutated(&both, &colr_666()).unwrap() == expected);
    // The values the clip already holds change nothing, so they are refused.
    let same = Op::SetColr {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    };
    assert!(matches!(
        mutated(&Spec::good().build(), &same),
        Err(MutateError::NotApplicable(_))
    ));
}

#[test]
fn clips_with_other_file_offsets_are_refused() {
    // saio in stbl and iloc in a meta hold absolute offsets that wouldn't move.
    let saio = Spec::with(|s| {
        s.extra_stbl_box = Some(crate::testkit::full(
            b"saio",
            0,
            0,
            &[0, 0, 0, 1, 0, 0, 0, 0],
        ))
    })
    .build();
    let refused = mutated(&saio, &Op::AddElst);
    assert!(
        matches!(&refused, Err(MutateError::Unsound(what)) if what.contains("saio")),
        "{refused:?}"
    );
    // A top-level meta after the media, with its own iloc, is copied as it is but not moved.
    let mut meta = Spec::with(|s| s.faststart = true).build();
    meta.extend(crate::testkit::boxed(b"meta", &[0; 4]));
    let refused = mutated(&meta, &Op::AddElst);
    assert!(
        matches!(refused, Err(MutateError::Unsound(_))),
        "{refused:?}"
    );
}

#[test]
fn sixty_four_bit_sizes_and_uuid_boxes_inside_moov_are_kept() {
    // moov/udta with a 64-bit size and a uuid child (also 64-bit), made through the rebuild.
    let mut patched = Vec::new();
    rebuild_with(
        &mut &Spec::good().build()[..],
        &mut patched,
        |_, moov, _| {
            let udta = moov.child_mut(b"udta").unwrap();
            udta.large = true;
            udta.children_mut().unwrap().push(Node {
                kind: FourCc(*b"uuid"),
                uuid: Some([7; 16]),
                large: true,
                body: Body::Raw(vec![1, 2, 3]),
            });
            Ok(())
        },
    )
    .unwrap();
    let read = |bytes: &[u8]| {
        let clip = Clip::read(&mut &bytes[..]).unwrap();
        assert!(clip.problems.is_empty(), "{:?}", clip.problems);
        let udta = clip
            .mp4
            .moov
            .as_ref()
            .unwrap()
            .child(b"udta")
            .unwrap()
            .clone();
        (udta.header, udta.child(b"uuid").unwrap().header)
    };
    assert_eq!(read(&patched), (16, 32));
    assert!(failing(&patched).is_empty());
    // Rebuilding it gives it back, and a change that grows moov keeps both headers.
    let mut same = Vec::new();
    rebuild(&mut &patched[..], None, &mut same).unwrap();
    assert!(same == patched);
    let grown = mutated(&patched, &Op::AddElst).unwrap();
    assert_eq!(read(&grown), (16, 32));
    assert_eq!(failing(&grown), ["container.no_editlist"]);
}
