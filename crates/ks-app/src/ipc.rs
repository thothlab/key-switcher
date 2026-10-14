//! Локальный сокет агента: `--status` и `--selftest` спрашивают работающий агент, чтобы
//! состояние и права были состоянием его процесса, а не процесса команды (урок sotto).
//! Протокол: клиент шлёт одну строку-команду (`status`, `selftest`), агент пишет текст
//! и закрывает соединение.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

/// Сколько сервер ждёт строку-команду от клиента.
pub const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Поднимает сокет; `Err(AddrInUse)` — на нём отвечает другой агент и не ушёл за `wait`
/// (при перезапуске новый экземпляр стартует, пока прежний ещё выходит).
pub fn serve(
    path: &Path,
    wait: Duration,
    respond: impl Fn(&str, &mut UnixStream) + Send + 'static,
) -> io::Result<()> {
    let deadline = std::time::Instant::now() + wait;
    while UnixStream::connect(path).is_ok() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "агент KeySwitcher уже запущен",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::Builder::new()
        .name("ks-ipc".to_owned())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                // Клиент, не приславший строку, не держит сервер дольше срока.
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_ok() {
                    respond(line.trim(), &mut stream);
                }
                let _ = stream.flush();
            }
        })?;
    Ok(())
}

/// Отправляет команду агенту и отдаёт строки ответа по мере прихода.
pub fn ask(path: &Path, command: &str, mut on_line: impl FnMut(&str)) -> io::Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    writeln!(stream, "{command}")?;
    for line in BufReader::new(stream).lines() {
        on_line(&line?);
    }
    Ok(())
}
