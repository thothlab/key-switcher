//! Событие evdev → нейтральное [`InputEvent`] модели исполнения. Состояние модификаторов —
//! то, что уже применил этот трекер (один на все клавиатуры), без запросов к ОС: Shift на
//! одной клавиатуре и буква на другой дают Shift+буква, как в композиторе.

use ks_core::{KeyDir, KeyEvent, Modifiers, PhysicalKey};
use ks_platform::intercept::{InputEvent, InputKind};

use crate::keymap::{self, EV_KEY, EV_LED, LED_CAPSL};

/// Событие evdev как есть и модификаторы на момент чтения. Намеренно без `Debug`: внутри
/// набранная клавиша.
#[derive(Clone, Copy)]
pub struct LinuxEvent {
    pub code: u16,
    /// 0 — отпускание, 1 — нажатие, 2 — автоповтор.
    pub value: i32,
    pub mods: Modifiers,
}

const SLOTS: [PhysicalKey; 8] = [
    PhysicalKey::ShiftLeft,
    PhysicalKey::ShiftRight,
    PhysicalKey::ControlLeft,
    PhysicalKey::ControlRight,
    PhysicalKey::AltLeft,
    PhysicalKey::AltRight,
    PhysicalKey::MetaLeft,
    PhysicalKey::MetaRight,
];

#[derive(Default)]
pub struct Tracker {
    held: [bool; 8],
    caps: bool,
    last_ms: u64,
}

impl Tracker {
    /// `caps_lock` — состояние индикатора при открытии клавиатуры.
    pub fn new(caps_lock: bool) -> Tracker {
        Tracker {
            held: [false; 8],
            caps: caps_lock,
            last_ms: 0,
        }
    }

    pub fn mods(&self) -> Modifiers {
        let h = |a: usize, b: usize| self.held[a] || self.held[b];
        Modifiers {
            shift: h(0, 1),
            caps_lock: self.caps,
            ctrl: h(2, 3),
            alt: h(4, 5),
            meta: h(6, 7),
        }
    }

    /// Клавиатура пропала или ядро выбросило события: зажатых модификаторов больше не знаем.
    pub fn release_all(&mut self) {
        self.held = [false; 8];
    }

    /// Событие evdev → событие модели; `None` — не клавиша и не клик (оси, синхронизация).
    /// Время событий строго возрастает (два события в одну миллисекунду получают соседние
    /// значения): по нему `guard::InputWatch` отличает ввод после решения о замене.
    pub fn input_event(
        &mut self,
        kind: u16,
        code: u16,
        value: i32,
        time_ms: u64,
    ) -> Option<InputEvent<LinuxEvent>> {
        match kind {
            EV_LED if code == LED_CAPSL => {
                self.caps = value != 0;
                return None;
            }
            EV_KEY => {}
            _ => return None,
        }
        let time_ms = time_ms.max(self.last_ms + 1);
        if keymap::is_pointer_button(code) {
            let native = LinuxEvent {
                code,
                value,
                mods: self.mods(),
            };
            if value != 1 {
                return None;
            }
            self.last_ms = time_ms;
            return Some(InputEvent::new(InputKind::MouseDown, None, time_ms, native));
        }
        // Прочие кнопки (BTN_*: инструменты тачпада, джойстики) — не клавиатура.
        if (0x100..0x160).contains(&code) {
            return None;
        }
        self.last_ms = time_ms;
        let key = keymap::to_physical(code);
        let dir = if value == 0 { KeyDir::Up } else { KeyDir::Down };
        if let Some(i) = SLOTS.iter().position(|k| *k == key) {
            self.held[i] = dir == KeyDir::Down;
        }
        if key == PhysicalKey::CapsLock && value == 1 {
            self.caps = !self.caps;
        }
        let mods = self.mods();
        let kind = if key.is_modifier() {
            InputKind::Modifier
        } else {
            InputKind::Key
        };
        let ev = KeyEvent {
            key,
            dir,
            mods,
            repeat: value == 2,
            time_ms,
        };
        Some(InputEvent::new(
            kind,
            Some(ev),
            time_ms,
            LinuxEvent { code, value, mods },
        ))
    }
}
