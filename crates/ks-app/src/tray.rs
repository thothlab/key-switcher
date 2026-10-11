//! Трей macOS/Windows на `tray-icon`: иконка-шаблон, подсказка «KeySwitcher — …»,
//! строка состояния и «Выход». Живёт на главном потоке внутри цикла `tao`.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use ks_platform::{TrayCommand, TrayHost, TrayModel};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::status::tooltip;

pub struct TrayIconHost {
    icon: TrayIcon,
    status_item: MenuItem,
    commands: Mutex<Option<Receiver<TrayCommand>>>,
}

impl TrayIconHost {
    /// `wake` будит цикл событий после выбора пункта меню.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> anyhow::Result<TrayIconHost> {
        let menu = Menu::new();
        let status_item = MenuItem::new("KeySwitcher", false, None);
        let quit = MenuItem::new("Выход", true, None);
        menu.append_items(&[&status_item, &PredefinedMenuItem::separator(), &quit])?;
        let (tx, rx) = mpsc::channel();
        let quit_id = quit.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id {
                let _ = tx.send(TrayCommand::Quit);
            }
            wake();
        }));
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("KeySwitcher");
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon()?);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon()?);
        let icon = builder.build()?;
        Ok(TrayIconHost {
            icon,
            status_item,
            commands: Mutex::new(Some(rx)),
        })
    }
}

impl TrayHost for TrayIconHost {
    fn update(&self, model: &TrayModel) {
        let text = tooltip(model.status);
        let _ = self.icon.set_tooltip(Some(&text));
        self.status_item.set_text(&text);
    }

    /// Отдаёт приёмник команд один раз; повторный вызов получает пустой канал.
    fn commands(&self) -> Receiver<TrayCommand> {
        let taken = self.commands.lock().ok().and_then(|mut c| c.take());
        taken.unwrap_or_else(|| mpsc::channel().1)
    }
}

/// Иконка 32×32: клавиша с буквой K, чёрная с альфой (шаблон для тёмной и светлой строки меню).
fn icon() -> anyhow::Result<Icon> {
    const N: u32 = 32;
    let seg = |px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let in_box = (3.0..29.0).contains(&px) && (5.0..27.0).contains(&py);
            let inner = (5.0..27.0).contains(&px) && (7.0..25.0).contains(&py);
            let border = in_box && !inner;
            let k = seg(px, py, (12.0, 10.0), (12.0, 22.0)) < 1.3
                || seg(px, py, (12.5, 16.5), (20.0, 10.0)) < 1.3
                || seg(px, py, (14.5, 15.0), (20.5, 22.0)) < 1.3;
            let alpha = if border || k { 255 } else { 0 };
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Ok(Icon::from_rgba(rgba, N, N)?)
}
