//! Состояние для иконки и для `--status`: чистые функции над `Backend` и статусом движка.

use std::fmt::Write as _;

use ks_core::EngineStatus;
use ks_platform::{Backend, PermissionKind, PermissionStatus, TrayStatus};

pub fn tray_status(
    permissions: &[PermissionStatus],
    capturing: bool,
    secure_input: bool,
    engine: &EngineStatus,
) -> TrayStatus {
    if !capturing || permissions.iter().any(|p| !p.granted) {
        return TrayStatus::NeedsPermissions;
    }
    match engine {
        EngineStatus::ConfigError(_) => TrayStatus::ConfigError,
        EngineStatus::NeedsLayouts => TrayStatus::NeedsLayouts,
        _ if secure_input => TrayStatus::SecureInput,
        EngineStatus::AutocorrectOff => TrayStatus::AutocorrectOff,
        EngineStatus::Working => TrayStatus::Working,
    }
}

/// Подсказка иконки и первая строка меню.
pub fn tooltip(status: TrayStatus) -> String {
    let state = match status {
        TrayStatus::Working => "работает",
        TrayStatus::AutocorrectOff => "автоисправление выключено",
        TrayStatus::SecureInput => "пауза: безопасный ввод",
        TrayStatus::NeedsPermissions => "нужны разрешения",
        TrayStatus::NeedsLayouts => "Нужны раскладки RU и EN",
        TrayStatus::ConfigError => "ошибка в файле настроек",
    };
    format!("KeySwitcher — {state}")
}

fn permission_name(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::InputMonitoring => "Мониторинг ввода (Input Monitoring)",
        PermissionKind::Accessibility => "Универсальный доступ (Accessibility)",
        PermissionKind::LinuxInputDevices => "доступ к /dev/input",
        PermissionKind::LinuxUinput => "доступ к /dev/uinput",
    }
}

/// Откуда взято состояние.
pub enum Source {
    /// Ответил работающий агент из своего процесса.
    Agent { pid: u32 },
    /// Агент не запущен: видно то, что видно этой команде.
    Local,
}

/// Текст `keyswitcher --status`. Набранных символов здесь нет и быть не может.
pub fn report(backend: &dyn Backend, engine: Option<&EngineStatus>, source: Source) -> String {
    let health = backend.health();
    let permissions = backend.permissions();
    let mut out = format!("KeySwitcher {}\n", env!("CARGO_PKG_VERSION"));
    match source {
        Source::Agent { pid } => {
            let _ = writeln!(out, "источник: работающий агент (pid {pid})");
        }
        Source::Local => {
            out += "источник: агент не запущен — показано то, что видно этой команде; права ниже \
                    относятся к процессу, из которого она запущена (например, Терминалу), а не к \
                    KeySwitcher.app\n";
        }
    }
    out += if health.capturing {
        "клавиатура отслеживается\n"
    } else {
        "клавиатура не отслеживается\n"
    };
    out += "права:\n";
    for p in &permissions {
        if p.granted {
            let _ = writeln!(out, "  {}: выдано", permission_name(p.kind));
        } else {
            let _ = writeln!(
                out,
                "  {}: НЕ выдано — {} {}",
                permission_name(p.kind),
                p.explanation,
                p.fix_hint
            );
        }
    }
    let _ = writeln!(
        out,
        "безопасный ввод: {}",
        if health.secure_input {
            "да (другое приложение держит Secure Input, перехват на паузе)"
        } else {
            "нет"
        }
    );
    out += "раскладки:\n";
    for name in &health.layouts {
        let _ = writeln!(out, "  {name}");
    }
    let layouts = backend.layouts();
    if layouts.pair().is_none() {
        out += "  нет пары EN + RU — Нужны раскладки RU и EN\n";
    }
    if let Some(engine) = engine {
        let state = match engine {
            EngineStatus::Working => "работает".to_owned(),
            EngineStatus::AutocorrectOff => "выключено".to_owned(),
            EngineStatus::NeedsLayouts => "стоит: Нужны раскладки RU и EN".to_owned(),
            EngineStatus::ConfigError(e) => format!("ошибка в файле настроек: {e}"),
        };
        let _ = writeln!(out, "автоисправление: {state}");
    }
    if !health.detail.is_empty() {
        let _ = writeln!(out, "{}", health.detail);
    }
    out
}
