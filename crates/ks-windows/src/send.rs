//! Что отправить в `SendInput`: последовательности нажатий без системных вызовов (проверяются
//! тестом на любой ОС). `SendInput` не сбрасывает уже зажатые клавиши, поэтому каждая пачка
//! сначала приводит модификаторы к нужному состоянию, а после клавиши возвращает их к тому, что
//! было зажато (`analysis/platforms.md` §2.3). «Зажатое сейчас» — состояние, которое уже применил
//! хук ([`HeldMods`]), а не `GetAsyncKeyState`. Пачка уходит одним вызовом `SendInput` и не
//! перемешивается с вводом пользователя.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use ks_core::PhysicalKey;
use ks_platform::exec::{Chord, KEY_PAUSE_MS, Poster};

use crate::keycode;
use crate::tap::{HeldMods, ModState, WinEvent};

/// Незанятый VK: нажимается перед отпусканием Alt/Win, чтобы одиночное отпускание не открыло
/// меню окна или «Пуск».
pub const MASK_VK: u16 = 0xE8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    /// Клавиша: VK и скан-код (с битом [`keycode::EXT`] — `KEYEVENTF_EXTENDEDKEY`).
    Key { vk: u16, scan: u16, up: bool },
    /// Одна единица UTF-16 через `KEYEVENTF_UNICODE` (`VK_PACKET`).
    Unicode { unit: u16, up: bool },
}

fn key_of(bit: u8, up: bool) -> Option<Stroke> {
    let key = ModState::key(bit)?;
    Some(Stroke::Key {
        vk: keycode::vk_of(key)?,
        scan: keycode::scan_of(key)?,
        up,
    })
}

fn tap_key(key: PhysicalKey) -> Option<[Stroke; 2]> {
    let (vk, scan) = (keycode::vk_of(key)?, keycode::scan_of(key)?);
    Some([
        Stroke::Key {
            vk,
            scan,
            up: false,
        },
        Stroke::Key { vk, scan, up: true },
    ])
}

fn bits(mask: u8) -> impl Iterator<Item = u8> {
    (0..8).map(|i| 1u8 << i).filter(move |b| mask & b != 0)
}

/// Нажатия и отпускания, переводящие модификаторы из `from` в `to` по семействам (Shift, Ctrl,
/// Alt, Win). Семейство, зажатое в обоих состояниях хоть одной стороной, не трогается; нужное —
/// нажимается теми сторонами, что в `to`; лишнее — отпускается всеми зажатыми сторонами.
pub fn transitions(from: ModState, to: ModState) -> Vec<Stroke> {
    let mut releases = Vec::new();
    let mut presses = Vec::new();
    let mut masked = false;
    for family in ModState::FAMILIES {
        let (have, want) = (from.0 & family, to.0 & family);
        if (have != 0) == (want != 0) {
            continue;
        }
        if want != 0 {
            presses.extend(bits(want).filter_map(|b| key_of(b, false)));
        } else {
            if family & (ModState::ALT | ModState::WIN) != 0 && !masked {
                releases.push(Stroke::Key {
                    vk: MASK_VK,
                    scan: 0,
                    up: false,
                });
                releases.push(Stroke::Key {
                    vk: MASK_VK,
                    scan: 0,
                    up: true,
                });
                masked = true;
            }
            releases.extend(bits(have).filter_map(|b| key_of(b, true)));
        }
    }
    releases.extend(presses);
    releases
}

/// `body` с модификаторами `wanted`; до и после — переходы от зажатого сейчас и обратно.
pub fn around(current: ModState, wanted: ModState, body: &[Stroke]) -> Vec<Stroke> {
    let mut out = transitions(current, wanted);
    out.extend_from_slice(body);
    out.extend(transitions(wanted, current));
    out
}

/// Нажатие или отпускание физической клавиши замены, с Shift или без; прочие модификаторы
/// отпущены. `None` — у клавиши нет кода Windows.
pub fn key_strokes(
    key: PhysicalKey,
    down: bool,
    shift: bool,
    current: ModState,
) -> Option<Vec<Stroke>> {
    let body = Stroke::Key {
        vk: keycode::vk_of(key)?,
        scan: keycode::scan_of(key)?,
        up: !down,
    };
    let wanted = if shift {
        ModState(ModState::LSHIFT)
    } else {
        ModState::default()
    };
    Some(around(current, wanted, &[body]))
}

/// Придержанное нажатие пользователя — с модификаторами момента захвата. Символ чужого
/// `KEYEVENTF_UNICODE` воспроизводится так же — единицей UTF-16.
pub fn replay_strokes(ev: &WinEvent, current: ModState) -> Vec<Stroke> {
    let body = if ev.vk == keycode::VK_PACKET {
        Stroke::Unicode {
            unit: ev.scan,
            up: !ev.down,
        }
    } else {
        Stroke::Key {
            vk: ev.vk,
            scan: ev.scan,
            up: !ev.down,
        }
    };
    around(current, ev.mods, &[body])
}

/// Ctrl + клавиша по VK (Ctrl+C, Ctrl+V): VK буквы в KBDRU тот же, что в US.
pub fn chord_strokes(vk: u16, current: ModState) -> Vec<Stroke> {
    let scan = match vk {
        keycode::VK_C => 0x2E,
        keycode::VK_V => 0x2F,
        _ => 0,
    };
    let body = [
        Stroke::Key {
            vk,
            scan,
            up: false,
        },
        Stroke::Key { vk, scan, up: true },
    ];
    around(current, ModState(ModState::LCTRL), &body)
}

