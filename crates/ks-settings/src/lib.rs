//! Окно настроек KeySwitcher (egui/eframe, отдельный процесс `keyswitcher --settings`).
//! Окно читает и пишет только файл настроек; агент следит за файлом и применяет правки сам.
//! Логика окна без отрисовки — [`model`], запись сочетания — [`record`], один экземпляр окна —
//! [`instance`], шрифты — [`fonts`].

pub mod about;
pub mod fonts;
pub mod instance;
pub mod model;
pub mod record;
#[cfg(debug_assertions)]
mod shot;
mod window;

pub use window::{EMPTY_APPS, EMPTY_WORDS};

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use ks_core::BuiltinApp;
use ks_platform::Backend;

/// Раздел окна (`--section <имя>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Hotkeys,
    Apps,
    Words,
    Permissions,
    About,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::General,
        Section::Hotkeys,
        Section::Apps,
        Section::Words,
        Section::Permissions,
        Section::About,
    ];

    /// Имя для командной строки.
    pub fn name(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Hotkeys => "hotkeys",
            Section::Apps => "apps",
            Section::Words => "words",
            Section::Permissions => "permissions",
            Section::About => "about",
        }
    }

    /// Заголовок в окне.
    pub fn title(self) -> &'static str {
        match self {
            Section::General => "Общие",
            Section::Hotkeys => "Горячие клавиши",
            Section::Apps => "Приложения-исключения",
            Section::Words => "Слова-исключения",
            Section::Permissions => "Разрешения",
            Section::About => "О программе",
        }
    }
}

impl FromStr for Section {
    type Err = String;

    fn from_str(s: &str) -> Result<Section, String> {
        Section::ALL
            .into_iter()
            .find(|sec| sec.name() == s)
            .ok_or_else(|| format!("неизвестный раздел: {s}"))
    }
}

/// Что окну даёт процесс `keyswitcher`: права и переход в системные настройки (бэкенд процесса
/// окна — окно, запущенное агентом, наследует его права), встроенный список приложений и
/// «Перезапустить KeySwitcher». `restart` блокирует до ответа нового агента — окно зовёт его на
/// своём потоке.
#[derive(Default)]
pub struct Host {
    pub backend: Option<Arc<dyn Backend>>,
    pub builtin: Vec<BuiltinApp>,
    pub restart: Option<Arc<dyn Fn() -> Result<(), String> + Send + Sync>>,
}

/// Окно без бэкенда: раздел «Разрешения» сообщает, что права не видны.
pub fn run(section: Option<Section>, config_path: &Path) -> anyhow::Result<()> {
    run_with(section, config_path, Host::default())
}

/// Окно настроек. Уже открыто (другой процесс) — оно поднимается и переходит на `section`,
/// этот процесс выходит.
pub fn run_with(section: Option<Section>, config_path: &Path, host: Host) -> anyhow::Result<()> {
    let dir = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("у файла настроек нет каталога"))?;
    std::fs::create_dir_all(dir)?;
    let guard = match instance::acquire(dir)? {
        instance::Instance::First(guard) => guard,
        instance::Instance::Second => {
            instance::raise(dir, section.unwrap_or(Section::General))?;
            return Ok(());
        }
    };
    window::show(
        section.unwrap_or(Section::General),
        config_path,
        host,
        guard,
    )
}
