//! Перевод событий хука `WH_KEYBOARD_LL`/`WH_MOUSE_LL` в нейтральный [`InputEvent`] общей модели
//! исполнения (`ks_platform::intercept`). Без системных вызовов — проверяется тестом на любой ОС;
//! колбэк (`sys::hook_thread`) только заполняет [`RawEvent`].
//!
//! У события LL-хука нет флагов модификаторов, поэтому их состояние ведёт [`Tracker`] по самим
//! событиям — своим и чужим, ровно как его видит система. Это состояние кладётся в нативное
//! событие ([`WinEvent::mods`]) на момент захвата: придержанная на время замены клавиша
//! воспроизводится с ним, а не с тем, что зажато физически к моменту воспроизведения.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use ks_core::{KeyDir, KeyEvent, Modifiers, PhysicalKey as K, PhysicalKey};
use ks_platform::intercept::{InputEvent, InputKind};

use crate::keycode;

pub use ks_platform::clock_ms;

/// `dwExtraInfo` своих событий замены: движок их не видит.
pub const MARK_OWN: usize = 0x4B53_0001;

/// Бит скан-кода фантомного левого Ctrl, который Windows шлёт перед правым Alt в раскладках с
/// AltGr (скан-код 0x21D).
const PHANTOM_CTRL: u32 = 0x200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawKind {
    KeyDown,
    KeyUp,
    MouseDown,
    /// Хук был снят системой (таймаут) и поставлен снова.
    Disabled,
    Other,
}

/// Поля `KBDLLHOOKSTRUCT`, нужные логике. Намеренно без `Debug`: код клавиши — набранный символ.
#[derive(Clone, Copy)]
pub struct RawEvent {
    pub kind: RawKind,
    pub vk: u16,
    /// `scanCode` как есть (с битом фантомного Ctrl).
    pub scan: u32,
    /// `LLKHF_EXTENDED`.
    pub extended: bool,
    /// `LLKHF_INJECTED`.
    pub injected: bool,
    pub extra: usize,
    pub time_ms: u64,
}

impl RawEvent {
    pub fn key(kind: RawKind, vk: u16, scan: u32, extended: bool) -> RawEvent {
        RawEvent {
            kind,
            vk,
            scan,
            extended,
            injected: false,
            extra: 0,
            time_ms: 0,
        }
    }

    pub fn other(kind: RawKind, time_ms: u64) -> RawEvent {
        RawEvent {
            time_ms,
            ..RawEvent::key(kind, 0, 0, false)
        }
    }
}

/// Зажатые модификаторы с учётом стороны.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModState(pub u8);

impl ModState {
    pub const LSHIFT: u8 = 0x01;
    pub const RSHIFT: u8 = 0x02;
    pub const LCTRL: u8 = 0x04;
    pub const RCTRL: u8 = 0x08;
    pub const LALT: u8 = 0x10;
    pub const RALT: u8 = 0x20;
    pub const LWIN: u8 = 0x40;
    pub const RWIN: u8 = 0x80;
    pub const SHIFT: u8 = Self::LSHIFT | Self::RSHIFT;
    pub const CTRL: u8 = Self::LCTRL | Self::RCTRL;
    pub const ALT: u8 = Self::LALT | Self::RALT;
    pub const WIN: u8 = Self::LWIN | Self::RWIN;
    /// Семейства модификаторов: сторона по умолчанию — левая.
    pub const FAMILIES: [u8; 4] = [Self::SHIFT, Self::CTRL, Self::ALT, Self::WIN];

    /// Бит клавиши-модификатора; CapsLock и прочие — `None`.
    pub fn bit(key: PhysicalKey) -> Option<u8> {
        Some(match key {
            K::ShiftLeft => Self::LSHIFT,
            K::ShiftRight => Self::RSHIFT,
            K::ControlLeft => Self::LCTRL,
            K::ControlRight => Self::RCTRL,
            K::AltLeft => Self::LALT,
            K::AltRight => Self::RALT,
            K::MetaLeft => Self::LWIN,
            K::MetaRight => Self::RWIN,
            _ => return None,
        })
    }

