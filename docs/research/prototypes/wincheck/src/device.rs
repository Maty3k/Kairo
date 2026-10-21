use windows::{
    core::{Interface, Result},
    Graphics::DirectX::Direct3D11::IDirect3DDevice,
    Win32::Foundation::{HMODULE, LUID},
    Win32::Graphics::{Direct3D::*, Direct3D11::*, Dxgi::*, Gdi::HMONITOR},
    Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice,
};

pub struct GpuDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub winrt: IDirect3DDevice, // !Send: build GpuDevice on the GPU thread
    pub luid: LUID,
}

/// The adapter that scans out `mon` (DDA requires it; WGC/AMF want it; never assume adapter 0).
pub fn adapter_for_monitor(f: &IDXGIFactory1, mon: HMONITOR) -> Result<Option<IDXGIAdapter1>> {
    for a in 0.. {
        let Ok(adapter) = (unsafe { f.EnumAdapters1(a) }) else { return Ok(None) }; // DXGI_ERROR_NOT_FOUND ends
        for o in 0.. {
            let Ok(out) = (unsafe { adapter.EnumOutputs(o) }) else { break };
            if unsafe { out.GetDesc()? }.Monitor == mon { return Ok(Some(adapter)); }
        }
    }
    Ok(None)
}

pub fn create_device(adapter: &IDXGIAdapter1) -> Result<GpuDevice> {
    let a: IDXGIAdapter = adapter.cast()?;
    let (mut dev, mut ctx) = (None, None);
    unsafe {
        D3D11CreateDevice(&a, D3D_DRIVER_TYPE_UNKNOWN /* explicit adapter */, HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, // FFmpeg QIs ID3D11VideoDevice
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION,
            Some(&mut dev), None, Some(&mut ctx))?;
    }
    let (device, context) = (dev.unwrap(), ctx.unwrap());
    let mt: ID3D11Multithread = context.cast()?;            // MS: query it from the immediate context
    let _was = unsafe { mt.SetMultithreadProtected(true) }; // FFmpeg does NOT do this for user devices
    let dxgi: IDXGIDevice = device.cast()?;
    let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
    let luid = unsafe { adapter.GetDesc1()? }.AdapterLuid;
    Ok(GpuDevice { device, context, winrt, luid })
}
