//! Виртуальная клавиатура uinput: стирание и перепечатка физическими кодами — одинаково для X11
//! и Wayland. Что нажато на устройстве, ведёт `vkbd::Keys` и отпускает всё сам. Своё устройство
//! из чтения исключается по имени.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, KeyCode};
use ks_core::{LayoutId, LayoutSet, PhysicalKey};
use ks_platform::exec::{Chord, KEY_PAUSE_MS, Poster};

use crate::keymap::{self, KEY_C, KEY_V, KEYS};
use crate::tap::LinuxEvent;
use crate::vkbd::{Emit, Keys};
use crate::{VIRTUAL_KEYBOARD, typing};

/// Композитору нужно время, чтобы увидеть новое устройство: события сразу после создания
/// теряются.
const SETTLE: Duration = Duration::from_millis(300);

/// Актуальная раскладка окружения (с коротким сроком); `None` — не узнать.
pub type LayoutProbe = Box<dyn Fn() -> Option<LayoutId> + Send + Sync>;

struct Dev(VirtualDevice);

impl Emit for Dev {
    fn emit(&mut self, code: u16, value: i32) -> io::Result<()> {
        self.0.emit(&[*evdev::KeyEvent::new(KeyCode(code), value)])
    }
}

pub struct Uinput {
    keys: Mutex<Keys<Dev>>,
    layouts: Arc<RwLock<LayoutSet>>,
    current: Arc<RwLock<LayoutId>>,
    probe: LayoutProbe,
}

impl Uinput {
    pub fn new(
        layouts: Arc<RwLock<LayoutSet>>,
        current: Arc<RwLock<LayoutId>>,
        probe: LayoutProbe,
    ) -> Uinput {
        Uinput {
            keys: Mutex::new(Keys::default()),
            layouts,
            current,
            probe,
        }
    }

    fn keys(&self) -> MutexGuard<'_, Keys<Dev>> {
        self.keys.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Устройство создано.
    pub fn ready(&self) -> bool {
        self.keys().ready()
    }

    /// Создаёт устройство, если его ещё нет; `false` — нет доступа к `/dev/uinput`.
    pub fn ensure(&self) -> bool {
        if self.ready() {
            return true;
        }
        let codes: AttributeSet<KeyCode> = KEYS.iter().map(|(c, _)| KeyCode(*c)).collect();
        let built = VirtualDevice::builder()
            .and_then(|b| b.name(VIRTUAL_KEYBOARD).with_keys(&codes))
            .and_then(|b| b.build());
        match built {
            Ok(d) => {
                self.keys().attach(Dev(d));
                std::thread::sleep(SETTLE);
                log::info!("виртуальная клавиатура uinput создана");
                true
            }
            Err(e) => {
                log::debug!("виртуальная клавиатура не создана: {e}");
                false
            }
        }
    }

    /// Отпускает всё, что устройство нажало (конец задания исполнителя).
    pub fn release_all(&self) {
        self.keys().release_all();
    }
}

impl Poster<LinuxEvent> for Uinput {
    fn can_type(&self, key: PhysicalKey) -> bool {
        keymap::to_evdev(key).is_some()
    }

    fn key(&self, key: PhysicalKey, down: bool, shift: bool) {
        if let Some(code) = keymap::to_evdev(key) {
            self.keys().key(code, down, shift);
        }
    }

    fn chord(&self, chord: Chord) {
        self.keys().chord(match chord {
            Chord::Copy => KEY_C,
            Chord::Paste => KEY_V,
        });
    }

    /// У uinput нет ввода Unicode: текст печатается клавишами раскладки, которая его набирает,
    /// по **актуальной** раскладке окружения (переключение могло примениться после срока).
    fn unicode(&self, text: &str) {
        let actual = (self.probe)();
        if let Some(id) = &actual {
            *self.current.write().unwrap_or_else(|e| e.into_inner()) = id.clone();
        }
        let strokes = {
            let set = self.layouts.read().unwrap_or_else(|e| e.into_inner());
            let current = actual.unwrap_or_else(|| {
                self.current
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            });
            typing::strokes_for(text, set.get(&current), &set)
        };
        let Some(strokes) = strokes else {
            log::warn!("замена не набирается клавишами включённых раскладок: не напечатана");
            return;
        };
        for s in strokes {
            self.key(s.key, true, s.shift);
            self.key(s.key, false, s.shift);
            std::thread::sleep(Duration::from_millis(KEY_PAUSE_MS));
        }
    }

    /// Без захвата нажатие пользователя уже дошло до приложения: повторять его нельзя.
    fn replay(&self, _native: &LinuxEvent) {}

    fn pause_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}
