//! Шов `ks-app` — фальшивый `Backend`: что показывает иконка и `--status`, какие сценарии
//! `--selftest` встроены.

use std::sync::Arc;

use ks_app::selftest::scenarios;
use ks_app::status::{Source, report, tooltip, tray_status};
use ks_core::{Action, AppId, EngineStatus, Lang, LayoutId, LayoutMap, LayoutSet};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus, TrayStatus,
};

struct FakeBackend {
    accessibility: bool,
    with_russian: bool,
}

impl FakeBackend {
    fn layouts_set(&self) -> LayoutSet {
        let mut v = vec![LayoutMap::new(
            LayoutId("com.apple.keylayout.US".into()),
            Lang::En,
            "U.S.",
        )];
        if self.with_russian {
            v.push(LayoutMap::new(
                LayoutId("com.apple.keylayout.RussianWin".into()),
                Lang::Ru,
                "Russian – PC",
            ));
        }
        LayoutSet(v)
    }
}

impl Backend for FakeBackend {
    fn start(&self, _h: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        Ok(())
    }
    fn execute(&self, _a: &Action) -> Result<(), BackendError> {
        Ok(())
    }
    fn layouts(&self) -> LayoutSet {
        self.layouts_set()
    }
    fn current_layout(&self) -> LayoutId {
        LayoutId("com.apple.keylayout.US".into())
    }
    fn focused_app(&self) -> (AppId, String) {
        (AppId::default(), String::new())
    }
    fn secure_input(&self) -> bool {
        false
    }
    fn permissions(&self) -> Vec<PermissionStatus> {
        let p = |kind, granted| PermissionStatus {
            kind,
            granted,
            explanation: "зачем".into(),
            fix_hint: "как".into(),
        };
        vec![
            p(PermissionKind::InputMonitoring, true),
            p(PermissionKind::Accessibility, self.accessibility),
        ]
    }
    fn request_permission(&self, _k: PermissionKind) {}
    fn open_permission_settings(&self, _k: PermissionKind) {}
    fn set_autostart(&self, _on: bool) -> Result<(), BackendError> {
        Ok(())
    }
    fn autostart(&self) -> bool {
        false
    }
    fn health(&self) -> BackendHealth {
        BackendHealth {
            capturing: true,
            layouts: self
                .layouts_set()
                .0
                .iter()
                .map(|m| m.name.clone())
                .collect(),
            secure_input: false,
            detail: String::new(),
        }
    }
}

#[test]
fn without_accessibility_the_icon_says_permissions_are_needed() {
    let b = FakeBackend {
        accessibility: false,
        with_russian: true,
    };
    let status = tray_status(&b.permissions(), true, false, &EngineStatus::Working);
    assert_eq!(status, TrayStatus::NeedsPermissions);
    assert_eq!(tooltip(status), "KeySwitcher — нужны разрешения");
}

#[test]
fn status_of_a_working_agent_names_capture_layouts_and_secure_input() {
    let b = FakeBackend {
        accessibility: true,
        with_russian: true,
    };
    let text = report(&b, Some(&EngineStatus::Working), Source::Agent { pid: 42 });
    assert!(text.contains("клавиатура отслеживается"), "{text}");
    assert!(text.contains("безопасный ввод: нет"), "{text}");
    assert!(
        text.contains("U.S.") && text.contains("Russian – PC"),
        "{text}"
    );
    assert!(text.contains("pid 42"), "{text}");
    assert!(
        text.contains("текущая раскладка: com.apple.keylayout.US\n"),
        "{text}"
    );
}

#[test]
fn status_names_the_missing_permission() {
    let b = FakeBackend {
        accessibility: false,
        with_russian: false,
    };
    let text = report(&b, None, Source::Local);
    assert!(
        text.contains("Универсальный доступ (Accessibility): НЕ выдано"),
        "{text}"
    );
    assert!(text.contains("Нужны раскладки RU и EN"), "{text}");
    assert!(text.contains("агент не запущен"), "{text}");
}

#[test]
fn embedded_selftest_scenarios_cover_this_task() {
    let names: Vec<String> = scenarios()
        .expect("сценарии читаются")
        .into_iter()
        .map(|s| s.name)
        .collect();
    for expected in [
        "Русское слово в английской раскладке",
        "Английская фраза в русской раскладке",
        "Enter как граница",
        "Tab не запускает исправление",
        "Быстрый набор",
        "Отмена исправления двойным Shift",
        "Ручная конвертация пропущенного слова",
        "Фраза в выделении",
        "Пустое выделение",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "нет сценария «{expected}»"
        );
    }
}