    /// Клавиша по одиночному биту.
    pub fn key(bit: u8) -> Option<PhysicalKey> {
        Some(match bit {
            Self::LSHIFT => K::ShiftLeft,
            Self::RSHIFT => K::ShiftRight,
            Self::LCTRL => K::ControlLeft,
            Self::RCTRL => K::ControlRight,
            Self::LALT => K::AltLeft,
            Self::RALT => K::AltRight,
            Self::LWIN => K::MetaLeft,
            Self::RWIN => K::MetaRight,
            _ => return None,
        })
    }

    pub fn with(self, bits: u8) -> ModState {
        ModState(self.0 | bits)
    }

    pub fn modifiers(self, caps_lock: bool) -> Modifiers {
        Modifiers {
            shift: self.0 & Self::SHIFT != 0,
            caps_lock,
            alt: self.0 & Self::ALT != 0,
            ctrl: self.0 & Self::CTRL != 0,
            meta: self.0 & Self::WIN != 0,
        }
    }
}

/// Модификаторы, которые уже применил хук: [`Tracker`] публикует их после каждого события,
/// исполнитель (`send::Typist`) берёт отсюда «зажатое сейчас». `GetAsyncKeyState` для этого не
/// годится: LL-колбэк вызывается до обновления асинхронного состояния, и задание, поставленное
/// на отпускании Shift/Alt (⇧⇧, ⌥⇧), видело бы отпущенный модификатор зажатым.
#[derive(Debug, Default)]
pub struct HeldMods(AtomicU8);

impl HeldMods {
    pub fn get(&self) -> ModState {
        ModState(self.0.load(Ordering::Acquire))
    }

    pub fn set(&self, mods: ModState) {
        self.0.store(mods.0, Ordering::Release);
    }
}

/// Событие Windows для воспроизведения придержанного нажатия. Без `Debug`.
#[derive(Clone, Copy, Default)]
pub struct WinEvent {
    pub vk: u16,
    /// Скан-код с битом [`keycode::EXT`].
    pub scan: u16,
    pub down: bool,
    /// Модификаторы, зажатые в момент захвата.
    pub mods: ModState,
}

/// Реальное состояние модификаторов от ОС (`GetAsyncKeyState`) — для пересинхронизации, когда
/// хук мог пропустить ввод. Зовётся вне колбэка хука.
#[derive(Clone)]
pub struct KeyState(pub Arc<dyn Fn() -> ModState + Send + Sync>);

impl std::fmt::Debug for KeyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyState")
    }
}

/// Состояние клавиатуры **пользователя** по событиям хука: модификаторы, CapsLock, зажатые
/// клавиши (для автоповтора). Свои события (`MARK_OWN`) его не меняют — поэтому оно не зависит от
/// того, когда хук увидит пачку `SendInput`.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    mods: ModState,
    caps: bool,
    down: [u64; 4],
    published: Option<Arc<HeldMods>>,
    key_state: Option<KeyState>,
}

impl Tracker {
    /// `caps_on` — CapsLock включён на момент установки хука.
    pub fn new(caps_on: bool) -> Tracker {
        Tracker {
            caps: caps_on,
            ..Tracker::default()
        }
    }

    /// Публиковать состояние модификаторов пользователя для исполнителя после каждого события.
    pub fn with_published(mut self, held: Arc<HeldMods>) -> Tracker {
        held.set(self.mods);
        self.published = Some(held);
        self
    }

    /// Откуда брать реальное состояние при пересинхронизации (`Disabled`, простой хука).
    pub fn with_key_state(mut self, source: KeyState) -> Tracker {
        self.key_state = Some(source);
        self
    }

    pub fn mods(&self) -> ModState {
        self.mods
    }

    fn held(&self, vk: u16) -> bool {
        let vk = usize::from(vk & 0xFF);
        self.down[vk / 64] & (1 << (vk % 64)) != 0
    }

