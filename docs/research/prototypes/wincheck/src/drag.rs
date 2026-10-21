//! Windows drag-out without tauri-plugin-drag (which pulls windows 0.52 and has open crash issues).
use windows::{core::HSTRING, Win32::{Foundation::HWND, System::{Com::{IBindCtx, IDataObject}, Ole::{IDropSource, DROPEFFECT, DROPEFFECT_COPY}},
    UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop}}};
/// Modal: run inside app.run_on_main_thread. NULL drop source => the Shell provides one (and a drag image).
pub unsafe fn drag_file(hwnd: HWND, path: &std::path::Path) -> windows::core::Result<DROPEFFECT> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None::<&IBindCtx>)?;
        let data: IDataObject = item.BindToHandler(None::<&IBindCtx>, &BHID_DataObject)?;
        SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY) // DROPEFFECT_NONE = cancelled
    }
}
