//! Контракт ОС-слоя KeySwitcher: перехват и эмуляция ввода, раскладки, права, трей.
//! Зависит только от `ks-core`; реализации — `ks-macos`, `ks-windows`, `ks-linux`.
//! Общая для всех ОС модель исполнения (перехват → движок → исполнитель с удержанием
//! нажатий) — [`intercept`] и [`exec`]; бэкенд даёт только перевод событий ОС и `Poster`.

pub mod exec;
pub mod intercept;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ks_core::{Action, AppId, Context, KeyEvent, LayoutId, LayoutSet, Outcome, ResetReason};

/// Обёртка над `Engine` под мьютексом; бэкенд зовёт её из своего потока перехвата.
pub trait KeyHandler: Send + Sync {
    fn on_key(&self, ev: &KeyEvent, ctx: &Context) -> Outcome;
    fn on_reset(&self, why: ResetReason);
    fn convert_text(&self, text: &str) -> Option<String>;
    /// Таймер ручного управления (одиночный Shift ждёт `T_double`): действия, созревшие к
    /// `now_ms` по часам [`clock_ms`]. Исполняет их бэкенд (`Backend::execute`).
    fn tick(&self, now_ms: u64) -> Vec<Action>;
    /// Момент, **строго после** которого `tick` действует (звать на `deadline + 1`);
    /// `None` — ждать нечего. Перечитывать после каждого `on_key`/`tick`.
    fn next_deadline(&self) -> Option<u64>;
}

/// Единые монотонные часы ОС-слоя, мс от первого вызова: ими меряется `KeyEvent.time_ms`,
/// ими же — срок `KeyHandler::next_deadline` и `tick`.
pub fn clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    InputMonitoring,
    Accessibility,
    LinuxInputDevices,
    LinuxUinput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionStatus {
    pub kind: PermissionKind,
    pub granted: bool,
    pub explanation: String,
    pub fix_hint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackendHealth {
    pub capturing: bool,
    pub layouts: Vec<String>,
    pub secure_input: bool,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// Нет права на перехват или эмуляцию ввода; ничего не стёрто и не напечатано.
    #[error("нет права на перехват или эмуляцию ввода")]
    NoPermission,
    #[error("не поддерживается: {0}")]
    Unsupported(String),
    #[error("ошибка ОС: {0}")]
    Os(String),
}

pub trait Backend: Send + Sync {
    /// Поднимает перехват на своём потоке.
    fn start(&self, handler: Arc<dyn KeyHandler>) -> Result<(), BackendError>;
    /// `Replace`, `SwitchLayout`, `ConvertSelection`; `ToggleAutocorrect` исполняет `ks-app`.
    /// Перед `Replace` проверяет право на эмуляцию ввода, без него — `Err(NoPermission)`.
    fn execute(&self, action: &Action) -> Result<(), BackendError>;
    fn layouts(&self) -> LayoutSet;
    fn current_layout(&self) -> LayoutId;
    /// Идентификатор приложения в фокусе и его имя для меню.
    fn focused_app(&self) -> (AppId, String);
    fn secure_input(&self) -> bool;
    fn permissions(&self) -> Vec<PermissionStatus>;
    fn request_permission(&self, kind: PermissionKind);
    fn open_permission_settings(&self, kind: PermissionKind);
    fn set_autostart(&self, on: bool) -> Result<(), BackendError>;
    fn autostart(&self) -> bool;
    fn health(&self) -> BackendHealth;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayStatus {
    Working,
    AutocorrectOff,
    SecureInput,
    NeedsPermissions,
    NeedsLayouts,
    ConfigError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayModel {
    pub status: TrayStatus,
    pub autocorrect: bool,
    /// Приложение в фокусе: идентификатор, имя для меню, исключено ли.
    pub focused_app: Option<(AppId, String, bool)>,
    pub undone_word: Option<String>,
    pub config_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleAutocorrect,
    ToggleApp(AppId),
    NeverWord(String),
    OpenSettings,
    Quit,
}

pub trait TrayHost {
    fn update(&self, model: &TrayModel);
    fn commands(&self) -> Receiver<TrayCommand>;
}
