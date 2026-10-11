//! Перевод событий `CGEventTap` в нейтральный [`InputEvent`] общей модели исполнения
//! (`ks_platform::intercept`: удержание, исполнитель, срок). Без системных вызовов — проверяется
//! тестом на любой ОС; колбэк (`sys::tap_thread`) только заполняет [`RawEvent`].

use ks_core::{KeyDir, KeyEvent};
use ks_platform::intercept::{InputEvent, InputKind};

use crate::keycode;

pub use ks_platform::clock_ms;
pub use ks_platform::intercept::BUSY_TIMEOUT_MS;

/// `kCGEventSourceUserData` своих событий замены: движок их не видит.
pub const MARK_OWN: i64 = 0x4B53_0001;
/// `kCGEventSourceUserData` событий `--selftest`: обрабатываются как нажатия пользователя.
pub const MARK_SELFTEST: i64 = 0x4B53_0002;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    KeyDown,
    KeyUp,
    FlagsChanged,
    MouseDown,
    DisabledByTimeout,
    DisabledByUserInput,
    Other,
}

/// Поля `CGEvent`, нужные логике. Намеренно без `Debug`: код клавиши — набранный символ.
#[derive(Clone, Copy)]
pub struct RawEvent {
    pub kind: RawKind,
    pub keycode: u16,
    pub flags: u64,
    pub autorepeat: bool,
    pub user_data: i64,
    pub time_ms: u64,
}

impl RawEvent {
    pub fn key(kind: RawKind, keycode: u16, flags: u64) -> RawEvent {
        RawEvent {
            kind,
            keycode,
            flags,
            autorepeat: false,
            user_data: 0,
            time_ms: 0,
        }
    }
}

/// Событие macOS для воспроизведения придержанного нажатия. Без `Debug`.
#[derive(Clone, Copy)]
pub struct MacEvent {
    pub keycode: u16,
    pub flags: u64,
    pub down: bool,
}

pub fn to_key_event(ev: &RawEvent) -> Option<KeyEvent> {
    let key = keycode::to_physical(ev.keycode);
    let dir = match ev.kind {
        RawKind::KeyDown => KeyDir::Down,
        RawKind::KeyUp => KeyDir::Up,
        RawKind::FlagsChanged => keycode::modifier_dir(key, ev.flags)?,
        _ => return None,
    };
    Some(KeyEvent {
        key,
        dir,
        mods: keycode::modifiers(ev.flags),
        repeat: ev.autorepeat,
        time_ms: ev.time_ms,
    })
}

/// `RawEvent` → событие общей модели: модификаторы из `flagsChanged` — отдельные `Modifier`
/// со стороной, свои события (`MARK_OWN`) помечены `own`.
pub fn input_event(ev: &RawEvent) -> InputEvent<MacEvent> {
    let kind = match ev.kind {
        RawKind::KeyDown | RawKind::KeyUp => InputKind::Key,
        RawKind::FlagsChanged => InputKind::Modifier,
        RawKind::MouseDown => InputKind::MouseDown,
        RawKind::DisabledByTimeout | RawKind::DisabledByUserInput => InputKind::Disabled,
        RawKind::Other => InputKind::Other,
    };
    let native = MacEvent {
        keycode: ev.keycode,
        flags: ev.flags,
        down: ev.kind == RawKind::KeyDown,
    };
    let mut out = InputEvent::new(kind, to_key_event(ev), ev.time_ms, native);
    out.own = ev.user_data == MARK_OWN;
    out
}
