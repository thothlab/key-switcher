//! `Config`: умолчания, запись и чтение, битый файл; `Hotkey`: разбор и печать.

use std::path::PathBuf;

use ks_core::{
    AppId, Config, ConfigError, Hotkey, HotkeyConfig, HotkeyError, Modifiers, Os, PhysicalKey,
    validate_hotkeys,
};

/// Пример файла из PRD «Модель данных» — это и есть умолчания macOS.
const PRD_EXAMPLE: &str = r#"
version = 1
autocorrect = true            # автоматика вкл/выкл
single_shift_switch = true    # одиночный Shift переключает раскладку
autostart = true

[hotkeys]                     # до двух назначений на действие; [] = без хоткея
undo_or_convert_word = ["DoubleShift", "Pause"]
convert_selection    = ["Alt+Shift"]               # macOS; на Windows/Linux по умолчанию ["Shift+Pause"]
toggle_autocorrect   = ["BothShifts"]

[apps]                        # bundle id (macOS), имя exe (Windows), app_id/WM_CLASS (Linux)
enabled  = []                 # включить автоматику в приложении из встроенного списка исключений
disabled = []                 # выключить в любом другом

[words]
never = []                    # не исправлять эти токены (набранная форма)
"#;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ks-core-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn combo(mods: Modifiers, key: Option<PhysicalKey>) -> Hotkey {
    Hotkey::Combo { mods, key }
}

#[test]
fn defaults_match_the_prd_example_file() {
    let dir = temp_dir("prd");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, PRD_EXAMPLE).unwrap();
    let mac = Config::default_for(Os::MacOs);
    assert_eq!(Config::load(&path).unwrap(), mac);

    assert_eq!(mac.version, 1);
    assert!(mac.autocorrect && mac.single_shift_switch && mac.autostart);
    let alt_shift = Modifiers {
        alt: true,
        shift: true,
        ..Modifiers::default()
    };
    assert_eq!(mac.hotkeys.convert_selection, vec![combo(alt_shift, None)]);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for os in [Os::Windows, Os::Linux] {
        assert_eq!(
            Config::default_for(os).hotkeys.convert_selection,
            vec![combo(shift, Some(PhysicalKey::Pause))]
        );
    }
}

#[test]
fn save_then_load_gives_the_same_config() {
    let mut config = Config::default_for(Os::Linux);
    config.autocorrect = false;
    config
        .apps
        .disabled
        .push(AppId("org.gnome.Terminal".to_owned()));
    config.words.never.push("ghbdtn".to_owned());
    config.hotkeys.toggle_autocorrect.clear();
    let path = temp_dir("roundtrip").join("nested").join("config.toml");
    config.save(&path).unwrap();
    assert_eq!(Config::load(&path).unwrap(), config);
}

#[test]
fn missing_file_gives_defaults() {
    let path = temp_dir("missing").join("config.toml");
    assert_eq!(
        Config::load(&path).unwrap(),
        Config::default_for(Os::current())
    );
}

#[test]
fn broken_file_is_a_config_error() {
    let dir = temp_dir("broken");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "version = 1\nautocorrect = maybe\n").unwrap();
    assert!(matches!(Config::load(&path), Err(ConfigError::Parse(_))));
    std::fs::write(&path, "version = 2\n").unwrap();
    assert!(matches!(
        Config::load(&path),
        Err(ConfigError::UnsupportedVersion(2))
    ));
}

#[test]
fn hotkeys_round_trip_through_text() {
    for text in [
        "DoubleShift",
        "BothShifts",
        "Alt+Shift",
        "Shift+Pause",
        "Ctrl+Alt+KeyK",
        "Pause",
    ] {
        assert_eq!(text.parse::<Hotkey>().unwrap().to_string(), text);
    }
    let ctrl_alt = Modifiers {
        ctrl: true,
        alt: true,
        ..Modifiers::default()
    };
    assert_eq!(
        "Ctrl+Alt+KeyK".parse(),
        Ok(combo(ctrl_alt, Some(PhysicalKey::KeyK)))
    );
    assert_eq!(
        "Shift+Ctrl+KeyK".parse::<Hotkey>().unwrap().to_string(),
        "Ctrl+Shift+KeyK"
    );
    assert!(matches!(
        "Shift+Bogus".parse::<Hotkey>(),
        Err(HotkeyError::Parse(_))
    ));
    assert!(matches!("".parse::<Hotkey>(), Err(HotkeyError::Parse(_))));
}

#[test]
fn hotkey_validation_rejects_duplicates_and_bare_letters() {
    let mut hotkeys = HotkeyConfig::default_for(Os::MacOs);
    assert_eq!(validate_hotkeys(&hotkeys), Ok(()));

    hotkeys.toggle_autocorrect = vec![Hotkey::DoubleShift];
    assert_eq!(
        validate_hotkeys(&hotkeys),
        Err(HotkeyError::Duplicate(Hotkey::DoubleShift))
    );

    let bare_k = combo(Modifiers::default(), Some(PhysicalKey::KeyK));
    hotkeys.toggle_autocorrect = vec![bare_k];
    assert_eq!(
        validate_hotkeys(&hotkeys),
        Err(HotkeyError::BareKey(bare_k))
    );
}
