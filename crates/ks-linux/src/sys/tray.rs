//! Трей Linux: StatusNotifierItem по D-Bus через `ksni`, без GTK. Меню строит `ks-app`
//! (замыкание [`MenuBuilder`]), команды идут в канал [`TrayHost::commands`].

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use ks_platform::{BackendError, TrayCommand, TrayHost, TrayModel, TrayStatus};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};

use crate::tray::{MenuBuilder, TrayEntry, TrayView, icon_name, needs_attention};

struct Sni {
    view: TrayView,
    status: TrayStatus,
    tx: Sender<TrayCommand>,
}

fn activate(cmd: Option<TrayCommand>) -> Box<dyn Fn(&mut Sni) + Send> {
    Box::new(move |t: &mut Sni| {
        if let Some(cmd) = &cmd {
            let _ = t.tx.send(cmd.clone());
        }
    })
}

impl ksni::Tray for Sni {
    fn id(&self) -> String {
        "keyswitcher".to_owned()
    }

    fn title(&self) -> String {
        "KeySwitcher".to_owned()
    }

    fn icon_name(&self) -> String {
        icon_name(self.status).to_owned()
    }

    fn status(&self) -> ksni::Status {
        if needs_attention(self.status) {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.view.tooltip.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.view
            .entries
            .iter()
            .map(|entry| match entry {
                TrayEntry::Separator => MenuItem::Separator,
                TrayEntry::Item {
                    label,
                    command,
                    checked: Some(checked),
                } => CheckmarkItem {
                    label: label.clone(),
                    enabled: command.is_some(),
                    checked: *checked,
                    activate: activate(command.clone()),
                    ..Default::default()
                }
                .into(),
                TrayEntry::Item {
                    label,
                    command,
                    checked: None,
                } => StandardItem {
                    label: label.clone(),
                    enabled: command.is_some(),
                    activate: activate(command.clone()),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}

pub struct LinuxTray {
    handle: Handle<Sni>,
    menu: MenuBuilder,
    last: Mutex<Option<TrayModel>>,
    commands: Mutex<Option<Receiver<TrayCommand>>>,
}

impl LinuxTray {
    /// Регистрирует значок на сессионной шине. Ошибка — нет D-Bus или хоста трея; агент
    /// работает и без значка.
    pub fn new(menu: MenuBuilder) -> Result<LinuxTray, BackendError> {
        let (tx, rx) = mpsc::channel();
        let sni = Sni {
            view: TrayView::default(),
            status: TrayStatus::Working,
            tx,
        };
        let handle = sni
            .spawn()
            .map_err(|e| BackendError::Os(format!("трей StatusNotifierItem: {e}")))?;
        Ok(LinuxTray {
            handle,
            menu,
            last: Mutex::new(None),
            commands: Mutex::new(Some(rx)),
        })
    }
}

impl TrayHost for LinuxTray {
    fn update(&self, model: &TrayModel) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(model) {
            return;
        }
        let view = (self.menu)(model);
        let status = model.status;
        self.handle.update(move |t: &mut Sni| {
            t.view = view;
            t.status = status;
        });
        *last = Some(model.clone());
    }

    /// Отдаёт приёмник команд один раз; повторный вызов получает пустой канал.
    fn commands(&self) -> Receiver<TrayCommand> {
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_else(|| mpsc::channel().1)
    }
}
