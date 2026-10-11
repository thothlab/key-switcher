//! Исполнение действий движка синтетическими событиями: стереть, переключить раскладку,
//! перепечатать физическими кодами; строка Unicode — запасной путь. Системные вызовы спрятаны
//! за [`Poster`] и [`Switcher`], чтобы порядок и отказ без прав проверялись тестом.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ks_core::{Action, LayoutId};
use ks_platform::BackendError;

use crate::keycode::{self, BACKSPACE, FLAG_SHIFT};
use crate::tap::{RawEvent, RawKind};

/// Пауза после каждого синтетического нажатия, мс (подбирается вживую).
pub const KEY_PAUSE_MS: u64 = 2;
/// Пауза после смены раскладки перед перепечаткой, мс.
pub const SWITCH_PAUSE_MS: u64 = 15;

/// Отправка своих синтетических событий; реализация помечает их как «свои».
pub trait Poster: Send + Sync {
    fn key(&self, keycode: u16, down: bool, flags: u64);
    fn unicode(&self, text: &str);
    fn pause_ms(&self, ms: u64);
}

/// Право на эмуляцию ввода и переключение раскладки.
pub trait Switcher: Send + Sync {
    fn post_allowed(&self) -> bool;
    /// Переключает раскладку и ждёт, пока смена применится; `false` — не вышло.
    fn select_layout(&self, id: &LayoutId) -> bool;
}

/// Сбои исполнителя, видимые в `health()` и `--status`.
#[derive(Default)]
pub struct ExecStats {
    layout_switch_failed: AtomicU64,
    last_failed_layout: Mutex<Option<LayoutId>>,
}

impl ExecStats {
    fn switch_failed(&self, id: &LayoutId) {
        self.layout_switch_failed.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut last) = self.last_failed_layout.lock() {
            *last = Some(id.clone());
        }
        log::warn!(
            "раскладка {} не переключилась: замена напечатана строкой Unicode",
            id.0
        );
    }

    pub fn layout_switch_failed(&self) -> u64 {
        self.layout_switch_failed.load(Ordering::Relaxed)
    }

    /// Строка для диагностики; `None`, если сбоев не было.
    pub fn summary(&self) -> Option<String> {
        let n = self.layout_switch_failed();
        let last = self.last_failed_layout.lock().ok()?.clone()?;
        Some(format!(
            "раскладка не переключилась при замене: {n} раз (последняя: {})",
            last.0
        ))
    }
}

pub struct Executor {
    poster: Arc<dyn Poster>,
    switcher: Arc<dyn Switcher>,
    stats: Arc<ExecStats>,
}

impl Executor {
    pub fn new(poster: Arc<dyn Poster>, switcher: Arc<dyn Switcher>) -> Executor {
        Executor {
            poster,
            switcher,
            stats: Arc::default(),
        }
    }

    pub fn stats(&self) -> Arc<ExecStats> {
        self.stats.clone()
    }

    pub fn post_allowed(&self) -> bool {
        self.switcher.post_allowed()
    }

    pub fn execute(&self, action: &Action) -> Result<(), BackendError> {
        self.execute_while(action, &|| true)
    }

    /// Как [`Executor::execute`], но перед каждым синтетическим событием спрашивает `alive`:
    /// задание, отменённое по сроку, больше ничего не отправляет.
    pub fn execute_while(
        &self,
        action: &Action,
        alive: &dyn Fn() -> bool,
    ) -> Result<(), BackendError> {
        let cancelled = || Err(BackendError::Os("задание отменено по сроку".to_owned()));
        match action {
            Action::Replace {
                erase,
                keys,
                text,
                target_layout,
                ..
            } => {
                // Без права CGEventPost молча теряет события: стёрли бы слово и не напечатали
                // замену. Поэтому проверка — до первого Backspace.
                if !self.switcher.post_allowed() {
                    return Err(BackendError::NoPermission);
                }
                for _ in 0..*erase {
                    if !alive() {
                        return cancelled();
                    }
                    self.tap(BACKSPACE, 0);
                }
                let layout_ready = match target_layout {
                    None => true,
                    Some(id) => {
                        let ok = self.switcher.select_layout(id);
                        if !ok {
                            self.stats.switch_failed(id);
                        }
                        self.poster.pause_ms(SWITCH_PAUSE_MS);
                        ok
                    }
                };
                let codes: Option<Vec<(u16, bool)>> = keys
                    .iter()
                    .map(|s| keycode::from_physical(s.key).map(|c| (c, s.shift)))
                    .collect();
                match codes {
                    Some(codes) if layout_ready && !codes.is_empty() => {
                        for (code, shift) in codes {
                            if !alive() {
                                return cancelled();
                            }
                            self.tap(code, if shift { FLAG_SHIFT } else { 0 });
                        }
                    }
                    _ if !alive() => return cancelled(),
                    _ => self.poster.unicode(text),
                }
                Ok(())
            }
            Action::SwitchLayout(id) => {
                if self.switcher.select_layout(id) {
                    Ok(())
                } else {
                    self.stats.switch_failed(id);
                    Err(BackendError::Os("раскладка не переключилась".to_owned()))
                }
            }
            Action::ConvertSelection => Err(BackendError::Unsupported(
                "конвертация выделения ещё не реализована".to_owned(),
            )),
            Action::ToggleAutocorrect => Ok(()),
        }
    }

    /// Воспроизводит нажатие пользователя, придержанное на время замены.
    pub fn replay(&self, ev: &RawEvent) {
        self.poster
            .key(ev.keycode, ev.kind == RawKind::KeyDown, ev.flags);
        if ev.kind == RawKind::KeyUp {
            self.poster.pause_ms(KEY_PAUSE_MS);
        }
    }

    fn tap(&self, code: u16, flags: u64) {
        self.poster.key(code, true, flags);
        self.poster.key(code, false, flags);
        self.poster.pause_ms(KEY_PAUSE_MS);
    }
}
