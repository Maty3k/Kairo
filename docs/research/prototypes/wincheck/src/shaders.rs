use windows::core::{s, PCSTR};
use windows::Win32::Graphics::Direct3D::{Fxc::*, ID3DBlob, ID3DInclude};
/// Runtime HLSL compile (d3dcompiler_47.dll via raw-dylib: no import lib, works under cargo-xwin).
pub unsafe fn compile(src: &str, entry: PCSTR, target: PCSTR) -> anyhow::Result<Vec<u8>> { unsafe {
    let (mut code, mut errs): (Option<ID3DBlob>, Option<ID3DBlob>) = (None, None);
    let r = D3DCompile(src.as_ptr().cast(), src.len(), s!("kairo_nv12.hlsl"), None, None::<&ID3DInclude>, entry, target,
                       D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS, 0, &mut code, Some(&mut errs as *mut _));
    if let Err(e) = r {
        let msg = errs.map(|b| String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned());
        anyhow::bail!("D3DCompile {e}: {}", msg.unwrap_or_default());
    }
    let b = code.unwrap();
    Ok(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec())
}}
pub fn _use() { let _ = unsafe { compile("", s!("PS_Y"), s!("ps_5_0")) }; }
