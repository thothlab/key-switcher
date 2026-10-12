//! Запросы к окружению: список раскладок, активная раскладка, смена раскладки, окно с фокусом.
//! X11 — `x11rb`; KDE — D-Bus `org.kde.keyboard` (таймаут вызова на соединении); Sway и
//! Hyprland — их сокеты с таймаутом из выданного бюджета. Точный срок на вызов (остаток бюджета
//! смены раскладки) даёт `switch::with_deadline` у вызывающего.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ks_core::LayoutId;
use ks_platform::exec::SWITCH_TIMEOUT_MS;

use super::x11::X11;
use crate::VIRTUAL_KEYBOARD;
use crate::desktop::{Environment, Session, SwitchMethod};
use crate::ipc::{Focus, hyprland, kde, sway};
use crate::layouts::{LayoutSpec, XkbCatalog};

/// Срок вызова вне смены раскладки (опрос состояния, список раскладок).
pub const IO_TIMEOUT: Duration = Duration::from_millis(400);

pub struct Desk {
    pub env: Environment,
    catalog: XkbCatalog,
    x11: Option<X11>,
    kde: Option<zbus::blocking::Connection>,
    sway: Option<PathBuf>,
    hypr: Option<PathBuf>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn stream(path: &Path, budget: Duration) -> Option<UnixStream> {
    let timeout = budget.min(IO_TIMEOUT);
    if timeout.is_zero() {
        return None;
    }
    let s = UnixStream::connect(path).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    s.set_write_timeout(Some(timeout)).ok()?;
    Some(s)
}

fn kde_connection() -> Option<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()
        .and_then(|b| {
            b.method_timeout(Duration::from_millis(SWITCH_TIMEOUT_MS))
                .build()
        })
        .map_err(|e| log::warn!("сессионная шина D-Bus недоступна: {e}"))
        .ok()
}

impl Desk {
    pub fn new(env: Environment) -> Desk {
        let method = env.switch;
        let x11 = if env.session == Session::X11 {
            X11::connect()
        } else {
            None
        };
        let kde = if method == Some(SwitchMethod::KdeDbus) {
            kde_connection()
        } else {
            None
        };
        let sway = if method == Some(SwitchMethod::SwayIpc) {
            var("SWAYSOCK").map(PathBuf::from)
        } else {
            None
        };
        let hypr = if method == Some(SwitchMethod::HyprlandIpc) {
            var("HYPRLAND_INSTANCE_SIGNATURE").and_then(|sig| {
                hyprland::socket_paths(var("XDG_RUNTIME_DIR").as_deref(), &sig)
                    .into_iter()
                    .find(|p| p.exists())
            })
        } else {
            None
        };
        Desk {
            env,
            catalog: XkbCatalog::system(),
            x11,
            kde,
            sway,
            hypr,
        }
    }

