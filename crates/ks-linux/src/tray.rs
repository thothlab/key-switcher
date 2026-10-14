//! Меню трея Linux в нейтральном виде. Строит его `ks-app` (`status::menu_entries` живёт там,
//! а `ks-app` зависит от `ks-linux`, не наоборот) и отдаёт `LinuxTray` замыканием; трей
//! (`ksni`, StatusNotifierItem по D-Bus) только показывает.

use ks_platform::{TrayCommand, TrayModel, TrayStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEntry {
    Item {
        label: String,
        /// `None` — пункт-надпись, недоступен для выбора.
        command: Option<TrayCommand>,
        /// `Some` — пункт с отметкой.
        checked: Option<bool>,
    },
    Separator,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayView {
    pub tooltip: String,
    pub entries: Vec<TrayEntry>,
    /// Значок состояния в нескольких размерах; пусто — значок из темы по [`icon_name`].
    pub icon: Vec<Pixmap>,
}

/// Растр значка RGBA 8 бит.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixmap {
    /// ARGB32 в сетевом порядке байт — формат `IconPixmap` StatusNotifierItem.
    pub fn argb(&self) -> Vec<u8> {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|[r, g, b, a]| [*a, *r, *g, *b])
            .collect()
    }
}

/// Модель → меню и подсказка; передаёт `ks-app`.
pub type MenuBuilder = Box<dyn Fn(&TrayModel) -> TrayView + Send + Sync>;

/// Имя значка из темы freedesktop по состоянию.
pub fn icon_name(status: TrayStatus) -> &'static str {
    match status {
        TrayStatus::Working => "input-keyboard",
        TrayStatus::AutocorrectOff => "input-keyboard-symbolic",
        TrayStatus::SecureInput => "changes-prevent",
        TrayStatus::NeedsPermissions | TrayStatus::NeedsLayouts | TrayStatus::ConfigError => {
            "dialog-warning"
        }
    }
}

/// Состояние требует внимания пользователя (StatusNotifierItem `NeedsAttention`).
pub fn needs_attention(status: TrayStatus) -> bool {
    matches!(
        status,
        TrayStatus::NeedsPermissions | TrayStatus::NeedsLayouts | TrayStatus::ConfigError
    )
}
