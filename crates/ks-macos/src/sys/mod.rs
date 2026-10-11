//! Системная часть бэкенда macOS: FFI к HIToolbox (TIS, `UCKeyTranslate`), CoreGraphics
//! (`CGEventTap`, `CGEventPost`, права), ApplicationServices (AX).
#![allow(unsafe_code)] // всё FFI бэкенда живёт здесь; модули логики остаются без unsafe

mod backend;
mod clipboard;
mod ffi;
mod post;
pub mod probe;
pub mod system;
mod tap_thread;

pub use backend::MacBackend;
