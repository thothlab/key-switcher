//! KeySwitcher.app на диске: путь бандла, запись автозапуска (LaunchAgent) и перезапуск после
//! выхода. Без системных вызовов macOS — только файлы и процессы, поэтому проверяется тестами.
//!
//! Автозапуск — файл `~/Library/LaunchAgents/com.keyswitcher.app.plist`: `launchd` читает его при
//! входе в систему и запускает исполняемый файл из `.app` (`RunAtLoad`). `KeepAlive` нет — иначе
//! «Выход» из меню перезапускал бы агент. `launchctl bootstrap` не зовётся: он поднял бы второй
//! агент рядом с работающим. SMAppService не взят: обвязки ServiceManagement нет среди
//! зависимостей проекта, а LaunchAgent прямо назван в приёмке PRD (история 37).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const BUNDLE_ID: &str = "com.keyswitcher.app";

/// `.app`, внутри которого лежит исполняемый файл (`X.app/Contents/MacOS/exe`).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

/// Бандл текущего процесса; `None` — запущено не из `.app` (`cargo run`, тесты).
pub fn current_bundle() -> Option<PathBuf> {
    bundle_of(&std::env::current_exe().ok()?)
}

/// Путь записи автозапуска в домашнем каталоге `home`.
pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{BUNDLE_ID}.plist"))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Содержимое записи автозапуска для исполняемого файла `exe`.
pub fn launch_agent_plist(exe: &Path) -> String {
    let exe = xml_escape(&exe.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{BUNDLE_ID}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>LimitLoadToSessionType</key><string>Aqua</string>
    <key>ProcessType</key><string>Interactive</string>
    <key>AbandonProcessGroup</key><true/>
    <key>AssociatedBundleIdentifiers</key>
    <array>
        <string>{BUNDLE_ID}</string>
    </array>
</dict>
</plist>
"#
    )
}

/// Ставит (`on`) или убирает запись автозапуска `file`. Файл с тем же содержимым не
/// переписывается. `true` — файл на диске изменился.
pub fn apply_launch_agent(file: &Path, exe: &Path, on: bool) -> io::Result<bool> {
    if !on {
        return match std::fs::remove_file(file) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        };
    }
    let text = launch_agent_plist(exe);
    if std::fs::read_to_string(file).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("plist.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, file)?;
    Ok(true)
}

/// Новый экземпляр бандла (`open -n -g`) для перезапуска. Его запускает LaunchServices, а не
/// этот процесс: потомков приложения macOS снимает вместе с ним, и ждущий выхода `sh` не доживал
/// до `open`. Новый агент сам ждёт, пока прежний освободит сокет.
#[cfg(unix)]
pub fn open_new_instance(bundle: &Path) -> io::Result<()> {
    let status = Command::new("/usr/bin/open")
        .args(["-n", "-g"])
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("open завершился: {status}")))
    }
}
