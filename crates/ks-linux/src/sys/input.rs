//! Чтение клавиатур и указателей evdev без захвата: поток на устройство, повторный обход
//! `/dev/input` подхватывает новые устройства. Своё uinput-устройство пропускается.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use evdev::{Device, KeyCode, LedCode};

use crate::VIRTUAL_KEYBOARD as DEVICE_NAME;
use crate::keymap::{EV_LED, LED_CAPSL};

pub enum Raw {
    Event {
        kind: u16,
        code: u16,
        value: i32,
    },
    /// Устройство пропало: зажатые на нём модификаторы неизвестны.
    Lost,
}

pub type Sink = Arc<dyn Fn(Raw) + Send + Sync>;
pub type Open = Arc<Mutex<HashSet<PathBuf>>>;

fn lock(open: &Open) -> std::sync::MutexGuard<'_, HashSet<PathBuf>> {
    open.lock().unwrap_or_else(|e| e.into_inner())
}

fn keyboard(dev: &Device) -> bool {
    dev.supported_keys().is_some_and(|keys| {
        [KeyCode::KEY_A, KeyCode::KEY_Z, KeyCode::KEY_SPACE]
            .iter()
            .all(|k| keys.contains(*k))
    })
}

fn pointer(dev: &Device) -> bool {
    dev.supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::BTN_LEFT) || keys.contains(KeyCode::BTN_TOUCH))
}

/// Открывает новые клавиатуры и указатели; `true` — какое-то устройство не открылось из-за прав.
pub fn scan(open: &Open, sink: &Sink) -> bool {
    let Ok(dir) = std::fs::read_dir("/dev/input") else {
        return false;
    };
    let mut denied = false;
    for entry in dir.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"));
        if !is_event || lock(open).contains(&path) {
            continue;
        }
        let mut dev = match Device::open(&path) {
            Ok(d) => d,
            Err(e) => {
                denied |= e.kind() == ErrorKind::PermissionDenied;
                continue;
            }
        };
        let is_keyboard = keyboard(&dev);
        if dev.name() == Some(DEVICE_NAME) || !(is_keyboard || pointer(&dev)) {
            continue;
        }
        let name = dev.name().unwrap_or("?").to_owned();
        if is_keyboard {
            let caps = dev
                .get_led_state()
                .is_ok_and(|leds| leds.contains(LedCode::LED_CAPSL));
            sink(Raw::Event {
                kind: EV_LED,
                code: LED_CAPSL,
                value: i32::from(caps),
            });
        }
        lock(open).insert(path.clone());
        let (open, sink) = (open.clone(), sink.clone());
        let spawned = std::thread::Builder::new()
            .name("ks-evdev".to_owned())
            .spawn(move || {
                log::info!("читаю устройство ввода: {name}");
                loop {
                    match dev.fetch_events() {
                        Ok(events) => {
                            for ev in events {
                                sink(Raw::Event {
                                    kind: ev.event_type().0,
                                    code: ev.code(),
                                    value: ev.value(),
                                });
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => {}
                        Err(e) => {
                            log::info!("устройство ввода {name} отключено: {e}");
                            break;
                        }
                    }
                }
                lock(&open).remove(&path);
                sink(Raw::Lost);
            });
        if let Err(e) = spawned {
            log::warn!("поток чтения устройства не запущен: {e}");
        }
    }
    denied
}

/// Хотя бы одно устройство `/dev/input/event*` открывается на чтение.
pub fn readable() -> bool {
    std::fs::read_dir("/dev/input").is_ok_and(|dir| {
        dir.flatten().any(|e| {
            let p = e.path();
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
                && std::fs::File::open(&p).is_ok()
        })
    })
}
