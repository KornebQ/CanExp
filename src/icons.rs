//! Explorer's own thumbnails (photo/video/PDF previews, or the file's icon when there is none), as egui images.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};

/// Requested thumbnail size in pixels (shown at 64 logical px, so this stays sharp at 200% scaling).
const PX: i32 = 128;

/// Loads thumbnails on a background thread: video thumbnails can take a while and must not freeze the UI.
pub struct Loader {
    tx: mpsc::Sender<(u64, PathBuf)>,
    pub rx: mpsc::Receiver<(PathBuf, Option<egui::ColorImage>)>,
    generation: Arc<AtomicU64>,
}

impl Loader {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, req_rx) = mpsc::channel::<(u64, PathBuf)>();
        let (res_tx, rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            for (gen, path) in req_rx {
                if gen != current.load(Ordering::Relaxed) {
                    continue; // user already left that folder
                }
                let img = thumbnail(&path);
                if res_tx.send((path, img)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Loader { tx, rx, generation }
    }

    pub fn request(&self, path: PathBuf) {
        let _ = self.tx.send((self.generation.load(Ordering::Relaxed), path));
    }

    /// Drop queued requests for the previous folder.
    pub fn new_folder(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

fn thumbnail(path: &Path) -> Option<egui::ColorImage> {
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hbm = factory.GetImage(SIZE { cx: PX, cy: PX }, SIIGBF_RESIZETOFIT).ok()?;
        let img = to_rgba(hbm);
        let _ = DeleteObject(hbm);
        img
    }
}

unsafe fn to_rgba(hbm: HBITMAP) -> Option<egui::ColorImage> {
    let mut bm = BITMAP::default();
    if GetObjectW(hbm, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    let dc = CreateCompatibleDC(None);
    let rows = GetDIBits(dc, hbm, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
    let _ = DeleteDC(dc);
    if rows == 0 {
        return None;
    }
    // Photos/videos come without alpha (all zero) -> opaque; icons come with premultiplied alpha.
    let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2); // BGRA -> RGBA
        if !has_alpha {
            p[3] = 255;
        }
    }
    Some(egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], &px))
}
