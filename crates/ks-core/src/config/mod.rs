//! Модель настроек и файл настроек (TOML, формат — PRD «Модель данных»).

mod hotkey;

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use hotkey::{Hotkey, HotkeyError, MAX_PER_ACTION, validate_hotkeys};

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
    /// Раз в сутки спрашивать GitHub о новой версии; ставится только по кнопке.
    pub check_updates: bool,
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
    /// Как в Punto Switcher: окно отмены открыто — отменить исправление, иначе — сменить
    /// раскладку выделенного текста.
    pub undo_or_convert_selection: Vec<Hotkey>,
    /// Переключить раскладку на другую из пары RU/EN.
    pub switch_layout: Vec<Hotkey>,
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
            // на macOS умолчания как в Punto Switcher: раскладка — ⌘Пробел, не одиночный Shift
            single_shift_switch: os != Os::MacOs,
            autostart: true,
            check_updates: true,
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
        let tmp = temp_path(path);
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, path)
    }
}

/// Временный файл записи рядом с `path`: свой у каждой записи (pid + счётчик) — файл пишут два
/// процесса, агент и окно настроек, и общий временный файл они перетирали бы друг у друга.
fn temp_path(path: &Path) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{name}.{}.{n}.tmp", std::process::id()))
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

/// Имя ключа для сообщений: только «голые» ключи TOML вне раздела `[words]`; прочее (кавычки,
/// не ASCII, любой ключ внутри `[words]` или `words.…`) может оказаться словом пользователя и
/// не показывается. `section` — раздел, в котором стоит ключ.
fn shown_key(section: Option<&str>, key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if bare && !in_words(section, key) {
        key.to_owned()
    } else {
        "(имя скрыто)".to_owned()
    }
}

/// Ключ лежит в разделе слов: `[words]`, `[words.…]` или пунктирный `words.…` сверху.
fn in_words(section: Option<&str>, key: &str) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_whitespace() && !matches!(c, '"' | '\''))
            .collect()
    };
    let words = |s: &str| s == "words" || s.starts_with("words.");
    let (section, key) = (section.map(norm), norm(key));
    // Имя из модели настроек — не слово пользователя.
    let known = (section.as_deref() == Some("words") && key == "never") || key == "words.never";
    !known && (section.is_some_and(|s| words(&s)) || key.starts_with("words."))
}

/// Раздел (`[имя]`), в котором стоит текст до `before`.
fn section_of(before: &str) -> Option<&str> {
    before.lines().rev().find_map(|l| {
        let t = l.trim().strip_prefix('[')?;
        t.trim_start_matches('[').split(']').next()
    })
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
            shown_key(section_of(&text[..line_start]), key)
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
                "undo_or_convert_selection",
                "switch_layout",
            ],
        ),
        ("apps", &["enabled", "disabled"]),
        ("words", &["never"]),
    ];
    const SCALARS: [&str; 5] = [
        "version",
        "autocorrect",
        "single_shift_switch",
        "autostart",
        "check_updates",
    ];
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut unknown = Vec::new();
    for (key, value) in &table {
        if SCALARS.contains(&key.as_str()) {
            continue;
        }
        let Some((_, known)) = SECTIONS.iter().find(|(name, _)| name == key) else {
            unknown.push(shown_key(None, key));
            continue;
        };
        if let Some(section) = value.as_table() {
            for inner in section.keys().filter(|k| !known.contains(&k.as_str())) {
                unknown.push(format!("{key}.{}", shown_key(Some(key), inner)));
            }
        }
    }
    unknown
}

