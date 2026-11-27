//! Interface language: Windows' UI language (Ukrainian, Russian, otherwise English), or CANEXP_LANG=en|uk|ru.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    En,
    Uk,
    Ru,
}

pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        match std::env::var("CANEXP_LANG").as_deref() {
            Ok("en") => return Lang::En,
            Ok("uk") => return Lang::Uk,
            Ok("ru") => return Lang::Ru,
            _ => {}
        }
        // primary language id: the low 10 bits of the LANGID
        match unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff {
            0x22 => Lang::Uk,
            0x19 => Lang::Ru,
            _ => Lang::En,
        }
    })
}

/// The text for the current language.
pub fn t(en: &'static str, uk: &'static str, ru: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Uk => uk,
        Lang::Ru => ru,
    }
}
