//! [U] Fallback if the NV12 render-target self-test fails: D3D11 video processor BGRA -> NV12,
//! written straight into the FFmpeg pool texture. Never feed BGRA to AMF (DCC corruption, AMF #574).
use core::mem::ManuallyDrop;
use windows::{core::{Interface, Result}, Win32::Graphics::{Direct3D11::*, Dxgi::Common::*}};

pub struct VpConverter { vdev: ID3D11VideoDevice, vctx: ID3D11VideoContext1, en: ID3D11VideoProcessorEnumerator, vp: ID3D11VideoProcessor }

impl VpConverter {
    pub fn new(dev: &ID3D11Device, ctx: &ID3D11DeviceContext, (iw, ih): (u32, u32), (ow, oh): (u32, u32), fps: u32) -> Result<Self> {
        let vdev: ID3D11VideoDevice = dev.cast()?;
        let vctx: ID3D11VideoContext1 = ctx.cast()?;
        let rate = DXGI_RATIONAL { Numerator: fps, Denominator: 1 };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, InputFrameRate: rate, InputWidth: iw, InputHeight: ih,
            OutputFrameRate: rate, OutputWidth: ow, OutputHeight: oh, Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED };
        let en = unsafe { vdev.CreateVideoProcessorEnumerator(&desc)? };
        let vp = unsafe { vdev.CreateVideoProcessor(&en, 0)? };
        unsafe {
            vctx.VideoProcessorSetStreamColorSpace1(&vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
            vctx.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709); // BT.709 limited
        }
        Ok(Self { vdev, vctx, en, vp })
    }
    /// dst = FFmpeg pool texture (NV12, BIND_RENDER_TARGET, dynamic pool => slice 0).
    pub fn blt(&self, src_bgra: &ID3D11Texture2D, dst_nv12: &ID3D11Texture2D) -> Result<()> {
        let ivd = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC { FourCC: 0, ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } } };
        let ovd = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC { ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } } };
        let (mut iv, mut ov) = (None, None);
        unsafe {
            self.vdev.CreateVideoProcessorInputView(src_bgra, &self.en, &ivd, Some(&mut iv))?;
            self.vdev.CreateVideoProcessorOutputView(dst_nv12, &self.en, &ovd, Some(&mut ov))?;
            let stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: ManuallyDrop::new(iv), ..Default::default() };
            let r = self.vctx.VideoProcessorBlt(&self.vp, ov.as_ref().unwrap(), 0, std::slice::from_ref(&stream));
            drop(ManuallyDrop::into_inner(stream.pInputSurface)); // release the input view
            r
        }
    }
}