/// Строка через `KEYEVENTF_UNICODE` при отпущенных модификаторах. Перевод строки и табуляция —
/// настоящими Enter и Tab: `VK_PACKET` с `\n` не отправляет форму и сообщение.
pub fn unicode_strokes(text: &str, current: ModState) -> Vec<Stroke> {
    let mut body = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let key = match ch {
            '\r' if chars.peek() == Some(&'\n') => continue,
            '\n' | '\r' => Some(PhysicalKey::Enter),
            '\t' => Some(PhysicalKey::Tab),
            _ => None,
        };
        if let Some(strokes) = key.and_then(tap_key) {
            body.extend(strokes);
            continue;
        }
        let mut buf = [0u16; 2];
        for &unit in ch.encode_utf16(&mut buf).iter() {
            body.push(Stroke::Unicode { unit, up: false });
            body.push(Stroke::Unicode { unit, up: true });
        }
    }
    around(current, ModState::default(), &body)
}

/// Куда уходят пачки: `SendInput` в продукте, список в тестах.
pub trait Sink: Send + Sync {
    fn send(&self, strokes: &[Stroke]);
    fn pause_ms(&self, ms: u64);
}

/// Модификаторы, которые пачка с `current` → `wanted` отпустит и нажмёт обратно.
pub fn restored(current: ModState, wanted: ModState) -> ModState {
    ModState(
        ModState::FAMILIES
            .iter()
            .filter(|&&f| current.0 & f != 0 && wanted.0 & f == 0)
            .fold(0, |acc, &f| acc | (current.0 & f)),
    )
}

/// `Poster` бэкенда Windows. «Зажатое сейчас» — модификаторы пользователя из хука
/// ([`HeldMods`], свои события туда не попадают). Что пачки нажали обратно за пользователя,
/// помнится; отпущенное им с тех пор отпускается перед следующей пачкой и в сверке
/// [`Typist::settle`] после конца задания (поток `ks-mods` бэкенда).
pub struct Typist<S> {
    held: Arc<HeldMods>,
    sink: S,
    /// Нажато обратно нашими пачками; под этим же замком пачки уходят по одной.
    left: Mutex<ModState>,
    notify: Option<Mutex<Sender<()>>>,
}

impl<S: Sink> Typist<S> {
    pub fn new(held: Arc<HeldMods>, sink: S) -> Typist<S> {
        Typist {
            held,
            sink,
            left: Mutex::new(ModState::default()),
            notify: None,
        }
    }

    /// Куда сообщать о каждой пачке: по тишине после неё зовётся [`Typist::settle`].
    pub fn with_notify(mut self, notify: Sender<()>) -> Typist<S> {
        self.notify = Some(Mutex::new(notify));
        self
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    fn left(&self) -> MutexGuard<'_, ModState> {
        self.left.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Отпускания того, что наши пачки нажали обратно, а пользователь уже отпустил.
    fn owed(left: &mut ModState, user: ModState) -> Vec<Stroke> {
        let stale = ModState(left.0 & !user.0);
        left.0 &= user.0;
        transitions(stale, ModState::default())
    }

    fn batch(&self, wanted: ModState, make: impl FnOnce(ModState) -> Vec<Stroke>) {
        let mut left = self.left();
        let user = self.held.get();
        let mut strokes = Self::owed(&mut left, user);
        strokes.extend(make(user));
        left.0 |= restored(user, wanted).0;
        self.sink.send(&strokes);
        drop(left);
        if let Some(notify) = &self.notify
            && let Ok(tx) = notify.lock()
        {
            let _ = tx.send(());
        }
    }

    /// Сверка с актуальным состоянием пользователя: отпускает то, что пачки оставили нажатым
    /// сверх него.
    pub fn settle(&self) {
        let mut left = self.left();
        let strokes = Self::owed(&mut left, self.held.get());
        if !strokes.is_empty() {
            self.sink.send(&strokes);
        }
    }

    /// Пачки оставили нажатым что-то, что пользователь держит (или держал) — сверка ещё нужна.
    pub fn owes(&self) -> bool {
        self.left().0 != 0
    }
}

impl<S: Sink> Poster<WinEvent> for Typist<S> {
    fn can_type(&self, key: PhysicalKey) -> bool {
        keycode::vk_of(key).is_some() && keycode::scan_of(key).is_some()
    }

    fn key(&self, key: PhysicalKey, down: bool, shift: bool) {
        if !self.can_type(key) {
            return;
        }
        let wanted = if shift {
            ModState(ModState::LSHIFT)
        } else {
            ModState::default()
        };
        self.batch(wanted, |user| {
            key_strokes(key, down, shift, user).unwrap_or_default()
        });
    }

    fn chord(&self, chord: Chord) {
        let vk = match chord {
            Chord::Copy => keycode::VK_C,
            Chord::Paste => keycode::VK_V,
        };
        self.batch(ModState(ModState::LCTRL), |user| chord_strokes(vk, user));
        self.sink.pause_ms(KEY_PAUSE_MS);
    }

    fn unicode(&self, text: &str) {
        self.batch(ModState::default(), |user| unicode_strokes(text, user));
    }

    fn replay(&self, native: &WinEvent) {
        self.batch(native.mods, |user| replay_strokes(native, user));
    }

    fn pause_ms(&self, ms: u64) {
        self.sink.pause_ms(ms);
    }
}