    fn set_held(&mut self, vk: u16, on: bool) {
        let vk = usize::from(vk & 0xFF);
        if on {
            self.down[vk / 64] |= 1 << (vk % 64);
        } else {
            self.down[vk / 64] &= !(1 << (vk % 64));
        }
    }

    fn publish(&self) {
        if let Some(held) = &self.published {
            held.set(self.mods);
        }
    }

    /// Хук мог пропустить ввод (снят системой, защищённый рабочий стол Win+L/Ctrl+Alt+Del):
    /// модификаторы берутся из реального состояния `real`, пропущенные отпускания забываются.
    pub fn resync(&mut self, real: ModState) {
        self.mods = real;
        self.down = [0; 4];
        for bit in (0..8).map(|i| 1u8 << i).filter(|b| real.0 & b != 0) {
            if let Some(vk) = ModState::key(bit).and_then(keycode::vk_of) {
                self.set_held(vk, true);
            }
        }
        self.publish();
    }

    /// Пересинхронизация по [`KeyState`], если он задан.
    pub fn resync_from_os(&mut self) {
        if let Some(source) = self.key_state.clone() {
            self.resync((source.0)());
        }
    }

    /// `RawEvent` → событие общей модели. Модификаторы и CapsLock — отдельные `Modifier` со
    /// стороной; автоповтор модификатора движку не отдаётся (как `flagsChanged` на macOS); свои
    /// события (`LLKHF_INJECTED` + [`MARK_OWN`]) помечены `own` и состояние не меняют. `Disabled`
    /// пересинхронизирует состояние с ОС.
    pub fn input_event(&mut self, ev: &RawEvent) -> InputEvent<WinEvent> {
        let down = match ev.kind {
            RawKind::KeyDown => true,
            RawKind::KeyUp => false,
            other => {
                let kind = match other {
                    RawKind::MouseDown => InputKind::MouseDown,
                    RawKind::Disabled => {
                        self.resync_from_os();
                        InputKind::Disabled
                    }
                    _ => InputKind::Other,
                };
                return InputEvent::new(kind, None, ev.time_ms, WinEvent::default());
            }
        };
        let own = ev.injected && ev.extra == MARK_OWN;
        let key = keycode::to_physical(ev.vk, ev.scan, ev.extended);
        let native = |mods| WinEvent {
            vk: ev.vk,
            scan: if ev.vk == keycode::VK_PACKET {
                // Символ чужого KEYEVENTF_UNICODE: единица UTF-16 целиком.
                ev.scan as u16
            } else {
                keycode::scan_code(ev.scan, ev.extended)
            },
            down,
            mods,
        };
        if own || (ev.scan & PHANTOM_CTRL != 0 && key == K::ControlLeft) {
            // Своё событие или фантомный Ctrl от AltGr: не нажатие пользователя, состояние не
            // меняет, движку не отдаётся.
            let kind = if ModState::bit(key).is_some() {
                InputKind::Modifier
            } else {
                InputKind::Key
            };
            let mut out = InputEvent::new(kind, None, ev.time_ms, native(self.mods));
            out.own = own;
            return out;
        }
        let was_held = self.held(ev.vk);
        self.set_held(ev.vk, down);
        let repeat = down && was_held;
        let modifier = ModState::bit(key);
        if let Some(bit) = modifier {
            if down {
                self.mods.0 |= bit;
            } else {
                self.mods.0 &= !bit;
            }
        }
        self.publish();
        if key == K::CapsLock && down && !repeat {
            self.caps = !self.caps;
        }
        let is_modifier = modifier.is_some() || key == K::CapsLock;
        let kind = if is_modifier {
            InputKind::Modifier
        } else {
            InputKind::Key
        };
        let kev = (!(is_modifier && repeat)).then_some(KeyEvent {
            key,
            dir: if down { KeyDir::Down } else { KeyDir::Up },
            mods: self.mods.modifiers(self.caps),
            repeat,
            time_ms: ev.time_ms,
        });
        InputEvent::new(kind, kev, ev.time_ms, native(self.mods))
    }
}
