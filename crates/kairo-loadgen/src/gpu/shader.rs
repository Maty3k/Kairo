// SPDX-License-Identifier: GPL-3.0-or-later
//! The shaders, compiled at run time (docs/research/17-loadgen-bench/README.md §2 item 3, §3.7):
//! `D3DCompile` from the inbox `d3dcompiler_47.dll`, which `windows` 0.62.2 imports raw-dylib, so
//! the Mac never needs an import library. Each entry point of [`crate::workload::HLSL`] compiles
//! with the production flags ([`FLAGS`]: optimisation level 3, strict, warnings as errors), as
//! `vs_5_0` or `ps_5_0`; `--detail off` adds the macro `KAIRO_DETAIL=0`. The source name is the
//! bare `loadgen.hlsl`, so the compiler's messages name no path, and there is no include handler
//! (the HLSL has no `#include`). A failed compile is exit 4, with the compiler's message.

use std::ffi::CString;

use windows::Win32::Graphics::Direct3D::Fxc::{
    D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCOMPILE_WARNINGS_ARE_ERRORS,
    D3DCompile,
};
use windows::Win32::Graphics::Direct3D::{D3D_SHADER_MACRO, ID3DBlob, ID3DInclude};
use windows::core::PCSTR;

use crate::summary::ShaderHashes;
use crate::workload::{DETAIL_MACRO, Entry, HLSL, HLSL_NAME, Workload};

/// The production flags: optimisation level 3, strict, warnings as errors (README §3.7).
pub const FLAGS: u32 =
    D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_WARNINGS_ARE_ERRORS;

/// A blob's bytes, copied out.
fn bytes(blob: &ID3DBlob) -> Vec<u8> {
    // SAFETY: the blob holds GetBufferSize() bytes at GetBufferPointer() for as long as it lives,
    // and they are copied before it goes.
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
    .to_vec()
}

/// A name for `D3DCompile`, NUL-terminated.
fn c_name(name: &str) -> Result<CString, String> {
    CString::new(name).map_err(|_| "a shader name holds a NUL".to_owned())
}

/// Compiles `entry` of [`HLSL`] with [`FLAGS`], with the picture's grain (`detail`) or without it
/// (`KAIRO_DETAIL=0`): its bytecode, or the compiler's message.
pub fn compile(entry: Entry, detail: bool) -> Result<Vec<u8>, String> {
    let source_name = c_name(HLSL_NAME)?;
    let function = c_name(entry.name)?;
    let target = c_name(entry.target)?;
    let macro_name = c_name(DETAIL_MACRO)?;
    let zero = c_name("0")?;
    // The list D3DCompile reads up to its closing null entry.
    let defines = [
        D3D_SHADER_MACRO {
            Name: PCSTR(macro_name.as_ptr().cast()),
            Definition: PCSTR(zero.as_ptr().cast()),
        },
        D3D_SHADER_MACRO {
            Name: PCSTR::null(),
            Definition: PCSTR::null(),
        },
    ];
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the source, the names and the macro list (closed by its null entry) outlive the
    // call; the two out-pointers are locals; with no include handler an `#include` would fail
    // to compile, and the HLSL has none.
    let compiled = unsafe {
        D3DCompile(
            HLSL.as_ptr().cast(),
            HLSL.len(),
            PCSTR(source_name.as_ptr().cast()),
            (!detail).then_some(defines.as_ptr()),
            None::<&ID3DInclude>,
            PCSTR(function.as_ptr().cast()),
            PCSTR(target.as_ptr().cast()),
            FLAGS,
            0,
            &mut code,
            Some(&raw mut errors),
        )
    };
    match (compiled, code) {
        (Ok(()), Some(code)) => Ok(bytes(&code)),
        (compiled, _) => {
            let message = errors
                .as_ref()
                .map(|blob| String::from_utf8_lossy(&bytes(blob)).into_owned())
                .unwrap_or_default();
            let message = message.trim_end_matches(['\0', '\r', '\n', ' ']);
            let code = compiled.err().map_or_else(
                || "no bytecode".to_owned(),
                |e| format!("{:#010x}", e.code().0 as u32),
            );
            Err(format!("{} didn't compile ({code}): {message}", entry.name))
        }
    }
}

/// A run's shaders, one per entry point of its workload, in `Workload::entries`' order: the
/// vertex shader, the workload's pixel shader, and `ps_fill` for bandwidth.
pub struct Shaders {
    workload: Workload,
    code: Vec<Vec<u8>>,
}

impl Shaders {
    /// Compiles `workload`'s entry points, with or without the grain.
    pub fn compile(workload: Workload, detail: bool) -> Result<Shaders, String> {
        let code = workload
            .entries()
            .iter()
            .map(|&entry| compile(entry, detail))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Shaders { workload, code })
    }

    /// The workload they were compiled for.
    pub fn workload(&self) -> Workload {
        self.workload
    }

    /// `entry`'s bytecode, if the workload compiles it.
    pub fn code(&self, entry: Entry) -> Option<&[u8]> {
        let index = self.workload.entries().iter().position(|&e| e == entry)?;
        self.code.get(index).map(Vec::as_slice)
    }

    /// Their FNV-1a 64 hashes, for the log header, the calibration line and the fingerprint.
    pub fn hashes(&self) -> Option<ShaderHashes> {
        let code: Vec<&[u8]> = self.code.iter().map(Vec::as_slice).collect();
        ShaderHashes::new(self.workload, &code)
    }
}
