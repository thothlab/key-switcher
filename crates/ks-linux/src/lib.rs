//! Бэкенд Linux: чтение клавиатур и мыши через evdev без захвата, ввод через виртуальную
//! клавиатуру uinput — одинаково для X11 и Wayland; смена раскладки в X11 (XKB), KDE Plasma
//! (D-Bus), Sway и Hyprland (IPC); таблицы раскладок из xkbcommon; трей StatusNotifierItem
//! (`ksni`) без GTK; автозапуск через XDG autostart.
//!
//! Чистая логика (коды клавиш, окружение, команды IPC, раскладки, поправки модели исполнения
//! без захвата, права, автозапуск) — в модулях верхнего уровня и проверяется на любой ОС;
//! системная часть `sys` собирается только под Linux.

/// Имя своей виртуальной клавиатуры uinput: по нему она исключается из чтения и из списков
/// клавиатур окружения.
pub const VIRTUAL_KEYBOARD: &str = "KeySwitcher virtual keyboard";

pub mod autostart;
pub mod desktop;
pub mod guard;
pub mod ipc;
pub mod json;
pub mod keymap;
pub mod layouts;
pub mod nograb;
pub mod perms;
pub mod selection;
pub mod switch;
pub mod tap;
pub mod tray;
pub mod typing;
pub mod vkbd;

#[cfg(target_os = "linux")]
mod sys;
#[cfg(target_os = "linux")]
pub use sys::{LinuxBackend, LinuxTray};

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
pub use stub::LinuxBackend;
