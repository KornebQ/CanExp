#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, ViewportCommand};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod icons;
mod sizes;
mod style;

/// Layout file inside each folder; double-clicking it in Explorer opens that folder here.
const LAYOUT_FILE: &str = "folder.cexp";
const CELL_W: f32 = 100.0;
const CELL_H: f32 = 116.0;
const ICON: f32 = 64.0;
const TITLE_H: f32 = 46.0;
const SIDEBAR_W: f32 = 190.0;

#[derive(Serialize, Deserialize, Default)]
struct Layout {
    items: HashMap<String, [f32; 2]>,
}

struct Item {
    name: String,
    is_dir: bool,
    size: u64,
    pos: Option<Pos2>,
}

/// App-wide settings in %APPDATA%\CanExp\settings.json (per-folder layouts stay in folder.cexp).
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    favorites: Vec<PathBuf>,
    sidebar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { favorites: vec![], sidebar: true }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("CanExp").join("settings.json"))
    }

    fn load() -> Self {
        Self::path().and_then(|p| fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self) -> std::io::Result<()> {
        let p = Self::path().ok_or(std::io::ErrorKind::NotFound)?;
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(p, serde_json::to_string_pretty(self)?)
    }
}

fn human_size(bytes: u64) -> String {
    let units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let (mut v, mut u) = (bytes as f64, 0);
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} Б") } else { format!("{v:.1} {}", units[u]).replace('.', ",") }
}

struct App {
    dir: PathBuf,
    path_input: String,
    items: Vec<Item>,
    history: Vec<PathBuf>,
    error: Option<String>,
    /// Thumbnail per file path; present-but-None = still loading or Windows has none.
    thumbs: HashMap<PathBuf, Option<egui::TextureHandle>>,
    loader: icons::Loader,
    selected: HashSet<String>,
    /// Last plainly or Ctrl-clicked item: the fixed end of a Shift+click range.
    anchor: Option<String>,
    /// Rubber-band selection: start point (relative to the canvas) and the selection kept under it (Ctrl).
    band: Option<(Vec2, HashSet<String>)>,
    /// Group drag in progress: pointer press point and each dragged item's start position.
    drag: Option<(Pos2, HashMap<String, Pos2>)>,
    settings: Settings,
    folder_sizes: sizes::FolderSizes,
    /// Folder size by path; present-but-None = still counting.
    dir_sizes: HashMap<PathBuf, Option<u64>>,
}

impl App {
    fn new(dir: PathBuf, ctx: egui::Context) -> Self {
        let mut app = App {
            dir: PathBuf::new(),
            path_input: String::new(),
            items: vec![],
            history: vec![],
            error: None,
            thumbs: HashMap::new(),
            loader: icons::Loader::new(ctx.clone()),
            folder_sizes: sizes::FolderSizes::new(ctx),
            dir_sizes: HashMap::new(),
            selected: HashSet::new(),
            anchor: None,
            band: None,
            drag: None,
            settings: Settings::load(),
        };
        app.load(dir);
        app
    }

    fn load(&mut self, dir: PathBuf) {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                self.error = Some(format!("{}: {e}", dir.display()));
                self.path_input = self.dir.display().to_string();
                return;
            }
        };
        let layout: Layout = fs::read_to_string(dir.join(LAYOUT_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut items: Vec<Item> = entries
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().eq_ignore_ascii_case(LAYOUT_FILE))
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let pos = layout.items.get(&name).map(|p| Pos2::new(p[0], p[1]));
                let is_dir = e.path().is_dir();
                let size = if is_dir { 0 } else { e.metadata().map_or(0, |m| m.len()) };
                Item { is_dir, size, name, pos }
            })
            .collect();
        items.sort_by_key(|i| (!i.is_dir, i.name.to_lowercase()));
        self.items = items;
        self.thumbs.clear();
        self.loader.new_folder();
        self.dir_sizes.clear(); // re-count on every visit so sizes don't go stale
        self.selected.clear();
        self.anchor = None;
        self.band = None;
        self.path_input = dir.display().to_string();
        self.dir = dir;
        self.error = None;
    }

    fn navigate(&mut self, dir: PathBuf) {
        let prev = self.dir.clone();
        self.load(dir);
        if self.error.is_none() && prev != self.dir {
            self.history.push(prev);
        }
    }

    fn save(&mut self) {
        let layout = Layout {
            items: self.items.iter().filter_map(|i| i.pos.map(|p| (i.name.clone(), [p.x, p.y]))).collect(),
        };
        let json = serde_json::to_string_pretty(&layout).expect("layout serializes");
        if let Err(e) = fs::write(self.dir.join(LAYOUT_FILE), json) {
            self.error = Some(format!("Не удалось сохранить раскладку: {e}"));
        }
    }

    /// Give items without a saved position the first free grid cell, like the desktop does.
    fn place_new(&mut self, width: f32) {
        let cols = ((width / CELL_W).floor() as i32).max(1);
        let mut occupied: HashSet<(i32, i32)> = self
            .items
            .iter()
            .filter_map(|i| i.pos)
            .map(|p| ((p.x / CELL_W).round() as i32, (p.y / CELL_H).round() as i32))
            .collect();
        for item in self.items.iter_mut().filter(|i| i.pos.is_none()) {
            let (c, r) = free_cell(&occupied, cols);
            occupied.insert((c, r));
            item.pos = Some(Pos2::new(c as f32 * CELL_W, r as f32 * CELL_H));
        }
    }
}

