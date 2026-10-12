//! Запись сочетания нажатием: события egui → `Hotkey`. Клавиша берётся по физическому положению
//! (`physical_key`), поэтому записанное не зависит от раскладки. Сочетание из одних модификаторов
//! (⌥⇧) записывается при отпускании, если их было не меньше двух. Pause egui не видит, а у F-клавиш
//! нет имени в `PhysicalKey`: Pause, Shift+Pause и жесты Shift выбираются из списка.

use eframe::egui::{self, Event, Key};
use ks_core::{Hotkey, Modifiers, PhysicalKey};

/// Что получилось после очередного кадра записи.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Ждём нажатия.
    Pending,
    Done(Hotkey),
    /// Escape без модификаторов — запись отменена.
    Cancelled,
    /// Клавишу записать нельзя; текст для окна.
    Refused(&'static str),
}

pub const REFUSED_KEY: &str = "Эту клавишу записать нельзя. F-клавиши не поддерживаются; Pause и \
                               Shift+Pause выбираются из списка жестов.";

/// Запись одного сочетания.
#[derive(Debug, Default)]
pub struct Recorder {
    /// Модификаторы, зажатые с начала текущего жеста (объединение).
    peak: Modifiers,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// События кадра и модификаторы, зажатые после них.
    pub fn feed(&mut self, events: &[Event], held: egui::Modifiers) -> Recorded {
        for ev in events {
            if let Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat: false,
                modifiers,
            } = ev
            {
                let mods = modifiers_of(*modifiers);
                let key = physical_key.unwrap_or(*key);
                if key == Key::Escape && count(mods) == 0 {
                    return Recorded::Cancelled;
                }
                return match physical_of(key) {
                    Some(key) => Recorded::Done(Hotkey::Combo {
                        mods,
                        key: Some(key),
                    }),
                    None => Recorded::Refused(REFUSED_KEY),
                };
            }
        }
        let now = modifiers_of(held);
        if count(now) > 0 {
            self.peak = union(self.peak, now);
            return Recorded::Pending;
        }
        let peak = std::mem::take(&mut self.peak);
        if count(peak) >= 2 {
            Recorded::Done(Hotkey::Combo {
                mods: peak,
                key: None,
            })
        } else {
            Recorded::Pending
        }
    }
}

/// Модификаторы egui → модификаторы KeySwitcher. ⌘ есть только на macOS (`mac_cmd`); `ctrl` —
/// физический Control на всех ОС.
pub fn modifiers_of(m: egui::Modifiers) -> Modifiers {
    Modifiers {
        shift: m.shift,
        caps_lock: false,
        alt: m.alt,
        ctrl: m.ctrl,
        meta: m.mac_cmd,
    }
}

fn count(m: Modifiers) -> usize {
    [m.shift, m.alt, m.ctrl, m.meta]
        .into_iter()
        .filter(|&b| b)
        .count()
}

fn union(a: Modifiers, b: Modifiers) -> Modifiers {
    Modifiers {
        shift: a.shift || b.shift,
        caps_lock: false,
        alt: a.alt || b.alt,
        ctrl: a.ctrl || b.ctrl,
        meta: a.meta || b.meta,
    }
}

/// Клавиша egui → `PhysicalKey`; `None` — у клавиши нет имени в `PhysicalKey` (F-клавиши и пр.).
pub fn physical_of(key: Key) -> Option<PhysicalKey> {
    use PhysicalKey as P;
    let letters = [
        (Key::A, P::KeyA),
        (Key::B, P::KeyB),
        (Key::C, P::KeyC),
        (Key::D, P::KeyD),
        (Key::E, P::KeyE),
        (Key::F, P::KeyF),
        (Key::G, P::KeyG),
        (Key::H, P::KeyH),
        (Key::I, P::KeyI),
        (Key::J, P::KeyJ),
        (Key::K, P::KeyK),
        (Key::L, P::KeyL),
        (Key::M, P::KeyM),
        (Key::N, P::KeyN),
        (Key::O, P::KeyO),
        (Key::P, P::KeyP),
        (Key::Q, P::KeyQ),
        (Key::R, P::KeyR),
        (Key::S, P::KeyS),
        (Key::T, P::KeyT),
        (Key::U, P::KeyU),
        (Key::V, P::KeyV),
        (Key::W, P::KeyW),
        (Key::X, P::KeyX),
        (Key::Y, P::KeyY),
        (Key::Z, P::KeyZ),
        (Key::Num0, P::Digit0),
        (Key::Num1, P::Digit1),
        (Key::Num2, P::Digit2),
        (Key::Num3, P::Digit3),
        (Key::Num4, P::Digit4),
        (Key::Num5, P::Digit5),
        (Key::Num6, P::Digit6),
        (Key::Num7, P::Digit7),
        (Key::Num8, P::Digit8),
        (Key::Num9, P::Digit9),
        (Key::Backtick, P::Backquote),
        (Key::Minus, P::Minus),
        (Key::Equals, P::Equal),
        (Key::OpenBracket, P::BracketLeft),
        (Key::CloseBracket, P::BracketRight),
        (Key::Backslash, P::Backslash),
        (Key::Semicolon, P::Semicolon),
        (Key::Quote, P::Quote),
        (Key::Comma, P::Comma),
        (Key::Period, P::Period),
        (Key::Slash, P::Slash),
        (Key::Space, P::Space),
        (Key::Enter, P::Enter),
        (Key::Tab, P::Tab),
        (Key::Backspace, P::Backspace),
        (Key::Delete, P::Delete),
        (Key::Escape, P::Escape),
        (Key::ArrowLeft, P::ArrowLeft),
        (Key::ArrowRight, P::ArrowRight),
        (Key::ArrowUp, P::ArrowUp),
        (Key::ArrowDown, P::ArrowDown),
        (Key::Home, P::Home),
        (Key::End, P::End),
        (Key::PageUp, P::PageUp),
        (Key::PageDown, P::PageDown),
    ];
    letters.into_iter().find(|(k, _)| *k == key).map(|(_, p)| p)
}

/// Подпись сочетания в окне: «Ctrl+Option+K», «двойной Shift», «Shift+Pause».
pub fn label(h: &Hotkey) -> String {
    match h {
        Hotkey::DoubleShift => "двойной Shift".to_owned(),
        Hotkey::BothShifts => "оба Shift".to_owned(),
        Hotkey::Combo { .. } => h
            .to_string()
            .split('+')
            .map(|part| match part {
                "Alt" if cfg!(target_os = "macos") => "Option".to_owned(),
                "Meta" if cfg!(target_os = "macos") => "Cmd".to_owned(),
                "Meta" => "Win".to_owned(),
                p => p
                    .strip_prefix("Key")
                    .or_else(|| p.strip_prefix("Digit"))
                    .unwrap_or(p)
                    .to_owned(),
            })
            .collect::<Vec<_>>()
            .join("+"),
    }
}

/// Жесты, которые выбираются из списка, а не записываются нажатием.
pub fn gestures() -> Vec<Hotkey> {
    ["DoubleShift", "BothShifts", "Pause", "Shift+Pause"]
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect()
}
