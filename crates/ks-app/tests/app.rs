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
fn without_ru_en_pair_the_icon_says_layouts_are_needed() {
    let b = FakeBackend {
        accessibility: true,
        with_russian: false,
    };
    let status = tray_status(&b.permissions(), true, false, &EngineStatus::NeedsLayouts);
    assert_eq!(status, TrayStatus::NeedsLayouts);
    assert_eq!(tooltip(status), "KeySwitcher — Нужны раскладки RU и EN");
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
    let path = std::env::temp_dir().join(format!("ks-test-{}.sock", std::process::id()));
    ks_app::ipc::serve(&path, |cmd, out| {
        let _ = writeln!(out, "команда: {cmd}");
        let _ = writeln!(out, "клавиатура отслеживается");
    })
    .expect("сокет поднят");
    let mut lines = Vec::new();
    ks_app::ipc::ask(&path, "status", |l| lines.push(l.to_owned())).expect("агент ответил");
    assert_eq!(lines, ["команда: status", "клавиатура отслеживается"]);
    let second = ks_app::ipc::serve(&path, |_, _| {});
    assert_eq!(second.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
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
