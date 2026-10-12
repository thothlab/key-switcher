//! Таблицы раскладок и идентификаторы Windows без системных вызовов. [`build_layout`] строит
//! [`LayoutMap`] из функции «клавиша + уровень → символ»: в продукте это `ToUnicodeEx`
//! (`sys::system`), в тестах — разбор KLC ([`crate::klc`]).
//!
//! `LayoutId` раскладки Windows — `HKL` как 8 шестнадцатеричных цифр младших 32 бит
//! (`04190419` — «Русская», `04090409` — «США»): из него без потерь восстанавливается `HKL` для
//! `WM_INPUTLANGCHANGEREQUEST`.

use ks_core::{Lang, LayoutId, LayoutMap, Level};

use crate::keycode::{self, VK_CAPITAL, VK_SHIFT};

pub const LEVELS: [Level; 4] = [Level::Base, Level::Shift, Level::Caps, Level::ShiftCaps];

/// Флаг `ToUnicodeEx` (бит 2, Windows 10 1607+): не менять состояние клавиатуры ядра.
pub const TOUNICODE_NO_STATE_CHANGE: u32 = 0x4;

/// Таблица раскладки по всем печатающим клавишам и четырём уровням; `translate(vk, скан-код,
/// уровень)`.
pub fn build_layout(
    id: LayoutId,
    lang: Lang,
    name: &str,
    translate: impl Fn(u16, u16, Level) -> Option<char>,
) -> LayoutMap {
    let mut map = LayoutMap::new(id, lang, name);
    for (key, scan, vk) in keycode::printable() {
        for level in LEVELS {
            if let Some(ch) = translate(vk, scan, level) {
                map.insert(key, level, ch);
            }
        }
    }
    map
}

/// Состояние клавиатуры для `ToUnicodeEx` на уровне: Shift зажат (`0x80`), CapsLock включён
/// (`0x01`).
pub fn key_state(level: Level) -> [u8; 256] {
    let mut state = [0u8; 256];
    if level.is_shift() {
        state[usize::from(VK_SHIFT)] = 0x80;
    }
    if matches!(level, Level::Caps | Level::ShiftCaps) {
        state[usize::from(VK_CAPITAL)] = 0x01;
    }
    state
}

/// Один видимый символ из вывода `ToUnicodeEx`: `n` — его возврат. Мёртвая клавиша (`n < 0`),
/// управляющие и составные символы — `None`.
pub fn printable_char(buf: &[u16], n: i32) -> Option<char> {
    let n = usize::try_from(n).ok()?;
    let mut chars = char::decode_utf16(buf.get(..n)?.iter().copied());
    let ch = chars.next()?.ok()?;
    if chars.next().is_some() || (ch as u32) < 0x20 || ch == '\u{7f}' {
        return None;
    }
    Some(ch)
}

/// `HKL` → `LayoutId`.
pub fn layout_id(hkl: isize) -> LayoutId {
    LayoutId(format!("{:08X}", hkl as u32))
}

/// `LayoutId` → `HKL` (знаковое расширение младших 32 бит, как у `HKL` на x64).
pub fn hkl_of(id: &LayoutId) -> Option<isize> {
    if id.0.len() != 8 {
        return None;
    }
    let raw = u32::from_str_radix(&id.0, 16).ok()?;
    Some(raw as i32 as isize)
}

/// Язык раскладки по основному языку `LANGID` (младшее слово `HKL`).
pub fn lang_of(hkl: isize) -> Lang {
    match (hkl as u32) & 0x3FF {
        0x19 => Lang::Ru,
        0x09 => Lang::En,
        _ => Lang::Other,
    }
}

/// Стандартная раскладка: `HKL` по KLID (`00000419` → `04190419`). Варианты (`00010419`,
/// `HKL` вида `F0xx0419`) так не выводятся — `None`.
pub fn hkl_for_klid(klid: &str) -> Option<isize> {
    let raw = u32::from_str_radix(klid, 16).ok()?;
    if klid.len() != 8 || raw > 0xFFFF {
        return None;
    }
    Some(((raw << 16) | raw) as i32 as isize)
}

/// KLID стандартной раскладки по `HKL` — ключ реестра с её названием; для вариантов — `None`.
pub fn klid_of(hkl: isize) -> Option<String> {
    let device = ((hkl as u32) >> 16) as u16;
    if device == 0 || device & 0xF000 == 0xF000 {
        return None;
    }
    Some(format!("0000{device:04X}"))
}