    fn sway(&self, kind: u32, payload: &str, budget: Duration) -> Option<String> {
        let mut s = stream(self.sway.as_deref()?, budget)?;
        s.write_all(&sway::frame(kind, payload)).ok()?;
        let mut head = [0u8; sway::HEADER_LEN];
        s.read_exact(&mut head).ok()?;
        let (len, _) = sway::header(&head)?;
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).ok()?;
        String::from_utf8(body).ok()
    }

    fn hypr(&self, request: &str, budget: Duration) -> Option<String> {
        let mut s = stream(self.hypr.as_deref()?, budget)?;
        s.write_all(request.as_bytes()).ok()?;
        let mut out = String::new();
        s.read_to_string(&mut out).ok()?;
        Some(out)
    }

    fn kde_call(&self, method: &str) -> Option<zbus::Message> {
        self.kde
            .as_ref()?
            .call_method(
                Some(kde::SERVICE),
                kde::PATH,
                Some(kde::INTERFACE),
                method,
                &(),
            )
            .ok()
    }

    fn kde_set(&self, index: u32) -> bool {
        let Some(conn) = &self.kde else {
            return false;
        };
        conn.call_method(
            Some(kde::SERVICE),
            kde::PATH,
            Some(kde::INTERFACE),
            "setLayout",
            &(index,),
        )
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        .unwrap_or(false)
    }

    fn sway_inputs(&self, budget: Duration) -> Option<(Vec<LayoutSpec>, usize)> {
        let reply = self.sway(sway::GET_INPUTS, "", budget)?;
        sway::keyboard_layouts(&reply, VIRTUAL_KEYBOARD, &self.catalog)
    }

    fn hypr_devices(&self, budget: Duration) -> Option<(Vec<LayoutSpec>, String)> {
        let reply = self.hypr(hyprland::DEVICES, budget)?;
        hyprland::keyboard_layouts(&reply, VIRTUAL_KEYBOARD, &self.catalog)
    }

    /// Раскладки окружения по порядку (индекс — аргумент смены раскладки).
    pub fn specs(&self) -> Vec<LayoutSpec> {
        let specs = match self.env.switch {
            Some(SwitchMethod::KdeDbus) => self
                .kde_call("getLayoutsList")
                .and_then(|m| m.body().deserialize::<Vec<(String, String, String)>>().ok())
                .map(|l| kde::layouts(&l, &self.catalog))
                .unwrap_or_default(),
            Some(SwitchMethod::SwayIpc) => self
                .sway_inputs(IO_TIMEOUT)
                .map(|(s, _)| s)
                .unwrap_or_default(),
            Some(SwitchMethod::HyprlandIpc) => self
                .hypr_devices(IO_TIMEOUT)
                .map(|(s, _)| s)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let specs = if specs.is_empty() {
            self.x11.as_ref().map(X11::layout_specs).unwrap_or_default()
        } else {
            specs
        };
        specs
            .into_iter()
            .map(|s| self.catalog.describe(s))
            .collect()
    }

    /// Позиция активной раскладки в `specs`.
    pub fn current_index(&self, specs: &[LayoutSpec], budget: Duration) -> Option<usize> {
        match self.env.switch {
            Some(SwitchMethod::KdeDbus) => self
                .kde_call("getLayout")
                .and_then(|m| m.body().deserialize::<u32>().ok())
                .map(|i| i as usize),
            Some(SwitchMethod::SwayIpc) => self.sway_inputs(budget).map(|(_, i)| i),
            Some(SwitchMethod::HyprlandIpc) => {
                let (_, active) = self.hypr_devices(budget)?;
                specs.iter().position(|s| s.description == active)
            }
            Some(SwitchMethod::XkbGroup) | None => self.x11.as_ref().and_then(X11::group),
        }
    }

    /// Активная раскладка.
    pub fn current(&self, specs: &[LayoutSpec], budget: Duration) -> Option<LayoutId> {
        let index = self.current_index(specs, budget)?;
        specs.get(index).map(LayoutSpec::id)
    }

    /// Команда смены раскладки на позицию `index`; ждать применения — дело вызывающего.
    pub fn switch_to(&self, index: usize, budget: Duration) -> bool {
        match self.env.switch {
            Some(SwitchMethod::KdeDbus) => u32::try_from(index).is_ok_and(|i| self.kde_set(i)),
            Some(SwitchMethod::SwayIpc) => self
                .sway(sway::RUN_COMMAND, &sway::switch_command(index), budget)
                .is_some_and(|r| sway::command_ok(&r)),
            Some(SwitchMethod::HyprlandIpc) => self
                .hypr(&hyprland::switch_command(index), budget)
                .is_some_and(|r| hyprland::command_ok(&r)),
            Some(SwitchMethod::XkbGroup) => self.x11.as_ref().is_some_and(|x| x.lock_group(index)),
            None => false,
        }
    }

    /// Окно с фокусом; на Wayland в KDE и GNOME — `None`.
    pub fn focused(&self) -> Option<Focus> {
        match self.env.switch {
            Some(SwitchMethod::SwayIpc) => self
                .sway(sway::GET_TREE, "", IO_TIMEOUT)
                .and_then(|r| sway::focused_app(&r)),
            Some(SwitchMethod::HyprlandIpc) => self
                .hypr(hyprland::ACTIVE_WINDOW, IO_TIMEOUT)
                .and_then(|r| hyprland::focused_app(&r)),
            _ => self.x11.as_ref()?.active_class().map(|c| (c.clone(), c)),
        }
    }
}
