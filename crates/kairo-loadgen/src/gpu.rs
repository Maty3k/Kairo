// SPDX-License-Identifier: GPL-3.0-or-later
//! Loadgen's own D3D11, DXGI and D3DKMT calls (docs/research/17-loadgen-bench/README.md §3.3–3.5,
//! §3.7, §3.8, §3.10). This is the only module of the crate allowed `unsafe`, and every block
//! carries a `// SAFETY:` comment (CLAUDE.md). Every call is on loadgen's own device, swap chain,
//! window or adapter; the device on the monitor's adapter, the back buffer's view, `Present` with
//! its lost device told apart and the frame-latency wait are `kairo-window`'s.
//!
//! - [`Chain`] (§3.3): a flip-discard swap chain of `--buffers` BGRA8 buffers at the monitor's
//!   size, with `ALLOW_TEARING | FRAME_LATENCY_WAITABLE_OBJECT`, `--latency` frames of latency
//!   and Alt+Enter switched off; every frame is `Present(0, DXGI_PRESENT_ALLOW_TEARING)`. It
//!   never resizes, and it is never made without the tearing flag outside the tests.
//! - [`Renderer`] (§3.4–3.5): the 32-byte constant buffer, filled each frame with
//!   `Map(WRITE_DISCARD)`, a rasterizer state with the scissor on and no culling, and one
//!   full-screen triangle from `SV_VertexID` drawn once per band, each scissored to its rows. The
//!   bandwidth workload's `R32_UINT` texture is filled once on the GPU by `ps_fill`, in 16
//!   bands, then bound as a shader resource.
//! - `shader` (§3.7): `D3DCompile` with the production flags; `queries` (§3.8): the ring of
//!   timestamp queries; `kmt` (§3.10): the preemption and HAGS read-outs.
//! - The present-mode hints (§3.3): hardware composition, the output's rotation and
//!   `GetFrameStatisticsMedia`'s composition mode, logged as hints, never verdicts. PresentMon's
//!   CSV says which present mode a run had.
//!
//! Never used: `WS_EX_LAYERED`, `WS_EX_TOPMOST`, `SetWindowDisplayAffinity`, the swap chain's
//! `DISPLAY_ONLY` flag (Kairo's capture must see loadgen in M2), and
//! `D3D11_CREATE_DEVICE_DISABLE_GPU_TIMEOUT`.

mod kmt;
mod queries;
mod shader;

pub use kmt::{AdapterCaps, read as adapter_caps};
pub use queries::{RING, Ring};
pub use shader::Shaders;

use kairo_window::gpu::{LatencyWait, PresentError, Presented, back_buffer};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_FILL_SOLID,
    D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE, D3D11_RASTERIZER_DESC, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT, ID3D11Buffer, ID3D11DepthStencilView,
    ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState,
    ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R32_UINT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_FEATURE_PRESENT_ALLOW_TEARING, DXGI_FRAME_STATISTICS_MEDIA, DXGI_MWA_NO_ALT_ENTER,
    DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGIFactory5, IDXGIOutput, IDXGIOutput6, IDXGISwapChain1, IDXGISwapChainMedia,
};
use windows::core::{BOOL, Interface};

use crate::workload::{
    self, FRAME_CONSTANTS_SIZE, FrameConstants, PS_FILL, Texture, VS_FULLSCREEN,
};

/// The bands `ps_fill` fills the bandwidth workload's texture in (README §3.5), whatever
/// `--bands` says: one integer hash a texel, trivial at any size.
const FILL_BANDS: u32 = 16;

/// Whether DXGI can present with `DXGI_PRESENT_ALLOW_TEARING` (README §3.3): without it loadgen
/// refuses (exit 3), and never falls back silently.
pub fn tearing_supported(factory: &IDXGIFactory2) -> bool {
    let Ok(factory) = factory.cast::<IDXGIFactory5>() else {
        return false;
    };
    let mut allowed = BOOL(0);
    // SAFETY: this feature's data is one BOOL; `allowed` is ours and its size is passed.
    let checked = unsafe {
        factory.CheckFeatureSupport(
            DXGI_FEATURE_PRESENT_ALLOW_TEARING,
            (&raw mut allowed).cast(),
            size_of::<BOOL>() as u32,
        )
    };
    checked.is_ok() && allowed.as_bool()
}

