use ffmpeg_next::{self as ff, ffi, packet::Mut as _, packet::Ref as _};
use crate::hw::check;

pub struct OutStream { pub params: ff::codec::Parameters, pub tb: ff::Rational, pub default: bool, pub title: &'static str }
pub struct SnapPacket { pub pkt: ff::Packet, pub out_index: usize, pub pts: i64, pub dts: i64, pub duration: i64, pub tb: ff::Rational }

/// One AVFormatContext per save, on the save thread. `tmp` must be a real file (faststart re-opens it by URL),
/// on the library volume (atomic rename). Packets arrive in global dts order (compare with av_compare_ts).
pub fn write_clip(tmp: &std::path::Path, streams: &[OutStream], packets: &[SnapPacket], meta: ff::Dictionary) -> anyhow::Result<()> {
    let mut octx = ff::format::output_as(tmp, "mp4")?;
    for s in streams {
        let mut st = octx.add_stream(ff::codec::Id::None)?;
        st.set_parameters(s.params.clone());  // codecpar from the encoder session (extradata, colour)
        st.set_time_base(s.tb);               // a hint only: movenc rewrites it in write_header
        let mut md = ff::Dictionary::new();
        md.set("title", s.title); md.set("handler_name", s.title); md.set("language", "und");
        st.set_metadata(md);
        unsafe {
            (*(*st.as_mut_ptr()).codecpar).codec_tag = 0; // let movenc pick avc1/hvc1/av01/mp4a
            // DEFAULT on the mix only: >1 default per type switches movenc to per-stream groups
            (*st.as_mut_ptr()).disposition = if s.default { ffi::AV_DISPOSITION_DEFAULT as i32 } else { 0 };
        }
    }
    octx.set_metadata(meta);
    let mut o = ff::Dictionary::new();
    o.set("movflags", "+faststart");
    o.set("use_editlist", "0");             // bf=0: no edts; avoid_negative_ts auto -> make_zero
    let unused = octx.write_header_with(o)?.iter().count(); // the returned Dictionary borrows octx: drop it here
    anyhow::ensure!(unused == 0, "unused muxer options");
    let tbs: Vec<ff::Rational> = octx.streams().map(|s| s.time_base()).collect(); // AFTER the header (60 fps -> 1/15360)
    for p in packets {
        let mut pkt = ff::Packet::empty();
        unsafe { check(ffi::av_packet_ref(pkt.as_mut_ptr(), p.pkt.as_ptr()))?; } // refcount bump; Packet::clone() deep-copies
        pkt.set_stream(p.out_index);
        pkt.set_pts(Some(p.pts)); pkt.set_dts(Some(p.dts)); pkt.set_duration(p.duration);
        pkt.rescale_ts(p.tb, tbs[p.out_index]);
        pkt.write_interleaved(&mut octx)?;  // consumes this ref
    }
    octx.write_trailer()?;                  // runs the faststart shift
    Ok(())                                  // then File::sync_all + std::fs::rename into Videos\Kairo\<Game>\
}

pub fn clip_metadata(title: &str, json: &str, version: &str, when_iso: &str) -> ff::Dictionary<'static> {
    let mut m = ff::Dictionary::new();
    m.set("title", title); m.set("comment", "Captured with Kairo");
    m.set("encoding_tool", &format!("Kairo {version}")); // replaces the Lavf ident
    m.set("description", json);                             // mirrors the library row for rescans
    m.set("date", when_iso); m.set("creation_time", when_iso);
    m
}
