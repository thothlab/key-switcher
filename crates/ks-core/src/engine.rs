//! Диспетчер нажатий: ручное управление (`input`) → буфер токена → решение на границе (`detect`).

use std::sync::Arc;

use crate::buffer::TokenBuffer;
use crate::input::{InputState, View};
use crate::{
    Action, Config, Context, Disposition, EngineStatus, KeyDir, KeyEvent, KeyStroke, Lang,
    LayoutId, LayoutMap, LayoutSet, Level, Lexicon, Modifiers, Outcome, PhysicalKey, ReplaceKind,
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

/// Слово, прочитанное в двух раскладках.
pub(crate) struct Converted {
    pub(crate) typed: String,
    pub(crate) converted: String,
    pub(crate) strokes: Vec<KeyStroke>,
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
    /// Действия ручного управления (жесты, хоткеи) стоят в `actions` первыми; при
    /// `Disposition::Pass` бэкенд исполняет их до того, как пропустить само нажатие.
    pub fn on_key(&mut self, ev: &KeyEvent, ctx: &Context) -> Outcome {
        self.buffer.note_app(&ctx.app);
        let view = View {
            config: &self.config,
            layouts: &self.layouts,
            buffer: &self.buffer,
        };
        let step = self.input.on_key(ev, ctx, &view);
        if let Some(layout) = step.relayout {
            self.buffer.relayout(layout);
        }
        if let Some(disposition) = step.consumed {
            return Outcome {
                disposition,
                actions: step.actions,
            };
        }
        let layout = step.switched_to.as_ref().unwrap_or(&ctx.layout);
        let mut outcome = self.typing(ev, ctx, layout);
        if !step.actions.is_empty() {
            let mut actions = step.actions;
            actions.append(&mut outcome.actions);
            outcome.actions = actions;
        }
        outcome
    }

    /// Таймер ручного управления: одиночный Shift ждёт `T_double` = 400 мс, не первая ли это
    /// половина двойного. Бэкенд зовёт `tick` к моменту `next_deadline()` — в тех же часах,
    /// что `KeyEvent.time_ms`, — и исполняет вернувшиеся действия. Не позвал — ожидание
    /// разрешит следующее событие, а переключение уйдёт в его `Outcome`.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Action> {
        self.input.tick(now_ms, &self.config)
    }

    /// Когда звать `tick`; `None` — ждать нечего.
    pub fn next_deadline(&self) -> Option<u64> {
        self.input.deadline()
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
        self.input.forget_word();
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

    /// Набранная форма последнего отменённого исправления — для пункта «Не исправлять „…“».
    pub fn last_undone(&self) -> Option<&str> {
        self.input.last_undone()
    }

    /// Конвертация выделенного текста по таблицам пары: латинских букв больше, чем
    /// кириллических, — в русскую раскладку, иначе в английскую; регистр сохраняется.
    /// `None` — конвертировать нечего (пусто, нет букв, нет пары раскладок).
    pub fn convert_text(&self, text: &str) -> Option<String> {
        let (en, ru) = self.layouts.pair()?;
        let letters = |map: &LayoutMap| {
            text.chars()
                .filter(|c| c.is_alphabetic() && map.stroke_for(*c).is_some())
                .count()
        };
        let (latin, cyrillic) = (letters(en), letters(ru));
        if latin + cyrillic == 0 {
            return None;
        }
        let (from, to) = if latin > cyrillic { (en, ru) } else { (ru, en) };
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

    /// Обычный набор: буфер слова и решение на границе. `layout` — раскладка, в которой
    /// печатается нажатие (уже переключённая, если перед ним сработал одиночный Shift).
    fn typing(&mut self, ev: &KeyEvent, ctx: &Context, layout: &LayoutId) -> Outcome {
        if ev.dir == KeyDir::Up || ev.key.is_modifier() {
            return Outcome::pass();
        }
        if ctx.secure_input || ctx.password_field || ev.mods.ctrl || ev.mods.meta || ev.mods.alt {
            self.buffer.clear();
            return Outcome::pass();
        }
        match ev.key {
            PhysicalKey::Space | PhysicalKey::Enter => {
                let outcome = self.decide(ev.key, ev.mods);
                self.input
                    .word_done(&self.buffer, ev, &ctx.app, &outcome.actions);
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
                    .get(layout)
                    .and_then(|map| map.char_for(key, level))
                    .is_some();
                if known {
                    self.buffer.push(key, level, layout);
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
        if self.config.words.contains(&word.typed) {
            return Outcome::pass();
        }
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
        word.strokes.push(KeyStroke {
            key,
            shift: level.is_shift(),
        });
    }
    Some(word)
}
