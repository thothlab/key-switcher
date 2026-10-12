//! Коды клавиш Windows ↔ [`PhysicalKey`]: скан-код набора 1 (позиция клавиши, смысл W3C
//! `KeyboardEvent.code`) и виртуальный код `VK_*`. Без системных вызовов — проверяется тестом на
//! любой ОС. Скан-код с префиксом E0 (`LLKHF_EXTENDED`, `KEYEVENTF_EXTENDEDKEY`) хранится с битом
//! [`EXT`].

use ks_core::{PhysicalKey as K, PhysicalKey};

/// Бит расширенной клавиши (префикс E0) в скан-коде.
pub const EXT: u16 = 0xE000;

pub const VK_BACK: u16 = 0x08;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_PAUSE: u16 = 0x13;
pub const VK_CAPITAL: u16 = 0x14;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;
/// Строка Unicode чужого `SendInput` (`KEYEVENTF_UNICODE`).
pub const VK_PACKET: u16 = 0xE7;
pub const VK_C: u16 = 0x43;
pub const VK_V: u16 = 0x56;

/// (клавиша, скан-код с [`EXT`], VK). VK печатающих клавиш — как в US и KBDRU; для других
/// раскладок он может отличаться, поэтому печатающие клавиши ищутся по скан-коду.
const TABLE: &[(PhysicalKey, u16, u16)] = &[
    (K::KeyA, 0x1E, 0x41),
    (K::KeyB, 0x30, 0x42),
    (K::KeyC, 0x2E, 0x43),
    (K::KeyD, 0x20, 0x44),
    (K::KeyE, 0x12, 0x45),
    (K::KeyF, 0x21, 0x46),
    (K::KeyG, 0x22, 0x47),
    (K::KeyH, 0x23, 0x48),
    (K::KeyI, 0x17, 0x49),
    (K::KeyJ, 0x24, 0x4A),
    (K::KeyK, 0x25, 0x4B),
    (K::KeyL, 0x26, 0x4C),
    (K::KeyM, 0x32, 0x4D),
    (K::KeyN, 0x31, 0x4E),
    (K::KeyO, 0x18, 0x4F),
    (K::KeyP, 0x19, 0x50),
    (K::KeyQ, 0x10, 0x51),
    (K::KeyR, 0x13, 0x52),
    (K::KeyS, 0x1F, 0x53),
    (K::KeyT, 0x14, 0x54),
    (K::KeyU, 0x16, 0x55),
    (K::KeyV, 0x2F, 0x56),
    (K::KeyW, 0x11, 0x57),
    (K::KeyX, 0x2D, 0x58),
    (K::KeyY, 0x15, 0x59),
    (K::KeyZ, 0x2C, 0x5A),
    (K::Digit1, 0x02, 0x31),
    (K::Digit2, 0x03, 0x32),
    (K::Digit3, 0x04, 0x33),
    (K::Digit4, 0x05, 0x34),
    (K::Digit5, 0x06, 0x35),
    (K::Digit6, 0x07, 0x36),
    (K::Digit7, 0x08, 0x37),
    (K::Digit8, 0x09, 0x38),
    (K::Digit9, 0x0A, 0x39),
    (K::Digit0, 0x0B, 0x30),
    (K::Minus, 0x0C, 0xBD),
    (K::Equal, 0x0D, 0xBB),
    (K::BracketLeft, 0x1A, 0xDB),
    (K::BracketRight, 0x1B, 0xDD),
    (K::Semicolon, 0x27, 0xBA),
    (K::Quote, 0x28, 0xDE),
    (K::Backquote, 0x29, 0xC0),
    (K::Backslash, 0x2B, 0xDC),
    (K::Comma, 0x33, 0xBC),
    (K::Period, 0x34, 0xBE),
    (K::Slash, 0x35, 0xBF),
    (K::IntlBackslash, 0x56, 0xE2),
    (K::Space, 0x39, 0x20),
    (K::Escape, 0x01, 0x1B),
    (K::Backspace, 0x0E, VK_BACK),
    (K::Tab, 0x0F, 0x09),
    (K::Enter, 0x1C, 0x0D),
    (K::ControlLeft, 0x1D, VK_LCONTROL),
    (K::ShiftLeft, 0x2A, VK_LSHIFT),
    (K::ShiftRight, 0x36, VK_RSHIFT),
    (K::AltLeft, 0x38, VK_LMENU),
    (K::CapsLock, 0x3A, VK_CAPITAL),
    (K::ControlRight, EXT | 0x1D, VK_RCONTROL),
    (K::AltRight, EXT | 0x38, VK_RMENU),
    (K::Home, EXT | 0x47, 0x24),
    (K::ArrowUp, EXT | 0x48, 0x26),
    (K::PageUp, EXT | 0x49, 0x21),
    (K::ArrowLeft, EXT | 0x4B, 0x25),
    (K::ArrowRight, EXT | 0x4D, 0x27),
    (K::End, EXT | 0x4F, 0x23),
    (K::ArrowDown, EXT | 0x50, 0x28),
    (K::PageDown, EXT | 0x51, 0x22),
    (K::Delete, EXT | 0x53, 0x2E),
    (K::MetaLeft, EXT | 0x5B, VK_LWIN),
    (K::MetaRight, EXT | 0x5C, VK_RWIN),
    // Pause приходит как E1 1D 45; в хуке — VK_PAUSE со скан-кодом 0x45 (см. `to_physical`).
    (K::Pause, 0x45, VK_PAUSE),
];