fn free_cell(occupied: &HashSet<(i32, i32)>, cols: i32) -> (i32, i32) {
    (0..).map(|n| (n % cols, n / cols)).find(|c| !occupied.contains(c)).unwrap()
}

/// File name wrapped to the cell width, cut with "…" after `rows` lines, centered like Explorer.
fn name_galley(ui: &egui::Ui, name: &str, rows: usize, color: Color32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(name.to_owned(), FontId::proportional(12.5), color, CELL_W - 10.0);
    job.wrap.max_rows = rows;
    job.wrap.overflow_character = Some('…');
    job.halign = egui::Align::Center;
    ui.fonts(|f| f.layout_job(job))
}

/// Flat Finder-style chevron button; `dir` is where the chevron points.
fn chevron(ui: &mut egui::Ui, dir: Vec2, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), if enabled { Sense::click() } else { Sense::hover() });
    let v = ui.visuals();
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, v.widgets.hovered.weak_bg_fill);
    }
    let color = if enabled { v.text_color() } else { v.weak_text_color().gamma_multiply(0.5) };
    let (c, side) = (rect.center(), Vec2::new(-dir.y, dir.x));
    let pts = vec![c - dir * 3.0 + side * 6.0, c + dir * 3.0, c - dir * 3.0 - side * 6.0];
    ui.painter().line(pts, Stroke::new(1.8_f32, color));
    enabled && resp.clicked()
}

/// macOS-style "show/hide sidebar" button: a window outline with a sidebar line.
fn sidebar_toggle(ui: &mut egui::Ui) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let v = ui.visuals();
    if resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, v.widgets.hovered.weak_bg_fill);
    }
    let stroke = Stroke::new(1.4_f32, v.text_color());
    let frame = Rect::from_center_size(rect.center(), Vec2::new(17.0, 13.0));
    ui.painter().rect_stroke(frame, 3.0, stroke, StrokeKind::Middle);
    let x = frame.left() + 6.0;
    ui.painter().line_segment([Pos2::new(x, frame.top()), Pos2::new(x, frame.bottom())], stroke);
    resp.clicked()
}

/// Five-pointed star; filled when `fill` is given.
fn paint_star(p: &egui::Painter, c: Pos2, r: f32, fill: Option<Color32>, stroke: Stroke) {
    let pts: Vec<Pos2> = (0..10)
        .map(|k| {
            let a = (-90.0 + k as f32 * 36.0).to_radians();
            let rad = if k % 2 == 0 { r } else { r * 0.45 };
            c + Vec2::new(a.cos(), a.sin()) * rad
        })
        .collect();
    if let Some(fill) = fill {
        // a star isn't convex, but each center-inner-tip-inner kite is
        for k in (0..10).step_by(2) {
            let kite = vec![c, pts[(k + 9) % 10], pts[k], pts[k + 1]];
            p.add(egui::Shape::convex_polygon(kite, fill, Stroke::NONE));
        }
    }
    p.add(egui::Shape::closed_line(pts, stroke));
}

