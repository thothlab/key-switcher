//! Швы общей модели исполнения без системных вызовов: колбэк перехвата, удержание нажатий,
//! исполнитель, срок, конвертация выделения. Событие ОС — фальшивое (`Native`).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ks_core::{
    Action, AppId, Context, Disposition, KeyDir, KeyEvent, KeyStroke, LayoutId, Modifiers, Outcome,
    PhysicalKey as K, ReplaceKind, ResetReason,
};
use ks_platform::exec::{Chord, Clipboard, ClipboardSnapshot, Executor, Poster, Switcher};
use ks_platform::intercept::{
    BUSY_TIMEOUT_MS, ContextSource, InputEvent, InputKind, Job, Pipeline, TapControl, TapLogic,
    TapShared, Verdict,
};
use ks_platform::{BackendError, KeyHandler};

/// Событие «ОС» для воспроизведения: клавиша и направление.
#[derive(Clone, Copy)]
struct Native {
    key: K,
    down: bool,
}

type Log = Arc<Mutex<Vec<String>>>;

/// Движок-заглушка: на пробел — замена с проглоченной границей, на «A» — смена раскладки
/// вместе с буквой; выделение «ghbdtn» конвертирует в «ПРИВЕТ».
#[derive(Default)]
struct FakeHandler {
    keys: Mutex<Vec<(K, KeyDir)>>,
    resets: Mutex<Vec<ResetReason>>,
    converted: Mutex<Vec<String>>,
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
        match (ev.key, ev.dir) {
            (K::KeyA, KeyDir::Down) => Outcome {
                disposition: Disposition::Pass,
                actions: vec![Action::SwitchLayout(LayoutId("ru".to_owned()))],
            },
            (K::Space, KeyDir::Down) => Outcome {
                disposition: Disposition::Swallow,
                actions: vec![replace_action()],
            },
            (K::Pause, KeyDir::Down) => Outcome {
                disposition: Disposition::Swallow,
                actions: vec![Action::ConvertSelection],
            },
            _ => Outcome::pass(),
        }
    }
    fn on_reset(&self, why: ResetReason) {
        self.resets.lock().unwrap().push(why);
    }
    fn convert_text(&self, text: &str) -> Option<String> {
        self.converted.lock().unwrap().push(text.to_owned());
        (text == "ghbdtn").then(|| "ПРИВЕТ".to_owned())
    }
    fn tick(&self, _now_ms: u64) -> Vec<Action> {
        Vec::new()
    }
    fn next_deadline(&self) -> Option<u64> {
        None
    }
}

