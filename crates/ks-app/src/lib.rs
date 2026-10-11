//! Процесс агента KeySwitcher: связка бэкенд → движок → бэкенд, трей, `--status`, `--selftest`.
//! Бинарник `keyswitcher` (`src/main.rs`) — тонкий разбор командной строки поверх этого.

pub mod handler;
#[cfg(unix)]
pub mod ipc;
pub mod paths;
pub mod selftest;
pub mod status;
pub mod timer;
#[cfg(any(target_os = "macos", windows))]
pub mod tray;
pub mod watch;

pub mod agent;
