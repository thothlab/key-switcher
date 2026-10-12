//! Бэкенд Windows: хук `WH_KEYBOARD_LL` на выделенном потоке со своим циклом сообщений,
//! исполнение замен через `SendInput`, смена раскладки окна с фокусом
//! (`WM_INPUTLANGCHANGEREQUEST`), таблицы раскладок из `ToUnicodeEx`, буфер обмена, автозапуск
//! через ключ `Run`. Модель исполнения (удержание, исполнитель, срок) — общая, в
//! `ks_platform::intercept`. Модули без системных вызовов (`keycode`, `tap`, `send`, `layouts`,
//! `klc`, `apps`, `clip`) проверяются тестами на любой ОС; системная часть `sys` — только Windows.

pub mod apps;
pub mod clip;
pub mod keycode;
pub mod klc;
pub mod layouts;
pub mod send;
pub mod tap;

#[cfg(windows)]
mod sys;
#[cfg(windows)]
pub use sys::WindowsBackend;

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use stub::WindowsBackend;
