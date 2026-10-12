//! Логика окна настроек без отрисовки: каждое действие окна приводит к ожидаемому `Config`
//! (сценарии `app-shell` «Окно настроек», `manual-control` «Переназначение горячих клавиш»,
//! `safety` «Очистка списка»).

use std::path::PathBuf;

use ks_core::{AppCategory, AppId, BuiltinApp, Config, Hotkey, HotkeyError, Os};
use ks_settings::Section;
use ks_settings::model::{AppError, Exclusion, HotkeyAction, Settings, WordError, builtin_rows};
use ks_settings::record;

const TERMINAL: &str = "com.apple.Terminal";
const TEXTEDIT: &str = "com.apple.TextEdit";

/// Классический набор жестов (⇧⇧ занят): на нём проверяются конфликты сочетаний.
fn settings() -> Settings {
    Settings::new(
        Ok(Config {
            hotkeys: ks_core::HotkeyConfig::classic(Os::MacOs),
            ..Config::default_for(Os::MacOs)
        }),
        vec![BuiltinApp {
            id: AppId(TERMINAL.into()),
            category: AppCategory::Terminal,
            name: "Терминал",
            family: None,
        }],
    )
}

fn hk(s: &str) -> Hotkey {
    s.parse().unwrap()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ks-settings-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.toml")
}

#[test]
fn removing_one_word_leaves_the_rest_and_clear_empties_the_list() {
    let mut s = settings();
    s.add_word("ghbdtn").unwrap();
    s.add_word("rjvg").unwrap();
    s.remove_word("ghbdtn");
    assert_eq!(s.config().words.never, ["rjvg"]);
    s.add_word("ghbdtn").unwrap();
    s.clear_words();
    assert!(s.config().words.never.is_empty());
}

#[test]
fn one_or_two_letter_word_is_refused_with_an_explanation_without_the_word() {
    let mut s = settings();
    for short in ["z", "zx", " q "] {
        let err = s.add_word(short).expect_err("короткое слово");
        assert_eq!(err, WordError::TooShort);
        assert!(!err.to_string().contains(short.trim()), "{err}");
        assert!(err.to_string().chars().count() > 20, "объяснение: {err}");
    }
    assert_eq!(s.add_word("   "), Err(WordError::Empty));
    s.add_word("rjvg").unwrap();
    assert_eq!(s.add_word("RJVG"), Err(WordError::Duplicate));
    assert_eq!(s.config().words.never, ["rjvg"]);
}

#[test]
fn builtin_exclusion_is_switched_on_and_off_per_app() {
    let mut s = settings();
    let terminal = AppId(TERMINAL.into());
    assert!(!s.builtin_enabled(&terminal));
    s.set_builtin_enabled(&terminal, true);
    assert_eq!(s.config().apps.enabled, std::slice::from_ref(&terminal));
    assert!(!s.config().apps.excludes(&terminal, s.builtin()));
    s.set_builtin_enabled(&terminal, true);
    assert_eq!(s.config().apps.enabled.len(), 1, "без дублей");
    s.set_builtin_enabled(&terminal, false);
    assert!(s.config().apps.enabled.is_empty());
    assert!(s.config().apps.excludes(&terminal, s.builtin()));
}

fn app(
    id: &str,
    category: AppCategory,
    name: &'static str,
    family: Option<&'static str>,
) -> BuiltinApp {
    BuiltinApp {
        id: AppId(id.into()),
        category,
        name,
        family,
    }
}

