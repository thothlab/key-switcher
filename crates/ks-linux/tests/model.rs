//! Исполнение без захвата на хосте: модификаторы перед заменой, отпускание всего нажатого
//! виртуальной клавиатурой, бюджет смены раскладки, владелец PRIMARY.

use std::cell::Cell;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ks_core::{Action, KeyStroke, LayoutId, Modifiers, PhysicalKey as K, ReplaceKind};
use ks_linux::guard::{Gate, Guarded, HeldMods, InputWatch, gate};
use ks_linux::selection::primary_belongs_to_focus;
use ks_linux::switch::{LayoutControl, select, with_deadline};
use ks_linux::vkbd::{Emit, Keys};
use ks_platform::BackendError;
use ks_platform::exec::{Chord, Executor, Poster, Switcher};

const SHIFT: u16 = 42; // KEY_LEFTSHIFT
const BACKSPACE: u16 = 14; // KEY_BACKSPACE

#[derive(Default)]
struct Recorder(Mutex<Vec<(PhysicalKeyName, bool)>>);
type PhysicalKeyName = String;

impl Poster<()> for Recorder {
    fn can_type(&self, _key: K) -> bool {
        true
    }
    fn key(&self, key: K, down: bool, _shift: bool) {
        self.0.lock().unwrap().push((key.to_string(), down));
    }
    fn chord(&self, _chord: Chord) {}
    fn unicode(&self, _text: &str) {}
    fn replay(&self, _native: &()) {}
    fn pause_ms(&self, _ms: u64) {}
}

struct Allowed;

impl Switcher for Allowed {
    fn post_allowed(&self) -> bool {
        true
    }
    fn select_layout(&self, _id: &LayoutId) -> bool {
        true
    }
}

fn replace() -> Action {
    Action::Replace {
        erase: 3,
        keys: vec![KeyStroke {
            key: K::KeyG,
            shift: false,
        }],
        text: "п".to_owned(),
        target_layout: None,
        kind: ReplaceKind::Auto,
    }
}

#[test]
fn held_ctrl_stops_replacement_before_anything_is_erased() {
    let held = Arc::new(HeldMods::default());
    held.set(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    let rec = Arc::new(Recorder::default());
    let watch = Arc::new(InputWatch::default());
    let exec = Executor::new(
        rec.clone(),
        Arc::new(Guarded::new(Allowed, held, watch, 20)),
    );
    let started = Instant::now();
    assert!(matches!(
        exec.execute(&replace()),
        Err(BackendError::NoPermission)
    ));
    assert!(rec.0.lock().unwrap().is_empty(), "ни одного Backspace");
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "ждали один раз"
    );
}

#[test]
fn held_shift_does_not_delay_replacement() {
    let held = Arc::new(HeldMods::default());
    held.set(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    let rec = Arc::new(Recorder::default());
    let watch = Arc::new(InputWatch::default());
    let exec = Executor::new(
        rec.clone(),
        Arc::new(Guarded::new(Allowed, held, watch, 300)),
    );
    let started = Instant::now();
    assert!(exec.execute(&replace()).is_ok());
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "без ожидания Shift"
    );
    let events = rec.0.lock().unwrap();
    assert_eq!(events.first(), Some(&("Backspace".to_owned(), true)));
    assert_eq!(
        events
            .iter()
            .filter(|(k, down)| k == "Backspace" && *down)
            .count(),
        3
    );
}

#[test]
fn key_typed_while_waiting_for_ctrl_cancels_replacement() {
    let held = Arc::new(HeldMods::default());
    held.set(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    let watch = Arc::new(InputWatch::default());
    let refused = Arc::new(AtomicBool::new(false));
    let flag = refused.clone();
    watch.set_on_refuse(Box::new(move || flag.store(true, Ordering::SeqCst)));
    watch.user_input(10);
    watch.arm(10); // замена решена по нажатию в момент 10
    let rec = Arc::new(Recorder::default());
    let guarded = Guarded::new(Allowed, held, watch.clone(), 1000);
    let exec = Executor::new(rec.clone(), Arc::new(guarded));
    let typist = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        watch.user_input(11); // буква во время ожидания Ctrl
    });
    let started = Instant::now();
    assert!(matches!(
        exec.execute(&replace()),
        Err(BackendError::NoPermission)
    ));
    typist.join().unwrap();
    assert!(rec.0.lock().unwrap().is_empty(), "ничего не стёрто");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "отмена сразу, не по сроку"
    );
    assert!(refused.load(Ordering::SeqCst), "движку сообщено об отказе");
}

