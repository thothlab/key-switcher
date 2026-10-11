//! Слово в двух раскладках: те же нажатия в раскладке набора и в другой раскладке пары.

use crate::{KeyStroke, LayoutMap, Level, PhysicalKey};

/// Слово, прочитанное в двух раскладках.
pub(crate) struct Converted {
    pub(crate) typed: String,
    pub(crate) converted: String,
    pub(crate) strokes: Vec<KeyStroke>,
}

/// Читает нажатия слова в раскладке `from` и в раскладке `to` на тех же уровнях;
/// `None`, если у какой-то клавиши нет символа в одной из раскладок.
pub(crate) fn convert_keys(
    keys: &[(PhysicalKey, Level)],
    from: &LayoutMap,
    to: &LayoutMap,
) -> Option<Converted> {
    let mut word = Converted {
        typed: String::with_capacity(keys.len() * 2),
        converted: String::with_capacity(keys.len() * 2),
        strokes: Vec::with_capacity(keys.len() + 1),
    };
    for &(key, level) in keys {
        word.typed.push(from.char_for(key, level)?);
        word.converted.push(to.char_for(key, level)?);
        word.strokes.push(stroke(key, level));
    }
    Some(word)
}

pub(super) fn stroke(key: PhysicalKey, level: Level) -> KeyStroke {
    KeyStroke {
        key,
        shift: level.is_shift(),
    }
}
