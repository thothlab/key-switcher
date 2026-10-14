//! Шрифты окна. Основной — Golos Text (Regular и Medium, встроены, OFL 1.1): гротеск, сделанный
//! под кириллицу. За ним — шрифты egui по умолчанию, а последними запасными — системные шрифты
//! символов: в Golos нет «↔» и модификаторов ⌘ ⌥ ⇧ ⌃, они встречаются в `THIRD_PARTY.md` и
//! подсказках. Системные шрифты читаются с диска при запуске окна, в бинарник не входят.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

const GOLOS_REGULAR: &[u8] = include_bytes!("../assets/fonts/GolosText-Regular.ttf");
const GOLOS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/GolosText-Medium.ttf");

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
const REGULAR: &str = "Golos Text Regular";
const MEDIUM: &str = "Golos Text Medium";

/// Golos Text первым в обоих пропорциональных семействах (обычном и `medium`), шрифты egui по
/// умолчанию — запасными, системные шрифты символов — последними. Нет ни одного файла символов —
/// стрелки покажутся квадратиками, текст читается.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts.font_data.insert(
        REGULAR.to_owned(),
        Arc::new(FontData::from_static(GOLOS_REGULAR)),
    );
    fonts.font_data.insert(
        MEDIUM.to_owned(),
        Arc::new(FontData::from_static(GOLOS_MEDIUM)),
    );
    let mut proportional = vec![REGULAR.to_owned()];
    proportional.extend(defaults.iter().cloned());
    let mut medium = vec![MEDIUM.to_owned(), REGULAR.to_owned()];
    medium.extend(defaults);
    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    fonts.families.insert(crate::theme::medium(), medium);
    for (i, path) in SYMBOL_FONTS.iter().enumerate() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let name = format!("{SYMBOLS}-{i}");
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            crate::theme::medium(),
        ] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    ctx.set_fonts(fonts);
}