/// Outline folder glyph for the sidebar, like Finder's.
fn paint_folder(p: &egui::Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3_f32, color);
    let body = Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(15.0, 11.0));
    p.rect_stroke(body, 2.0, stroke, StrokeKind::Middle);
    let tl = body.left_top();
    p.line(vec![tl + Vec2::new(1.0, 0.0), tl + Vec2::new(2.0, -2.5), tl + Vec2::new(6.0, -2.5), tl + Vec2::new(7.5, 0.0)], stroke);
}

const CAPTION_W: f32 = 46.0;

/// Windows 11 minimize / maximize-restore / close buttons at the right end of the title bar.
fn caption_buttons(ui: &mut egui::Ui, bar: Rect) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let text = ui.visuals().text_color();
    for i in 0..3 {
        let rect = Rect::from_min_size(Pos2::new(bar.right() - CAPTION_W * (3 - i) as f32, bar.top()), Vec2::new(CAPTION_W, bar.height()));
        let resp = ui.interact(rect, ui.id().with(("caption", i)), Sense::click());
        let close = i == 2;
        if resp.hovered() {
            let fill = if close { Color32::from_rgb(196, 43, 28) } else { ui.visuals().widgets.hovered.weak_bg_fill };
            ui.painter().rect_filled(rect, 0.0, fill);
        }
        let stroke = Stroke::new(1.0_f32, if close && resp.hovered() { Color32::WHITE } else { text });
        let (p, c) = (ui.painter(), rect.center());
        match i {
            0 => {
                p.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], stroke);
            }
            1 if maximized => {
                let front = Rect::from_center_size(c + Vec2::new(-1.5, 1.5), Vec2::splat(8.0));
                p.rect_stroke(front, 1.0, stroke, StrokeKind::Middle);
                p.line(vec![front.left_top() + Vec2::new(2.0, -2.0), front.right_top() + Vec2::new(2.0, -2.0), front.right_bottom() + Vec2::new(2.0, -2.0)], stroke);
            }
            1 => {
                p.rect_stroke(Rect::from_center_size(c, Vec2::splat(10.0)), 1.0, stroke, StrokeKind::Middle);
            }
            _ => {
                p.line_segment([c - Vec2::splat(5.0), c + Vec2::splat(5.0)], stroke);
                p.line_segment([c + Vec2::new(-5.0, 5.0), c + Vec2::new(5.0, -5.0)], stroke);
            }
        }
        if resp.clicked() {
            ctx.send_viewport_cmd(match i {
                0 => ViewportCommand::Minimized(true),
                1 => ViewportCommand::Maximized(!maximized),
                _ => ViewportCommand::Close,
            });
        }
    }
}