/// Скан-код события хука с битом [`EXT`]; служебные биты выше байта отбрасываются.
pub fn scan_code(scan: u32, extended: bool) -> u16 {
    let base = (scan & 0xFF) as u16;
    if extended { EXT | base } else { base }
}

/// Событие хука → физическая клавиша. Печатающие клавиши — по скан-коду (позиция не зависит от
/// раскладки); неизвестный скан-код или его отсутствие (чужой `SendInput` только с VK) — по VK
/// тех клавиш, у которых он от раскладки не зависит; иначе `Other(скан-код)`.
pub fn to_physical(vk: u16, scan: u32, extended: bool) -> PhysicalKey {
    if vk == VK_PAUSE {
        return K::Pause;
    }
    if vk == VK_PACKET {
        // В scanCode — символ UTF-16 (менеджер паролей, панель эмодзи, сенсорная клавиатура), не
        // позиция клавиши: движку — `Other`, слово сбрасывается.
        return K::Other(0x1_0000 | u32::from(VK_PACKET));
    }
    let code = scan_code(scan, extended);
    if code & 0xFF != 0
        && let Some((key, _, _)) = TABLE.iter().find(|(k, s, _)| *s == code && *k != K::Pause)
    {
        return *key;
    }
    let no_scan = code & 0xFF == 0;
    if let Some((key, _, _)) = TABLE
        .iter()
        .find(|(k, _, v)| *v == vk && (no_scan || !k.is_printable()))
    {
        return *key;
    }
    if no_scan {
        K::Other(0x1_0000 | u32::from(vk))
    } else {
        K::Other(u32::from(code))
    }
}

/// Скан-код физической клавиши (с [`EXT`]); `None` — у клавиши его нет.
pub fn scan_of(key: PhysicalKey) -> Option<u16> {
    TABLE.iter().find(|(k, _, _)| *k == key).map(|(_, s, _)| *s)
}

/// VK физической клавиши для `SendInput`; `None` — у клавиши его нет (`Other`).
pub fn vk_of(key: PhysicalKey) -> Option<u16> {
    TABLE.iter().find(|(k, _, _)| *k == key).map(|(_, _, v)| *v)
}

/// Печатающие клавиши `(клавиша, скан-код, VK)`: по ним строятся таблицы раскладок.
pub fn printable() -> impl Iterator<Item = (PhysicalKey, u16, u16)> {
    TABLE.iter().copied().filter(|(k, _, _)| k.is_printable())
}
