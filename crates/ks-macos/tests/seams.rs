//! Швы бэкенда macOS без системных вызовов: коды клавиш, колбэк перехвата, исполнитель.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ks_core::{
    Action, AppId, Context, Disposition, KeyDir, KeyEvent, KeyStroke, LayoutId, Outcome,
    PhysicalKey as K, ReplaceKind, ResetReason,
};
use ks_macos::exec::{Executor, Poster, Switcher};
use ks_macos::keycode;
use ks_macos::tap::{
    BUSY_TIMEOUT_MS, ContextSource, Job, MARK_OWN, Pipeline, RawEvent, RawKind, TapControl,
    TapLogic, TapShared, Verdict,
};
use ks_platform::{BackendError, KeyHandler};

// kVK_* из Carbon Events.h
const KVK_G: u16 = 5;
const KVK_H: u16 = 4;
const KVK_C: u16 = 8;
const KVK_K: u16 = 40;
const KVK_SPACE: u16 = 49;
const KVK_A: u16 = 0;

#[test]
fn mac_keycodes_map_to_w3c_keys_and_back() {
    let carbon = [
        (0, K::KeyA),
        (5, K::KeyG),
        (18, K::Digit1),
        (43, K::Comma),
        (50, K::Backquote),
        (10, K::IntlBackslash),
        (36, K::Enter),
        (48, K::Tab),
        (49, K::Space),
        (51, K::Backspace),
        (56, K::ShiftLeft),
        (60, K::ShiftRight),
        (55, K::MetaLeft),
        (123, K::ArrowLeft),
        (126, K::ArrowUp),
    ];
    for (code, key) in carbon {
        assert_eq!(keycode::to_physical(code), key, "kVK {code}");
        assert_eq!(keycode::from_physical(key), Some(code), "{key}");
    }
    assert_eq!(keycode::to_physical(105), K::Other(105));
    assert_eq!(keycode::from_physical(K::Pause), None);
    assert_eq!(keycode::printable().count(), 48);
}

#[test]
fn left_and_right_shift_are_told_apart_by_device_bits() {
    // NX_DEVICELSHIFTKEYMASK = 0x2, NX_DEVICERSHIFTKEYMASK = 0x4, Shift = 0x20000
    assert_eq!(
        keycode::modifier_dir(K::ShiftLeft, 0x2_0002),
        Some(KeyDir::Down)
    );
    assert_eq!(
        keycode::modifier_dir(K::ShiftRight, 0x2_0002),
        Some(KeyDir::Up)
    );
    assert_eq!(keycode::modifier_dir(K::ShiftLeft, 0), Some(KeyDir::Up));
    assert!(keycode::modifiers(0x2_0002).shift);
    assert!(keycode::modifiers(0x1_0000).caps_lock);
}

/// Движок-заглушка: на пробел отвечает заменой с проглоченной границей.
#[derive(Default)]
struct FakeHandler {
    keys: Mutex<Vec<(K, KeyDir)>>,
    resets: Mutex<Vec<ResetReason>>,
}

fn replace_action() -> Action {
    Action::Replace {
        erase: 2,
        keys: vec![
            KeyStroke {
                key: K::KeyG,
                shift: false,
            },
            KeyStroke {
                key: K::Space,
                shift: false,
            },
        ],
        text: "пр ".to_owned(),
        target_layout: Some(LayoutId("ru".to_owned())),
        kind: ReplaceKind::Auto,
    }
}

impl KeyHandler for FakeHandler {
    fn on_key(&self, ev: &KeyEvent, _ctx: &Context) -> Outcome {
        self.keys.lock().unwrap().push((ev.key, ev.dir));
        if ev.key == K::KeyA && ev.dir == KeyDir::Down {
            return Outcome {
                disposition: Disposition::Pass,
                actions: vec![Action::SwitchLayout(LayoutId("ru".to_owned()))],
            };
        }
        if ev.key == K::Space && ev.dir == KeyDir::Down {
            Outcome {
                disposition: Disposition::Swallow,
                actions: vec![replace_action()],
            }
        } else {
            Outcome::pass()
        }
    }
    fn on_reset(&self, why: ResetReason) {
        self.resets.lock().unwrap().push(why);
    }
    fn convert_text(&self, _text: &str) -> Option<String> {
        None
    }
}

struct FixedContext;
impl ContextSource for FixedContext {
    fn context(&self, _pid: i32) -> Context {
        Context {
            app: AppId("com.apple.TextEdit".to_owned()),
            layout: LayoutId("en".to_owned()),
            secure_input: false,
            password_field: false,
        }
    }
}

