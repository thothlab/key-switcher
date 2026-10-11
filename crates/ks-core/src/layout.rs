//! Таблицы раскладок: (физическая клавиша, уровень) ↔ символ.

use std::collections::HashMap;

use crate::{Lang, Modifiers, PhysicalKey};

/// Идентификатор раскладки ОС: `"com.apple.keylayout.RussianWin"`, HKL, `"us"`/`"ru"`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayoutId(pub String);

/// Уровень клавиши: какие из Shift и CapsLock активны.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Base,
    Shift,
    Caps,
    ShiftCaps,
}

impl Level {
    pub fn from_mods(mods: Modifiers) -> Level {
        match (mods.shift, mods.caps_lock) {
            (false, false) => Level::Base,
            (true, false) => Level::Shift,
            (false, true) => Level::Caps,
            (true, true) => Level::ShiftCaps,
        }
    }

    pub fn is_shift(self) -> bool {
        matches!(self, Level::Shift | Level::ShiftCaps)
    }
}

/// Нажатие для перепечатки: клавиша и нужен ли Shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyStroke {
    pub key: PhysicalKey,
    pub shift: bool,
}

/// Таблица одной раскладки. Строится бэкендом из API ОС (или тестом из фикстуры) через
/// [`LayoutMap::new`] и [`LayoutMap::insert`].
#[derive(Clone, Debug)]
pub struct LayoutMap {
    pub id: LayoutId,
    pub lang: Lang,
    pub name: String,
    chars: HashMap<(PhysicalKey, Level), char>,
    strokes: HashMap<char, KeyStroke>,
}

impl LayoutMap {
    pub fn new(id: LayoutId, lang: Lang, name: impl Into<String>) -> LayoutMap {
        LayoutMap {
            id,
            lang,
            name: name.into(),
            chars: HashMap::new(),
            strokes: HashMap::new(),
        }
    }

    /// Добавляет символ клавиши на уровне. Для обратного отображения учитываются уровни
    /// Base и Shift; при совпадении символа выигрывает Base, затем — первая вставка.
    pub fn insert(&mut self, key: PhysicalKey, level: Level, ch: char) {
        let ch = normalize(self.lang, ch);
        self.chars.insert((key, level), ch);
        let stroke = match level {
            Level::Base => KeyStroke { key, shift: false },
            Level::Shift => KeyStroke { key, shift: true },
            Level::Caps | Level::ShiftCaps => return,
        };
        match self.strokes.get(&ch) {
            Some(old) if !(old.shift && !stroke.shift) => {}
            _ => {
                self.strokes.insert(ch, stroke);
            }
        }
    }

    pub fn char_for(&self, k: PhysicalKey, l: Level) -> Option<char> {
        self.chars.get(&(k, l)).copied()
    }

    pub fn stroke_for(&self, c: char) -> Option<KeyStroke> {
        self.strokes.get(&c).copied()
    }
}

/// Латинская `Ë`/`ë` (U+00CB/U+00EB), которую `UCKeyTranslate` отдаёт в «Русская – ПК»
/// на Shift+`` ` ``, в русской раскладке становится кириллической `Ё`/`ё`.
fn normalize(lang: Lang, ch: char) -> char {
    match (lang, ch) {
        (Lang::Ru, '\u{00CB}') => 'Ё',
        (Lang::Ru, '\u{00EB}') => 'ё',
        _ => ch,
    }
}

/// Раскладки, включённые в системе.
#[derive(Clone, Debug, Default)]
pub struct LayoutSet(pub Vec<LayoutMap>);

impl LayoutSet {
    /// Первая английская и первая русская раскладки; `None`, если пары нет.
    pub fn pair(&self) -> Option<(&LayoutMap, &LayoutMap)> {
        let en = self.0.iter().find(|m| m.lang == Lang::En)?;
        let ru = self.0.iter().find(|m| m.lang == Lang::Ru)?;
        Some((en, ru))
    }

    pub fn get(&self, id: &LayoutId) -> Option<&LayoutMap> {
        self.0.iter().find(|m| &m.id == id)
    }
}
