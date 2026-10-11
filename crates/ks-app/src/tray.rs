//! Трей macOS/Windows на `tray-icon`/`muda`: иконка-шаблон по состоянию, подсказка
//! «KeySwitcher — …» и меню из [`menu_entries`]. Живёт на главном потоке внутри цикла `tao`;
//! меню пересобирается, только когда модель изменилась.

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
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(model) {
            return;
        }
        let status_changed = last.as_ref().map(|m| m.status) != Some(model.status);
        if status_changed {
            let _ = self.icon.set_tooltip(Some(tooltip(model.status)));
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

/// Иконка 32×32, чёрная с альфой (шаблон для тёмной и светлой строки меню): клавиша и знак
/// состояния. Работает — «K»; автоматика выключена — «K» перечёркнута; безопасный ввод —
/// пауза «‖»; нужны разрешения — «!»; нужны раскладки — «K» в пунктирной рамке; ошибка
/// настроек — «!» в пунктирной рамке.
fn icon(status: TrayStatus) -> anyhow::Result<Icon> {
    const N: u32 = 32;
    let seg = |px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    let k = |px, py| {
        seg(px, py, (12.0, 10.0), (12.0, 22.0)) < 1.3
            || seg(px, py, (12.5, 16.5), (20.0, 10.0)) < 1.3
            || seg(px, py, (14.5, 15.0), (20.5, 22.0)) < 1.3
    };
    let bang = |px, py| {
        seg(px, py, (16.0, 9.5), (16.0, 18.0)) < 1.5
            || seg(px, py, (16.0, 21.5), (16.0, 21.6)) < 1.7
    };
    let pause = |px, py| {
        seg(px, py, (12.5, 10.0), (12.5, 22.0)) < 1.6
            || seg(px, py, (19.5, 10.0), (19.5, 22.0)) < 1.6
    };
    let slash = |px, py| seg(px, py, (6.0, 25.0), (26.0, 7.0)) < 1.2;
    let dashed = matches!(status, TrayStatus::NeedsLayouts | TrayStatus::ConfigError);
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let in_box = (3.0..29.0).contains(&px) && (5.0..27.0).contains(&py);
            let inner = (5.0..27.0).contains(&px) && (7.0..25.0).contains(&py);
            let gap = dashed
                && ((x / 3) % 2 == 1 && !(7..25).contains(&y)
                    || (y / 3) % 2 == 1 && !(5..27).contains(&x));
            let border = in_box && !inner && !gap;
            let glyph = match status {
                TrayStatus::Working | TrayStatus::NeedsLayouts => k(px, py),
                TrayStatus::AutocorrectOff => k(px, py) || slash(px, py),
                TrayStatus::SecureInput => pause(px, py),
                TrayStatus::NeedsPermissions | TrayStatus::ConfigError => bang(px, py),
            };
            let alpha = if border || glyph { 255 } else { 0 };
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Ok(Icon::from_rgba(rgba, N, N)?)
}
