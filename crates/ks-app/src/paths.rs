//! Каталог настроек и файлы в нём.

use std::path::PathBuf;

/// `~/Library/Application Support/KeySwitcher`, `%APPDATA%\KeySwitcher`, `~/.config/keyswitcher`.
pub fn config_dir() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let name = if cfg!(target_os = "linux") {
        "keyswitcher"
    } else {
        "KeySwitcher"
    };
    Some(base.config_dir().join(name))
}

pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

/// Сокет, через который `--status` и `--selftest` спрашивают работающий агент.
pub fn agent_socket() -> Option<PathBuf> {
    config_dir().map(|d| d.join("agent.sock"))
}
