//! Автозапуск по XDG Autostart: `~/.config/autostart/keyswitcher.desktop`.

use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "keyswitcher.desktop";

/// Каталог автозапуска: `$XDG_CONFIG_HOME/autostart` (если путь абсолютный), иначе
/// `$HOME/.config/autostart`.
pub fn dir(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|p| p.starts_with('/')) {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(home.filter(|h| !h.is_empty())?).join(".config"),
    };
    Some(base.join("autostart"))
}

/// Значение `Exec=` для пути к программе по Desktop Entry Specification: зарезервированные
/// символы — в двойных кавычках с экранированием, `%` удваивается, обратная косая черта
/// экранируется ещё раз на уровне строки файла.
pub fn exec_value(path: &Path) -> String {
    let raw = path.display().to_string().replace('%', "%%");
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    let arg = if raw.chars().any(reserved) {
        let mut q = String::from("\"");
        for c in raw.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        q
    } else {
        raw
    };
    arg.replace('\\', "\\\\")
}

/// Содержимое файла автозапуска.
pub fn entry(exec: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=KeySwitcher\n\
         Comment=Автопереключение раскладки RU/EN\n\
         Exec={}\n\
         Icon=input-keyboard\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_value(exec)
    )
}

/// Файл не выключен ключами `Hidden=true` или `X-GNOME-Autostart-enabled=false`.
pub fn is_enabled(content: &str) -> bool {
    !content.lines().map(str::trim).any(|l| {
        l.eq_ignore_ascii_case("Hidden=true")
            || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}