/// `IDXGIOutput6::CheckHardwareCompositionSupport`'s flags for the monitor's output, the header's
/// `hwcomp=` (a hint); `None` if there is no such output or call.
pub fn hardware_composition(output: Option<&IDXGIOutput>) -> Option<u32> {
    let output: IDXGIOutput6 = output?.cast().ok()?;
    // SAFETY: reads a property of the output that shows loadgen's monitor.
    unsafe { output.CheckHardwareCompositionSupport() }.ok()
}

/// The monitor's output's `DXGI_MODE_ROTATION`, the header's `rotation=`.
pub fn rotation(output: Option<&IDXGIOutput>) -> Option<i32> {
    let output = output?;
    // SAFETY: reads the output's description.
    unsafe { output.GetDesc() }.ok().map(|desc| desc.Rotation.0)
}

/// The user-mode driver's version on the device's adapter, as `IDXGIAdapter::CheckInterfaceSupport`
/// gives it for `IDXGIDevice` (cookbook §9.6): the ready file's `umd_version`.
pub fn umd_version(device: &ID3D11Device) -> Option<i64> {
    let dxgi: IDXGIDevice = device.cast().ok()?;
    // SAFETY: the adapter our own device is on.
    let adapter = unsafe { dxgi.GetAdapter() }.ok()?;
    // SAFETY: asks that adapter which user-mode driver answers for the interface; reads only.
    unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok()
}

/// `GetDeviceRemovedReason`'s HRESULT, as its bits, if the device is gone (`e device_lost
/// reason=`).
pub fn removed_reason(device: &ID3D11Device) -> Option<u32> {
    // SAFETY: reads our own device's state.
    unsafe { device.GetDeviceRemovedReason() }
        .err()
        .map(|e| e.code().0 as u32)
}

/// Loadgen's swap chain (README §3.3).
pub struct Chain {
    // Fields drop in order: the frame-latency wait's handle before the swap chain.
    waitable: LatencyWait,
    target: [Option<ID3D11RenderTargetView>; 1],
    _buffer: ID3D11Texture2D,
    media: Option<IDXGISwapChainMedia>,
    swap: IDXGISwapChain1,
    device: ID3D11Device,
    flags: DXGI_PRESENT,
}

