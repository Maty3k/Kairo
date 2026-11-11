// SPDX-License-Identifier: GPL-3.0-or-later
//! What the bitstream says about the encoder's options, for the harness's read-back table
//! (docs/research/16-verify-testbed/README.md §8.5): in-band parameter sets at each IDR (AMF's
//! `header_spacing`) and filler data units (`filler_data`), reported as values without changing
//! any check's status.

use crate::checks::Status;
use crate::expect::Expect;
use crate::profile::{Plan, Profile, STRUCTURE_CHECKS};
use crate::testkit::Spec;
use crate::{Report, check};

fn report(spec: &Spec) -> Report {
    let text = r#"{"kairo_expect":1,"size":[320,180],"fps":60,"seconds":2,"tracks":[
        {"title":"Mix","channels":2,"default":true},{"title":"Game","channels":2,"default":false},
        {"title":"Mic","channels":1,"default":false}]}"#;
    let plan = Plan::new(Profile::Engine, false, Some(Expect::parse(text).unwrap())).unwrap();
    check(&mut spec.build().as_slice(), "clip.mp4", &plan)
}

fn value(report: &Report, id: &str, key: &str) -> serde_json::Value {
    let finding = report.checks.iter().find(|f| f.id == id).unwrap();
    finding.value[key].clone()
}

#[test]
fn parameter_sets_and_filler_units_are_counted() {
    // The good clip carries its parameter sets only in avcC, and no filler.
    let good = report(&Spec::good());
    assert_eq!(value(&good, "video.params", "filler_nals"), 0);
    assert_eq!(value(&good, "video.params", "inband_pps"), 0);
    assert_eq!(value(&good, "video.gop", "idr_with_params"), 0);
    // An SPS and a PPS before each of the two IDRs, as AMF writes them, and one filler unit:
    // counted, and every structure check still passes.
    let amf = report(&Spec::with(|s| {
        s.params_at_idr = true;
        s.filler_at = Some(5);
    }));
    for id in STRUCTURE_CHECKS {
        assert_eq!(
            amf.checks.iter().find(|f| f.id == id).unwrap().status,
            Status::Pass,
            "{id}: {}",
            amf.to_text()
        );
    }
    assert_eq!(value(&amf, "video.params", "inband_sps"), 2);
    assert_eq!(value(&amf, "video.params", "inband_pps"), 2);
    assert_eq!(value(&amf, "video.params", "filler_nals"), 1);
    assert_eq!(value(&amf, "video.gop", "idr"), 2);
    assert_eq!(value(&amf, "video.gop", "idr_with_params"), 2);
}
