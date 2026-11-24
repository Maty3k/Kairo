// SPDX-License-Identifier: GPL-3.0-or-later
//! Loadgen's own GPU time (docs/research/17-loadgen-bench/README.md §3.8): a ring of [`RING`]
//! query sets, each `{TIMESTAMP_DISJOINT, TIMESTAMP, TIMESTAMP}` around one frame's draws, so one
//! disjoint query per frame, as Learn advises. Frame f uses set f mod [`RING`].
//!
//! After each `Present` the pending sets are read oldest first with `GetData(…,
//! D3D11_ASYNC_GETDATA_DONOTFLUSH)` through the vtable, comparing the result with `S_OK`: the
//! 0.62.2 wrapper ends in `.ok()`, which would take `S_FALSE` ("not yet available") for success.
//! Reading stops at the first set that isn't ready, so times come in frame order. A set still
//! pending when its slot comes round again gives its frame an unknown time (`gpu_unknown`), so a
//! measured run never waits on a query; only the end of a run reads with a flush, bounded
//! ([`Ring::drain`]).

use std::time::{Duration, Instant};

use windows::Win32::Foundation::S_OK;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_ASYNC_GETDATA_DONOTFLUSH, D3D11_QUERY, D3D11_QUERY_DATA_TIMESTAMP_DISJOINT,
    D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP, D3D11_QUERY_TIMESTAMP_DISJOINT, ID3D11Device,
    ID3D11DeviceContext, ID3D11Query,
};
use windows::core::Interface;

use crate::summary::GpuTime;

/// The query sets in the ring \[proposed\]: with latency 2 and 3 buffers a frame's times arrive
/// two or three frames later, so 8 leaves room.
pub const RING: usize = 8;

/// One frame's queries.
struct Set {
    disjoint: ID3D11Query,
    begin: ID3D11Query,
    end: ID3D11Query,
}

/// A query of `kind` on `device`.
fn query(device: &ID3D11Device, kind: D3D11_QUERY) -> Result<ID3D11Query, String> {
    let desc = D3D11_QUERY_DESC {
        Query: kind,
        MiscFlags: 0,
    };
    let mut made = None;
    // SAFETY: a complete description and an out-pointer to a local.
    unsafe { device.CreateQuery(&desc, Some(&mut made)) }.map_err(|e| format!("query: {e}"))?;
    made.ok_or_else(|| "query: none".to_owned())
}

/// A query's data if it is ready (`S_OK`); `None` while it isn't, or if reading failed.
fn data<T: Default>(context: &ID3D11DeviceContext, query: &ID3D11Query, flags: u32) -> Option<T> {
    let mut value = T::default();
    // SAFETY: GetData writes at most size_of::<T>() bytes into `value`, which is ours, and T is
    // the query's own data type (u64 for a timestamp, D3D11_QUERY_DATA_TIMESTAMP_DISJOINT for the
    // disjoint query). The call goes through the vtable so that S_FALSE ("not yet available")
    // isn't taken for success.
    let hr = unsafe {
        (Interface::vtable(context).GetData)(
            Interface::as_raw(context),
            query.as_raw(),
            (&raw mut value).cast(),
            size_of::<T>() as u32,
            flags,
        )
    };
    (hr == S_OK).then_some(value)
}

/// The ring of query sets on one device's immediate context.
pub struct Ring {
    context: ID3D11DeviceContext,
    sets: Vec<Set>,
    /// The frame each set times while its result is pending.
    pending: [Option<u64>; RING],
    /// The next frame to time.
    next: u64,
    /// The oldest frame whose time isn't reported yet: the frames from here to `next` are
    /// pending.
    oldest: u64,
}