/// Скриншот пользователя: десяток строк com.jetbrains.* вместо названий. Семейство — одна строка с
/// названиями приложений, издания с одним названием не повторяются, категории идут по порядку.
#[test]
fn builtin_list_shows_names_and_one_row_per_family() {
    const JB: Option<&str> = Some("JetBrains IDE и Android Studio");
    let apps = [
        app(
            "com.google.android.studio",
            AppCategory::Ide,
            "Android Studio",
            JB,
        ),
        app(
            "com.apple.Terminal",
            AppCategory::Terminal,
            "Терминал",
            None,
        ),
        app(
            "com.jetbrains.intellij",
            AppCategory::Ide,
            "IntelliJ IDEA",
            JB,
        ),
        app(
            "com.jetbrains.intellij.ce",
            AppCategory::Ide,
            "IntelliJ IDEA",
            JB,
        ),
        app("com.jetbrains.pycharm", AppCategory::Ide, "PyCharm", JB),
        app("com.apple.dt.Xcode", AppCategory::Ide, "Xcode", None),
    ];
    let rows = builtin_rows(&apps);
    let titles: Vec<&str> = rows.iter().map(|r| r.title).collect();
    assert_eq!(
        titles,
        ["Терминал", "JetBrains IDE и Android Studio", "Xcode"]
    );
    assert_eq!(
        rows[1].members,
        ["Android Studio", "IntelliJ IDEA", "PyCharm"]
    );
    assert_eq!(rows[1].ids.len(), 4);
    assert!(rows[0].members.is_empty() && rows[2].members.is_empty());
}

/// Флажок семейства переключает все его приложения; меню «Не исправлять в …» на одном из них
/// даёт «частично», флажок строки снова выравнивает всё семейство.
#[test]
fn family_row_is_excluded_as_a_whole_and_shows_a_partial_state() {
    const JB: Option<&str> = Some("JetBrains");
    let apps = vec![
        app(
            "com.jetbrains.intellij",
            AppCategory::Ide,
            "IntelliJ IDEA",
            JB,
        ),
        app("com.jetbrains.pycharm", AppCategory::Ide, "PyCharm", JB),
    ];
    let mut s = Settings::new(Ok(Config::default_for(Os::MacOs)), apps);
    let row = s.builtin_rows().remove(0);
    assert_eq!(s.exclusion(&row), Exclusion::All);
    s.set_excluded(&row, false);
    assert_eq!(s.exclusion(&row), Exclusion::None);
    assert!(
        row.ids
            .iter()
            .all(|id| !s.config().apps.excludes(id, s.builtin()))
    );
    s.set_builtin_enabled(&row.ids[1], false);
    assert_eq!(s.exclusion(&row), Exclusion::Partly);
    s.set_excluded(&row, true);
    assert_eq!(s.exclusion(&row), Exclusion::All);
    assert!(s.config().apps.enabled.is_empty());
}

/// Флажок действия снят — назначения убраны и отданы окну; поставлен — вернулись целиком. Если
/// сочетание успели отдать другому действию, возврат отказан и назначения не меняются.
#[test]
fn hotkey_checkbox_parks_and_restores_both_assignments() {
    let mut s = settings();
    let undo = HotkeyAction::UndoOrConvertWord;
    let before = s.hotkeys(undo).to_vec();
    assert_eq!(before.len(), 2, "классика: двойной Shift и Pause");
    let parked = s.clear_hotkeys(undo);
    assert_eq!(parked, before);
    assert!(s.hotkeys(undo).is_empty());
    s.restore_hotkeys(undo, parked.clone()).unwrap();
    assert_eq!(s.hotkeys(undo), before.as_slice());

    s.clear_hotkeys(undo);
    s.set_hotkey(HotkeyAction::SwitchLayout, 0, Hotkey::DoubleShift)
        .unwrap();
    let taken = s.config().hotkeys.clone();
    assert_eq!(
        s.restore_hotkeys(undo, parked),
        Err(HotkeyError::Duplicate(Hotkey::DoubleShift))
    );
    assert_eq!(s.config().hotkeys, taken);
}

/// Подписи как в Punto Switcher: Cmd/Win первым, «Пробел», плюсы с пробелами.
#[test]
fn hotkey_labels_read_like_punto() {
    let cmd = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Win"
    };
    let alt = if cfg!(target_os = "macos") {
        "Option"
    } else {
        "Alt"
    };
    assert_eq!(record::label(&hk("Meta+Space")), format!("{cmd} + Пробел"));
    assert_eq!(
        record::label(&hk("Meta+Alt+Shift")),
        format!("{cmd} + {alt} + Shift")
    );
    assert_eq!(
        record::label(&hk("Ctrl+Alt+KeyK")),
        format!("Ctrl + {alt} + K")
    );
    assert_eq!(record::label(&Hotkey::DoubleShift), "Двойной Shift");
    assert!(record::presets().contains(&hk("Meta+Space")));
}

