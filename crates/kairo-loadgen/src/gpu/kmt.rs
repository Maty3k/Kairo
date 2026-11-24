// SPDX-License-Identifier: GPL-3.0-or-later
//! The D3DKMT read-outs (docs/research/17-loadgen-bench/README.md §3.10): the adapter's graphics
//! preemption granularity (`KMTQAITYPE_WDDM_1_2_CAPS`, the header's `preempt=`) and whether
//! hardware-accelerated GPU scheduling is on (`KMTQAITYPE_WDDM_2_7_CAPS`, `hags=`). Both come
//! from `D3DKMTQueryAdapterInfo` on a handle to loadgen's own adapter, opened by its LUID with
//! `D3DKMTOpenAdapterFromLuid` and closed at once: read-only, and no process is involved.
//! Microsoft marks the WDDM 2.7 caps "reserved for system use", so they serve logging only. A
//! read that fails gives `-` or `unknown`, and the run goes on.

use kairo_display::Luid;
use windows::Wdk::Graphics::Direct3D::{
    D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO, D3DKMT_WDDM_1_2_CAPS,
    D3DKMT_WDDM_2_7_CAPS, D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo,
    KMTQAITYPE_WDDM_1_2_CAPS, KMTQAITYPE_WDDM_2_7_CAPS, KMTQUERYADAPTERINFOTYPE,
};
use windows::Win32::Foundation::LUID;

use crate::log::Hags;

/// What the read-outs found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterCaps {
    /// `D3DKMDT_GRAPHICS_PREEMPTION_GRANULARITY`'s raw value (`log::preempt_name`), if read.
    pub preempt: Option<i32>,
    /// Hardware-accelerated GPU scheduling.
    pub hags: Hags,
}

/// Fills `data` with the adapter's information of `kind`; `true` if the call succeeded.
fn query<T>(adapter: u32, kind: KMTQUERYADAPTERINFOTYPE, data: &mut T) -> bool {
    let mut info = D3DKMT_QUERYADAPTERINFO {
        hAdapter: adapter,
        Type: kind,
        pPrivateDriverData: (data as *mut T).cast(),
        PrivateDriverDataSize: size_of::<T>() as u32,
    };
    // SAFETY: `info` points at `data`, which is ours and as large as the size it gives, so the
    // call writes inside it; the handle is our own open adapter's.
    unsafe { D3DKMTQueryAdapterInfo(&mut info) }.is_ok()
}

/// Reads the adapter with `luid`'s preemption granularity and HAGS state.
pub fn read(luid: Luid) -> AdapterCaps {
    let mut open = D3DKMT_OPENADAPTERFROMLUID {
        AdapterLuid: LUID {
            LowPart: luid.low,
            HighPart: luid.high,
        },
        hAdapter: 0,
    };
    // SAFETY: `open` is ours and complete; the call writes the handle into it. The handle is to
    // loadgen's own adapter, and it is closed below.
    if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) }.is_err() {
        return AdapterCaps {
            preempt: None,
            hags: Hags::Unknown,
        };
    }
    let adapter = open.hAdapter;
    let mut wddm_1_2 = D3DKMT_WDDM_1_2_CAPS::default();
    let preempt = query(adapter, KMTQAITYPE_WDDM_1_2_CAPS, &mut wddm_1_2)
        .then_some(wddm_1_2.PreemptionCaps.GraphicsPreemptionGranularity.0);
    let mut wddm_2_7 = D3DKMT_WDDM_2_7_CAPS::default();
    let hags = if query(adapter, KMTQAITYPE_WDDM_2_7_CAPS, &mut wddm_2_7) {
        // SAFETY: reads the union as its plain 32 bits, which every bit pattern is.
        Hags::from_wddm_2_7_caps(unsafe { wddm_2_7.Anonymous.Value })
    } else {
        Hags::Unknown
    };
    let close = D3DKMT_CLOSEADAPTER { hAdapter: adapter };
    // SAFETY: closes the handle opened above, once.
    let _ = unsafe { D3DKMTCloseAdapter(&close) };
    AdapterCaps { preempt, hags }
}
