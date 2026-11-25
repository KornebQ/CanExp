//! Folder sizes, summed on a background thread (a big folder can take seconds).

use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

pub struct FolderSizes {
    tx: mpsc::Sender<PathBuf>,
    pub rx: mpsc::Receiver<(PathBuf, u64)>,
}

impl FolderSizes {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, req_rx) = mpsc::channel::<PathBuf>();
        let (res_tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for path in req_rx {
                let size = dir_size(&path);
                if res_tx.send((path, size)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        FolderSizes { tx, rx }
    }

    pub fn request(&self, path: PathBuf) {
        let _ = self.tx.send(path);
    }
}

/// Total size of all files under `dir`. Unreadable entries are skipped; links and junctions are not
/// followed, so a junction loop (e.g. "Application Data") can't recurse forever.
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.path().symlink_metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) if m.is_file() => m.len(),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sums_nested_files() {
        let root = std::env::temp_dir().join(format!("canexp_size_test_{}", std::process::id()));
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("x.bin"), [0u8; 100]).unwrap();
        std::fs::write(root.join("a/b/y.bin"), [0u8; 23]).unwrap();
        assert_eq!(super::dir_size(&root), 123);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
