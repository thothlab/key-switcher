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
    #[error("горячие клавиши (раздел [hotkeys]): {0}")]
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
        let config: Config = toml::from_str(&text).map_err(|e| parse_error(&text, &e))?;
        for key in unknown_keys(&text) {
            log::warn!("файл настроек: неизвестный ключ {key} проигнорирован");
        }
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

impl AppRules {
    /// Автоматика выключена в `app`: пользователь выключил её здесь или приложение во
    /// встроенном списке `builtin` и не включено пользователем.
    pub fn excludes(&self, app: &AppId, builtin: &[AppId]) -> bool {
        if app.0.is_empty() {
            return false;
        }
        self.disabled.contains(app) || (builtin.contains(app) && !self.enabled.contains(app))
    }

    /// Пункт меню «Не исправлять в …» / «Исправлять в …»: переключает автоматику в `app`.
    pub fn toggle(&mut self, app: &AppId, builtin: &[AppId]) {
        if app.0.is_empty() {
            return;
        }
        let in_builtin = builtin.contains(app);
        if self.excludes(app, builtin) {
            self.disabled.retain(|a| a != app);
            if in_builtin && !self.enabled.contains(app) {
                self.enabled.push(app.clone());
            }
        } else if in_builtin {
            self.enabled.retain(|a| a != app);
        } else if !self.disabled.contains(app) {
            self.disabled.push(app.clone());
        }
    }
}

/// Имя ключа для сообщений: только «голые» ключи TOML; прочее (кавычки, не ASCII) может
/// оказаться словом пользователя и не показывается.
fn shown_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if bare {
        key.to_owned()
    } else {
        "(имя скрыто)".to_owned()
    }
}

/// Ошибка разбора без значений: сообщение toml/serde цитирует значение (`never = "слово"`
/// унесло бы слово в статус и лог), поэтому берутся только номер строки и имя ключа.
fn parse_error(text: &str, e: &toml::de::Error) -> ConfigError {
    let Some(before) = e
        .span()
        .and_then(|span| text.get(..span.start.min(text.len())))
    else {
        return ConfigError::Parse("ошибка синтаксиса".to_owned());
    };
    let line_no = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = text[line_start..].lines().next().unwrap_or("");
    match line.split_once('=').map(|(key, _)| key.trim()) {
        Some(key) if !key.is_empty() => ConfigError::Parse(format!(
            "строка {line_no}, ключ {}: неверное значение или синтаксис",
            shown_key(key)
        )),
        _ => ConfigError::Parse(format!("строка {line_no}: ошибка синтаксиса")),
    }
}

/// Ключи файла, которых нет в модели настроек (`раздел.ключ`); значения не читаются.
fn unknown_keys(text: &str) -> Vec<String> {
    const SECTIONS: [(&str, &[&str]); 3] = [
        (
            "hotkeys",
            &[
                "undo_or_convert_word",
                "convert_selection",
                "toggle_autocorrect",
            ],
        ),
        ("apps", &["enabled", "disabled"]),
        ("words", &["never"]),
    ];
    const SCALARS: [&str; 4] = ["version", "autocorrect", "single_shift_switch", "autostart"];
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut unknown = Vec::new();
    for (key, value) in &table {
        if SCALARS.contains(&key.as_str()) {
            continue;
        }
        let Some((_, known)) = SECTIONS.iter().find(|(name, _)| name == key) else {
            unknown.push(shown_key(key));
            continue;
        };
        if let Some(section) = value.as_table() {
            for inner in section.keys().filter(|k| !known.contains(&k.as_str())) {
                unknown.push(format!("{key}.{}", shown_key(inner)));
            }
        }
    }
    unknown
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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, Once};

    use super::*;

    static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, _m: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            LOGGED.lock().unwrap().push(record.args().to_string());
        }
        fn flush(&self) {}
    }

    fn capture_logs() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = log::set_logger(&Capture);
            log::set_max_level(log::LevelFilter::Trace);
        });
    }

    fn write(name: &str, text: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("ks-config-{}-{name}.toml", std::process::id()));
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn parse_error_names_line_and_key_but_never_the_value() {
        let text = "version = 1\n\n[words]\nnever = \"секретноеслово\"\n";
        let path = write("secret", text);
        let err = Config::load(&path).expect_err("строка вместо списка");
        let shown = err.to_string();
        assert!(!shown.contains("секретноеслово"), "{shown}");
        assert!(shown.contains("строка 4"), "{shown}");
        assert!(shown.contains("never"), "{shown}");
        let syntax = write("syntax", "version = 1\nautocorrect = \"тайна\n");
        let shown = Config::load(&syntax)
            .expect_err("незакрытая строка")
            .to_string();
        assert!(!shown.contains("тайна"), "{shown}");
        assert!(shown.contains("строка 2"), "{shown}");
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(syntax);
    }

    /// Ошибка горячих клавиш уходит в меню и `--status`: значений из файла в тексте нет.
    #[test]
    fn hotkey_error_text_carries_no_value_from_the_file() {
        let dup = "version = 1\n[hotkeys]\nconvert_selection = [\"Ctrl+Alt+KeyK\"]\n\
                   toggle_autocorrect = [\"Ctrl+Alt+KeyK\"]\n";
        let bare = "version = 1\n[hotkeys]\nundo_or_convert_word = [\"KeyQ\"]\n";
        for (name, text, value) in [("dup", dup, "KeyK"), ("bare", bare, "KeyQ")] {
            let path = write(name, text);
            let err = Config::load(&path).expect_err("неверные горячие клавиши");
            let shown = err.to_string();
            assert!(matches!(err, ConfigError::Hotkeys(_)), "{shown}");
            assert!(!shown.contains(value), "{shown}");
            assert!(shown.contains("hotkeys"), "{shown}");
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn unknown_keys_are_ignored_with_a_warning_naming_the_key_only() {
        capture_logs();
        let text = "version = 1\nfuture_flag = \"значение1\"\n[words]\nnever = [\"rjvg\"]\nextra = \"значение2\"\n";
        let path = write("unknown", text);
        let config = Config::load(&path).expect("неизвестные ключи не ошибка");
        assert_eq!(config.words.never, ["rjvg"]);
        let logged = LOGGED.lock().unwrap().join("\n");
        assert!(logged.contains("future_flag"), "{logged}");
        assert!(logged.contains("words.extra"), "{logged}");
        assert!(!logged.contains("значение"), "{logged}");
        let _ = fs::remove_file(path);
    }
}
