//! Поправки модели исполнения для чтения без захвата (evdev без `EVIOCGRAB`): нажатие
//! пользователя уже дошло до приложения, когда движок его увидел, поэтому `Swallow` ничего не
//! глотает, а придержанное не воспроизводится (`Poster::replay` — пустой). Обёртка над
//! `KeyHandler` правит `Outcome` движка до модели исполнения:
//!
//! - граница слова (пробел, знак) уже напечатана → `erase` на 1 больше;
//! - граница Enter → исправление снимается: строка могла уйти (тот же довод, что у отмены
//!   после Enter в ядре), а Backspace и повторный `\n` отправили бы её ещё раз;
//! - `Pass` + `SwitchLayout` с буквой → буква уже напечатана в прежней раскладке: стереть её и
//!   напечатать в новой;
//! - Ctrl+Z в окне отмены дошёл до приложения, и оно отменило своё → ожидание отмены движка
//!   снимается сбросом, чтобы не отменить дважды;
//! - `ConvertSelection` → чтение выделения (PRIMARY на X11) и ввод результата поверх него.

use std::sync::{Arc, RwLock};

use ks_core::{
    Action, Context, Disposition, KeyDir, KeyEvent, KeyStroke, LayoutSet, Level, Outcome,
    PhysicalKey, ReplaceKind, ResetReason,
};
use ks_platform::KeyHandler;

use crate::guard::InputWatch;
use crate::typing;

/// Читает выделенный текст; `None` — выделения нет или его не прочитать.
pub type SelectionReader = Box<dyn Fn() -> Option<String> + Send + Sync>;

pub struct Adjusted {
    pub outcome: Outcome,
    /// Нажатие дошло до приложения и сломало ожидание движка: сбросить слово в работе.
    pub reset: bool,
}

fn prints(key: PhysicalKey) -> bool {
    key.is_printable() || key == PhysicalKey::Space
}

fn is_replace(a: &Action) -> bool {
    matches!(a, Action::Replace { .. })
}

/// Поправка решения движка по событию, которое уже дошло до приложения.
pub fn adjust(mut out: Outcome, ev: &KeyEvent, layouts: &LayoutSet) -> Adjusted {
    let mut reset = false;
    if ev.dir != KeyDir::Down {
        return Adjusted {
            outcome: out,
            reset,
        };
    }
    let m = ev.mods;
    let chord = m.ctrl || m.alt || m.meta;
    let has_replace = out.actions.iter().any(is_replace);
    let swallow = out.disposition == Disposition::Swallow;
    match ev.key {
        PhysicalKey::Enter | PhysicalKey::Tab if has_replace => {
            out.actions.retain(|a| !is_replace(a));
            log::info!("исправление на границе строки не исполнено: без захвата она уже ушла");
        }
        PhysicalKey::KeyZ
            if m.ctrl && !m.alt && !m.meta && !m.shift && swallow && out.actions.is_empty() =>
        {
            reset = true;
        }
        key if prints(key) && !chord => {
            let level = Level::from_mods(m);
            let stroke = KeyStroke {
                key,
                shift: m.shift,
            };
            if has_replace {
                let mut keep = true;
                if let Some(Action::Replace {
                    erase,
                    keys,
                    text,
                    target_layout,
                    ..
                }) = out.actions.iter_mut().find(|a| is_replace(a))
                {
                    if swallow {
                        *erase += 1;
                    } else {
                        // Нажатие напечатано после слова в прежней раскладке: перепечатать его
                        // в раскладке цели следом за заменой.
                        match target_layout
                            .as_ref()
                            .and_then(|id| layouts.get(id))
                            .and_then(|map| map.char_for(key, level))
                        {
                            Some(c) => {
                                *erase += 1;
                                keys.push(stroke);
                                text.push(c);
                            }
                            None => keep = false,
                        }
                    }
                }
                if !keep {
                    out.actions.retain(|a| !is_replace(a));
                }
            } else if !swallow
                && let Some(slot) = out
                    .actions
                    .iter_mut()
                    .find(|a| matches!(a, Action::SwitchLayout(_)))
                && let Action::SwitchLayout(id) = slot
                && let Some(c) = layouts.get(id).and_then(|map| map.char_for(key, level))
            {
                *slot = Action::Replace {
                    erase: 1,
                    keys: vec![stroke],
                    text: c.to_string(),
                    target_layout: Some(id.clone()),
                    kind: ReplaceKind::Manual,
                };
            }
        }
        _ => {}
    }
    Adjusted {
        outcome: out,
        reset,
    }
}

/// `KeyHandler` для модели исполнения без захвата.
pub struct NoGrab {
    inner: Arc<dyn KeyHandler>,
    layouts: Arc<RwLock<LayoutSet>>,
    selection: Option<SelectionReader>,
    watch: Arc<InputWatch>,
}

impl NoGrab {
    /// `selection` — чтение выделения; `None` — в окружении его не прочитать (Wayland без
    /// data-control), конвертация выделения не выполняется.
    pub fn new(
        inner: Arc<dyn KeyHandler>,
        layouts: Arc<RwLock<LayoutSet>>,
        selection: Option<SelectionReader>,
        watch: Arc<InputWatch>,
    ) -> NoGrab {
        NoGrab {
            inner,
            layouts,
            selection,
            watch,
        }
    }

    /// Выделение → действие ввода конвертированного текста; содержимое не логируется.
    pub fn convert_selection(&self) -> Option<Action> {
        let Some(read) = &self.selection else {
            log::info!(
                "конвертация выделения в этом окружении недоступна (Wayland: нет доступа к \
                 выделению)"
            );
            return None;
        };
        let Some(text) = read() else {
            log::info!("выделение пусто: конвертировать нечего");
            return None;
        };
        let converted = self.inner.convert_text(&text)?;
        let layouts = self.layouts.read().unwrap_or_else(|e| e.into_inner());
        let action = typing::selection_action(&converted, &layouts);
        if action.is_none() {
            log::info!("выделение не набирается клавишами раскладок: не конвертировано");
        }
        action
    }
}

impl KeyHandler for NoGrab {
    fn on_key(&self, ev: &KeyEvent, ctx: &Context) -> Outcome {
        let out = self.inner.on_key(ev, ctx);
        let adjusted = {
            let layouts = self.layouts.read().unwrap_or_else(|e| e.into_inner());
            adjust(out, ev, &layouts)
        };
        if adjusted.reset {
            self.inner.on_reset(ResetReason::External);
        }
        let mut outcome = adjusted.outcome;
        if outcome
            .actions
            .iter()
            .any(|a| matches!(a, Action::ConvertSelection))
        {
            let actions = std::mem::take(&mut outcome.actions);
            outcome.actions = actions
                .into_iter()
                .filter_map(|a| match a {
                    Action::ConvertSelection => self.convert_selection(),
                    other => Some(other),
                })
                .collect();
        }
        if outcome.actions.iter().any(is_replace) {
            // Замена решена по этому нажатию: всё, что нажато позже, её отменит (guard).
            self.watch.arm(ev.time_ms);
        }
        outcome
    }

    fn on_reset(&self, why: ResetReason) {
        self.inner.on_reset(why);
    }

    fn convert_text(&self, text: &str) -> Option<String> {
        self.inner.convert_text(text)
    }

    fn tick(&self, now_ms: u64) -> Vec<Action> {
        self.inner.tick(now_ms)
    }

    fn next_deadline(&self) -> Option<u64> {
        self.inner.next_deadline()
    }
}
