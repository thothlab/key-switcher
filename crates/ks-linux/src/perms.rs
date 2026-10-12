//! Права Linux: чтение `/dev/input/event*` и запись в `/dev/uinput`. Без них — одна команда
//! разовой настройки (скрипт udev-правила). Проверку доступа делает `sys`, здесь — что показать.

use std::path::{Path, PathBuf};

use ks_platform::{PermissionKind, PermissionStatus};

pub const SCRIPT_NAME: &str = "install-udev-rule.sh";
/// Путь к скрипту в исходниках — когда рядом с исполняемым файлом его нет.
pub const REPO_SCRIPT: &str = "scripts/linux/install-udev-rule.sh";

/// Итог проверки доступа.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access {
    /// Хотя бы одно устройство `/dev/input/event*` открывается на чтение.
    pub input: bool,
    /// `/dev/uinput` открывается на запись.
    pub uinput: bool,
}

/// Где может лежать скрипт при установке: рядом с исполняемым файлом или в
/// `../share/keyswitcher/`.
pub fn script_candidates(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    vec![
        dir.join(SCRIPT_NAME),
        dir.join("../share/keyswitcher").join(SCRIPT_NAME),
    ]
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Команда настройки: `sudo sh <скрипт>`; скрипт не найден — путь в исходниках.
pub fn fix_command(script: Option<&Path>) -> String {
    let path = script
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| REPO_SCRIPT.to_owned());
    format!("sudo sh {}", shell_quote(&path))
}

/// Статусы для раздела «Разрешения»: у обоих прав одна и та же команда настройки.
pub fn statuses(access: Access, fix: &str) -> Vec<PermissionStatus> {
    vec![
        PermissionStatus {
            kind: PermissionKind::LinuxInputDevices,
            granted: access.input,
            explanation: "Чтение клавиатур и мыши (/dev/input/event*): без него KeySwitcher не \
                          видит нажатий и кликов. Набранный текст никуда не сохраняется."
                .to_owned(),
            fix_hint: fix.to_owned(),
        },
        PermissionStatus {
            kind: PermissionKind::LinuxUinput,
            granted: access.uinput,
            explanation: "Виртуальная клавиатура (/dev/uinput): через неё стирается и \
                          перепечатывается слово. Без неё исправления выключены."
                .to_owned(),
            fix_hint: fix.to_owned(),
        },
    ]
}
