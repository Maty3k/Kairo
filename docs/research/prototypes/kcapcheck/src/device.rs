use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_NOT_FOUND,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
    IDXGIFactory6, IDXGIOutput,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::{Interface, Result};

/// One D3D11 device shared by capture, the convert/scale shader and libavcodec (AMF/QSV/NVENC).
pub struct GpuDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub mt: ID3D11Multithread,
    /// WinRT wrapper that WGC needs (`Direct3D11CaptureFramePool::CreateFreeThreaded`).
    pub winrt: IDirect3DDevice,
}

pub fn factory() -> Result<IDXGIFactory1> {
    unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
}

/// The adapter (and output) that scans out `monitor`. Required for DDA; best choice for WGC too.
pub fn adapter_for_monitor(
    factory: &IDXGIFactory1,
    monitor: HMONITOR,
) -> Result<Option<(IDXGIAdapter1, IDXGIOutput)>> {
    let mut a = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(a) } {
            Ok(x) => x,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut o = 0;
        loop {
            match unsafe { adapter.EnumOutputs(o) } {
                Ok(output) => {
                    let desc = unsafe { output.GetDesc()? };
                    if desc.Monitor.0 == monitor.0 {
                        return Ok(Some((adapter, output)));
                    }
                    o += 1;
                }
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e),
            }
        }
        a += 1;
    }
}

/// Fallback when no monitor is known yet: the high-performance GPU (not adapter 0, which can be
/// the Ryzen iGPU on AM5 desktops if it is enabled in the BIOS).
pub fn high_performance_adapter(factory: &IDXGIFactory1) -> Result<IDXGIAdapter1> {
    let f6: IDXGIFactory6 = factory.cast()?;
    unsafe { f6.EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) }
}

pub fn create_device(adapter: Option<&IDXGIAdapter1>) -> Result<GpuDevice> {
    let adapter: Option<IDXGIAdapter> = adapter.map(|a| a.cast()).transpose()?;
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut got = D3D_FEATURE_LEVEL::default();
    unsafe {
        D3D11CreateDevice(
            adapter.as_ref(),
            // An explicit adapter requires D3D_DRIVER_TYPE_UNKNOWN.
            if adapter.is_some() { D3D_DRIVER_TYPE_UNKNOWN } else { D3D_DRIVER_TYPE_HARDWARE },
            HMODULE::default(),
            // BGRA for WGC/DDA surfaces; VIDEO_SUPPORT because FFmpeg's d3d11va_device_init
            // QueryInterfaces ID3D11VideoDevice (its own device_create passes this flag).
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut got),
            Some(&mut context),
        )?;
    }
    let device = device.unwrap();
    let context = context.unwrap();

    // Multithread protection: WGC FrameArrived (worker thread), our GPU thread and libavcodec all
    // touch the immediate context. MS docs: query ID3D11Multithread from the immediate context.
    let mt: ID3D11Multithread = context.cast()?;
    let _previous = unsafe { mt.SetMultithreadProtected(true) };

    let dxgi: IDXGIDevice = device.cast()?;
    let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
    Ok(GpuDevice { device, context, mt, winrt })
}
