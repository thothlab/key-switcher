//! Трей macOS/Windows на `tray-icon`/`muda`: значок по состоянию ([`crate::tray_art`]), подсказка
//! «KeySwitcher — …» и меню из [`menu_entries`]. Живёт на главном потоке внутри цикла `tao`;
//! меню пересобирается, только когда модель изменилась, значок — когда сменилось состояние
//! (на Windows ещё и тема панели задач).

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use ks_platform::{TrayCommand, TrayHost, TrayModel, TrayStatus};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::status::{MenuEntry, menu_entries, tooltip};

type Commands = Arc<Mutex<HashMap<MenuId, TrayCommand>>>;

pub struct TrayIconHost {
    icon: TrayIcon,
    ids: Commands,
    last: Mutex<Option<TrayModel>>,
    /// Состояние и тема, под которые нарисован значок.
    drawn: Mutex<Option<(TrayStatus, bool)>>,
    commands: Mutex<Option<Receiver<TrayCommand>>>,
}

impl TrayIconHost {
    /// `wake` будит цикл событий после выбора пункта меню.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> anyhow::Result<TrayIconHost> {
        let (tx, rx) = mpsc::channel();
        let ids: Commands = Arc::default();
        let lookup = ids.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let cmd = lookup
                .lock()
                .ok()
                .and_then(|ids| ids.get(&event.id).cloned());
            if let Some(cmd) = cmd {
                let _ = tx.send(cmd);
            }
            wake();
        }));
        let builder = TrayIconBuilder::new().with_tooltip("KeySwitcher");
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon(TrayStatus::Working)?);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon(TrayStatus::Working)?);
        let icon = builder.build()?;
        Ok(TrayIconHost {
            icon,
            ids,
            last: Mutex::new(None),
            drawn: Mutex::new(Some((TrayStatus::Working, light_taskbar()))),
            commands: Mutex::new(Some(rx)),
        })
    }

    fn rebuild(&self, model: &TrayModel) -> anyhow::Result<()> {
        let menu = Menu::new();
        let mut ids = HashMap::new();
        for entry in menu_entries(model) {
            match entry {
                MenuEntry::Separator => menu.append(&PredefinedMenuItem::separator())?,
                MenuEntry::Item {
                    label,
                    command,
                    checked: Some(checked),
                } => {
                    let item = CheckMenuItem::new(&label, command.is_some(), checked, None);
                    if let Some(cmd) = command {
                        ids.insert(item.id().clone(), cmd);
                    }
                    menu.append(&item)?;
                }
                MenuEntry::Item {
                    label,
                    command,
                    checked: None,
                } => {
                    let item = MenuItem::new(&label, command.is_some(), None);
                    if let Some(cmd) = command {
                        ids.insert(item.id().clone(), cmd);
                    }
                    menu.append(&item)?;
                }
            }
        }
        if let Ok(mut map) = self.ids.lock() {
            *map = ids;
        }
        self.icon.set_menu(Some(Box::new(menu)));
        Ok(())
    }
}

impl TrayHost for TrayIconHost {
    fn update(&self, model: &TrayModel) {
        let wanted = (model.status, light_taskbar());
        let mut drawn = self.drawn.lock().unwrap_or_else(|e| e.into_inner());
        if *drawn != Some(wanted) {
            *drawn = Some(wanted);
            match icon(model.status) {
                #[cfg(target_os = "macos")]
                Ok(img) => {
                    let _ = self.icon.set_icon_templated(Some(img));
                }
                #[cfg(not(target_os = "macos"))]
                Ok(img) => {
                    let _ = self.icon.set_icon(Some(img));
                }
                Err(e) => log::warn!("иконка состояния не построена: {e}"),
            }
        }
        drop(drawn);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(model) {
            return;
        }
        if last.as_ref().map(|m| m.status) != Some(model.status) {
            let _ = self.icon.set_tooltip(Some(tooltip(model.status)));
        }
        if let Err(e) = self.rebuild(model) {
            log::warn!("меню не собрано: {e}");
        }
        *last = Some(model.clone());
    }

    /// Отдаёт приёмник команд один раз; повторный вызов получает пустой канал.
    fn commands(&self) -> Receiver<TrayCommand> {
        let taken = self.commands.lock().ok().and_then(|mut c| c.take());
        taken.unwrap_or_else(|| mpsc::channel().1)
    }
}

/// Панель задач Windows светлая: значок рисуется тёмным. На macOS значок — шаблон, тема не важна.
fn light_taskbar() -> bool {
    #[cfg(windows)]
    {
        ks_windows::taskbar_light()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Значок состояния (`design/icons/tray`): macOS — чёрный шаблон 36×36, Windows — вариант под
/// тему панели задач и размер малого значка.
fn icon(status: TrayStatus) -> anyhow::Result<Icon> {
    #[cfg(target_os = "macos")]
    let art = crate::tray_art::macos(status);
    #[cfg(windows)]
    let art = crate::tray_art::windows(status, light_taskbar(), ks_windows::small_icon_px());
    let art =
        art.ok_or_else(|| anyhow::anyhow!("нет растра значка {}", crate::tray_art::stem(status)))?;
    Ok(Icon::from_rgba(art.rgba, art.width, art.height)?)
}
