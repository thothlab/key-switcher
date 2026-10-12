//! Коды evdev `KEY_*` (`linux/input-event-codes.h`) ↔ `PhysicalKey`. Без системных вызовов.
//! Код XKB = код evdev + 8.

use ks_core::PhysicalKey as K;

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_LED: u16 = 0x11;
pub const SYN_DROPPED: u16 = 3;
pub const LED_CAPSL: u16 = 0x01;

pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_A: u16 = 30;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_Z: u16 = 44;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_SPACE: u16 = 57;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
/// Касание тачпада: «тап как клик» синтезирует libinput, ядро даёт только `BTN_TOUCH`.
pub const BTN_TOUCH: u16 = 0x14a;

/// Все клавиши, которые знает `PhysicalKey` (кроме `Other`), с кодами evdev.
pub const KEYS: &[(u16, K)] = &[
    (1, K::Escape),
    (2, K::Digit1),
    (3, K::Digit2),
    (4, K::Digit3),
    (5, K::Digit4),
    (6, K::Digit5),
    (7, K::Digit6),
    (8, K::Digit7),
    (9, K::Digit8),
    (10, K::Digit9),
    (11, K::Digit0),
    (12, K::Minus),
    (13, K::Equal),
    (14, K::Backspace),
    (15, K::Tab),
    (16, K::KeyQ),
    (17, K::KeyW),
    (18, K::KeyE),
    (19, K::KeyR),
    (20, K::KeyT),
    (21, K::KeyY),
    (22, K::KeyU),
    (23, K::KeyI),
    (24, K::KeyO),
    (25, K::KeyP),
    (26, K::BracketLeft),
    (27, K::BracketRight),
    (28, K::Enter),
    (29, K::ControlLeft),
    (30, K::KeyA),
    (31, K::KeyS),
    (32, K::KeyD),
    (33, K::KeyF),
    (34, K::KeyG),
    (35, K::KeyH),
    (36, K::KeyJ),
    (37, K::KeyK),
    (38, K::KeyL),
    (39, K::Semicolon),
    (40, K::Quote),
    (41, K::Backquote),
    (42, K::ShiftLeft),
    (43, K::Backslash),
    (44, K::KeyZ),
    (45, K::KeyX),
    (46, K::KeyC),
    (47, K::KeyV),
    (48, K::KeyB),
    (49, K::KeyN),
    (50, K::KeyM),
    (51, K::Comma),
    (52, K::Period),
    (53, K::Slash),
    (54, K::ShiftRight),
    (56, K::AltLeft),
    (57, K::Space),
    (58, K::CapsLock),
    (86, K::IntlBackslash),
    (97, K::ControlRight),
    (100, K::AltRight),
    (102, K::Home),
    (103, K::ArrowUp),
    (104, K::PageUp),
    (105, K::ArrowLeft),
    (106, K::ArrowRight),
    (107, K::End),
    (108, K::ArrowDown),
    (109, K::PageDown),
    (111, K::Delete),
    (119, K::Pause),
    (125, K::MetaLeft),
    (126, K::MetaRight),
];

/// Код evdev → физическая клавиша; неизвестный код — `Other(code)`.
pub fn to_physical(code: u16) -> K {
    KEYS.iter()
        .find(|(c, _)| *c == code)
        .map(|(_, k)| *k)
        .unwrap_or(K::Other(u32::from(code)))
}

/// Физическая клавиша → код evdev; `Other(n)` — сырой код, если он помещается в `u16`.
pub fn to_evdev(key: K) -> Option<u16> {
    if let K::Other(n) = key {
        return u16::try_from(n).ok();
    }
    KEYS.iter().find(|(_, k)| *k == key).map(|(c, _)| *c)
}

/// Кнопка мыши или касание тачпада: сброс слова в работе.
pub fn is_pointer_button(code: u16) -> bool {
    matches!(code, BTN_LEFT | BTN_RIGHT | BTN_MIDDLE | BTN_TOUCH)
}

/// Клавиши с символом (буквы, цифры, знаки) — для таблиц раскладки.
pub fn printable() -> impl Iterator<Item = (u16, K)> {
    KEYS.iter().copied().filter(|(_, k)| k.is_printable())
}
