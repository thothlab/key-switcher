//! Диспетчер нажатий: ручное управление (`input`) → буфер токена → решение на границе (`detect`).

use std::sync::Arc;

use crate::buffer::TokenBuffer;
use crate::detect::{self, Decision, Segment};
use crate::input::{InputState, View};
use crate::{
    Action, Config, Context, Disposition, EngineStatus, KeyDir, KeyEvent, KeyStroke, Lang,
    LayoutId, LayoutMap, LayoutSet, Level, Lexicon, Modifiers, Os, Outcome, PhysicalKey,
    ReplaceKind, ResetReason,
};

/// Движок: нажатия и контекст на вход, действия на выход. Как действия исполняются, не знает.
pub struct Engine {
    config: Config,
    lexicon: Arc<Lexicon>,
    layouts: LayoutSet,
    buffer: TokenBuffer,
    input: InputState,
    /// Язык последних решённых слов: переживает границу, умирает при сбросе буфера.
    segment: Segment,
    /// Раскладка, в которой печаталось прошлое событие; `None` — после сброса контекста.
    layout: Option<LayoutId>,
    /// ОС, чьё стандартное «отменить» (⌘Z или Ctrl+Z) отменяет автоисправление.
    os: Os,
}

impl Engine {
    pub fn new(config: Config, lexicon: Arc<Lexicon>, layouts: LayoutSet) -> Engine {
        Engine {
            config,
            lexicon,
            layouts,
            buffer: TokenBuffer::default(),
            input: InputState::default(),
            segment: Segment::default(),
            layout: None,
            os: Os::current(),
        }
    }

    /// Чьё стандартное «отменить» слушать: macOS — ⌘Z, Windows и Linux — Ctrl+Z.
    /// По умолчанию — ОС сборки (`Os::current()`); другая нужна тестам и разбору чужой ОС.
    pub fn with_os(mut self, os: Os) -> Engine {
        self.os = os;
        self
    }

    /// Граница слова (пробел, Enter) при исправлении проглатывается и входит в `Replace`.
    /// Действия ручного управления (жесты, хоткеи) стоят в `actions` первыми; при
    /// `Disposition::Pass` бэкенд исполняет их до того, как пропустить само нажатие.
    pub fn on_key(&mut self, ev: &KeyEvent, ctx: &Context) -> Outcome {
        if self.buffer.note_app(&ctx.app) {
            self.segment.reset();
            self.layout = None;
        }
        let view = View {
            config: &self.config,
            layouts: &self.layouts,
            buffer: &self.buffer,
            os: self.os,
        };
        let step = self.input.on_key(ev, ctx, &view);
        self.note_layout(&ctx.layout);
        if let Some(layout) = step.relayout {
            self.buffer.relayout(layout);
        }
        // отмена и ручная конвертация говорят, на каком языке слово на самом деле
        for action in &step.actions {
            if let Action::Replace {
                kind: ReplaceKind::Undo | ReplaceKind::Manual,
                target_layout: Some(id),
                ..
            } = action
                && let Some(map) = self.layouts.get(id)
            {
                self.segment.set(map.lang);
            }
        }
        if let Some(disposition) = step.consumed {
            if ev.dir == KeyDir::Down && !ev.key.is_modifier() {
                // хоткей посреди слова (⌃⌥K, Shift+Pause) прерывает его, как любое сочетание
                // с Cmd/Ctrl/Alt: остаток не должен решаться вместе с началом
                self.buffer.clear();
            }
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
        self.forget();
        self.input.on_reset(why);
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
        self.forget();
    }

    pub fn set_layouts(&mut self, layouts: LayoutSet) {
        self.layouts = layouts;
        self.forget();
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
            self.forget();
            return Outcome::pass();
        }
        match ev.key {
            PhysicalKey::Space | PhysicalKey::Enter => {
                let outcome = self.decide(ev.key, ev.mods, ctx);
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
                    self.buffer.interrupt();
                }
                Outcome::pass()
            }
            // клавиша без символа: каретка на месте, остаток слова — неполный
            PhysicalKey::Other(_) | PhysicalKey::Pause => {
                self.buffer.interrupt();
                Outcome::pass()
            }
            // Tab, Escape, стрелки, Home/End, PageUp/PageDown: слово и контекст забыты
            _ => {
                self.forget();
                Outcome::pass()
            }
        }
    }

    /// Сброс без решения: слово и контекст сегмента забыты.
    fn forget(&mut self) {
        self.buffer.clear();
        self.segment.reset();
        self.layout = None;
    }

    /// Раскладку сменили руками — одиночным Shift, сочетанием ОС, из меню (видно по
    /// `Context.layout`): контекст сегмента переходит на язык новой раскладки, чтобы
    /// «Slack», ⇧, «мы» не стало «vs». Свои переключения (исправление, отмена, ручная
    /// конвертация) ведут туда же, куда сегмент уже поставлен решением или отменой.
    fn note_layout(&mut self, layout: &LayoutId) {
        if self.layout.as_ref() == Some(layout) {
            return;
        }
        if self.layout.replace(layout.clone()).is_none() {
            // первое событие после сброса: сравнивать не с чем, контекст уже пуст
            return;
        }
        match self.layouts.get(layout).map(|map| map.lang) {
            Some(lang @ (Lang::En | Lang::Ru)) => self.segment.set(lang),
            _ => self.segment.reset(),
        }
    }

    /// Решение на границе слова (`detect`); исправление — `Replace` с проглоченной границей.
    fn decide(&mut self, boundary: PhysicalKey, mods: Modifiers, ctx: &Context) -> Outcome {
        if !self.config.autocorrect || self.buffer.is_empty() {
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
        // отменённое в этом сеансе — как слово из `words.never`: не трогать, сегмент не менять
        if self.input.was_undone(self.buffer.keys(), from) {
            return Outcome::pass();
        }
        let decision = detect::decide(&detect::Input {
            keys: self.buffer.keys(),
            from,
            to,
            ctx,
            config: &self.config,
            lexicon: &self.lexicon,
            segment: self.segment.lang(),
            edited: self.buffer.is_edited(),
        });
        self.segment.note(&decision);
        let Decision::Fix(fix) = decision else {
            return Outcome::pass();
        };
        let mut keys = fix.strokes;
        keys.push(KeyStroke {
            key: boundary,
            shift: mods.shift,
        });
        let mut text = fix.text;
        text.push(if boundary == PhysicalKey::Enter {
            '\n'
        } else {
            ' '
        });
        Outcome {
            disposition: Disposition::Swallow,
            actions: vec![Action::Replace {
                erase: self.buffer.keys().len(),
                keys,
                text,
                target_layout: fix.target,
                kind: fix.kind,
            }],
        }
    }
}
