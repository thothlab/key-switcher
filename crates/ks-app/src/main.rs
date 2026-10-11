//! KeySwitcher — агент автопереключения раскладки RU↔EN.
//! Сейчас заготовка: печатает версию и состояние бэкенда своей ОС.

use std::sync::Arc;

use ks_platform::Backend;

#[cfg(target_os = "macos")]
fn backend() -> Arc<dyn Backend> {
    Arc::new(ks_macos::MacBackend::new())
}

#[cfg(windows)]
fn backend() -> Arc<dyn Backend> {
    Arc::new(ks_windows::WindowsBackend::new())
}

#[cfg(target_os = "linux")]
fn backend() -> Arc<dyn Backend> {
    Arc::new(ks_linux::LinuxBackend::new())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
compile_error!("KeySwitcher собирается только под macOS, Windows и Linux");

fn main() {
    println!("KeySwitcher {}", env!("CARGO_PKG_VERSION"));
    let health = backend().health();
    println!(
        "backend: capturing={} ({})",
        health.capturing, health.detail
    );
}
