//! Бэкенд macOS: активный `CGEventTap` на своём потоке, исполнение замен синтетическими
//! событиями, таблицы раскладок из `UCKeyTranslate`, права TCC. Модель исполнения (удержание,
//! исполнитель, срок) — общая, в `ks_platform::intercept`; здесь — перевод событий (`tap`), коды
//! (`keycode`), таблицы (`layouts`) без системных вызовов и системная часть `sys` (только macOS).

pub mod keycode;
pub mod layouts;
pub mod tap;

#[cfg(target_os = "macos")]
mod sys;
#[cfg(target_os = "macos")]
pub use sys::{MacBackend, probe, system};

#[cfg(not(target_os = "macos"))]
mod stub;
#[cfg(not(target_os = "macos"))]
pub use stub::MacBackend;
