//! Построение [`LayoutMap`] из функции перевода «код клавиши + уровень → символ».
//! Системная часть (`TIS*`, `UCKeyTranslate`) живёт в `sys::tis` и передаёт сюда замыкание.

use ks_core::{Lang, LayoutId, LayoutMap, Level};

use crate::keycode;

pub const LEVELS: [Level; 4] = [Level::Base, Level::Shift, Level::Caps, Level::ShiftCaps];

/// Таблица раскладки по всем печатающим клавишам и четырём уровням.
pub fn build_layout(
    id: LayoutId,
    lang: Lang,
    name: &str,
    translate: impl Fn(u16, Level) -> Option<char>,
) -> LayoutMap {
    let mut map = LayoutMap::new(id, lang, name);
    for (code, key) in keycode::printable() {
        for level in LEVELS {
            if let Some(ch) = translate(code, level) {
                map.insert(key, level, ch);
            }
        }
    }
    map
}

/// Состояние модификаторов для `UCKeyTranslate`: `(shiftKey | alphaLock) >> 8`.
pub fn uc_modifier_state(level: Level) -> u32 {
    match level {
        Level::Base => 0,
        Level::Shift => 2,
        Level::Caps => 4,
        Level::ShiftCaps => 6,
    }
}

/// Один видимый символ из вывода `UCKeyTranslate`; управляющие и составные — `None`.
pub fn printable_char(utf16: &[u16]) -> Option<char> {
    let mut chars = char::decode_utf16(utf16.iter().copied());
    let ch = chars.next()?.ok()?;
    if chars.next().is_some() || (ch as u32) < 0x20 || ch == '\u{7f}' {
        return None;
    }
    Some(ch)
}

/// Язык раскладки по первому коду из `kTISPropertyInputSourceLanguages`.
pub fn lang_of(languages: &[String]) -> Lang {
    match languages
        .first()
        .map(|l| l.split(['-', '_']).next().unwrap_or(""))
    {
        Some("ru") => Lang::Ru,
        Some("en") => Lang::En,
        _ => Lang::Other,
    }
}