impl HotkeyConfig {
    /// `convert_selection`: macOS — `Alt+Shift`, иначе `Shift+Pause`.
    /// Умолчания. На macOS — как в Punto Switcher: ⌘⌥⇧ отменяет исправление или меняет
    /// раскладку выделенного текста, ⌘Пробел переключает раскладку; остальное без хоткея.
    /// На Windows и Linux — классический набор.
    pub fn default_for(os: Os) -> HotkeyConfig {
        match os {
            Os::MacOs => HotkeyConfig {
                undo_or_convert_word: vec![],
                convert_selection: vec![],
                toggle_autocorrect: vec![],
                undo_or_convert_selection: vec![Hotkey::Combo {
                    mods: Modifiers {
                        meta: true,
                        alt: true,
                        shift: true,
                        ..Modifiers::default()
                    },
                    key: None,
                }],
                switch_layout: vec![Hotkey::Combo {
                    mods: Modifiers {
                        meta: true,
                        ..Modifiers::default()
                    },
                    key: Some(PhysicalKey::Space),
                }],
            },
            Os::Windows | Os::Linux => HotkeyConfig::classic(os),
        }
    }

    /// Классический набор: ⇧⇧ и Pause — отменить/конвертировать слово, ⌥⇧ (macOS) или
    /// Shift+Pause — выделение, оба Shift — вкл/выкл автоматики. На нём держится `--selftest`.
    pub fn classic(os: Os) -> HotkeyConfig {
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
            undo_or_convert_selection: vec![],
            switch_layout: vec![],
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
        assert!(logged.contains("words."), "{logged}");
        assert!(!logged.contains("значение"), "{logged}");
        let _ = fs::remove_file(path);
    }

    /// Ключ внутри `[words]` может быть набранным словом: ни в журнал, ни в текст ошибки.
    #[test]
    fn keys_inside_words_are_never_shown() {
        capture_logs();
        let path = write(
            "wordkey",
            "version = 1\n[words]\nnever = []\nghbdtnlog = 1\n",
        );
        Config::load(&path).expect("неизвестный ключ не ошибка");
        let logged = LOGGED.lock().unwrap().join("\n");
        assert!(!logged.contains("ghbdtnlog"), "{logged}");
        for (name, text, word) in [
            (
                "wordsec",
                "version = 1\n[words]\nghbdtnsec = \n",
                "ghbdtnsec",
            ),
            ("worddot", "version = 1\nwords.ghbdtndot = \n", "ghbdtndot"),
            ("wordarr", "version = 1\n[words]\nnever = 5\n", "never"),
        ] {
            let p = write(name, text);
            let shown = Config::load(&p).expect_err("битый файл").to_string();
            if word == "never" {
                assert!(shown.contains("строка 3"), "{shown}");
            } else {
                assert!(!shown.contains(word), "{shown}");
            }
            let _ = fs::remove_file(p);
        }
        let _ = fs::remove_file(path);
    }

    /// PRD 31: до двух назначений на действие.
    #[test]
    fn more_than_two_hotkeys_per_action_are_rejected() {
        let mut h = HotkeyConfig::classic(Os::MacOs);
        h.convert_selection.push("Ctrl+Alt+KeyK".parse().unwrap());
        assert_eq!(validate_hotkeys(&h), Ok(()), "два — можно");
        h.convert_selection.push("Ctrl+Alt+KeyJ".parse().unwrap());
        let err = validate_hotkeys(&h).expect_err("три — нельзя");
        assert!(!err.to_string().contains("Key"), "{err}");
    }

    /// Ревью 07: файл пишут два процесса (агент и окно настроек) — у каждой записи свой временный
    /// файл, rename атомарен, хвостов не остаётся.
    #[test]
    fn every_save_uses_its_own_temp_file() {
        let path =
            std::env::temp_dir().join(format!("ks-config-{}-saves.toml", std::process::id()));
        let (first, second) = (temp_path(&path), temp_path(&path));
        assert_ne!(first, second);
        let pid = std::process::id().to_string();
        assert!(
            first.to_string_lossy().contains(&pid),
            "{}",
            first.display()
        );
        assert_eq!(first.parent(), path.parent(), "rename в пределах каталога");

        let writers: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        Config::default_for(Os::current()).save(&path).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        Config::load(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let leftovers = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with(&name) && n.ends_with(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }
}
