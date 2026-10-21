//! Per-source aligner (research 10 §5.4): QPC timestamps -> 48 kHz timeline; PI loop on placement error
//! drives rubato's async resampler ratio. Pure Rust: unit-tested on the Mac.
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler};
use std::collections::VecDeque;

const CHUNK: usize = 480;            // 10 ms
const HARD: f64 = 2400.0;            // > 50 ms error => re-anchor (silence fill or trim)
const CLIP: f64 = 96.0;              // innovation clip (2 ms) against jitter/outliers
const ALPHA: f64 = 0.02;             // ~0.5 s low-pass on the error
const TAU: f64 = 15.0;
const KP: f64 = 2.0 / (TAU * 48_000.0);
const KI: f64 = KP / TAU;
const LIM: f64 = 1.001;              // +-1000 ppm

pub struct SourceAligner {
    ch: usize, rs: Async<f32>, in_acc: Vec<f32>, scratch: Vec<f32>,
    out: VecDeque<f32>, out_pos: i64, pred: f64, err_lp: f64, integ: f64, pub ratio: f64, locked: bool,
}

impl SourceAligner {
    pub fn new(ch: usize) -> Self {
        let rs = Async::<f32>::new_poly(1.0, 1.01, PolynomialDegree::Septic, CHUNK, ch, FixedAsync::Input).expect("rubato");
        let scratch = vec![0.0; rs.output_frames_max() * ch];
        Self { ch, rs, in_acc: Vec::with_capacity(CHUNK * ch * 8), scratch, out: VecDeque::with_capacity(48_000 * ch),
               out_pos: 0, pred: 0.0, err_lp: 0.0, integ: 0.0, ratio: 1.0, locked: false }
    }
    /// `ts` = timeline sample index of the packet's first frame (from BufferInfo.timestamp, 100 ns QPC).
    pub fn push(&mut self, ts: i64, data: &[f32], disc: bool, ts_err: bool) {
        let frames = data.len() / self.ch;
        let e = ts as f64 - self.pred;
        if !self.locked || disc || e.abs() > HARD { self.resync(ts); }
        else if !ts_err {                                   // TIMESTAMP_ERROR: place by prediction only
            let innov = (e - self.err_lp).clamp(-CLIP, CLIP);
            self.err_lp += ALPHA * innov;
            self.integ += self.err_lp * frames as f64 / 48_000.0;
            self.ratio = (1.0 + KP * self.err_lp + KI * self.integ).clamp(1.0 / LIM, LIM);
            let _ = self.rs.set_resample_ratio_relative(self.ratio, true);
        }
        self.pred += frames as f64 * self.ratio;
        self.in_acc.extend_from_slice(data);
        while self.in_acc.len() >= CHUNK * self.ch {
            let n_in = self.rs.input_frames_next();
            let input = InterleavedSlice::new(&self.in_acc[..n_in * self.ch], self.ch, n_in).unwrap();
            let cap = self.scratch.len() / self.ch;
            let mut output = InterleavedSlice::new_mut(&mut self.scratch, self.ch, cap).unwrap();
            let (_, written) = self.rs.process_into_buffer(&input, &mut output, None).unwrap();
            self.out.extend(&self.scratch[..written * self.ch]);
            self.in_acc.drain(..n_in * self.ch);
        }
    }
    fn resync(&mut self, ts: i64) {
        self.rs.reset();
        self.in_acc.clear();
        let start = ts - self.rs.output_delay() as i64;
        let end = self.out_pos + (self.out.len() / self.ch) as i64;
        if self.out.is_empty() { self.out_pos = start; }
        else if start > end { self.out.extend(std::iter::repeat_n(0.0, (start - end) as usize * self.ch)); }
        else { self.out.truncate(((start - self.out_pos).max(0) as usize) * self.ch); } // real code: 2-5 ms fades
        self.pred = ts as f64; self.err_lp = 0.0; // keep `integ`: the rate estimate survives re-anchors
        self.locked = true;
    }
    /// Mixer: frames [pos, pos+n) into dst; missing = silence; late = dropped.
    pub fn read_block(&mut self, pos: i64, n: usize, dst: &mut [f32]) {
        let ch = self.ch;
        dst[..n * ch].fill(0.0);
        if self.out_pos < pos {
            let drop = (((pos - self.out_pos) as usize) * ch).min(self.out.len());
            self.out.drain(..drop);
            self.out_pos += (drop / ch) as i64;
            if self.out.is_empty() { self.out_pos = self.out_pos.max(pos + n as i64); return; }
        }
        let lead = ((self.out_pos - pos).max(0) as usize).min(n);
        let take = (self.out.len() / ch).min(n - lead);
        for (i, s) in self.out.drain(..take * ch).enumerate() { dst[lead * ch + i] = s; }
        self.out_pos += take as i64;
    }
}

#[cfg(test)]
mod t {
    use super::*;
    #[test] fn locks_onto_a_100ppm_fast_source() {
        let mut a = SourceAligner::new(2);
        let data = vec![0.1f32; 480 * 2];
        let ppm = 100e-6;
        let mut seed = 12345u64;
        let mut jitter = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 144.0 }; // +-1.5 ms
        let mut dst = vec![0f32; 480 * 2];
        for k in 0..(120 * 100) {                                    // 120 s of 10 ms packets
            let true_pos = k as f64 * 480.0 / (1.0 + ppm);           // source clock 100 ppm fast vs QPC
            a.push((true_pos + jitter()).round() as i64, &data, false, false);
            if k > 20 { a.read_block((k as i64 - 20) * 480, 480, &mut dst); } // mixer ~200 ms behind
        }
        assert!((a.ratio - 1.0 / (1.0 + ppm)).abs() < 20e-6, "ratio {}", a.ratio);
        assert!(a.err_lp.abs() < 24.0, "err {}", a.err_lp);          // < 0.5 ms placement error
    }
}
