//! Отправка синтетических событий клавиатуры с пометкой в `kCGEventSourceUserData`.

use std::time::Duration;

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use ks_core::PhysicalKey;
use ks_platform::exec::{Chord, KEY_PAUSE_MS, Poster};

use crate::keycode::{self, FLAG_CMD, FLAG_SHIFT};
use crate::tap::MacEvent;

/// Строка Unicode в одном событии надёжно проходит до ~20 UTF-16 символов.
const UNICODE_CHUNK: usize = 20;
/// kVK_ANSI_C, kVK_ANSI_V: Cmd-сочетания macOS идут по физической клавише в любой раскладке.
const KVK_C: u16 = 8;
const KVK_V: u16 = 9;

pub struct CgPoster {
    pub mark: i64,
}

impl CgPoster {
    fn event(&self, keycode: u16, down: bool, flags: u64) -> Option<CGEvent> {
        // Свой источник: флаги события — ровно те, что заданы, а не зажатые пользователем.
        let source = CGEventSource::new(CGEventSourceStateID::Private).ok()?;
        let event = CGEvent::new_keyboard_event(source, keycode, down).ok()?;
        event.set_flags(CGEventFlags::from_bits_retain(flags));
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.mark);
        Some(event)
    }

    fn post(&self, keycode: u16, down: bool, flags: u64) {
        if let Some(event) = self.event(keycode, down, flags) {
            event.post(CGEventTapLocation::HID);
        }
    }

    /// Нажатие или отпускание модификатора: событие `FlagsChanged` с флагами всего зажатого
    /// (`keycode::modifier_steps`), как от клавиатуры, а не `KeyDown`/`KeyUp`.
    pub(crate) fn flags_changed(&self, keycode: u16, flags: u64) {
        if let Some(event) = self.event(keycode, flags != 0, flags) {
            event.set_type(CGEventType::FlagsChanged);
            event.post(CGEventTapLocation::HID);
        }
    }
}

impl Poster<MacEvent> for CgPoster {
    fn can_type(&self, key: PhysicalKey) -> bool {
        keycode::from_physical(key).is_some()
    }

    fn key(&self, key: PhysicalKey, down: bool, shift: bool) {
        if let Some(code) = keycode::from_physical(key) {
            self.post(code, down, if shift { FLAG_SHIFT } else { 0 });
        }
    }

    fn chord(&self, chord: Chord) {
        let code = match chord {
            Chord::Copy => KVK_C,
            Chord::Paste => KVK_V,
        };
        self.post(code, true, FLAG_CMD);
        self.post(code, false, FLAG_CMD);
        self.pause_ms(KEY_PAUSE_MS);
    }

    fn unicode(&self, text: &str) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        for chunk in utf16.chunks(UNICODE_CHUNK) {
            for down in [true, false] {
                if let Some(event) = self.event(0, down, 0) {
                    event.set_string_from_utf16_unchecked(chunk);
                    event.post(CGEventTapLocation::HID);
                }
            }
            self.pause_ms(KEY_PAUSE_MS);
        }
    }

    fn replay(&self, native: &MacEvent) {
        self.post(native.keycode, native.down, native.flags);
    }

    fn pause_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}