impl Chain {
    /// A flip-discard swap chain on `device` (made through `factory`, the one its adapter came
    /// from) for window `hwnd`'s client area of `size`, with `buffers` buffers and `latency`
    /// frames of latency through the waitable object. `tearing` adds the tearing flag and
    /// presents with `DXGI_PRESENT_ALLOW_TEARING`; loadgen always asks for it, having refused a
    /// factory without it ([`tearing_supported`]), and only the tests go without.
    pub fn new(
        factory: &IDXGIFactory2,
        device: &ID3D11Device,
        hwnd: isize,
        size: (u32, u32),
        buffers: u32,
        latency: u32,
        tearing: bool,
    ) -> Result<Chain, String> {
        let mut flags = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32;
        if tearing {
            flags |= DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32;
        }
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size.0,
            Height: size.1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: buffers,
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: flags,
            ..Default::default()
        };
        let window = HWND(hwnd as *mut _);
        // SAFETY: our device, our own window and a complete description.
        let swap = unsafe { factory.CreateSwapChainForHwnd(device, window, &desc, None, None) }
            .map_err(|e| format!("swap chain: {e}"))?;
        // SAFETY: the factory that made the swap chain, for its own window: Alt+Enter never
        // switches it to exclusive fullscreen, where tearing presents are refused.
        unsafe { factory.MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER) }
            .map_err(|e| format!("window association: {e}"))?;
        let waitable = LatencyWait::new(&swap, latency)?;
        let (buffer, view) = back_buffer(device, &swap)?;
        Ok(Chain {
            waitable,
            target: [Some(view)],
            _buffer: buffer,
            media: swap.cast().ok(),
            swap,
            device: device.clone(),
            flags: if tearing {
                DXGI_PRESENT_ALLOW_TEARING
            } else {
                DXGI_PRESENT(0)
            },
        })
    }

    /// Waits up to `ms` for the swap chain to take another frame; `true` if it will.
    pub fn wait(&self, ms: u32) -> bool {
        self.waitable.wait(ms)
    }

    /// Back buffer 0's view, the frame's target: `Present` unbinds it, so every frame binds it
    /// again.
    pub fn target(&self) -> &[Option<ID3D11RenderTargetView>; 1] {
        &self.target
    }

    /// `Present(0, DXGI_PRESENT_ALLOW_TEARING)` (the flags 0 in the tests without tearing).
    pub fn present(&self) -> Result<Presented, PresentError> {
        kairo_window::gpu::present(&self.device, &self.swap, 0, self.flags)
    }

    /// `IDXGISwapChainMedia::GetFrameStatisticsMedia`'s composition mode, the present hint's
    /// `media=`. Documented for decode swap chains, so `None` on an RGB one is no surprise
    /// \[Unverified\].
    pub fn media_hint(&self) -> Option<i32> {
        let media = self.media.as_ref()?;
        let mut stats = DXGI_FRAME_STATISTICS_MEDIA::default();
        // SAFETY: `stats` is ours, written by the call.
        unsafe { media.GetFrameStatisticsMedia(&mut stats) }.ok()?;
        Some(stats.CompositionMode.0)
    }
}

/// The bandwidth workload's texture and its view for the reads (which holds the texture too).
struct Big {
    _texture: ID3D11Texture2D,
    view: [Option<ID3D11ShaderResourceView>; 1],
}

/// Draws the picture and the workload's load, in bands (README §3.4–3.5).
pub struct Renderer {
    context: ID3D11DeviceContext,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    buffer: ID3D11Buffer,
    constants: [Option<ID3D11Buffer>; 1],
    raster: ID3D11RasterizerState,
    big: Option<Big>,
    viewport: D3D11_VIEWPORT,
    scissors: Vec<RECT>,
}

/// A viewport over `width` × `height`.
fn viewport(width: u32, height: u32) -> D3D11_VIEWPORT {
    D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: width as f32,
        Height: height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }
}

/// One scissor rectangle per band of `bands` over `width` × `height`.
fn band_rects(width: u32, height: u32, bands: u32) -> Vec<RECT> {
    workload::bands(height, bands)
        .map(|(top, bottom)| RECT {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: bottom as i32,
        })
        .collect()
}

/// A vertex shader from `code`.
fn vertex_shader(device: &ID3D11Device, code: &[u8]) -> Result<ID3D11VertexShader, String> {
    let mut shader = None;
    // SAFETY: bytecode D3DCompile made for vs_5_0; an out-pointer to a local.
    unsafe { device.CreateVertexShader(code, None, Some(&mut shader)) }
        .map_err(|e| format!("vertex shader: {e}"))?;
    shader.ok_or_else(|| "vertex shader: none".to_owned())
}

/// A pixel shader from `code`.
fn pixel_shader(device: &ID3D11Device, code: &[u8]) -> Result<ID3D11PixelShader, String> {
    let mut shader = None;
    // SAFETY: bytecode D3DCompile made for ps_5_0; an out-pointer to a local.
    unsafe { device.CreatePixelShader(code, None, Some(&mut shader)) }
        .map_err(|e| format!("pixel shader: {e}"))?;
    shader.ok_or_else(|| "pixel shader: none".to_owned())
}