#[test]
fn gate_waits_only_for_ctrl_alt_meta() {
    let quiet = || false;
    let calls = Cell::new(0);
    let ctrl_then_free = || {
        calls.set(calls.get() + 1);
        Modifiers {
            ctrl: calls.get() < 3,
            ..Modifiers::default()
        }
    };
    let mut slept = 0;
    assert_eq!(
        gate(&ctrl_then_free, &quiet, &mut |ms| slept += ms, 300),
        Gate::Ready
    );
    assert_eq!(slept, 10, "два шага по 5 мс до отпускания");

    let mut slept = 0;
    let shift = || Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    assert_eq!(
        gate(&shift, &quiet, &mut |ms| slept += ms, 300),
        Gate::Ready
    );
    assert_eq!(slept, 0, "Shift не ждём");

    let mut slept = 0;
    let alt = || Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    assert_eq!(
        gate(&alt, &quiet, &mut |ms| slept += ms, 300),
        Gate::ModifiersHeld
    );
    assert_eq!(slept, 300);

    let none = Modifiers::default;
    assert_eq!(
        gate(&none, &|| true, &mut |_| {}, 300),
        Gate::UserTyped,
        "ввод после решения"
    );
}

/// Фальшивое устройство: само помнит, что на нём зажато; запись номер `fail_at` падает.
#[derive(Default)]
struct FakeDev {
    down: Arc<Mutex<HashSet<u16>>>,
    calls: usize,
    fail_at: Vec<usize>,
}

impl Emit for FakeDev {
    fn emit(&mut self, code: u16, value: i32) -> io::Result<()> {
        self.calls += 1;
        if self.fail_at.contains(&self.calls) {
            return Err(io::Error::other("запись не прошла"));
        }
        let mut down = self.down.lock().unwrap();
        if value == 0 {
            down.remove(&code);
        } else {
            down.insert(code);
        }
        Ok(())
    }
}

#[test]
fn failed_write_leaves_no_modifier_held() {
    let down = Arc::new(Mutex::new(HashSet::new()));
    let mut keys = Keys::default();
    // Shift нажат (запись 1), буква не записалась (запись 2).
    keys.attach(FakeDev {
        down: down.clone(),
        calls: 0,
        fail_at: vec![2],
    });
    keys.key(30, true, true);
    assert!(
        down.lock().unwrap().is_empty(),
        "Shift отпущен сразу после ошибки"
    );
    keys.key(30, false, true);
    assert!(down.lock().unwrap().is_empty());
    assert!(keys.pressed().is_empty());
}

#[test]
fn interrupted_stroke_is_released_at_job_end() {
    let down = Arc::new(Mutex::new(HashSet::new()));
    let mut keys = Keys::default();
    // Отпускание Shift после ошибки тоже не прошло (запись 3) — остаётся до следующей попытки.
    keys.attach(FakeDev {
        down: down.clone(),
        calls: 0,
        fail_at: vec![2, 3],
    });
    keys.key(BACKSPACE, true, true);
    assert_eq!(*down.lock().unwrap(), HashSet::from([SHIFT]));
    keys.release_all();
    assert!(
        down.lock().unwrap().is_empty(),
        "конец задания: всё отпущено"
    );
    keys.chord(46);
    assert!(down.lock().unwrap().is_empty());
}

/// Окружение, отвечающее с задержкой; соблюдает выданный бюджет, как настоящие вызовы.
struct Slow {
    latency: Duration,
    reports: usize,
}

impl Slow {
    fn wait(&self, budget: Duration) -> bool {
        std::thread::sleep(self.latency.min(budget));
        self.latency <= budget
    }
}

impl LayoutControl for Slow {
    fn switch_to(&self, _index: usize, budget: Duration) -> bool {
        self.wait(budget)
    }
    fn current(&self, budget: Duration) -> Option<usize> {
        self.wait(budget).then_some(self.reports)
    }
}

#[test]
fn layout_switch_never_exceeds_its_budget() {
    let total = Duration::from_millis(120);
    let start = Instant::now();
    assert!(!select(
        &Slow {
            latency: Duration::from_millis(400),
            reports: 1
        },
        1,
        total
    ));
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "медленный ответ срезан бюджетом"
    );

    let start = Instant::now();
    assert!(!select(
        &Slow {
            latency: Duration::from_millis(30),
            reports: 0
        },
        1,
        total
    ));
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "раскладка не подтвердилась в срок"
    );

    assert!(select(
        &Slow {
            latency: Duration::from_millis(5),
            reports: 1
        },
        1,
        total
    ));

    let start = Instant::now();
    let late = with_deadline(Duration::from_millis(30), || {
        std::thread::sleep(Duration::from_millis(300));
        1
    });
    assert_eq!(late, None);
    assert!(start.elapsed() < Duration::from_millis(150));
    assert_eq!(with_deadline(Duration::from_millis(500), || 7), Some(7));
}

#[test]
fn primary_is_trusted_only_from_the_focused_client() {
    let mask = 0x001F_FFFF;
    assert!(primary_belongs_to_focus(0x0240_0012, 0x0240_0012, mask));
    assert!(
        primary_belongs_to_focus(0x0240_0007, 0x0240_0012, mask),
        "окно того же клиента"
    );
    assert!(
        !primary_belongs_to_focus(0x0260_0001, 0x0240_0012, mask),
        "другой клиент"
    );
    assert!(
        !primary_belongs_to_focus(0, 0x0240_0012, mask),
        "выделения нет"
    );
    assert!(
        !primary_belongs_to_focus(0x0240_0007, 0, mask),
        "фокус неизвестен"
    );
}
