//! Отправка синтетических событий клавиатуры с пометкой в `kCGEventSourceUserData`.

use std::time::Duration;

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use crate::exec::Poster;

/// Строка Unicode в одном событии надёжно проходит до ~20 UTF-16 символов.
const UNICODE_CHUNK: usize = 20;

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
}

impl Poster for CgPoster {
    fn key(&self, keycode: u16, down: bool, flags: u64) {
        if let Some(event) = self.event(keycode, down, flags) {
            event.post(CGEventTapLocation::HID);
        }
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
            self.pause_ms(crate::exec::KEY_PAUSE_MS);
        }
    }

    fn pause_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}
