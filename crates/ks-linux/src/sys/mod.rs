//! Системная часть бэкенда Linux: evdev/uinput, X11 (`x11rb`), D-Bus (`zbus`), IPC Sway и
//! Hyprland, таблицы xkbcommon (`dlopen`), трей `ksni`.

mod backend;
mod desk;
mod input;
mod tray;
mod uinput;
mod x11;
mod xkb;

pub use backend::LinuxBackend;
pub use tray::LinuxTray;
