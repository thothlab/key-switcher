//! Системная часть бэкенда Windows: Win32 через крейт `windows` (хуки, `SendInput`, раскладки,
//! буфер обмена, реестр).
#![allow(unsafe_code)] // всё FFI бэкенда живёт здесь; модули логики остаются без unsafe

mod backend;
mod system;

pub use backend::WindowsBackend;
pub use system::{small_icon_px, taskbar_light};
