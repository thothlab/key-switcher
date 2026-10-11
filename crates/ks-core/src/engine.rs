//! Диспетчер нажатий: жесты (`input`) → буфер токена → решение на границе (`detect`).

use std::sync::Arc;

use crate::buffer::TokenBuffer;
use crate::input::InputState;
use crate::{
    Action, Config, Context, Disposition, EngineStatus, KeyDir, KeyEvent, KeyStroke, Lang,
    LayoutMap, LayoutSet, Level, Lexicon, Modifiers, Outcome, PhysicalKey, ReplaceKind,
    ResetReason, detect,
};

/// Движок: нажатия и контекст на вход, действия на выход. Как действия исполняются, не знает.
pub struct Engine {
    config: Config,
    lexicon: Arc<Lexicon>,
    layouts: LayoutSet,
    buffer: TokenBuffer,
    input: InputState,
}

/// Слово буфера, прочитанное в двух раскладках.
struct Converted {
    typed: String,
    converted: String,
    strokes: Vec<KeyStroke>,
}

impl Engine {
    pub fn new(config: Config, lexicon: Arc<Lexicon>, layouts: LayoutSet) -> Engine {
        Engine {
            config,
            lexicon,
            layouts,
            buffer: TokenBuffer::default(),
            input: InputState::default(),
        }
    }

    /// Граница слова (пробел, Enter) при исправлении проглатывается и входит в `Replace`.
    pub fn on_key(&mut self, ev: &KeyEvent, ctx: &Context) -> Outcome {
        if let Some(outcome) = self.input.on_key(ev, ctx, &self.config) {
            return outcome;
        }
        if ev.dir == KeyDir::Up || ev.key.is_modifier() {
            return Outcome::pass();
        }
        self.buffer.note_app(&ctx.app);
        if ctx.secure_input || ctx.password_field || ev.mods.ctrl || ev.mods.meta || ev.mods.alt {
            self.buffer.clear();
            return Outcome::pass();
        }
        match ev.key {
            PhysicalKey::Space | PhysicalKey::Enter => {
                let outcome = self.decide(ev.key, ev.mods);
                self.buffer.clear();
                outcome
            }
            PhysicalKey::Backspace => {
                self.buffer.erase_back();
                Outcome::pass()
            }
            PhysicalKey::Delete => {
                self.buffer.mark_edited();
                Outcome::pass()
            }
            key if key.is_printable() => {
                let level = Level::from_mods(ev.mods);
                let known = self
                    .layouts
                    .get(&ctx.layout)
                    .and_then(|map| map.char_for(key, level))
                    .is_some();
                if known {
                    self.buffer.push(key, level, &ctx.layout);
                } else {
                    self.buffer.clear();
                }
                Outcome::pass()
            }
            _ => {
                self.buffer.clear();
                Outcome::pass()
            }
        }
    }

    pub fn on_reset(&mut self, why: ResetReason) {
        self.buffer.clear();
        self.input.on_reset(why);
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
        self.buffer.clear();
    }

    pub fn set_layouts(&mut self, layouts: LayoutSet) {
        self.layouts = layouts;
        self.buffer.clear();
    }

    pub fn status(&self) -> EngineStatus {
        if self.layouts.pair().is_none() {
            EngineStatus::NeedsLayouts
        } else if !self.config.autocorrect {
            EngineStatus::AutocorrectOff
        } else {
            EngineStatus::Working
        }
    }

    /// Набранная форма последнего отменённого исправления. TODO(task 04): запись исправления и отмена.
    pub fn last_undone(&self) -> Option<&str> {
        None
    }

    /// Конвертация выделенного текста по таблицам пары: направление — по раскладке,
    /// в которой букв текста больше. `None` — конвертировать нечего.
    pub fn convert_text(&self, text: &str) -> Option<String> {
        let (en, ru) = self.layouts.pair()?;
        let letters = |map: &LayoutMap| {
            text.chars()
                .filter(|c| c.is_alphabetic() && map.stroke_for(*c).is_some())
                .count()
        };
        let (from, to) = if letters(ru) > letters(en) {
            (ru, en)
        } else {
            (en, ru)
        };
        let converted: String = text
            .chars()
            .map(|c| {
                from.stroke_for(c)
                    .and_then(|s| {
                        let level = if s.shift { Level::Shift } else { Level::Base };
                        to.char_for(s.key, level)
                    })
                    .unwrap_or(c)
            })
            .collect();
        (converted != text).then_some(converted)
    }

    fn decide(&self, boundary: PhysicalKey, mods: Modifiers) -> Outcome {
        if !self.config.autocorrect || self.buffer.is_empty() || self.buffer.is_edited() {
            return Outcome::pass();
        }
        let Some((en, ru)) = self.layouts.pair() else {
            return Outcome::pass();
        };
        let Some(from) = self.buffer.layout().and_then(|id| self.layouts.get(id)) else {
            return Outcome::pass();
        };
        let to = match from.lang {
            Lang::En => ru,
            Lang::Ru => en,
            Lang::Other => return Outcome::pass(),
        };
        let Some(word) = convert_keys(self.buffer.keys(), from, to) else {
            return Outcome::pass();
        };
        if !detect::should_convert(
            &word.typed,
            from.lang,
            &word.converted,
            to.lang,
            &self.lexicon,
        ) {
            return Outcome::pass();
        }
        let erase = word.strokes.len();
        let mut keys = word.strokes;
        keys.push(KeyStroke {
            key: boundary,
            shift: mods.shift,
        });
        let mut text = word.converted;
        text.push(if boundary == PhysicalKey::Enter {
            '\n'
        } else {
            ' '
        });
        Outcome {
            disposition: Disposition::Swallow,
            actions: vec![Action::Replace {
                erase,
                keys,
                text,
                target_layout: Some(to.id.clone()),
                kind: ReplaceKind::Auto,
            }],
        }
    }
}

/// Читает нажатия слова в раскладке `from` и в раскладке `to` на тех же уровнях;
/// `None`, если у какой-то клавиши нет символа в одной из раскладок.
fn convert_keys(
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
        word.strokes.push(KeyStroke {
            key,
            shift: level.is_shift(),
        });
    }
    Some(word)
}
