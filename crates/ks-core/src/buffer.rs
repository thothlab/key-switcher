//! Буфер токена: нажатия текущего слова с момента последнего сброса.

use crate::{AppId, LayoutId, Level, PhysicalKey};

#[derive(Debug, Default)]
pub(crate) struct TokenBuffer {
    keys: Vec<(PhysicalKey, Level)>,
    layout: Option<LayoutId>,
    app: Option<AppId>,
    edited: bool,
}

impl TokenBuffer {
    /// Сброс без решения: слово забыто, флаг правки снят.
    pub(crate) fn clear(&mut self) {
        self.keys.clear();
        self.layout = None;
        self.edited = false;
    }

    /// Смена приложения сбрасывает слово.
    pub(crate) fn note_app(&mut self, app: &AppId) {
        if self.app.as_ref() != Some(app) {
            self.clear();
            self.app = Some(app.clone());
        }
    }

    /// Нажатие символьной клавиши; смена раскладки посреди слова начинает слово заново.
    pub(crate) fn push(&mut self, key: PhysicalKey, level: Level, layout: &LayoutId) {
        if self.layout.as_ref() != Some(layout) {
            self.clear();
            self.layout = Some(layout.clone());
        }
        self.keys.push((key, level));
    }

    /// Backspace внутри слова: последний символ стёрт, слово помечено как правленое.
    pub(crate) fn erase_back(&mut self) {
        if self.keys.pop().is_some() {
            self.edited = true;
        }
    }

    /// Delete внутри слова: слово помечено как правленое.
    pub(crate) fn mark_edited(&mut self) {
        if !self.keys.is_empty() {
            self.edited = true;
        }
    }

    /// Слово сконвертировано вручную: дальше оно в раскладке `layout`, автоматика его не трогает.
    pub(crate) fn relayout(&mut self, layout: LayoutId) {
        self.layout = Some(layout);
        self.edited = true;
    }

    pub(crate) fn keys(&self) -> &[(PhysicalKey, Level)] {
        &self.keys
    }

    pub(crate) fn layout(&self) -> Option<&LayoutId> {
        self.layout.as_ref()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub(crate) fn is_edited(&self) -> bool {
        self.edited
    }
}