#[cfg(unix)]
#[test]
fn status_is_answered_by_the_running_agent_over_its_socket_and_only_one_agent_listens() {
    use std::io::Write;
    use std::time::Duration;
    let path = std::env::temp_dir().join(format!("ks-test-{}.sock", std::process::id()));
    ks_app::ipc::serve(&path, Duration::ZERO, |cmd, out| {
        let _ = writeln!(out, "команда: {cmd}");
        let _ = writeln!(out, "клавиатура отслеживается");
    })
    .expect("сокет поднят");
    let mut lines = Vec::new();
    ks_app::ipc::ask(&path, "status", |l| lines.push(l.to_owned())).expect("агент ответил");
    assert_eq!(lines, ["команда: status", "клавиатура отслеживается"]);
    let second = ks_app::ipc::serve(&path, Duration::ZERO, |_, _| {});
    assert_eq!(second.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn restarted_agent_waits_until_the_previous_one_releases_the_socket() {
    use std::time::Duration;
    let path = std::env::temp_dir().join(format!("ks-test-relaunch-{}.sock", std::process::id()));
    ks_app::ipc::serve(&path, Duration::ZERO, |_, _| {}).expect("прежний агент");
    let released = path.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let _ = std::fs::remove_file(&released);
    });
    ks_app::ipc::serve(&path, Duration::from_secs(5), |_, _| {})
        .expect("новый агент дождался, пока прежний уйдёт");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn selftest_failure_report_names_expected_text_but_never_the_field_content() {
    let report = ks_app::selftest::mismatch("привет ", "секрет пароль").expect("расхождение");
    assert!(report.contains("\"привет \""), "{report}");
    assert!(report.contains("13 симв."), "{report}");
    assert!(
        report.contains("первое расхождение на символе 0"),
        "{report}"
    );
    assert!(
        !report.contains("секрет") && !report.contains("пароль"),
        "{report}"
    );
    let tail = ks_app::selftest::mismatch("привет ", "привет").unwrap();
    assert!(tail.contains("на символе 6"), "{tail}");
    assert_eq!(ks_app::selftest::mismatch("good ", "good "), None);
}

#[cfg(unix)]
#[test]
fn silent_client_does_not_block_the_agent_socket() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    let path = std::env::temp_dir().join(format!("ks-test-silent-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    ks_app::ipc::serve(&path, Duration::ZERO, |_, out| {
        let _ = writeln!(out, "клавиатура отслеживается");
    })
    .expect("сокет поднят");
    let _silent = UnixStream::connect(&path).expect("молчащий клиент");
    let (tx, rx) = std::sync::mpsc::channel();
    let ask_path = path.clone();
    std::thread::spawn(move || {
        let mut lines = Vec::new();
        let _ = ks_app::ipc::ask(&ask_path, "status", |l| lines.push(l.to_owned()));
        let _ = tx.send(lines);
    });
    let lines = rx
        .recv_timeout(ks_app::ipc::READ_TIMEOUT + Duration::from_secs(3))
        .expect("второй клиент получил ответ, не дожидаясь молчащего");
    assert_eq!(lines, ["клавиатура отслеживается"]);
    let _ = std::fs::remove_file(&path);
}

/// ⌘Z в сценарии selftest: отдельное нажатие Cmd, клавиша с флагом Cmd, отпускание Cmd —
/// как от клавиатуры (задача 10: одиночный Cmd окно отмены не закрывает).
#[test]
fn cmd_z_token_is_a_chord_with_a_separate_modifier_press() {
    use ks_app::selftest::{Scenario, Stroke};
    use ks_core::PhysicalKey as K;
    let s = Scenario {
        name: "chord".into(),
        layout_before: "en".into(),
        input: vec![
            "Meta+KeyZ".into(),
            "Shift+KeyA".into(),
            "Ctrl+Alt+KeyK".into(),
        ],
        expect_text: String::new(),
        expect_layout: "en".into(),
        delay_ms: None,
        select: None,
        clipboard: None,
    };
    assert_eq!(
        s.strokes().expect("сочетания разбираются"),
        vec![
            Stroke::Chord {
                mods: vec![K::MetaLeft],
                key: K::KeyZ
            },
            Stroke::Key(K::KeyA, true),
            Stroke::Chord {
                mods: vec![K::ControlLeft, K::AltLeft],
                key: K::KeyK
            },
        ]
    );
    let bad = Scenario {
        input: vec!["Hyper+KeyZ".into()],
        ..s
    };
    assert!(bad.strokes().is_err(), "неизвестный модификатор — отказ");
}

#[cfg(target_os = "macos")]
#[test]
fn every_tray_state_has_a_black_template_icon_at_2x() {
    for status in ks_app::tray_art::ALL {
        let art = ks_app::tray_art::macos(status).expect("растр значка");
        assert_eq!((art.width, art.height), (36, 36), "{status:?}");
        assert!(
            art.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3] == [0, 0, 0] || p[3] == 0),
            "шаблон строки меню — только чёрный с альфой: {status:?}"
        );
        assert!(
            art.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
            "{status:?}"
        );
    }
}

#[test]
fn newer_release_is_found_in_the_github_list_and_the_asset_matches_this_os() {
    use ks_app::update::{is_newer, parse_latest};
    let json = r#"[{"tag_name": "v0.1.10", "assets": [
        {"browser_download_url": "https://x/KeySwitcher-0.1.10-macos-universal.dmg"},
        {"browser_download_url": "https://x/KeySwitcher-0.1.10-windows-x86_64.zip"},
        {"browser_download_url": "https://x/KeySwitcher-0.1.10-linux-x86_64.tar.gz"}]},
        {"tag_name": "v0.1.9", "assets": []}]"#;
    let release = parse_latest(json).expect("релиз");
    assert_eq!(release.version, "0.1.10");
    assert!(release.asset.expect("файл").contains("0.1.10"));
    assert!(
        is_newer("0.1.10", "0.1.9"),
        "сравнение по числам, не по строке"
    );
    assert!(!is_newer("0.1.1", "0.1.1"));
    assert!(!is_newer("0.1.0", "0.1.1"));
}
