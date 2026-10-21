//! windows-rs maps S_FALSE ("not ready") to Ok(()) in GetData: call through the vtable and compare to S_OK.
use windows::{core::Interface, Win32::{Foundation::S_OK, Graphics::Direct3D11::*}};
pub unsafe fn ready<T: Default>(ctx: &ID3D11DeviceContext, q: &ID3D11Query) -> Option<T> {
    let mut v = T::default();
    let hr = unsafe { (Interface::vtable(ctx).GetData)(Interface::as_raw(ctx), q.as_raw(),
        &mut v as *mut T as *mut _, size_of::<T>() as u32, D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32) };
    (hr == S_OK).then_some(v)
}
pub fn queries(dev: &ID3D11Device) -> windows::core::Result<[ID3D11Query; 3]> {
    let mk = |q: D3D11_QUERY| -> windows::core::Result<ID3D11Query> {
        let mut out = None;
        unsafe { dev.CreateQuery(&D3D11_QUERY_DESC { Query: q, MiscFlags: 0 }, Some(&mut out))? };
        Ok(out.unwrap())
    };
    Ok([mk(D3D11_QUERY_TIMESTAMP_DISJOINT)?, mk(D3D11_QUERY_TIMESTAMP)?, mk(D3D11_QUERY_TIMESTAMP)?])
}
pub fn _types(ctx: &ID3D11DeviceContext, q: &ID3D11Query) {
    let _d: Option<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT> = unsafe { ready(ctx, q) };
    let _t: Option<u64> = unsafe { ready(ctx, q) };
}
