//! Исполнение действий движка синтетическими событиями — общее для всех ОС: стереть,
//! переключить раскладку, перепечатать физическими клавишами; строка Unicode — запасной путь;
//! конвертация выделения через буфер обмена с восстановлением. Системные вызовы — за
//! [`Poster`], [`Switcher`] и [`Clipboard`] бэкенда; порядок и отказы проверяются тестом.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ks_core::{Action, AppId, KeyDir, LayoutId, PhysicalKey};

use crate::BackendError;
use crate::intercept::InputEvent;

/// Пауза после каждого синтетического нажатия, мс (подбирается вживую).
pub const KEY_PAUSE_MS: u64 = 2;
/// Пауза после смены раскладки перед перепечаткой, мс.
pub const SWITCH_PAUSE_MS: u64 = 15;
/// Сколько исполнитель ждёт переключения раскладки; дольше — замена печатается Unicode.
pub const SWITCH_TIMEOUT_MS: u64 = 500;
/// Сколько срока удержания должно оставаться у задания перед первым Backspace замены:
/// переключение раскладки может занять весь `SWITCH_TIMEOUT_MS`, и допечатка не должна
/// попасть под срез.
pub const REPLACE_RESERVE_MS: u64 = SWITCH_TIMEOUT_MS + 300;
/// Сколько ждать, пока копирование выделения сменит буфер обмена; не сменило — выделение пусто.
pub const COPY_TIMEOUT_MS: u64 = 300;
/// Шаг опроса буфера обмена после копирования.
pub const COPY_POLL_MS: u64 = 10;
/// После `COPY_TIMEOUT_MS` ещё столько ждём опоздавшее копирование, чтобы вернуть буфер.
pub const LATE_COPY_MS: u64 = 500;
/// Пауза после вставки перед восстановлением буфера обмена: приложение должно успеть
/// прочитать вставляемое.
pub const RESTORE_DELAY_MS: u64 = 250;

/// Сочетание буфера обмена этой ОС (Cmd+C/Cmd+V на macOS, Ctrl+C/Ctrl+V на прочих).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    Copy,
    Paste,
}

/// Отправка своих синтетических событий; реализация помечает их как «свои», и перехват
/// отдаёт их назад с `InputEvent::own`. `N` — событие ОС для воспроизведения придержанного.
pub trait Poster<N>: Send + Sync {
    /// У физической клавиши есть код этой ОС.
    fn can_type(&self, key: PhysicalKey) -> bool;
    /// Нажатие или отпускание физической клавиши, с Shift или без.
    fn key(&self, key: PhysicalKey, down: bool, shift: bool);
    fn chord(&self, chord: Chord);
    fn unicode(&self, text: &str);
    /// Воспроизводит придержанное нажатие пользователя как есть.
    fn replay(&self, native: &N);
    fn pause_ms(&self, ms: u64);
}

/// Право на эмуляцию ввода и переключение раскладки.
pub trait Switcher: Send + Sync {
    fn post_allowed(&self) -> bool;
    /// Переключает раскладку и ждёт, пока смена применится, не дольше `SWITCH_TIMEOUT_MS`;
    /// `false` — не вышло.
    fn select_layout(&self, id: &LayoutId) -> bool;
}

/// Снимок буфера обмена: элементы, у каждого — пары «тип, данные». Намеренно без `Debug`:
/// внутри то, что скопировал пользователь.
#[derive(Clone, Default)]
pub struct ClipboardSnapshot(pub Vec<Vec<(String, Vec<u8>)>>);

/// Буфер обмена ОС для конвертации выделения.
pub trait Clipboard: Send + Sync {
    /// В приложении вставка через буфер обмена работает (в Terminal и iTerm2 — нет).
    fn usable_in(&self, app: &AppId) -> bool;
    /// Счётчик изменений буфера: растёт при каждой записи.
    fn change_count(&self) -> i64;
    fn snapshot(&self) -> ClipboardSnapshot;
    fn restore(&self, snapshot: &ClipboardSnapshot);
    fn text(&self) -> Option<String>;
    fn set_text(&self, text: &str);
}

/// Срок задания исполнителя.
pub trait Lease {
    /// Задание не срезано по сроку: можно отправлять следующее событие.
    fn alive(&self) -> bool;
    /// Перед необратимым шагом: до среза должно остаться не меньше `ms` (живой исполнитель
    /// продлевает срок). `false` — задание уже срезано.
    fn reserve(&self, ms: u64) -> bool;
}

/// Срок без среза: `Executor::execute` вне очереди.
struct Unbounded;

impl Lease for Unbounded {
    fn alive(&self) -> bool {
        true
    }
    fn reserve(&self, _ms: u64) -> bool {
        true
    }
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

fn cancelled() -> Result<(), BackendError> {
    Err(BackendError::Os("задание отменено по сроку".to_owned()))
}

pub struct Executor<N> {
    poster: Arc<dyn Poster<N>>,
    switcher: Arc<dyn Switcher>,
    clipboard: Option<Arc<dyn Clipboard>>,
    stats: Arc<ExecStats>,
}

impl<N> Executor<N> {
    pub fn new(poster: Arc<dyn Poster<N>>, switcher: Arc<dyn Switcher>) -> Executor<N> {
        Executor {
            poster,
            switcher,
            clipboard: None,
            stats: Arc::default(),
        }
    }

