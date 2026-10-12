//! Перед первым Backspace замены. Захвата нет, поэтому всё, что пользователь нажал после
//! нажатия, вызвавшего замену, уже в приложении, и число стираемых символов устарело: такая
//! замена отменяется (ничего не стёрто, движку — сброс). Ctrl, Alt и Meta пользователя uinput
//! отпустить не может, а композитор складывает модификаторы клавиатур: зажатый Ctrl превратил бы
//! Backspace в Ctrl+Backspace. Их ждём один раз (до `RELEASE_WAIT_MS`), всё ещё зажаты — замена
//! не начинается. Shift не ждём: на Backspace он не влияет, а Shift для букв виртуальная
//! клавиатура нажимает и отпускает сама.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ks_core::{LayoutId, Modifiers};
use ks_platform::exec::Switcher;

/// Сколько ждать отпускания Ctrl, Alt и Meta перед заменой.
pub const RELEASE_WAIT_MS: u64 = 300;
const STEP_MS: u64 = 5;

const SHIFT: u8 = 1;
const CTRL: u8 = 2;
const ALT: u8 = 4;
const META: u8 = 8;

/// Модификаторы, которые пользователь держит сейчас (по чтению evdev).
#[derive(Default)]
pub struct HeldMods(AtomicU8);

impl HeldMods {
    pub fn set(&self, m: Modifiers) {
        let bit = |on: bool, b: u8| if on { b } else { 0 };
        let bits = bit(m.shift, SHIFT) | bit(m.ctrl, CTRL) | bit(m.alt, ALT) | bit(m.meta, META);
        self.0.store(bits, Ordering::Relaxed);
    }

    pub fn get(&self) -> Modifiers {
        let b = self.0.load(Ordering::Relaxed);
        Modifiers {
            shift: b & SHIFT != 0,
            ctrl: b & CTRL != 0,
            alt: b & ALT != 0,
            meta: b & META != 0,
            caps_lock: false,
        }
    }
}

/// Ввод пользователя относительно решения о замене. Время — `KeyEvent::time_ms`, у событий
/// evdev оно строго возрастает (`tap::Tracker`) и служит порядковым номером.
#[derive(Default)]
pub struct InputWatch {
    /// Время последнего немодификаторного нажатия или клика пользователя.
    last: AtomicU64,
    /// Время нажатия, по которому решена ожидающая замена.
    armed: AtomicU64,
    on_refuse: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl InputWatch {
    /// Нажатие (не модификатор) или клик пользователя, прочитанные в момент `time_ms`.
    pub fn user_input(&self, time_ms: u64) {
        self.last.fetch_max(time_ms, Ordering::Relaxed);
    }

    /// Замена решена по нажатию в момент `time_ms`.
    pub fn arm(&self, time_ms: u64) {
        self.armed.store(time_ms, Ordering::Relaxed);
    }

    /// После нажатия, вызвавшего замену, пользователь что-то нажал.
    pub fn typed_since_armed(&self) -> bool {
        self.last.load(Ordering::Relaxed) > self.armed.load(Ordering::Relaxed)
    }

    /// Чем сообщить движку об отказе (сброс слова в работе); ставится один раз.
    pub fn set_on_refuse(&self, f: Box<dyn Fn() + Send + Sync>) {
        let _ = self.on_refuse.set(f);
    }

    fn refuse(&self) {
        if let Some(f) = self.on_refuse.get() {
            f();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Ready,
    /// Ctrl, Alt или Meta так и не отпущены.
    ModifiersHeld,
    /// Пользователь нажал клавишу или кликнул после решения о замене.
    UserTyped,
}

/// Решение перед первым Backspace: ждёт отпускания Ctrl, Alt и Meta не дольше `wait_ms`,
/// прерываясь, как только `typed()` сообщит о новом вводе. Зажатый Shift не ждётся.
pub fn gate(
    held: &dyn Fn() -> Modifiers,
    typed: &dyn Fn() -> bool,
    sleep: &mut dyn FnMut(u64),
    wait_ms: u64,
) -> Gate {
    let mut waited = 0;
    loop {
        if typed() {
            return Gate::UserTyped;
        }
        let m = held();
        if !(m.ctrl || m.alt || m.meta) {
            return Gate::Ready;
        }
        if waited >= wait_ms {
            return Gate::ModifiersHeld;
        }
        let step = STEP_MS.min(wait_ms - waited);
        sleep(step);
        waited += step;
    }
}

/// `Switcher`, у которого `post_allowed` — проверка исполнителя перед первым Backspace замены —
/// ещё и проходит [`gate`]; отказ сообщается движку через [`InputWatch::set_on_refuse`].
pub struct Guarded<S> {
    pub inner: S,
    held: Arc<HeldMods>,
    watch: Arc<InputWatch>,
    wait_ms: u64,
}

impl<S> Guarded<S> {
    pub fn new(inner: S, held: Arc<HeldMods>, watch: Arc<InputWatch>, wait_ms: u64) -> Guarded<S> {
        Guarded {
            inner,
            held,
            watch,
            wait_ms,
        }
    }
}

impl<S: Switcher> Switcher for Guarded<S> {
    fn post_allowed(&self) -> bool {
        if !self.inner.post_allowed() {
            return false;
        }
        let mut sleep = |ms| std::thread::sleep(Duration::from_millis(ms));
        let held = || self.held.get();
        let typed = || self.watch.typed_since_armed();
        match gate(&held, &typed, &mut sleep, self.wait_ms) {
            Gate::Ready => true,
            Gate::ModifiersHeld => {
                log::info!("замена не начата: зажат Ctrl, Alt или Meta, ничего не стёрто");
                self.watch.refuse();
                false
            }
            Gate::UserTyped => {
                log::info!("замена отменена: ввод продолжился до её начала, ничего не стёрто");
                self.watch.refuse();
                false
            }
        }
    }

    fn select_layout(&self, id: &LayoutId) -> bool {
        self.inner.select_layout(id)
    }
}
