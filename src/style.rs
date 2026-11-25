//! macOS-like look: system font, flat rounded widgets, blue accent, translucent acrylic window.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Stroke, Theme};
use std::sync::Arc;

pub const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let mut load = |name: &str, file: &str| {
        let bytes = std::fs::read(format!(r"{dir}\Fonts\{file}")).ok()?;
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
        Some(name.to_string())
    };
    let ui = load("ui", "segoeui.ttf");
    let semibold = load("bold", "seguisb.ttf");
    let defaults = fonts.families[&FontFamily::Proportional].clone();
    if let Some(ui) = &ui {
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, ui.clone());
    }
    // the bold family must exist even when Segoe is missing, so it falls back to the regular chain
    let bold_chain = semibold.into_iter().chain(ui).chain(defaults).collect();
    fonts.families.insert(bold(), bold_chain);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        // Thin bar hugging the window edge; the gap to the edge equals its collapsed width.
        s.spacing.scroll.floating_width = 3.0;
        s.spacing.scroll.bar_outer_margin = 3.0;
        s.spacing.scroll.bar_width = 7.0; // hovered width (egui default 10)
        let v = &mut s.visuals;
        v.panel_fill = Color32::TRANSPARENT;
        v.window_corner_radius = 10.into();
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = 7.into();
            w.bg_stroke = Stroke::NONE;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::NONE;
    });
    ctx.style_mut_of(Theme::Dark, |s| {
        let v = &mut s.visuals;
        v.extreme_bg_color = Color32::from_black_alpha(70);
        v.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(16);
        v.widgets.hovered.weak_bg_fill = Color32::from_white_alpha(28);
        v.widgets.active.weak_bg_fill = Color32::from_white_alpha(40);
        v.override_text_color = Some(Color32::from_gray(235));
    });
    ctx.style_mut_of(Theme::Light, |s| {
        let v = &mut s.visuals;
        v.extreme_bg_color = Color32::from_black_alpha(12);
        v.widgets.inactive.weak_bg_fill = Color32::from_black_alpha(10);
        v.widgets.hovered.weak_bg_fill = Color32::from_black_alpha(20);
        v.widgets.active.weak_bg_fill = Color32::from_black_alpha(32);
        v.override_text_color = Some(Color32::from_gray(25));
    });
}

fn system_is_dark() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map_or(true, |light| light == 0)
}

/// Blurred acrylic behind the window (Windows 11 22H2+; older systems just keep a plain frame).
pub fn backdrop(cc: &eframe::CreationContext) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

    let Ok(handle) = cc.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = HWND(h.hwnd.get() as *mut _);
    let dark = system_is_dark() as i32;
    let kind = DWMSBT_TRANSIENTWINDOW.0; // acrylic: more see-through than Mica
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as _, 4);
        let round = DWMWCP_ROUND.0; // frameless windows lose the rounded corners otherwise
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
    }
}
