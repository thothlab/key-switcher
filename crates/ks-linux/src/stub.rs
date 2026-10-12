//! Заглушка вне Linux: крейт собирается на любой ОС ради тестов чистой логики.

use std::sync::Arc;

use ks_core::{Action, AppId, LayoutId, LayoutSet};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus,
};

const ONLY_LINUX: &str = "бэкенд Linux работает только под Linux";

#[derive(Debug, Default)]
pub struct LinuxBackend;

impl LinuxBackend {
    pub fn new() -> LinuxBackend {
        LinuxBackend
    }
}

impl Backend for LinuxBackend {
    fn start(&self, _handler: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(ONLY_LINUX.to_owned()))
    }

    fn execute(&self, _action: &Action) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(ONLY_LINUX.to_owned()))
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
        Err(BackendError::Unsupported(ONLY_LINUX.to_owned()))
    }

    fn autostart(&self) -> bool {
        false
    }

    fn health(&self) -> BackendHealth {
        BackendHealth {
            detail: ONLY_LINUX.to_owned(),
            ..BackendHealth::default()
        }
    }
}