impl Renderer {
    /// The renderer of `shaders`' workload on `device`, drawing `size` in `bands` bands, with
    /// `texture` for the bandwidth workload (filled here, on the GPU, from its seed).
    pub fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        shaders: &Shaders,
        texture: Option<Texture>,
        size: (u32, u32),
        bands: u32,
    ) -> Result<Renderer, String> {
        let workload = shaders.workload();
        let code = |entry| {
            shaders
                .code(entry)
                .ok_or_else(|| format!("{} wasn't compiled", entry.name))
        };
        let vertex = vertex_shader(device, code(VS_FULLSCREEN)?)?;
        let pixel = pixel_shader(device, code(workload.pixel_shader())?)?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: FRAME_CONSTANTS_SIZE as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut buffer = None;
        // SAFETY: a complete description, no initial data, and an out-pointer to a local.
        unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }
            .map_err(|e| format!("constant buffer: {e}"))?;
        let buffer = buffer.ok_or("constant buffer: none")?;
        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: false.into(),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: true.into(),
            ScissorEnable: true.into(),
            MultisampleEnable: false.into(),
            AntialiasedLineEnable: false.into(),
        };
        let mut raster = None;
        // SAFETY: a complete description and an out-pointer to a local.
        unsafe { device.CreateRasterizerState(&raster_desc, Some(&mut raster)) }
            .map_err(|e| format!("rasterizer state: {e}"))?;
        let mut renderer = Renderer {
            context: context.clone(),
            vertex,
            pixel,
            constants: [Some(buffer.clone())],
            buffer,
            raster: raster.ok_or("rasterizer state: none")?,
            big: None,
            viewport: viewport(size.0, size.1),
            scissors: band_rects(size.0, size.1, bands),
        };
        if workload.uses_texture() {
            let texture = texture.ok_or("the bandwidth workload needs its texture")?;
            let fill = pixel_shader(device, code(PS_FILL)?)?;
            renderer.big = Some(renderer.fill(device, texture, &fill)?);
        }
        Ok(renderer)
    }

    /// Makes the bandwidth workload's `R32_UINT` texture and fills it with `ps_fill` from its
    /// seed, in [`FILL_BANDS`] bands; then unbinds it as a render target, so it can be read.
    fn fill(
        &self,
        device: &ID3D11Device,
        texture: Texture,
        fill: &ID3D11PixelShader,
    ) -> Result<Big, String> {
        let (width, height) = texture.size();
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R32_UINT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut made = None;
        // SAFETY: a complete description, no initial data, and an out-pointer to a local.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut made)) }
            .map_err(|e| format!("the {} MiB texture: {e}", texture.mib()))?;
        let big = made.ok_or("texture: none")?;
        let mut target = None;
        // SAFETY: a view of our own texture; an out-pointer to a local.
        unsafe { device.CreateRenderTargetView(&big, None, Some(&mut target)) }
            .map_err(|e| format!("the texture's render target: {e}"))?;
        let mut view = None;
        // SAFETY: as above.
        unsafe { device.CreateShaderResourceView(&big, None, Some(&mut view)) }
            .map_err(|e| format!("the texture's shader resource: {e}"))?;
        // ps_fill reads only the seed.
        self.set_constants(&FrameConstants::new(0, 0, Some(texture), (width, height)))?;
        let fill_target = [target];
        let ctx = &self.context;
        // SAFETY: binds our own texture's view, states and shaders to our own context.
        unsafe {
            ctx.OMSetRenderTargets(Some(&fill_target), None::<&ID3D11DepthStencilView>);
            ctx.RSSetViewports(Some(&[viewport(width, height)]));
            ctx.RSSetState(&self.raster);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None::<&ID3D11InputLayout>);
            ctx.VSSetShader(&self.vertex, None);
            ctx.PSSetShader(fill, None);
            ctx.PSSetConstantBuffers(0, Some(&self.constants));
        }
        for band in band_rects(width, height, FILL_BANDS) {
            // SAFETY: one scissored draw of the full-screen triangle, on our own context.
            unsafe {
                ctx.RSSetScissorRects(Some(&[band]));
                ctx.Draw(3, 0);
            }
        }
        // SAFETY: unbinds every render target from our own context, so the texture can be bound
        // as a shader resource.
        unsafe { ctx.OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>) };
        Ok(Big {
            _texture: big,
            view: [view],
        })
    }

    /// Writes this frame's constants into the constant buffer (`Map(WRITE_DISCARD)`).
    pub fn set_constants(&self, constants: &FrameConstants) -> Result<(), String> {
        let bytes = constants.bytes();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: maps our own dynamic buffer for writing; it is unmapped below.
        unsafe {
            self.context.Map(
                &self.buffer,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )
        }
        .map_err(|e| format!("constant buffer: {e}"))?;
        if !mapped.pData.is_null() {
            // SAFETY: Map gave a writable pointer to the buffer's FRAME_CONSTANTS_SIZE bytes,
            // which `bytes` doesn't overlap.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData.cast(), bytes.len());
            }
        }
        // SAFETY: the buffer mapped above, unmapped once.
        unsafe { self.context.Unmap(&self.buffer, 0) };
        if mapped.pData.is_null() {
            return Err("constant buffer: no memory mapped".to_owned());
        }
        Ok(())
    }

    /// Draws the frame into `target` with the constants set last: the pipeline's state, then one
    /// scissored `Draw(3, 0)` per band. It allocates nothing.
    pub fn draw(&self, target: &[Option<ID3D11RenderTargetView>; 1]) {
        let ctx = &self.context;
        // SAFETY: binds our own view, states, shaders, buffer and texture to our own context.
        unsafe {
            ctx.OMSetRenderTargets(Some(target), None::<&ID3D11DepthStencilView>);
            ctx.RSSetViewports(Some(&[self.viewport]));
            ctx.RSSetState(&self.raster);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None::<&ID3D11InputLayout>);
            ctx.VSSetShader(&self.vertex, None);
            ctx.PSSetShader(&self.pixel, None);
            ctx.PSSetConstantBuffers(0, Some(&self.constants));
            if let Some(big) = &self.big {
                ctx.PSSetShaderResources(0, Some(&big.view));
            }
        }
        for band in &self.scissors {
            // SAFETY: one scissored draw of the full-screen triangle, on our own context.
            unsafe {
                ctx.RSSetScissorRects(Some(std::slice::from_ref(band)));
                ctx.Draw(3, 0);
            }
        }
    }

    /// The bandwidth workload's texture.
    #[cfg(test)]
    fn texture(&self) -> Option<&ID3D11Texture2D> {
        self.big.as_ref().map(|big| &big._texture)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::summary::GpuTime;
    use crate::visual::{Picture, mix32};
    use crate::workload::{DEFAULT_SEED, ENTRIES, PS_ALU, PS_BW, Workload};
    use kairo_core::timeline::HNS_PER_SECOND;
    use kairo_window::gpu::create_device;
    use kairo_window::platform::{Frame, Window};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;

    /// Research 17 §6's WARP target, count, texture and bands.
    const SIZE: (u32, u32) = (320, 180);
    const COUNT: u32 = 4;
    const TEXTURE_MIB: u32 = 1;
    const BANDS: u32 = 4;

    /// Whether CI says it has a desktop (`KAIRO_CI_DESKTOP`, set by Windows CI's test step, as
    /// kairo-window's cover tests read it): there a missing window or swap chain fails, since a
    /// passing test's report never shows.
    fn on_ci_desktop() -> bool {
        std::env::var_os("KAIRO_CI_DESKTOP").is_some()
    }

    /// Reports `why` and skips the test, or fails it on CI's desktop.
    fn skip(why: &str) {
        assert!(!on_ci_desktop(), "KAIRO_CI_DESKTOP is set: {why}");
        eprintln!("{why}: reported, not failed");
    }

    /// The DXGI factory a device's adapter came from.
    fn factory_of(device: &ID3D11Device) -> IDXGIFactory2 {
        let dxgi: IDXGIDevice = device.cast().unwrap();
        // SAFETY: the adapter our own device is on, and the factory it came from.
        unsafe { dxgi.GetAdapter().unwrap().GetParent::<IDXGIFactory2>() }.unwrap()
    }

    fn texture_2d(
        device: &ID3D11Device,
        size: (u32, u32),
        format: DXGI_FORMAT,
        staging: bool,
    ) -> ID3D11Texture2D {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: if staging {
                D3D11_USAGE_STAGING
            } else {
                D3D11_USAGE_DEFAULT
            },
            BindFlags: if staging {
                0
            } else {
                D3D11_BIND_RENDER_TARGET.0 as u32
            },
            CPUAccessFlags: if staging {
                D3D11_CPU_ACCESS_READ.0 as u32
            } else {
                0
            },
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: a complete description and an out-pointer to a local.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }.unwrap();
        texture.unwrap()
    }

    /// `texture`'s pixels, 4 bytes each, row after row.
    fn read_back(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        size: (u32, u32),
        format: DXGI_FORMAT,
    ) -> Vec<u8> {
        let staging = texture_2d(device, size, format, true);
        // SAFETY: two textures of one size and format, neither mapped.
        unsafe { context.CopyResource(&staging, texture) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: maps our staging texture for reading; unmapped below.
        unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.unwrap();
        let row = size.0 as usize * 4;
        let mut out = Vec::with_capacity(row * size.1 as usize);
        for y in 0..size.1 as usize {
            // SAFETY: Map gave `size.1` rows of RowPitch bytes, each at least `row` long.
            let line = unsafe {
                std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    row,
                )
            };
            out.extend_from_slice(line);
        }
        // SAFETY: the texture mapped above, after its last read.
        unsafe { context.Unmap(&staging, 0) };
        out
    }

    /// Research 17 §6, W tier (1): every entry point compiles through D3DCompile with the
    /// production flags, with and without the grain, and twice gives the same bytes.
    #[test]
    fn every_entry_point_compiles_the_same_twice() {
        for entry in ENTRIES {
            for detail in [true, false] {
                let first = shader::compile(entry, detail)
                    .unwrap_or_else(|e| panic!("{} detail {detail}: {e}", entry.name));
                let second = shader::compile(entry, detail).unwrap();
                assert!(!first.is_empty(), "{}", entry.name);
                assert_eq!(first, second, "{} detail {detail}", entry.name);
                // DXBC: the container starts with its magic.
                assert_eq!(&first[..4], b"DXBC", "{}", entry.name);
            }
        }
        // --detail off drops the grain from the two shaders that draw the picture, and the
        // header's hashes show which it was.
        for entry in [PS_ALU, PS_BW] {
            assert_ne!(
                shader::compile(entry, true).unwrap(),
                shader::compile(entry, false).unwrap(),
                "{}",
                entry.name
            );
        }
        for workload in Workload::ALL {
            let on = Shaders::compile(workload, true).unwrap().hashes().unwrap();
            let off = Shaders::compile(workload, false).unwrap().hashes().unwrap();
            assert_ne!(on.text(), off.text());
            assert_eq!(on.text().split(',').count(), workload.entries().len());
        }
    }

    /// A frame's picture time: whole seconds and a fraction, past the scroll's first wrap.
    const TIMES: [i64; 3] = [0, 7 * HNS_PER_SECOND + 3_000_000, 720_000_000];

    /// Research 17 §6, W tier (2): on WARP, offscreen at 320 × 180, each workload at count 4 (a
    /// 1 MiB texture for bandwidth) in 4 bands, with and without the grain, draws what
    /// `visual.rs` describes within ±2 codes, everywhere (the target starts magenta, so a row no
    /// band drew fails); and the timestamp ring gives every frame back, in order, resolved if
    /// WARP gives timestamps (reported, not failed, if it doesn't).
    #[test]
    fn warp_draws_the_picture_within_two_codes() {
        for workload in Workload::ALL {
            for detail in [true, false] {
                let case = format!("{} detail {detail}", workload.name());
                let (device, context) = create_device(None, D3D_DRIVER_TYPE_WARP).unwrap();
                let shaders = Shaders::compile(workload, detail).unwrap();
                let texture = workload
                    .uses_texture()
                    .then(|| Texture::new(TEXTURE_MIB, DEFAULT_SEED).unwrap());
                let renderer =
                    Renderer::new(&device, &context, &shaders, texture, SIZE, BANDS).unwrap();
                let target = texture_2d(&device, SIZE, DXGI_FORMAT_B8G8R8A8_UNORM, false);
                let mut view = None;
                // SAFETY: a view of our own texture; an out-pointer to a local.
                unsafe { device.CreateRenderTargetView(&target, None, Some(&mut view)) }.unwrap();
                let views = [view];
                let mut ring = Ring::new(&device, &context).unwrap();
                for t_hns in TIMES {
                    if let Some(view) = &views[0] {
                        // SAFETY: our own view; magenta, which the picture never has.
                        unsafe { context.ClearRenderTargetView(view, &[1.0, 0.0, 1.0, 1.0]) };
                    }
                    let constants = FrameConstants::new(t_hns, COUNT, texture, SIZE);
                    renderer.set_constants(&constants).unwrap();
                    let (frame, evicted) = ring.begin();
                    assert_eq!(evicted, None);
                    renderer.draw(&views);
                    ring.end();
                    let pixels =
                        read_back(&device, &context, &target, SIZE, DXGI_FORMAT_B8G8R8A8_UNORM);
                    let picture = Picture::new(&constants, detail);
                    let mut worst = 0_i32;
                    let mut first_bad = None;
                    for y in 0..SIZE.1 {
                        for x in 0..SIZE.0 {
                            let at = 4 * (y * SIZE.0 + x) as usize;
                            let [b, g, r, a] =
                                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]];
                            let want = picture.codes(x, y);
                            let off = [r, g, b]
                                .iter()
                                .zip(want)
                                .map(|(&got, want)| (i32::from(got) - i32::from(want)).abs())
                                .max()
                                .unwrap_or(0);
                            worst = worst.max(off);
                            if (off > 2 || a != 255) && first_bad.is_none() {
                                first_bad = Some((x, y, [r, g, b, a], want));
                            }
                        }
                    }
                    assert_eq!(
                        first_bad, None,
                        "{case}, frame {frame} at {t_hns}: (x, y, drawn RGBA, wanted RGB)"
                    );
                    eprintln!("{case}, frame {frame}: within {worst} codes of visual.rs");
                }
                let mut out = [(u64::MAX, GpuTime::Unknown); RING];
                let n = ring.drain(Duration::from_secs(2), &mut out);
                let frames: Vec<u64> = out[..n].iter().map(|&(frame, _)| frame).collect();
                assert_eq!(frames, [0, 1, 2], "{case}: every frame, in order");
                let resolved = out[..n]
                    .iter()
                    .filter(|(_, time)| *time != GpuTime::Unknown)
                    .count();
                if resolved == 0 {
                    eprintln!("{case}: WARP gave no timestamps: reported, not failed");
                } else {
                    eprintln!("{case}: {:?}", &out[..n]);
                }
            }
        }
    }

    /// The ring numbers frames from 0, gives a set still pending when its slot comes round back
    /// as that frame's (always the oldest pending one), and reports the rest in frame order.
    #[test]
    fn the_ring_reports_frames_in_order_and_gives_up_the_oldest() {
        let (device, context) = create_device(None, D3D_DRIVER_TYPE_WARP).unwrap();
        let mut ring = Ring::new(&device, &context).unwrap();
        assert!(!ring.pending());
        let mut evicted = Vec::new();
        for frame in 0..RING as u64 + 2 {
            let (n, old) = ring.begin();
            assert_eq!(n, frame);
            evicted.extend(old);
            ring.end();
        }
        assert_eq!(evicted, [0, 1], "the two oldest, in order");
        assert!(ring.pending());
        let mut out = [(u64::MAX, GpuTime::Unknown); RING];
        let n = ring.drain(Duration::from_secs(2), &mut out);
        let frames: Vec<u64> = out[..n].iter().map(|&(frame, _)| frame).collect();
        assert_eq!(frames, (2..RING as u64 + 2).collect::<Vec<_>>());
        assert!(!ring.pending());
        // Nothing left: a poll and a drain give nothing back.
        assert_eq!(ring.poll(true, &mut out), 0);
        assert_eq!(ring.drain(Duration::from_millis(10), &mut out), 0);
        // The next frame takes the next number.
        assert_eq!(ring.begin(), (RING as u64 + 2, None));
        ring.end();
        assert_eq!(ring.abandon(&mut out), 1);
        assert_eq!(out[0], (RING as u64 + 2, GpuTime::Unknown));
    }

    /// The bandwidth workload's texture holds `mix32(x ^ mix32(y ^ seed))` at every texel, as
    /// `ps_fill` writes it in its bands.
    #[test]
    fn warp_fills_the_texture_from_the_seed() {
        let (device, context) = create_device(None, D3D_DRIVER_TYPE_WARP).unwrap();
        let shaders = Shaders::compile(Workload::Bw, true).unwrap();
        let texture = Texture::new(TEXTURE_MIB, 0x0123_4567).unwrap();
        let renderer =
            Renderer::new(&device, &context, &shaders, Some(texture), SIZE, BANDS).unwrap();
        let size = texture.size();
        let bytes = read_back(
            &device,
            &context,
            renderer.texture().unwrap(),
            size,
            DXGI_FORMAT_R32_UINT,
        );
        for y in 0..size.1 {
            for x in 0..size.0 {
                let at = 4 * (y * size.0 + x) as usize;
                let got =
                    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
                let want = mix32(x ^ mix32(y ^ texture.seed()));
                assert_eq!(got, want, "texel ({x}, {y})");
            }
        }
    }

    /// Research 17 §6, W tier (3): a hidden window with loadgen's flip-discard swap chain on WARP
    /// presents 30 frames through the latency wait, the bands and the query ring, with the
    /// tearing flag only if DXGI reports support for it (reported, not failed, if the runner has
    /// no usable desktop).
    #[test]
    fn a_hidden_window_presents_30_frames_on_warp() {
        let (device, context) = create_device(None, D3D_DRIVER_TYPE_WARP).unwrap();
        let factory = factory_of(&device);
        let tearing = tearing_supported(&factory);
        let window = match Window::create(
            "KairoLoadgen",
            "Kairo loadgen",
            Frame::Popup,
            0,
            0,
            SIZE.0 as i32,
            SIZE.1 as i32,
        ) {
            Ok(window) => window,
            Err(e) => return skip(&format!("no window ({e})")),
        };
        let chain = match Chain::new(&factory, &device, window.hwnd(), SIZE, 3, 2, tearing) {
            Ok(chain) => chain,
            Err(e) => return skip(&format!("no swap chain (tearing {tearing}: {e})")),
        };
        let shaders = Shaders::compile(Workload::Alu, true).unwrap();
        let renderer = Renderer::new(&device, &context, &shaders, None, SIZE, BANDS).unwrap();
        let mut ring = Ring::new(&device, &context).unwrap();
        let mut out = [(0, GpuTime::Unknown); RING];
        let (mut occluded, mut waited, mut timed) = (0, 0, 0);
        for frame in 0..30_i64 {
            window.pump();
            waited += u32::from(chain.wait(100));
            renderer
                .set_constants(&FrameConstants::new(frame * 80_000, COUNT, None, SIZE))
                .unwrap();
            let (n, _) = ring.begin();
            assert_eq!(n, frame as u64);
            renderer.draw(chain.target());
            ring.end();
            match chain.present() {
                Ok(Presented::Queued) => {}
                Ok(Presented::Occluded) => occluded += 1,
                Err(e) => panic!("frame {frame} (tearing {tearing}): {e}"),
            }
            timed += ring.poll(false, &mut out);
        }
        timed += ring.drain(Duration::from_secs(2), &mut out);
        assert!(!ring.pending());
        assert_eq!(removed_reason(&device), None);
        eprintln!(
            "30 frames presented on WARP: tearing {tearing}, {occluded} occluded, {waited} latency \
             waits signalled, {timed} frames given back by the ring, media hint {:?}",
            chain.media_hint()
        );
    }
}