impl Ring {
    /// [`RING`] query sets on `device`, read through `context`.
    pub fn new(device: &ID3D11Device, context: &ID3D11DeviceContext) -> Result<Ring, String> {
        let sets = (0..RING)
            .map(|_| {
                Ok(Set {
                    disjoint: query(device, D3D11_QUERY_TIMESTAMP_DISJOINT)?,
                    begin: query(device, D3D11_QUERY_TIMESTAMP)?,
                    end: query(device, D3D11_QUERY_TIMESTAMP)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Ring {
            context: context.clone(),
            sets,
            pending: [None; RING],
            next: 0,
            oldest: 0,
        })
    }

    fn slot(frame: u64) -> usize {
        (frame % RING as u64) as usize
    }

    /// Opens the next frame's set, before its draws: `Begin` on its disjoint query, `End` on its
    /// first timestamp. Returns the frame's number (from 0, one per call) and, if its set was
    /// still pending from frame − [`RING`], that frame, whose time is now unknown. That frame is
    /// always the oldest pending one, so times stay in frame order.
    pub fn begin(&mut self) -> (u64, Option<u64>) {
        let frame = self.next;
        let slot = Ring::slot(frame);
        let evicted = self.pending[slot].take();
        if let Some(old) = evicted {
            self.oldest = old + 1;
        }
        self.pending[slot] = Some(frame);
        self.next += 1;
        let set = &self.sets[slot];
        // SAFETY: our own queries on our own context, a disjoint query begun once per frame
        // around the timestamps (Begin on a query already in flight restarts it).
        unsafe {
            self.context.Begin(&set.disjoint);
            self.context.End(&set.begin);
        }
        (frame, evicted)
    }

    /// Closes the frame [`Ring::begin`] opened, after its draws: `End` on its second timestamp,
    /// then on its disjoint query.
    pub fn end(&mut self) {
        let Some(frame) = self.next.checked_sub(1) else {
            return;
        };
        let set = &self.sets[Ring::slot(frame)];
        // SAFETY: our own queries on our own context, closing the set begun above.
        unsafe {
            self.context.End(&set.end);
            self.context.End(&set.disjoint);
        }
    }

    /// Whether any frame's time is still to come.
    pub fn pending(&self) -> bool {
        self.oldest < self.next
    }

    /// The oldest pending frame's time, if its set is ready.
    fn read_oldest(&self, flags: u32) -> Option<GpuTime> {
        let set = &self.sets[Ring::slot(self.oldest)];
        let disjoint: D3D11_QUERY_DATA_TIMESTAMP_DISJOINT =
            data(&self.context, &set.disjoint, flags)?;
        let begin: u64 = data(&self.context, &set.begin, flags)?;
        let end: u64 = data(&self.context, &set.end, flags)?;
        Some(GpuTime::from_ticks(
            begin,
            end,
            disjoint.Frequency,
            disjoint.Disjoint.as_bool(),
        ))
    }

    /// Reads the pending sets oldest first into `out`, stopping at the first that isn't ready
    /// (or when `out` is full): `(frame, time)` in frame order. Never flushes unless `flush`.
    /// Returns how many it wrote.
    pub fn poll(&mut self, flush: bool, out: &mut [(u64, GpuTime)]) -> usize {
        let flags = if flush {
            0
        } else {
            D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32
        };
        let mut written = 0;
        while self.pending() && written < out.len() {
            let Some(time) = self.read_oldest(flags) else {
                break;
            };
            out[written] = (self.oldest, time);
            written += 1;
            self.pending[Ring::slot(self.oldest)] = None;
            self.oldest += 1;
        }
        written
    }

    /// The end of a run (README §3.8: "the final read may flush, bounded"): reads every pending
    /// set, flushing, until all are read or `limit` has passed; the frames left get an unknown
    /// time. `(frame, time)` into `out` in frame order; returns how many.
    pub fn drain(&mut self, limit: Duration, out: &mut [(u64, GpuTime); RING]) -> usize {
        let until = Instant::now() + limit;
        let mut written = 0;
        loop {
            written += self.poll(true, &mut out[written..]);
            if !self.pending() || Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        written + self.abandon(&mut out[written..])
    }

    /// Gives every pending frame an unknown time without reading, as when the device is gone:
    /// `(frame, Unknown)` into `out` in frame order; returns how many.
    pub fn abandon(&mut self, out: &mut [(u64, GpuTime)]) -> usize {
        let mut written = 0;
        while self.pending() && written < out.len() {
            out[written] = (self.oldest, GpuTime::Unknown);
            written += 1;
            self.pending[Ring::slot(self.oldest)] = None;
            self.oldest += 1;
        }
        written
    }
}