#[test]
fn own_app_list_adds_and_removes_ids() {
    let mut s = settings();
    s.add_app(&format!(" {TEXTEDIT} ")).unwrap();
    assert_eq!(s.config().apps.disabled, [AppId(TEXTEDIT.into())]);
    assert_eq!(s.add_app(TEXTEDIT), Err(AppError::Duplicate));
    assert_eq!(s.add_app("  "), Err(AppError::Empty));
    s.remove_app(&AppId(TEXTEDIT.into()));
    assert!(s.config().apps.disabled.is_empty());
}

#[test]
fn recorded_hotkey_replaces_and_removed_hotkey_disables_the_action() {
    let mut s = settings();
    let sel = HotkeyAction::ConvertSelection;
    s.set_hotkey(sel, 0, hk("Ctrl+Alt+KeyK")).unwrap();
    assert_eq!(s.config().hotkeys.convert_selection, [hk("Ctrl+Alt+KeyK")]);
    s.remove_hotkey(sel, 0);
    assert!(
        s.config().hotkeys.convert_selection.is_empty(),
        "без хоткея"
    );
    assert_eq!(s.hotkeys(sel), &[] as &[Hotkey]);
}

#[test]
fn conflicting_bare_or_third_hotkey_is_refused_and_assignments_stay() {
    let mut s = settings();
    let before = s.config().hotkeys.clone();
    let sel = HotkeyAction::ConvertSelection;
    assert_eq!(
        s.set_hotkey(sel, 0, Hotkey::DoubleShift),
        Err(HotkeyError::Duplicate(Hotkey::DoubleShift))
    );
    assert_eq!(
        s.set_hotkey(sel, 1, hk("KeyQ")),
        Err(HotkeyError::BareKey(hk("KeyQ")))
    );
    assert_eq!(s.config().hotkeys, before, "прежние назначения не изменены");
    let undo = HotkeyAction::UndoOrConvertWord;
    assert_eq!(s.hotkeys(undo).len(), 2);
    assert_eq!(
        s.set_hotkey(undo, 2, hk("Ctrl+Alt+KeyJ")),
        Err(HotkeyError::TooMany)
    );
    assert_eq!(s.config().hotkeys, before);
}

#[test]
fn edit_rereads_the_file_and_never_writes_over_a_broken_one() {
    let path = temp("edit");
    let mut s = Settings::load(&path, vec![]);
    std::fs::write(&path, "version = 1\n[words]\nnever = [\"rjvg\"]\n").unwrap();
    s.edit(&path, |s| s.add_app(TEXTEDIT).map_err(|e| e.to_string()))
        .unwrap();
    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.words.never, ["rjvg"], "внешняя правка цела");
    assert_eq!(saved.apps.disabled, [AppId(TEXTEDIT.into())]);

    let broken = "version = 1\n[words]\nnever = [\"rjvg\"\n";
    std::fs::write(&path, broken).unwrap();
    let err = s
        .edit(&path, |s| s.add_word("ghbdtn").map_err(|e| e.to_string()))
        .expect_err("битый файл");
    assert!(!err.contains("rjvg"), "{err}");
    assert!(s.error().is_some() && !s.writable());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        broken,
        "байт в байт"
    );
}

#[test]
fn every_section_has_a_command_line_name() {
    for section in Section::ALL {
        assert_eq!(section.name().parse::<Section>(), Ok(section));
    }
    assert_eq!("permissions".parse::<Section>(), Ok(Section::Permissions));
    assert_eq!("about".parse::<Section>(), Ok(Section::About));
    assert!("nope".parse::<Section>().is_err());
}
