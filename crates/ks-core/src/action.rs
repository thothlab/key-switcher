//! Контекст нажатия и действия, которые движок отдаёт наружу.

use crate::{AppId, KeyStroke, LayoutId};

/// Снимок окружения на момент нажатия; его собирает бэкенд.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub app: AppId,
    pub layout: LayoutId,
    pub secure_input: bool,
    pub password_field: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceKind {
    Auto,
    Manual,
    Undo,
    CaseFix,
}

/// Что сделать с исходным нажатием: пропустить в приложение или проглотить.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Pass,
    Swallow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Стереть `erase` символов, переключиться на `target_layout`, напечатать `keys`
    /// (запасной путь — строка `text`). Проглоченная граница слова входит в `keys` и `text`.
    Replace {
        erase: usize,
        keys: Vec<KeyStroke>,
        text: String,
        target_layout: Option<LayoutId>,
        kind: ReplaceKind,
    },
    SwitchLayout(LayoutId),
    ConvertSelection,
    ToggleAutocorrect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub disposition: Disposition,
    pub actions: Vec<Action>,
}

impl Outcome {
    /// Пропустить нажатие, ничего не делать (без аллокации).
    pub fn pass() -> Outcome {
        Outcome {
            disposition: Disposition::Pass,
            actions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineStatus {
    Working,
    AutocorrectOff,
    NeedsLayouts,
    ConfigError(String),
}