/// Resize from the window edges: the native frame is off, so the edges are ours to handle.
/// Returns true while the pointer is on an edge, so the title bar doesn't start a move instead.
fn edge_resize(ctx: &egui::Context) -> bool {
    use egui::viewport::ResizeDirection as D;
    use egui::CursorIcon as C;
    const M: f32 = 6.0;
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return false;
    }
    let Some(p) = ctx.pointer_hover_pos() else { return false };
    let r = ctx.screen_rect();
    let (l, rt, t, b) = (p.x < r.left() + M, p.x > r.right() - M, p.y < r.top() + M, p.y > r.bottom() - M);
    let (dir, icon) = match (l, rt, t, b) {
        (true, _, true, _) => (D::NorthWest, C::ResizeNorthWest),
        (_, true, true, _) => (D::NorthEast, C::ResizeNorthEast),
        (true, _, _, true) => (D::SouthWest, C::ResizeSouthWest),
        (_, true, _, true) => (D::SouthEast, C::ResizeSouthEast),
        (true, ..) => (D::West, C::ResizeWest),
        (_, true, ..) => (D::East, C::ResizeEast),
        (_, _, true, _) => (D::North, C::ResizeNorth),
        (.., true) => (D::South, C::ResizeSouth),
        _ => return false,
    };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
    true
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // let the acrylic backdrop show through
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok((path, size)) = self.folder_sizes.rx.try_recv() {
            self.dir_sizes.insert(path, Some(size));
        }
        while let Ok((path, img)) = self.loader.rx.try_recv() {
            if path.parent() == Some(self.dir.as_path()) {
                let tex = img.map(|img| ctx.load_texture(path.display().to_string(), img, Default::default()));
                self.thumbs.insert(path, tex);
            }
        }

        let mut go: Option<PathBuf> = None;
        let dark = ctx.style().visuals.dark_mode;
        let tint = |a: u8| if dark { Color32::from_black_alpha(a) } else { Color32::from_white_alpha(a) };
        let on_edge = edge_resize(ctx);
        // Windows swaps the acrylic for a flat, lighter fill when the window loses focus; darken it instead.
        // Painted first on the background layer, so it sits under the panels and content.
        if ctx.input(|i| i.viewport().focused) == Some(false) {
            let shade = Color32::from_black_alpha(if dark { 120 } else { 40 });
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, shade);
        }

        egui::TopBottomPanel::top("title")
            .exact_height(TITLE_H)
            .frame(egui::Frame::NONE.fill(tint(40)).inner_margin(egui::Margin::symmetric(14, 0)))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let bar = ui.max_rect().expand2(Vec2::new(14.0, 0.0));
                let drag = ui.interact(bar, ui.id().with("titlebar"), Sense::click_and_drag());
                if drag.drag_started() && !on_edge {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }

                caption_buttons(ui, bar);
                // centered in the window, but never under the caption buttons
                let max_half = bar.right() - CAPTION_W * 3.0 - 12.0 - bar.center().x;
                let field_w = (bar.width() * 0.42).clamp(180.0, 560.0).min(max_half * 2.0);
                let field = Rect::from_center_size(bar.center(), Vec2::new(field_w, 28.0));

                ui.horizontal_centered(|ui| {
                    if sidebar_toggle(ui) {
                        self.settings.sidebar = !self.settings.sidebar;
                        let _ = self.settings.save();
                        // Widen/narrow the window to the right by the sidebar width; the top-left corner stays put.
                        let (maximized, inner) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().inner_rect));
                        if let (false, Some(inner)) = (maximized, inner) {
                            let d = if self.settings.sidebar { SIDEBAR_W } else { -SIDEBAR_W };
                            ctx.send_viewport_cmd(ViewportCommand::InnerSize(inner.size() + Vec2::new(d, 0.0)));
                        }
                    }
                    ui.add_space(4.0);
                    if chevron(ui, Vec2::new(-1.0, 0.0), !self.history.is_empty()) {
                        let prev = self.history.pop().unwrap();
                        self.load(prev);
                    }
                    if chevron(ui, Vec2::new(0.0, -1.0), self.dir.parent().is_some()) {
                        go = self.dir.parent().map(Path::to_path_buf);
                    }
                    ui.add_space(6.0);
                    let title = self.dir.file_name().map_or(self.dir.display().to_string(), |n| n.to_string_lossy().into_owned());
                    let room = (field.left() - ui.cursor().left() - 12.0).max(0.0);
                    ui.allocate_ui(Vec2::new(room, 24.0), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(title).family(style::bold()).size(15.0)).truncate().selectable(false));
                    });
                });

                // Segoe UI sits low in its line box, so the top margin is smaller to center the text visually.
                // The field's frame is drawn here so the star sits inside it without being part of the TextEdit.
                let text_margin = egui::Margin { left: 10, right: 4, top: 4, bottom: 6 };
                ui.painter().rect_filled(field, 7.0, ui.visuals().extreme_bg_color);
                let text_rect = Rect::from_min_max(field.min, field.max - Vec2::new(30.0, 0.0));
                let r = ui.put(
                    text_rect,
                    egui::TextEdit::singleline(&mut self.path_input).margin(text_margin).vertical_align(egui::Align::Center).frame(false),
                );
                if r.has_focus() {
                    ui.painter().rect_stroke(field, 7.0, Stroke::new(1.0_f32, style::ACCENT), StrokeKind::Inside);
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = Some(PathBuf::from(self.path_input.trim()));
                }
                let star = Rect::from_center_size(Pos2::new(field.right() - 15.0, field.center().y), Vec2::splat(22.0));
                let star_resp = ui.interact(star, ui.id().with("star"), Sense::click()).on_hover_text("Избранное");
                let fav = self.settings.favorites.contains(&self.dir);
                let color = if fav { Color32::from_rgb(255, 196, 0) } else { ui.visuals().weak_text_color() };
                let color = if star_resp.hovered() && !fav { ui.visuals().text_color() } else { color };
                paint_star(ui.painter(), star.center(), 7.0, fav.then_some(color), Stroke::new(1.2_f32, color));
                if star_resp.clicked() {
                    if fav {
                        self.settings.favorites.retain(|f| *f != self.dir);
                    } else {
                        self.settings.favorites.push(self.dir.clone());
                    }
                    if let Err(e) = self.settings.save() {
                        self.error = Some(format!("Не удалось сохранить избранное: {e}"));
                    }
                }
            });

        let text = if self.selected.is_empty() {
            format!("Объектов: {}", self.items.len())
        } else {
            let (mut total, mut counting, mut count) = (0u64, false, 0);
            for item in self.items.iter().filter(|i| self.selected.contains(&i.name)) {
                count += 1;
                if !item.is_dir {
                    total += item.size;
                    continue;
                }
                let path = self.dir.join(&item.name);
                match self.dir_sizes.get(&path) {
                    Some(Some(size)) => total += size,
                    Some(None) => counting = true,
                    None => {
                        self.dir_sizes.insert(path.clone(), None);
                        self.folder_sizes.request(path);
                        counting = true;
                    }
                }
            }
            let size = human_size(total) + if counting { " (считается…)" } else { "" };
            format!("Выбрано: {} из {}\u{2003}·\u{2003}{}", count, self.items.len(), size)
        };
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::NONE.fill(tint(40)).inner_margin(egui::Margin::symmetric(12, 4)))
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| match &self.error {
                    Some(e) => ui.colored_label(Color32::from_rgb(255, 105, 97), e),
                    None => ui.label(egui::RichText::new(text).size(11.0).weak()),
                });
            });

        if self.settings.sidebar {
            egui::SidePanel::left("sidebar")
                .exact_width(SIDEBAR_W)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::NONE.fill(tint(22)).inner_margin(egui::Margin { left: 8, right: 8, top: 10, bottom: 8 }))
                .show(ctx, |ui| {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Избранное").size(11.0).family(style::bold()).weak());
                    });
                    if self.settings.favorites.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Нажмите ☆ в адресной строке, чтобы добавить папку").size(11.0).weak());
                        });
                    }
                    let mut remove = None;
                    for (i, fav) in self.settings.favorites.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
                        let v = ui.visuals();
                        if *fav == self.dir {
                            ui.painter().rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(10, 132, 255, 70));
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, v.widgets.hovered.weak_bg_fill);
                        }
                        paint_folder(ui.painter(), rect.left_center() + Vec2::new(16.0, 0.0), Color32::from_rgb(255, 196, 61));
                        let name = fav.file_name().map_or(fav.display().to_string(), |n| n.to_string_lossy().into_owned());
                        let mut job = egui::text::LayoutJob::simple_singleline(name, FontId::proportional(13.0), v.text_color());
                        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 40.0);
                        let galley = ui.fonts(|f| f.layout_job(job));
                        let at = Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0);
                        ui.painter().galley(at, galley, v.text_color());
                        if resp.clicked() {
                            go = Some(fav.clone());
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Убрать из избранного").clicked() {
                                remove = Some(i);
                                ui.close_menu();
                            }
                        });
                        resp.on_hover_text(fav.display().to_string());
                    }
                    if let Some(i) = remove {
                        self.settings.favorites.remove(i);
                        let _ = self.settings.save();
                    }
                });
        }

        // No panel margin: the scroll bars sit at the window edge (style.rs keeps a gap the width of a collapsed bar);
        // the canvas carries its own padding instead.
        const PAD: f32 = 12.0;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            self.place_new(ui.available_width() - PAD * 2.0);
            let mut dirty = false;
            let mut open: Option<usize> = None;
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            let mut shift_click: Option<usize> = None;
            egui::ScrollArea::both().drag_to_scroll(false).show(ui, |ui| {
                let extent = self.items.iter().filter_map(|i| i.pos).fold(Vec2::ZERO, |m, p| m.max(p.to_vec2() + Vec2::new(CELL_W, CELL_H)));
                let (area, bg) = ui.allocate_exact_size((extent + Vec2::splat(PAD * 2.0)).max(ui.available_size()), Sense::click_and_drag());
                let canvas = area.shrink(PAD);
                if bg.clicked() && !ctrl && !shift {
                    self.selected.clear();
                }
                if bg.drag_started() {
                    if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                        let keep = if ctrl || shift { self.selected.clone() } else { HashSet::new() };
                        self.band = Some((p - canvas.min, keep));
                    }
                }

                // Interaction first, painting after.
                // Move every dragged item to its start position plus the pointer's offset from the press point:
                // the group keeps its shape, and summing per-frame deltas can't drift or nudge items on a click.
                if let (Some((origin, starts)), Some(p)) = (&self.drag, ui.input(|i| i.pointer.interact_pos())) {
                    let mut delta = p - *origin;
                    // stop the whole group at the canvas edge instead of squashing the items that hit it
                    let min = starts.values().fold(Pos2::new(f32::MAX, f32::MAX), |m, s| m.min(*s));
                    delta = delta.max(-min.to_vec2());
                    for item in self.items.iter_mut() {
                        if let Some(start) = starts.get(&item.name) {
                            item.pos = Some(*start + delta);
                        }
                    }
                }

                let mut cells = Vec::with_capacity(self.items.len());
                let (mut drag_started, mut drag_stopped) = (false, false);
                for (idx, item) in self.items.iter_mut().enumerate() {
                    let pos = item.pos.unwrap();
                    let rect = Rect::from_min_size(canvas.min + pos.to_vec2(), Vec2::new(CELL_W, CELL_H));
                    let resp = ui.interact(rect, ui.id().with(&item.name), Sense::click_and_drag());
                    drag_started |= resp.drag_started();
                    drag_stopped |= resp.drag_stopped();
                    // Click selects, Ctrl+click toggles, Shift+click selects a range (handled after the loop);
                    // dragging an unselected item selects just it.
                    if resp.clicked() && shift {
                        shift_click = Some(idx);
                    } else if resp.clicked() && ctrl {
                        if !self.selected.remove(&item.name) {
                            self.selected.insert(item.name.clone());
                        }
                        self.anchor = Some(item.name.clone());
                    } else if resp.clicked() || (resp.drag_started() && !self.selected.contains(&item.name)) {
                        self.selected = HashSet::from([item.name.clone()]);
                        self.anchor = Some(item.name.clone());
                    }
                    if resp.double_clicked() {
                        open = Some(idx);
                    }
                    cells.push((idx, rect, resp.hovered() || resp.dragged()));
                }
                if drag_started {
                    // the selection already includes the grabbed item (see above), so the group is the selection
                    let starts = self.items.iter().filter(|i| self.selected.contains(&i.name)).map(|i| (i.name.clone(), i.pos.unwrap())).collect();
                    self.drag = ui.input(|i| i.pointer.press_origin()).map(|o| (o, starts));
                }
                if self.drag.is_some() && (drag_stopped || !ui.input(|i| i.pointer.primary_down())) {
                    self.drag = None;
                    dirty = true;
                }

                if let Some(idx) = shift_click {
                    // Range in folder order (name order, like the sorted list), from the anchor to the clicked item.
                    let a = self.anchor.as_ref().and_then(|n| self.items.iter().position(|i| &i.name == n)).unwrap_or(idx);
                    let range = self.items[a.min(idx)..=a.max(idx)].iter().map(|i| i.name.clone());
                    if ctrl {
                        self.selected.extend(range);
                    } else {
                        self.selected = range.collect();
                    }
                }

                let mut band_rect = None;
                if let Some((start, keep)) = &self.band {
                    if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                        let r = Rect::from_two_pos(canvas.min + *start, p);
                        let hit = cells.iter().filter(|(_, cell, _)| cell.intersects(r)).map(|&(i, ..)| self.items[i].name.clone());
                        self.selected = keep.iter().cloned().chain(hit).collect();
                        band_rect = Some(r);
                    }
                    if bg.drag_stopped() || !bg.dragged() {
                        self.band = None;
                    }
                }

                // shortcut: thumbnails load in folder order, not visible-first; fine for hundreds of files, slow for many thousands.
                for &(idx, rect, hovered) in &cells {
                    let item = &self.items[idx];
                    let path = self.dir.join(&item.name);
                    if !self.thumbs.contains_key(&path) {
                        self.thumbs.insert(path.clone(), None);
                        self.loader.request(path.clone());
                    }
                    let sel = self.selected.contains(&item.name);
                    let p = ui.painter();
                    let icon_box = Rect::from_min_size(rect.min + Vec2::new((CELL_W - ICON) / 2.0, 8.0), Vec2::splat(ICON));
                    let text_color = ui.visuals().text_color();
                    let galley = name_galley(ui, &item.name, 2, text_color);
                    let text_top = icon_box.bottom() + 6.0;
                    let hl = Rect::from_min_max(rect.min + Vec2::new(2.0, 0.0), rect.max - Vec2::new(2.0, 2.0));
                    if sel {
                        p.rect_filled(hl, 8.0, Color32::from_rgba_unmultiplied(10, 132, 255, 70));
                        p.rect_stroke(hl, 8.0, Stroke::new(1.0_f32, style::ACCENT.gamma_multiply(0.8)), StrokeKind::Inside);
                    } else if hovered {
                        p.rect_filled(hl, 8.0, if dark { Color32::from_white_alpha(12) } else { Color32::from_black_alpha(10) });
                    }
                    match self.thumbs.get(&path) {
                        Some(Some(tex)) => {
                            let size = tex.size_vec2();
                            let fit = size * (ICON / size.x.max(size.y));
                            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            p.image(tex.id(), Rect::from_center_size(icon_box.center(), fit), uv, Color32::WHITE);
                        }
                        _ => {
                            p.rect_filled(icon_box.shrink(12.0), 8.0, if dark { Color32::from_white_alpha(10) } else { Color32::from_black_alpha(8) });
                        }
                    }
                    p.galley(Pos2::new(rect.center().x, text_top), galley, text_color);
                }
                if let Some(r) = band_rect {
                    ui.painter().rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(10, 132, 255, 40));
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0_f32, style::ACCENT), StrokeKind::Inside);
                }
            });
            if dirty {
                self.save();
            }
            if let Some(idx) = open {
                let path = self.dir.join(&self.items[idx].name);
                if self.items[idx].is_dir {
                    go = Some(path);
                } else if let Err(e) = std::process::Command::new("explorer").arg(&path).spawn() {
                    self.error = Some(format!("Не удалось открыть {}: {e}", path.display()));
                }
            }
        });

        if let Some(dir) = go {
            self.navigate(dir);
        }
    }
}

