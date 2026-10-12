//! Команды и ответы IPC окружений: Sway (i3-ipc поверх `$SWAYSOCK`), Hyprland (сокет
//! `.socket.sock`), KDE Plasma (D-Bus `org.kde.keyboard`). Только формирование и разбор —
//! сокеты и шина открываются в `sys`.

use std::path::PathBuf;

use ks_core::Lang;

use crate::json::Json;
use crate::layouts::{LayoutSpec, XkbCatalog, specs_from_lists};

/// Одно и то же устройство: Sway отдаёт имя как есть, Hyprland — в нижнем регистре с `-`
/// вместо пробелов.
pub fn same_device(name: &str, wanted: &str) -> bool {
    let norm = |s: &str| s.trim().to_lowercase().replace(' ', "-");
    norm(name) == norm(wanted)
}

/// В списке есть пара «латинская + русская».
pub fn has_pair(specs: &[LayoutSpec]) -> bool {
    specs.iter().any(|s| s.lang() == Lang::En) && specs.iter().any(|s| s.lang() == Lang::Ru)
}

/// Приложение в фокусе: идентификатор (`app_id` или класс окна) и имя для меню.
pub type Focus = (String, String);

pub mod sway {
    use super::*;

    pub const MAGIC: &[u8; 6] = b"i3-ipc";
    /// Длина заголовка: магия, длина тела (u32 LE), тип (u32 LE).
    pub const HEADER_LEN: usize = 14;
    pub const RUN_COMMAND: u32 = 0;
    pub const GET_TREE: u32 = 4;
    pub const GET_INPUTS: u32 = 100;

    /// Сообщение i3-ipc: заголовок + тело.
    pub fn frame(kind: u32, payload: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(payload.as_bytes());
        out
    }

    /// Заголовок ответа → (длина тела, тип); чужая магия — `None`.
    pub fn header(raw: &[u8]) -> Option<(usize, u32)> {
        if raw.len() < HEADER_LEN || &raw[..6] != MAGIC {
            return None;
        }
        let len = u32::from_le_bytes(raw[6..10].try_into().ok()?) as usize;
        let kind = u32::from_le_bytes(raw[10..14].try_into().ok()?);
        Some((len, kind))
    }

    /// Команда смены раскладки на всех клавиатурах (`index` — позиция в списке раскладок).
    pub fn switch_command(index: usize) -> String {
        format!("input type:keyboard xkb_switch_layout {index}")
    }

    /// Ответ на `RUN_COMMAND`: `[{"success": true}, …]` — все команды прошли.
    pub fn command_ok(reply: &str) -> bool {
        let Some(json) = Json::parse(reply) else {
            return false;
        };
        let items = json.items();
        !items.is_empty()
            && items
                .iter()
                .all(|r| r.get("success").and_then(Json::as_bool) == Some(true))
    }

    /// Ответ на `GET_INPUTS` → раскладки и индекс активной у физической клавиатуры с парой
    /// RU+EN (своя виртуальная `skip_name` исключена); пары нет ни у одной — первая клавиатура
    /// с раскладками. Sway отдаёт описания («English (US)»), коды — из каталога.
    pub fn keyboard_layouts(
        reply: &str,
        skip_name: &str,
        catalog: &XkbCatalog,
    ) -> Option<(Vec<LayoutSpec>, usize)> {
        let json = Json::parse(reply)?;
        let keyboards: Vec<(Vec<LayoutSpec>, usize)> = json
            .items()
            .iter()
            .filter(|input| {
                input.get("type").and_then(Json::as_str) == Some("keyboard")
                    && !input
                        .get("name")
                        .and_then(Json::as_str)
                        .is_some_and(|n| same_device(n, skip_name))
            })
            .filter_map(|input| {
                let specs: Vec<LayoutSpec> = input
                    .get("xkb_layout_names")?
                    .items()
                    .iter()
                    .filter_map(Json::as_str)
                    .map(|d| catalog.resolve(d))
                    .collect();
                let active = input
                    .get("xkb_active_layout_index")
                    .and_then(Json::as_u64)
                    .unwrap_or(0) as usize;
                (!specs.is_empty()).then_some((specs, active))
            })
            .collect();
        let pick = keyboards
            .iter()
            .position(|(specs, _)| has_pair(specs))
            .unwrap_or(0);
        keyboards.into_iter().nth(pick)
    }

    /// Ответ на `GET_TREE` → окно с фокусом: `app_id` (Wayland) или класс окна (XWayland).
    pub fn focused_app(reply: &str) -> Option<Focus> {
        fn walk(node: &Json) -> Option<Focus> {
            if node.get("focused").and_then(Json::as_bool) == Some(true) {
                let id = node
                    .get("app_id")
                    .and_then(Json::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        node.get("window_properties")?
                            .get("class")
                            .and_then(Json::as_str)
                    })?;
                return Some((id.to_owned(), id.to_owned()));
            }
            ["nodes", "floating_nodes"]
                .iter()
                .filter_map(|k| node.get(k))
                .flat_map(Json::items)
                .find_map(walk)
        }
        walk(&Json::parse(reply)?)
    }
}

