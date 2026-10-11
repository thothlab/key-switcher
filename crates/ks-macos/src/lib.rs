//! Бэкенд macOS: активный `CGEventTap` на своём потоке, исполнение замен синтетическими
//! событиями, таблицы раскладок из `UCKeyTranslate`, права TCC. Логика колбэка и исполнителя
//! вынесена в модули без системных вызовов (`tap`, `exec`, `keycode`, `layouts`) и проверяется
//! тестами на любой ОС; системная часть — `sys` (только macOS).

pub mod exec;
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
