//! Буфер токена: нажатия текущего слова с момента последнего сброса.

use crate::{AppId, LayoutId, Level, PhysicalKey};

#[derive(Debug, Default)]
pub(crate) struct TokenBuffer {
    keys: Vec<(PhysicalKey, Level)>,
    layout: Option<LayoutId>,
    app: Option<AppId>,
    edited: bool,
    /// Начало слова потеряно (смена раскладки посреди слова, клавиша без символа), а каретка
    /// не двигалась: остаток до границы — не отдельное слово, автоматика его не трогает.
    incomplete: bool,
}

impl TokenBuffer {
    /// Граница или сброс: слово забыто, флаги правки и неполного слова сняты.
    pub(crate) fn clear(&mut self) {
        self.keys.clear();
        self.layout = None;
        self.edited = false;
        self.incomplete = false;
    }

    /// Смена приложения сбрасывает слово; `true` — приложение сменилось.
    pub(crate) fn note_app(&mut self, app: &AppId) -> bool {
        if self.app.as_ref() == Some(app) {
            return false;
        }
        self.clear();
        self.app = Some(app.clone());
        true
    }

    /// Нажатие символьной клавиши; смена раскладки посреди слова делает слово неполным.
    pub(crate) fn push(&mut self, key: PhysicalKey, level: Level, layout: &LayoutId) {
        if self.layout.as_ref() != Some(layout) {
            if !self.keys.is_empty() {
                self.interrupt();
            }
            self.layout = Some(layout.clone());
        }
        self.keys.push((key, level));
    }

    /// Клавиша без символа посреди набора: набранное забыто, остаток слова — неполный.
    /// На пустом буфере — без эффекта: следующее слово начинается целым.
    pub(crate) fn interrupt(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        self.keys.clear();
        self.layout = None;
        self.incomplete = true;
    }

    /// Backspace внутри слова: последний символ стёрт, слово помечено как правленое.
    /// На пустом буфере — без эффекта.
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

    /// Правка, ручная конвертация или неполное слово: автоматика пропускает.
    pub(crate) fn is_edited(&self) -> bool {
        self.edited || self.incomplete
    }
}
