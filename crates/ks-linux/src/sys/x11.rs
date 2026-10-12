//! X11 через `x11rb` (чистый Rust): раскладки из `_XKB_RULES_NAMES`, группа XKB, окно с фокусом
//! (`_NET_ACTIVE_WINDOW` → `WM_CLASS`), выделение PRIMARY.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, CreateWindowAux, Window, WindowClass};
use x11rb::rust_connection::RustConnection;

use crate::layouts::{LayoutSpec, parse_rules_names};
use crate::selection::primary_belongs_to_focus;

/// Сколько ждать ответа владельца PRIMARY.
const SELECTION_TIMEOUT: Duration = Duration::from_millis(300);

fn atom(conn: &RustConnection, name: &str) -> Option<u32> {
    conn.intern_atom(false, name.as_bytes())
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
}

pub struct X11 {
    conn: RustConnection,
    root: Window,
    active: u32,
    rules: u32,
    xkb: bool,
}

impl X11 {
    pub fn connect() -> Option<X11> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let xkb = conn
            .xkb_use_extension(1, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.supported);
        let active = atom(&conn, "_NET_ACTIVE_WINDOW")?;
        let rules = atom(&conn, "_XKB_RULES_NAMES")?;
        Some(X11 {
            conn,
            root,
            active,
            rules,
            xkb,
        })
    }

    fn property(&self, win: Window, prop: u32, kind: impl Into<u32>) -> Option<Vec<u8>> {
        let reply = self
            .conn
            .get_property(false, win, prop, kind.into(), 0, 1 << 16)
            .ok()?
            .reply()
            .ok()?;
        Some(reply.value)
    }

    pub fn layout_specs(&self) -> Vec<LayoutSpec> {
        self.property(self.root, self.rules, AtomEnum::STRING)
            .map(|raw| parse_rules_names(&raw))
            .unwrap_or_default()
    }

    /// Активная группа XKB основной клавиатуры.
    pub fn group(&self) -> Option<usize> {
        if !self.xkb {
            return None;
        }
        let state = self
            .conn
            .xkb_get_state(xkb::ID::USE_CORE_KBD.into())
            .ok()?
            .reply()
            .ok()?;
        Some(usize::from(u8::from(state.group)))
    }

    /// `XkbLockGroup`: закрепить группу `index` основной клавиатуры.
    pub fn lock_group(&self, index: usize) -> bool {
        let Ok(group) = u8::try_from(index) else {
            return false;
        };
        self.xkb
            && self
                .conn
                .xkb_latch_lock_state(
                    xkb::ID::USE_CORE_KBD.into(),
                    0u16.into(),
                    0u16.into(),
                    true,
                    xkb::Group::from(group),
                    0u16.into(),
                    false,
                    0,
                )
                .is_ok()
            && self.conn.flush().is_ok()
    }

    /// Класс окна с фокусом (вторая строка `WM_CLASS`).
    pub fn active_class(&self) -> Option<String> {
        let raw = self.property(self.root, self.active, AtomEnum::WINDOW)?;
        let win = u32::from_ne_bytes(raw.get(..4)?.try_into().ok()?);
        if win == 0 {
            return None;
        }
        let class = self.property(win, AtomEnum::WM_CLASS.into(), AtomEnum::STRING)?;
        let mut parts = class.split(|b| *b == 0).filter(|p| !p.is_empty());
        let instance = parts.next();
        let class = parts.next().or(instance)?;
        Some(String::from_utf8_lossy(class).into_owned())
    }
}

/// Текст выделения PRIMARY в UTF-8; `None` — выделения нет, владелец не ответил за 300 мс или
/// ответ большой (INCR не поддерживается). Своё соединение: ожидание события не мешает
/// остальным запросам. Содержимое не логируется.
pub fn primary_text() -> Option<String> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let id_mask = conn.setup().resource_id_mask;
    let primary: u32 = AtomEnum::PRIMARY.into();
    let owner = conn.get_selection_owner(primary).ok()?.reply().ok()?.owner;
    let active = atom(&conn, "_NET_ACTIVE_WINDOW")?;
    let focused = conn
        .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()
        .and_then(|mut v| v.next())
        .unwrap_or(0);
    if !primary_belongs_to_focus(owner, focused, id_mask) {
        log::info!("выделение PRIMARY принадлежит не окну в фокусе: не конвертирую");
        return None;
    }
    let utf8 = atom(&conn, "UTF8_STRING")?;
    let prop = atom(&conn, "KS_SELECTION")?;
    let win = conn.generate_id().ok()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .ok()?;
    conn.convert_selection(win, primary, utf8, prop, x11rb::CURRENT_TIME)
        .ok()?;
    conn.flush().ok()?;
    let deadline = Instant::now() + SELECTION_TIMEOUT;
    let text = loop {
        match conn.poll_for_event() {
            Ok(Some(Event::SelectionNotify(ev))) if ev.requestor == win => {
                if ev.property == x11rb::NONE {
                    break None;
                }
                let reply = conn
                    .get_property(true, win, prop, AtomEnum::ANY, 0, 1 << 20)
                    .ok()
                    .and_then(|c| c.reply().ok());
                break reply
                    .filter(|r| r.type_ == utf8)
                    .and_then(|r| String::from_utf8(r.value).ok());
            }
            Ok(Some(_)) => {}
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) | Err(_) => break None,
        }
    };
    let _ = conn.destroy_window(win);
    let _ = conn.flush();
    text
}
