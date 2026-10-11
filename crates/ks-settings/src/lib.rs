//! Окно настроек KeySwitcher (egui/eframe, отдельный процесс `keyswitcher --settings`).
//! Заготовка: окно добавляет задача 07.

use std::path::Path;

/// Раздел окна для `--section <имя>`. TODO(task 07): состав разделов.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Hotkeys,
    Apps,
    Words,
}

pub fn run(section: Option<Section>, config_path: &Path) -> anyhow::Result<()> {
    let _ = (section, config_path);
    anyhow::bail!("окно настроек ещё не реализовано")
}