struct FixedContext(&'static str);
impl ContextSource for FixedContext {
    fn context(&self) -> Context {
        Context {
            app: AppId(self.0.to_owned()),
            layout: LayoutId("en".to_owned()),
            secure_input: false,
            password_field: false,
        }
    }
}

const TEXTEDIT: &str = "com.apple.TextEdit";

#[derive(Default)]
struct CountingControl(AtomicUsize);
impl TapControl for CountingControl {
    fn reenable(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Tap {
    logic: TapLogic<Native>,
    handler: Arc<FakeHandler>,
    shared: Arc<TapShared<Native>>,
    jobs: mpsc::Receiver<Job<Native>>,
    app: &'static str,
}

fn tap_in(post_allowed: bool, app: &'static str) -> Tap {
    let handler = Arc::new(FakeHandler::default());
    let shared = Arc::new(TapShared::default());
    shared.post_allowed.store(post_allowed, Ordering::SeqCst);
    let (tx, jobs) = mpsc::channel();
    let logic = TapLogic::new(
        handler.clone(),
        Arc::new(FixedContext(app)),
        shared.clone(),
        tx,
    );
    Tap {
        logic,
        handler,
        shared,
        jobs,
        app,
    }
}

fn tap(post_allowed: bool) -> Tap {
    tap_in(post_allowed, TEXTEDIT)
}

fn event(kind: InputKind, key: K, dir: KeyDir, time_ms: u64) -> InputEvent<Native> {
    let kev = KeyEvent {
        key,
        dir,
        mods: Modifiers::default(),
        repeat: false,
        time_ms,
    };
    let native = Native {
        key,
        down: dir == KeyDir::Down,
    };
    InputEvent::new(kind, Some(kev), time_ms, native)
}

fn press_at(t: &Tap, key: K, time_ms: u64) -> Verdict {
    let control = CountingControl::default();
    let down = t
        .logic
        .handle(&event(InputKind::Key, key, KeyDir::Down, time_ms), &control);
    t.logic
        .handle(&event(InputKind::Key, key, KeyDir::Up, time_ms), &control);
    down
}

fn press(t: &Tap, key: K) -> Verdict {
    press_at(t, key, 0)
}

#[test]
fn tap_disabled_by_the_system_is_enabled_again_and_keeps_working() {
    let t = tap(true);
    let control = CountingControl::default();
    for _ in 0..2 {
        let ev = event(InputKind::Disabled, K::Other(0), KeyDir::Down, 0);
        assert_eq!(t.logic.handle(&ev, &control), Verdict::Pass);
    }
    assert_eq!(
        control.0.load(Ordering::SeqCst),
        2,
        "перехват включён снова оба раза"
    );
    assert_eq!(t.shared.reenabled.load(Ordering::SeqCst), 2);
    assert_eq!(press(&t, K::KeyG), Verdict::Pass);
    assert_eq!(t.handler.keys.lock().unwrap()[0], (K::KeyG, KeyDir::Down));
}

#[test]
fn without_post_permission_nothing_is_swallowed_and_nothing_is_replaced() {
    let t = tap(false);
    for key in [K::KeyG, K::KeyH, K::Space] {
        assert_eq!(press(&t, key), Verdict::Pass);
    }
    assert!(t.jobs.try_recv().is_err(), "замена не начата");
    assert!(t.handler.keys.lock().unwrap().is_empty());
}

#[test]
fn own_synthetic_events_bypass_the_engine() {
    let t = tap(true);
    let mut ev = event(InputKind::Key, K::KeyG, KeyDir::Down, 0);
    ev.own = true;
    assert_eq!(
        t.logic.handle(&ev, &CountingControl::default()),
        Verdict::Pass
    );
    assert!(t.handler.keys.lock().unwrap().is_empty());
}

#[test]
fn mouse_click_resets_the_word() {
    let t = tap(true);
    let ev = event(InputKind::MouseDown, K::Other(0), KeyDir::Down, 0);
    assert_eq!(
        t.logic.handle(&ev, &CountingControl::default()),
        Verdict::Pass
    );
    assert_eq!(
        *t.handler.resets.lock().unwrap(),
        vec![ResetReason::MouseClick]
    );
}

/// Пишет свои события строками: «Backspace:down», «select:ru», «unicode:…», «replay:KeyC:down».
/// Если задан буфер обмена — копирование и вставка работают с ним.
#[derive(Default)]
struct Desk {
    log: Log,
    clip: Mutex<Clip>,
}

#[derive(Default)]
struct Clip {
    count: i64,
    text: Option<String>,
    /// Что выделено в поле; `None` — пусто, Cmd+C буфер не трогает.
    selection: Option<String>,
}

impl Desk {
    fn push(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }
    fn lines(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Poster<Native> for Desk {
    fn can_type(&self, key: K) -> bool {
        key != K::Pause
    }
    fn key(&self, key: K, down: bool, shift: bool) {
        let dir = if down { "down" } else { "up" };
        let shift = if shift { ":shift" } else { "" };
        self.push(format!("{key}:{dir}{shift}"));
    }
    fn chord(&self, chord: Chord) {
        let mut clip = self.clip.lock().unwrap();
        match chord {
            Chord::Copy => {
                self.push("copy".to_owned());
                if let Some(sel) = clip.selection.clone() {
                    clip.count += 1;
                    clip.text = Some(sel);
                }
            }
            Chord::Paste => self.push(format!("paste:{}", clip.text.clone().unwrap_or_default())),
        }
    }
    fn unicode(&self, text: &str) {
        self.push(format!("unicode:{text}"));
    }
    fn replay(&self, native: &Native) {
        let dir = if native.down { "down" } else { "up" };
        self.push(format!("replay:{}:{dir}", native.key));
    }
    fn pause_ms(&self, _ms: u64) {}
}

impl Clipboard for Desk {
    fn usable_in(&self, app: &AppId) -> bool {
        app.0 != "com.apple.Terminal"
    }
    fn change_count(&self) -> i64 {
        self.clip.lock().unwrap().count
    }
    fn snapshot(&self) -> ClipboardSnapshot {
        let text = self.clip.lock().unwrap().text.clone().unwrap_or_default();
        ClipboardSnapshot(vec![vec![(
            "public.utf8-plain-text".to_owned(),
            text.into_bytes(),
        )]])
    }
    fn restore(&self, snapshot: &ClipboardSnapshot) {
        let mut clip = self.clip.lock().unwrap();
        clip.count += 1;
        clip.text = snapshot
            .0
            .first()
            .and_then(|item| item.first())
            .map(|(_, data)| String::from_utf8(data.clone()).unwrap());
    }
    fn text(&self) -> Option<String> {
        self.clip.lock().unwrap().text.clone()
    }
    fn set_text(&self, text: &str) {
        let mut clip = self.clip.lock().unwrap();
        clip.count += 1;
        clip.text = Some(text.to_owned());
    }
}

struct FakeSwitcher {
    allowed: bool,
    log: Log,
}
impl Switcher for FakeSwitcher {
    fn post_allowed(&self) -> bool {
        self.allowed
    }
    fn select_layout(&self, id: &LayoutId) -> bool {
        self.log.lock().unwrap().push(format!("select:{}", id.0));
        true
    }
}

fn executor(allowed: bool) -> (Executor<Native>, Arc<Desk>) {
    let desk = Arc::new(Desk::default());
    let switcher = Arc::new(FakeSwitcher {
        allowed,
        log: desk.log.clone(),
    });
    let exec = Executor::new(desk.clone(), switcher).with_clipboard(desk.clone());
    (exec, desk)
}

/// Конвейер с часами, стоящими на `clock` (тест двигает их сам).
fn pipeline_at(t: &Tap, exec: Executor<Native>, clock: Arc<AtomicU64>) -> Pipeline<Native> {
    Pipeline::new(
        t.handler.clone(),
        Arc::new(FixedContext(t.app)),
        t.shared.clone(),
        exec,
    )
    .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)))
}

fn pipeline(t: &Tap, exec: Executor<Native>) -> Pipeline<Native> {
    pipeline_at(t, exec, Arc::new(AtomicU64::new(0)))
}

#[test]
fn replace_without_post_permission_fails_and_erases_nothing() {
    let (exec, desk) = executor(false);
    let result = exec.execute(&replace_action());
    assert!(
        matches!(result, Err(BackendError::NoPermission)),
        "ожидался NoPermission"
    );
    assert!(
        desk.lines().is_empty(),
        "ни одного события, раскладка не тронута"
    );
}

#[test]
fn replace_erases_switches_layout_and_retypes_physical_keys() {
    let (exec, desk) = executor(true);
    exec.execute(&replace_action()).unwrap();
    let expected = [
        "Backspace:down",
        "Backspace:up",
        "Backspace:down",
        "Backspace:up",
        "select:ru",
        "KeyG:down",
        "KeyG:up",
        "Space:down",
        "Space:up",
    ];
    assert_eq!(desk.lines(), expected);
}

#[test]
fn keys_typed_during_replacement_are_held_and_replayed_in_order() {
    let t = tap(true);
    assert_eq!(press(&t, K::Space), Verdict::Swallow);
    let job = t.jobs.try_recv().expect("замена передана исполнителю");
    assert!(t.shared.busy());
    assert_eq!(
        press(&t, K::KeyC),
        Verdict::Swallow,
        "нажатие во время замены придержано"
    );
    assert_eq!(press(&t, K::KeyK), Verdict::Swallow);

    let (exec, desk) = executor(true);
    pipeline(&t, exec).run(job);
    assert!(!t.shared.busy());
    let lines = desk.lines();
    assert_eq!(
        lines[lines.len() - 4..],
        [
            "replay:KeyC:down",
            "replay:KeyC:up",
            "replay:KeyK:down",
            "replay:KeyK:up"
        ]
    );
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
    let down = event(InputKind::Key, K::KeyA, KeyDir::Down, 0);
    assert_eq!(
        t.logic.handle(&down, &CountingControl::default()),
        Verdict::Swallow,
        "буква придержана"
    );
    let job = t.jobs.try_recv().expect("действие передано исполнителю");
    let (exec, desk) = executor(true);
    pipeline(&t, exec).run(job);
    assert_eq!(desk.lines(), ["select:ru", "replay:KeyA:down"]);
    assert!(!t.shared.busy());
}

#[test]
fn modifier_with_actions_passes_at_once_and_is_not_replayed() {
    let t = tap(true);
    let shift = event(InputKind::Modifier, K::KeyA, KeyDir::Down, 0);
    assert_eq!(
        t.logic.handle(&shift, &CountingControl::default()),
        Verdict::Pass
    );
    let job = t.jobs.try_recv().expect("действие передано исполнителю");
    assert!(job.then_post.is_none(), "модификатор не воспроизводится");
}

#[test]
fn focus_change_during_replacement_waits_behind_held_keys() {
    let t = tap(true);
    assert_eq!(press(&t, K::Space), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    assert_eq!(press(&t, K::KeyC), Verdict::Swallow);
    t.logic.reset(ResetReason::FocusChanged);
    assert!(
        t.handler.resets.lock().unwrap().is_empty(),
        "сброс не обгоняет замену"
    );
    let (exec, _) = executor(true);
    pipeline(&t, exec).run(job);
    assert_eq!(
        *t.handler.resets.lock().unwrap(),
        vec![ResetReason::FocusChanged]
    );
    t.logic.reset(ResetReason::FocusChanged);
    assert_eq!(
        t.handler.resets.lock().unwrap().len(),
        2,
        "без замены — сразу"
    );
}

struct PanickingPoster;
impl Poster<Native> for PanickingPoster {
    fn can_type(&self, _key: K) -> bool {
        true
    }
    fn key(&self, _key: K, _down: bool, _shift: bool) {
        panic!("сбой исполнителя (тест)");
    }
    fn chord(&self, _chord: Chord) {}
    fn unicode(&self, _text: &str) {}
    fn replay(&self, _native: &Native) {}
    fn pause_ms(&self, _ms: u64) {}
}

#[test]
fn executor_panic_releases_the_keyboard_and_drops_held_keys() {
    let t = tap(true);
    assert_eq!(press(&t, K::Space), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    assert_eq!(press(&t, K::KeyC), Verdict::Swallow);
    let switcher = Arc::new(FakeSwitcher {
        allowed: true,
        log: Log::default(),
    });
    let p = pipeline(&t, Executor::new(Arc::new(PanickingPoster), switcher));
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.run(job)));
    assert!(run.is_err(), "исполнитель упал");
    assert!(!t.shared.busy(), "клавиатура отпущена");
    assert_eq!(t.shared.held(), 0, "удержанное сброшено");
    assert_eq!(
        press(&t, K::KeyK),
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
    assert_eq!(press_at(&t, K::Space, 0), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    assert_eq!(press_at(&t, K::KeyC, 10), Verdict::Swallow);
    let late = BUSY_TIMEOUT_MS + 1;
    assert_eq!(
        press_at(&t, K::KeyK, late),
        Verdict::Pass,
        "срок вышел — нажатие проходит"
    );
    assert!(!t.shared.busy() && t.shared.held() == 0 && t.shared.stalled());
    assert_eq!(
        press_at(&t, K::Space, late + 10),
        Verdict::Pass,
        "пока исполнитель висит, замен нет"
    );
    let (exec, desk) = executor(true);
    pipeline(&t, exec).run(job);
    assert!(desk.lines().is_empty(), "устаревшая замена не исполнена");
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
    let desk = Arc::new(Desk::default());
    let exec: Executor<Native> = Executor::new(desk.clone(), Arc::new(BrokenSwitcher));
    exec.execute(&replace_action()).unwrap();
    assert_eq!(desk.lines().last().unwrap(), "unicode:пр ");
    assert_eq!(exec.stats().layout_switch_failed(), 1);
    assert!(exec.stats().summary().unwrap().contains("ru"));
}

/// Как кончится висящее переключение раскладки.
#[derive(Clone, Copy)]
enum Release {
    Switched,
    TimedOut,
    Panic,
}

/// Переключение раскладки, которое висит до сигнала (как при занятом главном потоке).
struct BlockingSwitcher {
    entered: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<Release>>,
}
impl Switcher for BlockingSwitcher {
    fn post_allowed(&self) -> bool {
        true
    }
    fn select_layout(&self, _id: &LayoutId) -> bool {
        self.entered.lock().unwrap().send(()).unwrap();
        match self.release.lock().unwrap().recv().unwrap() {
            Release::Switched => true,
            Release::TimedOut => false,
            Release::Panic => panic!("сбой после отвисания (тест)"),
        }
    }
}

/// Замена начинает стирать в момент `start_ms` по часам исполнителя и зависает на переключении
/// раскладки; пока висит, пользователь жмёт «C» в момент `press_ms`.
fn hang_in_switch(start_ms: u64, press_ms: u64, release: Release) -> (Tap, Arc<Desk>, Verdict) {
    let t = tap(true);
    assert_eq!(press_at(&t, K::Space, 0), Verdict::Swallow);
    let job = t.jobs.try_recv().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let desk = Arc::new(Desk::default());
    let switcher = Arc::new(BlockingSwitcher {
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let p = pipeline_at(
        &t,
        Executor::new(desk.clone(), switcher),
        Arc::new(AtomicU64::new(start_ms)),
    );
    let worker = std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.run(job)));
    });
    entered_rx.recv().unwrap();
    let verdict = press_at(&t, K::KeyC, press_ms);
    release_tx.send(release).unwrap();
    worker.join().unwrap();
    (t, desk, verdict)
}

const ERASED: [&str; 4] = [
    "Backspace:down",
    "Backspace:up",
    "Backspace:down",
    "Backspace:up",
];

#[test]
fn job_hung_mid_replacement_sends_nothing_after_the_deadline_cut_it_off() {
    let late = BUSY_TIMEOUT_MS + 1;
    let (t, desk, verdict) = hang_in_switch(0, late, Release::Switched);
    assert_eq!(verdict, Verdict::Pass, "срок отпустил клавиатуру");
    assert_eq!(desk.lines(), ERASED, "после отвисания ничего не отправлено");
    assert!(!t.shared.stalled(), "исполнитель отвис — пауза снята");
    assert_eq!(press_at(&t, K::KeyG, BUSY_TIMEOUT_MS + 100), Verdict::Pass);
    let last = *t.handler.keys.lock().unwrap().last().unwrap();
    assert_eq!(last, (K::KeyG, KeyDir::Up), "движок снова получает нажатия");
}

#[test]
fn job_that_panics_after_the_deadline_does_not_leave_corrections_paused() {
    let (t, desk, _) = hang_in_switch(0, BUSY_TIMEOUT_MS + 1, Release::Panic);
    assert_eq!(desk.lines(), ERASED, "только Backspace до зависания");
    assert!(!t.shared.stalled(), "пауза не осталась навсегда");
}

/// Ревью 02 (повторное): замена, начатая поздно в цепочке заданий (за 300 мс до среза
/// первого), не должна попасть под срез посреди переключения раскладки — иначе стёртое
/// слово теряется. Срок продлевается перед первым Backspace, Unicode-допечатка доходит.
#[test]
fn late_replacement_keeps_enough_deadline_to_type_its_unicode_fallback() {
    let (t, desk, verdict) = hang_in_switch(1200, 1600, Release::TimedOut);
    assert_eq!(
        verdict,
        Verdict::Swallow,
        "замена жива — нажатие придержано, не срезано"
    );
    let lines = desk.lines();
    assert_eq!(lines[..4], ERASED);
    assert!(
        lines.contains(&"unicode:пр ".to_owned()),
        "стёртое слово допечатано: {lines:?}"
    );
    assert_eq!(
        lines[lines.len() - 2..],
        ["replay:KeyC:down", "replay:KeyC:up"],
        "придержанное — после замены"
    );
    assert!(!t.shared.busy() && !t.shared.stalled());
}

fn selection_run(app: &'static str, selection: Option<&str>) -> (Arc<FakeHandler>, Arc<Desk>) {
    let t = tap_in(true, app);
    let hotkey = event(InputKind::Key, K::Pause, KeyDir::Down, 0);
    assert_eq!(
        t.logic.handle(&hotkey, &CountingControl::default()),
        Verdict::Swallow
    );
    let job = t
        .jobs
        .try_recv()
        .expect("конвертация выделения передана исполнителю");
    let (exec, desk) = executor(true);
    {
        let mut clip = desk.clip.lock().unwrap();
        clip.text = Some("старое".to_owned());
        clip.selection = selection.map(str::to_owned);
    }
    pipeline(&t, exec).run(job);
    assert!(!t.shared.busy());
    (t.handler.clone(), desk)
}

#[test]
fn selection_is_converted_through_the_clipboard_and_the_clipboard_is_restored() {
    let (handler, desk) = selection_run(TEXTEDIT, Some("ghbdtn"));
    assert_eq!(*handler.converted.lock().unwrap(), ["ghbdtn"]);
    assert_eq!(desk.lines(), ["copy", "paste:ПРИВЕТ"]);
    assert_eq!(
        desk.text().as_deref(),
        Some("старое"),
        "буфер обмена восстановлен"
    );
}

#[test]
fn empty_selection_changes_neither_the_text_nor_the_clipboard() {
    let (handler, desk) = selection_run(TEXTEDIT, None);
    assert!(handler.converted.lock().unwrap().is_empty());
    assert_eq!(desk.lines(), ["copy"], "вставки нет");
    assert_eq!(desk.text().as_deref(), Some("старое"));
    assert_eq!(desk.change_count(), 0, "буфер обмена не тронут");
}

#[test]
fn selection_in_terminal_is_left_alone() {
    let (_, desk) = selection_run("com.apple.Terminal", Some("ghbdtn"));
    assert!(desk.lines().is_empty(), "ни копирования, ни вставки");
    assert_eq!(desk.text().as_deref(), Some("старое"));
}