#[derive(Default)]
struct CountingControl(AtomicUsize);
impl TapControl for CountingControl {
    fn reenable(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Tap {
    logic: TapLogic,
    handler: Arc<FakeHandler>,
    shared: Arc<TapShared>,
    jobs: mpsc::Receiver<Job>,
}

fn tap(post_allowed: bool) -> Tap {
    let handler = Arc::new(FakeHandler::default());
    let shared = Arc::new(TapShared::default());
    shared.post_allowed.store(post_allowed, Ordering::SeqCst);
    let (tx, jobs) = mpsc::channel();
    let logic = TapLogic::new(handler.clone(), Arc::new(FixedContext), shared.clone(), tx);
    Tap {
        logic,
        handler,
        shared,
        jobs,
    }
}

fn press(t: &Tap, code: u16) -> Verdict {
    press_at(t, code, 0)
}

fn press_at(t: &Tap, code: u16, time_ms: u64) -> Verdict {
    let control = CountingControl::default();
    let mut ev = RawEvent::key(RawKind::KeyDown, code, 0);
    ev.time_ms = time_ms;
    let down = t.logic.handle(&ev, &control);
    ev.kind = RawKind::KeyUp;
    t.logic.handle(&ev, &control);
    down
}

#[test]
fn tap_disabled_by_timeout_is_enabled_again_and_keeps_working() {
    let t = tap(true);
    let control = CountingControl::default();
    for kind in [RawKind::DisabledByTimeout, RawKind::DisabledByUserInput] {
        let ev = RawEvent::key(kind, 0, 0);
        assert_eq!(t.logic.handle(&ev, &control), Verdict::Pass);
    }
    assert_eq!(
        control.0.load(Ordering::SeqCst),
        2,
        "перехват включён снова оба раза"
    );
    assert_eq!(t.shared.reenabled.load(Ordering::SeqCst), 2);
    assert_eq!(press(&t, KVK_G), Verdict::Pass);
    assert_eq!(t.handler.keys.lock().unwrap()[0], (K::KeyG, KeyDir::Down));
}

#[test]
fn without_post_permission_nothing_is_swallowed_and_nothing_is_replaced() {
    let t = tap(false);
    for code in [KVK_G, KVK_H, KVK_SPACE] {
        assert_eq!(press(&t, code), Verdict::Pass);
    }
    assert!(t.jobs.try_recv().is_err(), "замена не начата");
    assert!(t.handler.keys.lock().unwrap().is_empty());
}

#[test]
fn own_synthetic_events_bypass_the_engine() {
    let t = tap(true);
    let mut ev = RawEvent::key(RawKind::KeyDown, KVK_G, 0);
    ev.user_data = MARK_OWN;
    assert_eq!(
        t.logic.handle(&ev, &CountingControl::default()),
        Verdict::Pass
    );
    assert!(t.handler.keys.lock().unwrap().is_empty());
}

#[test]
fn mouse_click_resets_the_word() {
    let t = tap(true);
    let ev = RawEvent::key(RawKind::MouseDown, 0, 0);
    assert_eq!(
        t.logic.handle(&ev, &CountingControl::default()),
        Verdict::Pass
    );
    assert_eq!(
        *t.handler.resets.lock().unwrap(),
        vec![ResetReason::MouseClick]
    );
}

type Log = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct RecordingPoster(Log);
impl Poster for RecordingPoster {
    fn key(&self, keycode: u16, down: bool, flags: u64) {
        let dir = if down { "down" } else { "up" };
        self.0
            .lock()
            .unwrap()
            .push(format!("{keycode}:{dir}:{flags:x}"));
    }
    fn unicode(&self, text: &str) {
        self.0.lock().unwrap().push(format!("unicode:{text}"));
    }
    fn pause_ms(&self, _ms: u64) {}
}

struct FakeSwitcher {
    allowed: bool,
    selected: Mutex<Vec<String>>,
    log: Log,
}
impl Switcher for FakeSwitcher {
    fn post_allowed(&self) -> bool {
        self.allowed
    }
    fn select_layout(&self, id: &LayoutId) -> bool {
        self.selected.lock().unwrap().push(id.0.clone());
        self.log.lock().unwrap().push(format!("select:{}", id.0));
        true
    }
}

fn executor(allowed: bool) -> (Executor, Arc<RecordingPoster>, Arc<FakeSwitcher>) {
    let poster = Arc::new(RecordingPoster::default());
    let switcher = Arc::new(FakeSwitcher {
        allowed,
        selected: Mutex::new(Vec::new()),
        log: poster.0.clone(),
    });
    (
        Executor::new(poster.clone(), switcher.clone()),
        poster,
        switcher,
    )
}

#[test]
fn replace_without_post_permission_fails_and_erases_nothing() {
    let (exec, poster, switcher) = executor(false);
    let result = exec.execute(&replace_action());
    assert!(
        matches!(result, Err(BackendError::NoPermission)),
        "ожидался NoPermission"
    );
    assert!(poster.0.lock().unwrap().is_empty(), "ни одного события");
    assert!(
        switcher.selected.lock().unwrap().is_empty(),
        "раскладка не тронута"
    );
}

#[test]
fn replace_erases_switches_layout_and_retypes_physical_keys() {
    let (exec, poster, switcher) = executor(true);
    exec.execute(&replace_action()).unwrap();
    assert_eq!(*switcher.selected.lock().unwrap(), vec!["ru".to_owned()]);
    let expected = [
        "51:down:0",
        "51:up:0",
        "51:down:0",
        "51:up:0",
        "select:ru",
        "5:down:0",
        "5:up:0",
        "49:down:0",
        "49:up:0",
    ];
    assert_eq!(*poster.0.lock().unwrap(), expected);
}

#[test]
fn keys_typed_during_replacement_are_held_and_replayed_in_order() {
    let t = tap(true);
    assert_eq!(press(&t, KVK_SPACE), Verdict::Swallow);
    let job = t.jobs.try_recv().expect("замена передана исполнителю");
    assert!(t.shared.busy());
    assert_eq!(
        press(&t, KVK_C),
        Verdict::Swallow,
        "нажатие во время замены придержано"
    );
    assert_eq!(press(&t, KVK_K), Verdict::Swallow);

    let (exec, poster, _) = executor(true);
    let pipeline = Pipeline {
        handler: t.handler.clone(),
        ctx: Arc::new(FixedContext),
        shared: t.shared.clone(),
        exec,
    };
    pipeline.run(job);
    assert!(!t.shared.busy());
    let posted = poster.0.lock().unwrap();
    let tail: Vec<&str> = posted
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(tail, ["8:down:0", "8:up:0", "40:down:0", "40:up:0"]);
    let seen: Vec<K> = t
        .handler
        .keys
        .lock()
        .unwrap()
        .iter()
        .map(|(k, _)| *k)
        .collect();
    assert_eq!(seen[seen.len() - 4..], [K::KeyC, K::KeyC, K::KeyK, K::KeyK]);
}

#[test]
fn key_passed_with_actions_reaches_the_app_only_after_the_actions() {
    let t = tap(true);
    let control = CountingControl::default();
    let down = RawEvent::key(RawKind::KeyDown, KVK_A, 0);
    assert_eq!(
        t.logic.handle(&down, &control),
        Verdict::Swallow,
        "буква придержана"
    );
    let job = t.jobs.try_recv().expect("действие передано исполнителю");
    let (exec, poster, _) = executor(true);
    let pipeline = Pipeline {
        handler: t.handler.clone(),
        ctx: Arc::new(FixedContext),
        shared: t.shared.clone(),
        exec,
    };
    pipeline.run(job);
    assert_eq!(*poster.0.lock().unwrap(), ["select:ru", "0:down:0"]);
    assert!(!t.shared.busy());
}

struct PanickingPoster;
impl Poster for PanickingPoster {
    fn key(&self, _keycode: u16, _down: bool, _flags: u64) {
        panic!("сбой исполнителя (тест)");
    }
    fn unicode(&self, _text: &str) {}
    fn pause_ms(&self, _ms: u64) {}
}

fn pipeline(t: &Tap, exec: Executor) -> Pipeline {
    Pipeline {
        handler: t.handler.clone(),
        ctx: Arc::new(FixedContext),
        shared: t.shared.clone(),
        exec,
    }
}

#[test]
fn executor_panic_releases_the_keyboard_and_drops_held_keys() {
    let t = tap(true);
    assert_eq!(press(&t, KVK_SPACE), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    assert_eq!(press(&t, KVK_C), Verdict::Swallow);
    let (_, _, switcher) = executor(true);
    let p = pipeline(&t, Executor::new(Arc::new(PanickingPoster), switcher));
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.run(job)));
    assert!(run.is_err(), "исполнитель упал");
    assert!(!t.shared.busy(), "клавиатура отпущена");
    assert_eq!(t.shared.held(), 0, "удержанное сброшено");
    assert_eq!(
        press(&t, KVK_K),
        Verdict::Pass,
        "следующее нажатие проходит"
    );
    assert!(
        t.handler
            .resets
            .lock()
            .unwrap()
            .contains(&ResetReason::External)
    );
}

#[test]
fn hung_executor_is_cut_off_by_the_deadline_and_its_stale_job_is_not_executed() {
    let t = tap(true);
    assert_eq!(press_at(&t, KVK_SPACE, 0), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    assert_eq!(press_at(&t, KVK_C, 10), Verdict::Swallow);
    let late = BUSY_TIMEOUT_MS + 1;
    assert_eq!(
        press_at(&t, KVK_K, late),
        Verdict::Pass,
        "срок вышел — нажатие проходит"
    );
    assert!(!t.shared.busy() && t.shared.held() == 0 && t.shared.stalled());
    assert_eq!(
        press_at(&t, KVK_SPACE, late + 10),
        Verdict::Pass,
        "пока исполнитель висит, замен нет"
    );
    let (exec, poster, _) = executor(true);
    pipeline(&t, exec).run(job);
    assert!(
        poster.0.lock().unwrap().is_empty(),
        "устаревшая замена не исполнена"
    );
    assert!(!t.shared.stalled(), "исполнитель отвис — пауза снята");
}

struct BrokenSwitcher;
impl Switcher for BrokenSwitcher {
    fn post_allowed(&self) -> bool {
        true
    }
    fn select_layout(&self, _id: &LayoutId) -> bool {
        false
    }
}

#[test]
fn failed_layout_switch_falls_back_to_unicode_and_is_counted() {
    let poster = Arc::new(RecordingPoster::default());
    let exec = Executor::new(poster.clone(), Arc::new(BrokenSwitcher));
    exec.execute(&replace_action()).unwrap();
    assert_eq!(poster.0.lock().unwrap().last().unwrap(), "unicode:пр ");
    assert_eq!(exec.stats().layout_switch_failed(), 1);
    assert!(exec.stats().summary().unwrap().contains("ru"));
}

/// Переключение раскладки, которое висит до сигнала (как `on_main` при занятом главном потоке).
struct BlockingSwitcher {
    entered: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<bool>>,
}
impl Switcher for BlockingSwitcher {
    fn post_allowed(&self) -> bool {
        true
    }
    fn select_layout(&self, _id: &LayoutId) -> bool {
        self.entered.lock().unwrap().send(()).unwrap();
        let panic_after = self.release.lock().unwrap().recv().unwrap();
        assert!(!panic_after, "сбой после отвисания (тест)");
        true
    }
}

/// Задание зависает посреди замены (после Backspace), срок срезает его, затем оно отвисает.
fn hang_mid_replacement(panic_after: bool) -> (Tap, Arc<RecordingPoster>) {
    let t = tap(true);
    assert_eq!(press_at(&t, KVK_SPACE, 0), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let poster = Arc::new(RecordingPoster::default());
    let switcher = Arc::new(BlockingSwitcher {
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let p = pipeline(&t, Executor::new(poster.clone(), switcher));
    let worker = std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.run(job)));
    });
    entered_rx.recv().unwrap();
    let late = BUSY_TIMEOUT_MS + 1;
    assert_eq!(
        press_at(&t, KVK_C, late),
        Verdict::Pass,
        "срок отпустил клавиатуру"
    );
    release_tx.send(panic_after).unwrap();
    worker.join().unwrap();
    (t, poster)
}

#[test]
fn job_hung_mid_replacement_sends_nothing_after_the_deadline_cut_it_off() {
    let (t, poster) = hang_mid_replacement(false);
    let erased = ["51:down:0", "51:up:0", "51:down:0", "51:up:0"];
    assert_eq!(
        *poster.0.lock().unwrap(),
        erased,
        "после отвисания ничего не отправлено"
    );
    assert!(!t.shared.stalled(), "исполнитель отвис — пауза снята");
    assert_eq!(press_at(&t, KVK_G, BUSY_TIMEOUT_MS + 100), Verdict::Pass);
    let last = *t.handler.keys.lock().unwrap().last().unwrap();
    assert_eq!(last, (K::KeyG, KeyDir::Up), "движок снова получает нажатия");
}

#[test]
fn job_that_panics_after_the_deadline_does_not_leave_corrections_paused() {
    let (t, poster) = hang_mid_replacement(true);
    assert_eq!(
        poster.0.lock().unwrap().len(),
        4,
        "только Backspace до зависания"
    );
    assert!(!t.shared.stalled(), "пауза не осталась навсегда");
}
