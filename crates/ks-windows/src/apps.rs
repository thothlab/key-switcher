//! Приложения и автозапуск без системных вызовов: `AppId` по пути exe, где не работает вставка
//! через буфер обмена, команда для ключа `Run`.

use ks_core::AppId;

/// Здесь Ctrl+C/Ctrl+V значат другое (прерывание, вставка с подтверждением), и выделение через
/// буфер обмена не конвертируется. Окно консоли conhost может принадлежать процессу оболочки,
/// поэтому в списке и оболочки.
pub const NO_PASTE: [&str; 8] = [
    "windowsterminal.exe",
    "openconsole.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wsl.exe",
    "mintty.exe",
];

/// Значение в `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "KeySwitcher";

/// `AppId` Windows — имя exe в нижнем регистре (`C:\Windows\notepad.exe` → `notepad.exe`).
pub fn app_id(exe_path: &str) -> AppId {
    let name = exe_path.rsplit(['\\', '/']).next().unwrap_or(exe_path);
    AppId(name.to_lowercase())
}

/// Вставка через буфер обмена работает в приложении.
pub fn paste_usable(app: &AppId) -> bool {
    !NO_PASTE.contains(&app.0.as_str())
}

/// Команда автозапуска: путь в кавычках (в пути бывают пробелы).
pub fn run_command(exe_path: &str) -> String {
    format!("\"{exe_path}\"")
}
