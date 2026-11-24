// SPDX-License-Identifier: GPL-3.0-or-later
//! Loadgen's own D3D11, DXGI and D3DKMT calls (docs/research/17-loadgen-bench/README.md §3.3–3.5,
//! §3.7, §3.8, §3.10): the flip-model swap chain with tearing and its latency wait, the shaders
//! compiled at run time from [`crate::workload::HLSL`] with `D3DCompile`, the constant buffer, the
//! bandwidth workload's texture and its fill, the scissored bands, the ring of timestamp queries,
//! and the read-outs (preemption granularity, HAGS, hardware composition, the present hints).
//! This is the only module of the crate allowed `unsafe`, and every block carries a `// SAFETY:`
//! comment (CLAUDE.md). Every call is on loadgen's own device, swap chain, window or adapter; the
//! window, the device on the monitor's adapter and `Present` itself are `kairo-window`'s.
//!
//! Not built yet: research 17's PR 4 fills this module in its second part, with Windows CI's
//! W-tier tests (§6): every entry point through `D3DCompile` with the production flags, the WARP
//! read-back held to [`crate::visual`], and a flip swap chain presenting on WARP.
