//! Dragging files out of CanExp into other programs (browser, messenger, Explorer…) via Windows OLE drag-and-drop.

use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{IDropSource, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
use windows::Win32::UI::Shell::{
    BHID_DataObject, ILCreateFromPathW, ILFree, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHFileOperationW, FOF_ALLOWUNDO,
    FO_COPY, FO_MOVE, SHFILEOPSTRUCTW,
};

/// Explorer's rule for a plain drop: same drive moves, another drive copies.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_uppercase());
    root(a).is_some() && root(a) == root(b)
}

/// Copies or moves `paths` into `dir` through the shell: progress window, name-conflict prompts, undo in Explorer.
pub fn receive(hwnd: HWND, paths: &[PathBuf], dir: &Path, moving: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    // both lists are double-NUL-terminated
    let from: Vec<u16> = paths.iter().flat_map(|p| p.as_os_str().encode_wide().chain([0])).chain([0]).collect();
    let to: Vec<u16> = dir.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: if moving { FO_MOVE } else { FO_COPY },
        pFrom: PCWSTR(from.as_ptr()),
        pTo: PCWSTR(to.as_ptr()),
        fFlags: FOF_ALLOWUNDO.0 as u16,
        ..Default::default()
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc == 0 || op.fAnyOperationsAborted.as_bool() { Ok(()) } else { Err(format!("{} {rc:#x}", crate::lang::t("error", "помилка", "ошибка"))) }
}

/// Mouse position in the window's client area, in physical pixels.
pub fn cursor_in_client(hwnd: HWND) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { (GetCursorPos(&mut p).is_ok() && ScreenToClient(hwnd, &mut p).as_bool()).then_some((p.x, p.y)) }
}

pub fn hwnd(frame: &eframe::Frame) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::Win32(h) = frame.window_handle().ok()?.as_raw() else { return None };
    Some(HWND(h.hwnd.get() as *mut _))
}

/// Runs a system drag of `paths` and returns when it is dropped or cancelled.
/// The data object comes from the shell itself, so targets receive exactly what a drag from Explorer gives them.
pub fn drag_out(hwnd: HWND, paths: &[PathBuf]) -> windows::core::Result<()> {
    unsafe {
        let pidls: Vec<_> = paths.iter().map(|p| ILCreateFromPathW(&HSTRING::from(p.as_path()))).filter(|p| !p.is_null()).collect();
        let result = (|| {
            let items = SHCreateShellItemArrayFromIDLists(&pidls.iter().map(|p| *p as *const _).collect::<Vec<_>>())?;
            let data: IDataObject = items.BindToHandler(None, &BHID_DataObject)?;
            // no drop source of our own: the shell's default one also draws the drag image
            SHDoDragDrop(hwnd, &data, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK).map(|_| ())
        })();
        for p in pidls {
            ILFree(Some(p));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn same_volume_by_drive_letter() {
        assert!(super::same_volume(Path::new(r"C:\a\b.txt"), Path::new(r"c:\x")));
        assert!(!super::same_volume(Path::new(r"C:\a"), Path::new(r"D:\a")));
        assert!(super::same_volume(Path::new(r"\\srv\share\a"), Path::new(r"\\srv\share\b")));
    }
}
