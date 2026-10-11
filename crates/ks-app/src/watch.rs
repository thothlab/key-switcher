//! Слежение за файлом настроек (`notify`). Смотрим каталог, а не файл: `Config::save` и
//! редакторы пишут временный файл и переименовывают его, слежение за самим файлом умерло бы
//! после первой записи. События одного сохранения склеиваются (редактор может обрезать файл и
//! писать его в несколько приёмов).

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};

pub type Watcher = notify::RecommendedWatcher;

/// Сколько ждать после первого события, прежде чем перечитать файл.
const SETTLE: Duration = Duration::from_millis(200);

/// Зовёт `on_change` после изменения файла `path`; слежение живёт, пока жив `Watcher`.
pub fn watch(path: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Watcher> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(notify::Error::io)?;
    let name = path.file_name().map(|n| n.to_owned());
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res
            && event.paths.iter().any(|p| p.file_name() == name.as_deref())
        {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    std::thread::Builder::new()
        .name("ks-config-watch".to_owned())
        .spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(SETTLE);
                while rx.try_recv().is_ok() {}
                on_change();
            }
        })
        .map_err(notify::Error::io)?;
    Ok(watcher)
}
