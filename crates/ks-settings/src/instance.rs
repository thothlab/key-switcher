//! Один экземпляр окна настроек. Первый процесс держит блокировку `settings.lock` в каталоге
//! настроек; следующий пишет имя раздела в `settings.raise` и выходит, а первый подхватывает файл
//! опросом: поднимает окно и переходит на раздел. Файлы не содержат ничего, кроме имени раздела.

use std::fs::File;
use std::io;
use std::path::Path;

use crate::Section;

pub const LOCK_FILE: &str = "settings.lock";
pub const RAISE_FILE: &str = "settings.raise";

/// Блокировка первого окна; снимается вместе с процессом.
pub struct Guard {
    _file: File,
}

pub enum Instance {
    First(Guard),
    Second,
}

/// Берёт блокировку окна в каталоге `dir`.
pub fn acquire(dir: &Path) -> io::Result<Instance> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => {
            // Просьба, оставшаяся от прошлого окна, не должна сменить раздел нового.
            let _ = std::fs::remove_file(dir.join(RAISE_FILE));
            Ok(Instance::First(Guard { _file: file }))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(Instance::Second),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Второй процесс: просит открытое окно подняться на разделе `section`.
pub fn raise(dir: &Path, section: Section) -> io::Result<()> {
    std::fs::write(dir.join(RAISE_FILE), section.name())
}

/// Первое окно: есть ли просьба подняться, и на каком разделе. Просьба снимается.
pub fn take_raise(dir: &Path) -> Option<Section> {
    let path = dir.join(RAISE_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(text.trim().parse().unwrap_or(Section::General))
}
