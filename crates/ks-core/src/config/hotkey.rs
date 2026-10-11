//! Горячие клавиши: один парсер текстового представления для настроек и окна настроек.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::HotkeyConfig;
use crate::{Modifiers, PhysicalKey};

/// `"DoubleShift"`, `"BothShifts"`, `"Alt+Shift"`, `"Shift+Pause"`, `"Ctrl+Alt+KeyK"`.
/// `Combo` без `key` — только модификаторы (⌥⇧).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Hotkey {
    DoubleShift,
    BothShifts,
    Combo {
        mods: Modifiers,
        key: Option<PhysicalKey>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
/// Текст ошибки уходит в меню, `--status` и журнал, поэтому значения в нём нет: само
/// сочетание — в полезной нагрузке, окно настроек показывает его рядом с объяснением.
pub enum HotkeyError {
    #[error("не удалось разобрать сочетание")]
    Parse(String),
    #[error("одно сочетание назначено дважды")]
    Duplicate(Hotkey),
    #[error("сочетание — одиночная буква или цифра без модификатора")]
    BareKey(Hotkey),
}

impl TryFrom<String> for Hotkey {
    type Error = HotkeyError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Hotkey> for String {
    fn from(hotkey: Hotkey) -> String {
        hotkey.to_string()
    }
}

impl FromStr for Hotkey {
    type Err = HotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || HotkeyError::Parse(s.to_owned());
        match s.trim() {
            "DoubleShift" => return Ok(Hotkey::DoubleShift),
            "BothShifts" => return Ok(Hotkey::BothShifts),
            _ => {}
        }
        let mut mods = Modifiers::default();
        let mut key = None;
        for part in s.split('+').map(str::trim) {
            match part {
                "Ctrl" | "Control" => mods.ctrl = true,
                "Alt" | "Option" => mods.alt = true,
                "Shift" => mods.shift = true,
                "Meta" | "Cmd" | "Win" | "Super" => mods.meta = true,
                name => {
                    if key.is_some() {
                        return Err(invalid());
                    }
                    key = Some(name.parse::<PhysicalKey>().map_err(|_| invalid())?);
                }
            }
        }
        if key.is_none() && mods == Modifiers::default() {
            return Err(invalid());
        }
        Ok(Hotkey::Combo { mods, key })
    }
}

/// Печать в каноническом порядке: Ctrl, Alt, Shift, Meta, клавиша.
impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (mods, key) = match self {
            Hotkey::DoubleShift => return f.write_str("DoubleShift"),
            Hotkey::BothShifts => return f.write_str("BothShifts"),
            Hotkey::Combo { mods, key } => (mods, key),
        };
        let names = [
            (mods.ctrl, "Ctrl"),
            (mods.alt, "Alt"),
            (mods.shift, "Shift"),
            (mods.meta, "Meta"),
        ];
        let mut sep = "";
        for (on, name) in names {
            if on {
                write!(f, "{sep}{name}")?;
                sep = "+";
            }
        }
        if let Some(key) = key {
            write!(f, "{sep}{key}")?;
        }
        Ok(())
    }
}

/// Дубли между всеми действиями и одиночная буква или цифра без модификатора — ошибка.
pub fn validate_hotkeys(h: &HotkeyConfig) -> Result<(), HotkeyError> {
    let mut seen = HashSet::new();
    let all = h
        .undo_or_convert_word
        .iter()
        .chain(&h.convert_selection)
        .chain(&h.toggle_autocorrect);
    for hotkey in all {
        if let Hotkey::Combo {
            mods,
            key: Some(key),
        } = hotkey
        {
            let bare = !(mods.ctrl || mods.alt || mods.shift || mods.meta);
            if bare && key.is_letter_or_digit() {
                return Err(HotkeyError::BareKey(*hotkey));
            }
        }
        if !seen.insert(*hotkey) {
            return Err(HotkeyError::Duplicate(*hotkey));
        }
    }
    Ok(())
}
