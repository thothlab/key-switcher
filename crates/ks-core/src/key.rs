//! Физические клавиши, модификаторы и нажатия.

use std::fmt;
use std::str::FromStr;

macro_rules! physical_keys {
    (alnum: [$($a:ident),* $(,)?], punct: [$($p:ident),* $(,)?], other: [$($o:ident),* $(,)?]) => {
        /// Физическая клавиша; смысл совпадает с `KeyboardEvent.code` (W3C).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum PhysicalKey {
            $($a,)*
            $($p,)*
            $($o,)*
            /// Всё прочее — сырой код ОС; буфер такое нажатие сбрасывает.
            Other(u32),
        }

        const NAMED_KEYS: &[(PhysicalKey, &str)] = &[
            $((PhysicalKey::$a, stringify!($a)),)*
            $((PhysicalKey::$p, stringify!($p)),)*
            $((PhysicalKey::$o, stringify!($o)),)*
        ];

        impl PhysicalKey {
            /// Буква `KeyA`…`KeyZ` или цифра `Digit0`…`Digit9` основного блока.
            pub fn is_letter_or_digit(self) -> bool {
                matches!(self, $(PhysicalKey::$a)|*)
            }

            /// Клавиша, которая печатает символ (буквы, цифры, знаки) — то, что копится в слове.
            pub fn is_printable(self) -> bool {
                matches!(self, $(PhysicalKey::$a)|* | $(PhysicalKey::$p)|*)
            }
        }
    };
}

physical_keys!(
    alnum: [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM,
        KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ],
    punct: [
        Backquote, Minus, Equal, BracketLeft, BracketRight, Backslash, IntlBackslash,
        Semicolon, Quote, Comma, Period, Slash,
    ],
    other: [
        Space, Enter, Tab, Backspace, Delete, Escape,
        ArrowLeft, ArrowRight, ArrowUp, ArrowDown, Home, End, PageUp, PageDown,
        ShiftLeft, ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight,
        MetaLeft, MetaRight, CapsLock, Pause,
    ]
);

impl PhysicalKey {
    /// Имя W3C (`"KeyA"`, `"Comma"`); у `Other` имени нет.
    pub fn code(self) -> Option<&'static str> {
        NAMED_KEYS
            .iter()
            .find(|(key, _)| *key == self)
            .map(|(_, name)| *name)
    }

    /// Shift, Ctrl, Alt, Meta с обеих сторон и CapsLock.
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            PhysicalKey::ShiftLeft
                | PhysicalKey::ShiftRight
                | PhysicalKey::ControlLeft
                | PhysicalKey::ControlRight
                | PhysicalKey::AltLeft
                | PhysicalKey::AltRight
                | PhysicalKey::MetaLeft
                | PhysicalKey::MetaRight
                | PhysicalKey::CapsLock
        )
    }
}

impl fmt::Display for PhysicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PhysicalKey::Other(raw) => write!(f, "Other({raw})"),
            key => f.write_str(key.code().unwrap_or("?")),
        }
    }
}

/// Имя клавиши, которого нет среди кодов W3C.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("неизвестная клавиша «{0}»")]
pub struct UnknownKey(pub String);

impl FromStr for PhysicalKey {
    type Err = UnknownKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((key, _)) = NAMED_KEYS.iter().find(|(_, name)| *name == s) {
            return Ok(*key);
        }
        s.strip_prefix("Other(")
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(|raw| raw.parse().ok())
            .map(PhysicalKey::Other)
            .ok_or_else(|| UnknownKey(s.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub caps_lock: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyDir {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: PhysicalKey,
    pub dir: KeyDir,
    pub mods: Modifiers,
    pub repeat: bool,
    pub time_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetReason {
    MouseClick,
    FocusChanged,
    SecureInput,
    External,
}