    /// Буфер обмена для `ConvertSelection`; без него конвертация выделения не поддерживается.
    pub fn with_clipboard(mut self, clipboard: Arc<dyn Clipboard>) -> Executor<N> {
        self.clipboard = Some(clipboard);
        self
    }

    pub fn stats(&self) -> Arc<ExecStats> {
        self.stats.clone()
    }

    pub fn post_allowed(&self) -> bool {
        self.switcher.post_allowed()
    }

    /// Действие без срока; `ConvertSelection` здесь не поддерживается (нужен движок —
    /// её исполняет `Pipeline`).
    pub fn execute(&self, action: &Action) -> Result<(), BackendError> {
        self.execute_with(action, &Unbounded)
    }

    /// Как [`Executor::execute`], но со сроком: перед каждым синтетическим событием спрашивает
    /// `lease.alive()`, перед первым Backspace замены резервирует `REPLACE_RESERVE_MS`.
    pub fn execute_with(&self, action: &Action, lease: &dyn Lease) -> Result<(), BackendError> {
        match action {
            Action::Replace {
                erase,
                keys,
                text,
                target_layout,
                ..
            } => {
                // Без права события теряются молча: стёрли бы слово и не напечатали замену.
                // Поэтому проверка — до первого Backspace.
                if !self.switcher.post_allowed() {
                    return Err(BackendError::NoPermission);
                }
                if !lease.reserve(REPLACE_RESERVE_MS) {
                    return cancelled();
                }
                for _ in 0..*erase {
                    if !lease.alive() {
                        return cancelled();
                    }
                    self.tap(PhysicalKey::Backspace, false);
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
                let typable = !keys.is_empty() && keys.iter().all(|s| self.poster.can_type(s.key));
                if layout_ready && typable {
                    for stroke in keys {
                        if !lease.alive() {
                            return cancelled();
                        }
                        self.tap(stroke.key, stroke.shift);
                    }
                } else if lease.alive() {
                    self.poster.unicode(text);
                } else {
                    return cancelled();
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
                "конвертация выделения идёт через Pipeline".to_owned(),
            )),
            Action::ToggleAutocorrect => Ok(()),
        }
    }

    /// Выделение в приложении `app`: копирование → `convert` → вставка → прежний буфер обмена.
    /// Буфер не сменился после копирования — выделение пусто, ничего не трогается. Текст
    /// выделения не логируется.
    pub fn convert_selection(
        &self,
        app: &AppId,
        convert: &dyn Fn(&str) -> Option<String>,
        lease: &dyn Lease,
    ) -> Result<(), BackendError> {
        if !self.switcher.post_allowed() {
            return Err(BackendError::NoPermission);
        }
        let Some(clip) = &self.clipboard else {
            return Err(BackendError::Unsupported(
                "конвертация выделения без буфера обмена".to_owned(),
            ));
        };
        if !clip.usable_in(app) {
            log::info!(
                "выделение в {} не конвертируется: вставка через буфер обмена там не работает",
                app.0
            );
            return Ok(());
        }
        if !lease.reserve(COPY_TIMEOUT_MS + RESTORE_DELAY_MS + REPLACE_RESERVE_MS) || !lease.alive()
        {
            return cancelled();
        }
        let saved = clip.snapshot();
        let before = clip.change_count();
        self.poster.chord(Chord::Copy);
        let mut waited = 0;
        while clip.change_count() == before {
            if waited >= COPY_TIMEOUT_MS {
                // Медленное приложение могло выполнить Cmd+C позже срока: тогда выделение
                // заменило бы буфер пользователя навсегда. Ждём ещё немного и, если копия
                // всё же пришла, возвращаем прежнее содержимое — замена не делается.
                if lease.reserve(LATE_COPY_MS) {
                    let mut late = 0;
                    while late < LATE_COPY_MS && lease.alive() {
                        self.poster.pause_ms(COPY_POLL_MS);
                        late += COPY_POLL_MS;
                        if clip.change_count() != before {
                            clip.restore(&saved);
                            log::info!("копирование опоздало: буфер обмена восстановлен");
                            return Ok(());
                        }
                    }
                }
                log::info!("выделение пусто: конвертировать нечего");
                return Ok(());
            }
            self.poster.pause_ms(COPY_POLL_MS);
            waited += COPY_POLL_MS;
        }
        // Буфер уже наш: дальше любой исход заканчивается восстановлением.
        let result = match clip.text().as_deref().and_then(convert) {
            None => Ok(()),
            Some(_) if !lease.alive() => cancelled(),
            Some(converted) => {
                clip.set_text(&converted);
                self.poster.chord(Chord::Paste);
                self.poster.pause_ms(RESTORE_DELAY_MS);
                Ok(())
            }
        };
        clip.restore(&saved);
        result
    }

    /// Воспроизводит нажатие пользователя, придержанное на время замены.
    pub fn replay(&self, ev: &InputEvent<N>) {
        self.poster.replay(&ev.native);
        if ev.key.is_some_and(|k| k.dir == KeyDir::Up) {
            self.poster.pause_ms(KEY_PAUSE_MS);
        }
    }

    fn tap(&self, key: PhysicalKey, shift: bool) {
        self.poster.key(key, true, shift);
        self.poster.key(key, false, shift);
        self.poster.pause_ms(KEY_PAUSE_MS);
    }
}