pub mod hyprland {
    use super::*;

    /// Запрос списка устройств в JSON.
    pub const DEVICES: &str = "j/devices";
    /// Запрос окна с фокусом в JSON.
    pub const ACTIVE_WINDOW: &str = "j/activewindow";

    // TODO(prd): пути сокета и формат ответов Hyprland (`main`, `active_keymap`) не сверены с
    // живым Hyprland — проверить в VM (docs/build-linux.md).
    /// Где искать сокет команд: с Hyprland 0.40 — `$XDG_RUNTIME_DIR/hypr/<сигнатура>/`,
    /// раньше — `/tmp/hypr/<сигнатура>/`.
    pub fn socket_paths(runtime_dir: Option<&str>, signature: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(dir) = runtime_dir.filter(|d| !d.is_empty()) {
            out.push(
                PathBuf::from(dir)
                    .join("hypr")
                    .join(signature)
                    .join(".socket.sock"),
            );
        }
        out.push(
            PathBuf::from("/tmp/hypr")
                .join(signature)
                .join(".socket.sock"),
        );
        out
    }

    /// Команда смены раскладки на всех клавиатурах.
    // TODO(prd): аргумент устройства `all` не проверен вживую (Hyprland wiki называет `all` и
    // `current`).
    pub fn switch_command(index: usize) -> String {
        format!("switchxkblayout all {index}")
    }

    pub fn command_ok(reply: &str) -> bool {
        reply.trim() == "ok"
    }

    /// Ответ `j/devices` → раскладки и описание активной у физической клавиатуры с парой
    /// RU+EN (главная — в первую очередь; своя виртуальная `skip_name` исключена); пары нет ни
    /// у одной — главная или первая.
    pub fn keyboard_layouts(
        reply: &str,
        skip_name: &str,
        catalog: &XkbCatalog,
    ) -> Option<(Vec<LayoutSpec>, String)> {
        let json = Json::parse(reply)?;
        let keyboards: Vec<(bool, Vec<LayoutSpec>, String)> = json
            .get("keyboards")?
            .items()
            .iter()
            .filter(|k| {
                !k.get("name")
                    .and_then(Json::as_str)
                    .is_some_and(|n| same_device(n, skip_name))
            })
            .map(|kb| {
                let layouts = kb.get("layout").and_then(Json::as_str).unwrap_or("");
                let variants = kb.get("variant").and_then(Json::as_str).unwrap_or("");
                let specs: Vec<LayoutSpec> = specs_from_lists(layouts, variants)
                    .into_iter()
                    .map(|s| catalog.describe(s))
                    .collect();
                let active = kb
                    .get("active_keymap")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_owned();
                let main = kb.get("main").and_then(Json::as_bool) == Some(true);
                (main, specs, active)
            })
            .collect();
        let pick = keyboards
            .iter()
            .position(|(main, specs, _)| *main && has_pair(specs))
            .or_else(|| keyboards.iter().position(|(_, specs, _)| has_pair(specs)))
            .or_else(|| keyboards.iter().position(|(main, _, _)| *main))
            .unwrap_or(0);
        keyboards
            .into_iter()
            .nth(pick)
            .map(|(_, specs, active)| (specs, active))
    }

    /// Ответ `j/activewindow` → класс окна; пустой объект — фокуса нет.
    pub fn focused_app(reply: &str) -> Option<Focus> {
        let json = Json::parse(reply)?;
        let class = json
            .get("class")
            .and_then(Json::as_str)
            .filter(|s| !s.is_empty())?;
        Some((class.to_owned(), class.to_owned()))
    }
}

pub mod kde {
    use super::*;

    pub const SERVICE: &str = "org.kde.keyboard";
    pub const PATH: &str = "/Layouts";
    pub const INTERFACE: &str = "org.kde.KeyboardLayouts";

    // TODO(prd): сигнатура `getLayoutsList` (`a(sss)`) взята по исходникам KWin, вживую не
    // сверена.
    /// Ответ `getLayoutsList` (`a(sss)`: короткое имя, отображаемое имя, полное описание) →
    /// раскладки по порядку; индекс в списке — аргумент `setLayout`. Вариант — по описанию
    /// из каталога XKB («Russian (Windows)» → `ru(winkeys)`), если его код совпадает.
    pub fn layouts(list: &[(String, String, String)], catalog: &XkbCatalog) -> Vec<LayoutSpec> {
        list.iter()
            .map(
                |(short, _display, long)| match catalog.by_description(long) {
                    Some(spec) if spec.layout == *short => spec,
                    _ => {
                        let mut spec = LayoutSpec::new(short, "");
                        spec.description = long.trim().to_owned();
                        spec
                    }
                },
            )
            .collect()
    }
}
