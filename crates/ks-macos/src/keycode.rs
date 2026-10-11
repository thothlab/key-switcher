//! Виртуальные коды клавиш macOS (`kVK_*` из Carbon `Events.h`) ↔ [`PhysicalKey`] и флаги
//! `CGEventFlags` → [`Modifiers`]. Коды 10 и 50 названы так же, как в фикстурах задачи 01
//! (`scripts/macos/dump-layouts.swift`): 10 — `IntlBackslash`, 50 — `Backquote`.

use ks_core::{KeyDir, Modifiers, PhysicalKey as K, PhysicalKey};

const TABLE: &[(u16, PhysicalKey)] = &[
    (0, K::KeyA),
    (1, K::KeyS),
    (2, K::KeyD),
    (3, K::KeyF),
    (4, K::KeyH),
    (5, K::KeyG),
    (6, K::KeyZ),
    (7, K::KeyX),
    (8, K::KeyC),
    (9, K::KeyV),
    (10, K::IntlBackslash),
    (11, K::KeyB),
    (12, K::KeyQ),
    (13, K::KeyW),
    (14, K::KeyE),
    (15, K::KeyR),
    (16, K::KeyY),
    (17, K::KeyT),
    (18, K::Digit1),
    (19, K::Digit2),
    (20, K::Digit3),
    (21, K::Digit4),
    (22, K::Digit6),
    (23, K::Digit5),
    (24, K::Equal),
    (25, K::Digit9),
    (26, K::Digit7),
    (27, K::Minus),
    (28, K::Digit8),
    (29, K::Digit0),
    (30, K::BracketRight),
    (31, K::KeyO),
    (32, K::KeyU),
    (33, K::BracketLeft),
    (34, K::KeyI),
    (35, K::KeyP),
    (37, K::KeyL),
    (38, K::KeyJ),
    (39, K::Quote),
    (40, K::KeyK),
    (41, K::Semicolon),
    (42, K::Backslash),
    (43, K::Comma),
    (44, K::Slash),
    (45, K::KeyN),
    (46, K::KeyM),
    (47, K::Period),
    (50, K::Backquote),
    (36, K::Enter),
    (48, K::Tab),
    (49, K::Space),
    (51, K::Backspace),
    (53, K::Escape),
    (54, K::MetaRight),
    (55, K::MetaLeft),
    (56, K::ShiftLeft),
    (57, K::CapsLock),
    (58, K::AltLeft),
    (59, K::ControlLeft),
    (60, K::ShiftRight),
    (61, K::AltRight),
    (62, K::ControlRight),
    (115, K::Home),
    (116, K::PageUp),
    (117, K::Delete),
    (119, K::End),
    (121, K::PageDown),
    (123, K::ArrowLeft),
    (124, K::ArrowRight),
    (125, K::ArrowDown),
    (126, K::ArrowUp),
];

/// `kVK_Delete` — Backspace.
pub const BACKSPACE: u16 = 51;

/// `CGEventFlags`: независимые от устройства биты.
pub const FLAG_CAPS: u64 = 0x0001_0000;
pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CTRL: u64 = 0x0004_0000;
pub const FLAG_ALT: u64 = 0x0008_0000;
pub const FLAG_CMD: u64 = 0x0010_0000;

/// Код macOS → физическая клавиша; неизвестный код — `Other(code)`.
pub fn to_physical(code: u16) -> PhysicalKey {
    TABLE
        .iter()
        .find(|(c, _)| *c == code)
        .map_or(K::Other(u32::from(code)), |(_, k)| *k)
}

/// Физическая клавиша → код macOS; `None`, если на Mac такой клавиши нет (`Pause`).
pub fn from_physical(key: PhysicalKey) -> Option<u16> {
    if let K::Other(raw) = key {
        return u16::try_from(raw).ok();
    }
    TABLE.iter().find(|(_, k)| *k == key).map(|(c, _)| *c)
}

/// Клавиши, печатающие символ: по ним строятся таблицы раскладок.
pub fn printable() -> impl Iterator<Item = (u16, PhysicalKey)> {
    TABLE.iter().copied().filter(|(_, k)| k.is_printable())
}

pub fn modifiers(flags: u64) -> Modifiers {
    Modifiers {
        shift: flags & FLAG_SHIFT != 0,
        caps_lock: flags & FLAG_CAPS != 0,
        alt: flags & FLAG_ALT != 0,
        ctrl: flags & FLAG_CTRL != 0,
        meta: flags & FLAG_CMD != 0,
    }
}

/// Направление события `FlagsChanged` для клавиши-модификатора: по зависимым от устройства
/// битам (`NX_DEVICEL*KEYMASK`), чтобы левый и правый Shift различались.
pub fn modifier_dir(key: PhysicalKey, flags: u64) -> Option<KeyDir> {
    let bit = match key {
        K::ControlLeft => 0x0001,
        K::ShiftLeft => 0x0002,
        K::ShiftRight => 0x0004,
        K::MetaLeft => 0x0008,
        K::MetaRight => 0x0010,
        K::AltLeft => 0x0020,
        K::AltRight => 0x0040,
        K::ControlRight => 0x2000,
        K::CapsLock => FLAG_CAPS,
        _ => return None,
    };
    Some(if flags & bit != 0 {
        KeyDir::Down
    } else {
        KeyDir::Up
    })
}
