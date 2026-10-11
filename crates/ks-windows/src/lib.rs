//! Бэкенд Windows — заготовка. Перехват, эмуляцию ввода и таблицы раскладок из API ОС
//! добавляет задача этого бэкенда; до тех пор всё, что зовётся при старте, возвращает `Err`.

use std::sync::Arc;

use ks_core::{Action, AppId, LayoutId, LayoutSet};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus,
};

const NOT_YET: &str = "бэкенд Windows ещё не реализован";

#[derive(Debug, Default)]
pub struct WindowsBackend;

impl WindowsBackend {
    pub fn new() -> WindowsBackend {
        WindowsBackend
    }
}

impl Backend for WindowsBackend {
    fn start(&self, _handler: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        log::warn!("{NOT_YET}");
        Err(BackendError::Unsupported(NOT_YET.to_owned()))
    }

    fn execute(&self, _action: &Action) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(NOT_YET.to_owned()))
    }

    fn layouts(&self) -> LayoutSet {
        LayoutSet::default()
    }

    fn current_layout(&self) -> LayoutId {
        LayoutId::default()
    }

    fn focused_app(&self) -> (AppId, String) {
        (AppId::default(), String::new())
    }

    fn secure_input(&self) -> bool {
        false
    }

    fn permissions(&self) -> Vec<PermissionStatus> {
        Vec::new()
    }

    fn request_permission(&self, _kind: PermissionKind) {}

    fn open_permission_settings(&self, _kind: PermissionKind) {}

    fn set_autostart(&self, _on: bool) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(NOT_YET.to_owned()))
    }

    fn autostart(&self) -> bool {
        false
    }

    fn health(&self) -> BackendHealth {
        BackendHealth {
            detail: NOT_YET.to_owned(),
            ..BackendHealth::default()
        }
    }
}
