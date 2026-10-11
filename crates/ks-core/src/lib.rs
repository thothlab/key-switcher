//! Ядро KeySwitcher: типы, буфер токена, движок распознавания раскладки, настройки.
//!
//! Ядро не зависит от ОС и собирается без системных крейтов. Словари берутся из `ks-data`,
//! оттуда же приходят `Lang` и `AppId` (реэкспорт ниже), чтобы `Engine::new` мог принять `Arc<Lexicon>`.

mod action;
mod buffer;
pub mod config;
mod detect;
mod engine;
mod input;
mod key;
mod layout;

pub use action::{Action, Context, Disposition, EngineStatus, Outcome, ReplaceKind};
pub use config::{
    AppRules, Config, ConfigError, Hotkey, HotkeyConfig, HotkeyError, Os, WordRules,
    validate_hotkeys,
};
pub use engine::Engine;
pub use key::{KeyDir, KeyEvent, Modifiers, PhysicalKey, ResetReason, UnknownKey};
pub use ks_data::{AppId, Lang, Lexicon};
pub use layout::{KeyStroke, LayoutId, LayoutMap, LayoutSet, Level};
