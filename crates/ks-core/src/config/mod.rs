//! Модель настроек и файл настроек (TOML, формат — PRD «Модель данных»).

mod hotkey;

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use hotkey::{Hotkey, HotkeyError, validate_hotkeys};

use crate::{AppId, Modifiers, PhysicalKey};

/// Версия формата файла настроек.
const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub autocorrect: bool,
    pub single_shift_switch: bool,
    pub autostart: bool,
    pub hotkeys: HotkeyConfig,
    pub apps: AppRules,
    pub words: WordRules,
}

/// До двух назначений на действие; пустой список — без хоткея.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub undo_or_convert_word: Vec<Hotkey>,
    pub convert_selection: Vec<Hotkey>,
    pub toggle_autocorrect: Vec<Hotkey>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppRules {
    /// Включить автоматику в приложении из встроенного списка исключений.
    pub enabled: Vec<AppId>,
    /// Выключить автоматику в любом другом приложении.
    pub disabled: Vec<AppId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WordRules {
    /// Не исправлять эти токены (набранная форма).
    pub never: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("не удалось прочитать файл настроек: {0}")]
    Io(#[from] io::Error),
    /// Текст ошибки разбора без содержимого файла: в файле лежит список слов пользователя.
    #[error("файл настроек повреждён: {0}")]
    Parse(String),
    #[error("неизвестная версия файла настроек: {0}")]
    UnsupportedVersion(u32),
    #[error(transparent)]
    Hotkeys(#[from] HotkeyError),
}

impl Default for Config {
    fn default() -> Self {
        Config::default_for(Os::current())
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        HotkeyConfig::default_for(Os::current())
    }
}

impl Config {
    pub fn default_for(os: Os) -> Config {
        Config {
            version: VERSION,
            autocorrect: true,
            single_shift_switch: true,
            autostart: true,
            hotkeys: HotkeyConfig::default_for(os),
            apps: AppRules::default(),
            words: WordRules::default(),
        }
    }

    /// Нет файла → умолчания (создать файл — дело вызывающего). Битый файл, неизвестная
    /// версия или неверные хоткеи → ошибка; файл не трогается.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(ConfigError::Io(e)),
        };
        let config: Config = toml::from_str(&text).map_err(|e| {
            let line = e
                .span()
                .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
            match line {
                Some(line) => ConfigError::Parse(format!("строка {line}: {}", e.message())),
                None => ConfigError::Parse(e.message().to_owned()),
            }
        })?;
        if config.version != VERSION {
            return Err(ConfigError::UnsupportedVersion(config.version));
        }
        validate_hotkeys(&config.hotkeys)?;
        Ok(config)
    }

    /// Атомарная запись: временный файл рядом + rename.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, path)
    }
}

impl HotkeyConfig {
    /// `convert_selection`: macOS — `Alt+Shift`, иначе `Shift+Pause`.
    pub fn default_for(os: Os) -> HotkeyConfig {
        let pause = Hotkey::Combo {
            mods: Modifiers::default(),
            key: Some(PhysicalKey::Pause),
        };
        let convert_selection = match os {
            Os::MacOs => Hotkey::Combo {
                mods: Modifiers {
                    alt: true,
                    shift: true,
                    ..Modifiers::default()
                },
                key: None,
            },
            Os::Windows | Os::Linux => Hotkey::Combo {
                mods: Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                key: Some(PhysicalKey::Pause),
            },
        };
        HotkeyConfig {
            undo_or_convert_word: vec![Hotkey::DoubleShift, pause],
            convert_selection: vec![convert_selection],
            toggle_autocorrect: vec![Hotkey::BothShifts],
        }
    }
}