/// Bind .cexp to this exe for the current user (no admin needed); re-run on each start so a moved exe stays bound.
fn register_extension() -> std::io::Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let exe = std::env::current_exe()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.create_subkey(r"Software\Classes\.cexp")?.0.set_value("", &"CanExp.Folder")?;
    hkcu.create_subkey(r"Software\Classes\CanExp.Folder")?.0.set_value("", &"Раскладка папки CanExp")?;
    hkcu.create_subkey(r"Software\Classes\CanExp.Folder\shell\open\command")?
        .0
        .set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
    // .cexp files show the app's icon; Explorer is told only when it changed, since that refreshes all its windows
    let icon = format!("\"{}\",0", exe.display());
    let (key, _) = hkcu.create_subkey(r"Software\Classes\CanExp.Folder\DefaultIcon")?;
    if key.get_value::<String, _>("").ok().as_deref() != Some(icon.as_str()) {
        key.set_value("", &icon)?;
        unsafe {
            use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }
    Ok(())
}

fn main() -> eframe::Result {
    let _ = register_extension(); // not fatal: the app still works without the association

    // Argument: a .cexp file (opened from Explorer) -> its folder; a folder -> that folder.
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .map(|p| if p.is_file() { p.parent().map(Path::to_path_buf).unwrap_or(p) } else { p })
        .filter(|p| p.is_dir())
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("C:\\"));

    eframe::run_native(
        "CanExp",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([960.0, 620.0])
                .with_min_inner_size([480.0, 300.0])
                .with_transparent(true)
                .with_decorations(false) // our own title bar carries the address field
                // the running window's icon (title, taskbar, Alt+Tab); without it eframe shows its own "e"
                .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("assets/icon.png is a valid PNG")),
            ..Default::default()
        },
        Box::new(|cc| {
            style::apply(&cc.egui_ctx);
            style::backdrop(cc);
            Ok(Box::new(App::new(dir, cc.egui_ctx.clone())))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_cell_skips_occupied() {
        let occ: HashSet<_> = [(0, 0), (1, 0), (0, 1)].into();
        assert_eq!(free_cell(&occ, 2), (1, 1));
        assert_eq!(free_cell(&HashSet::new(), 3), (0, 0));
    }

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(512), "512 Б");
        assert_eq!(human_size(1536), "1,5 КБ");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3,0 ГБ");
    }
}
