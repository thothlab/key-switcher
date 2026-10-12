//! Шрифты окна. Кириллица и типографские знаки есть в шрифтах egui по умолчанию (Ubuntu-Light,
//! Hack), а стрелок «→ ↔» из подсказок раздела «Разрешения» и из `THIRD_PARTY.md` там нет. Поэтому
//! последними запасными шрифтами обоих семейств подключаются системные шрифты символов — читаются
//! с диска при запуске окна, в бинарник ничего не встраивается.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// Кандидаты по ОС; подключаются все читаемые, по порядку.
const SYMBOL_FONTS: &[&str] = if cfg!(target_os = "macos") {
    &["/System/Library/Fonts/Apple Symbols.ttf"]
} else if cfg!(windows) {
    &["C:\\Windows\\Fonts\\seguisym.ttf"]
} else {
    &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ]
};

const SYMBOLS: &str = "system-symbols";

/// Шрифты egui по умолчанию плюс системные шрифты символов последними запасными. Нет ни одного
/// файла — остаются шрифты по умолчанию (стрелки покажутся квадратиками, текст читается).
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (i, path) in SYMBOL_FONTS.iter().enumerate() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let name = format!("{SYMBOLS}-{i}");
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    ctx.set_fonts(fonts);
}
