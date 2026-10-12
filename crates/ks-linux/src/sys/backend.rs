//! `LinuxBackend`: evdev без захвата → общая модель исполнения (`ks_platform::intercept`) через
//! `NoGrab` → исполнитель с uinput и сменой раскладки окружения.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use ks_core::{Action, AppId, Context, KeyDir, LayoutId, LayoutSet, ResetReason};
use ks_platform::exec::{ExecStats, Executor, Poster, SWITCH_TIMEOUT_MS, Switcher};
use ks_platform::intercept::{
    ContextSource, InputKind, Job, Pipeline, TapControl, TapLogic, TapShared, spawn_executor,
    submit,
};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus, clock_ms,
};

use super::desk::{Desk, IO_TIMEOUT};
use super::input::{self, Open, Raw, Sink};
use super::uinput::{LayoutProbe, Uinput};
use super::{x11, xkb};
use crate::desktop::{Environment, Session};
use crate::guard::{Guarded, HeldMods, InputWatch, RELEASE_WAIT_MS};
use crate::ipc::Focus;
use crate::layouts::{LayoutSpec, index_of};
use crate::nograb::{NoGrab, SelectionReader};
use crate::switch::{LayoutControl, select, with_deadline};
use crate::tap::{LinuxEvent, Tracker};
use crate::{autostart, perms};

/// Опрос раскладки и окна с фокусом.
const POLL: Duration = Duration::from_millis(300);
/// Каждый какой опрос заново обходить `/dev/input` и пробовать uinput (≈ 3 с).
const RESCAN_EVERY: u32 = 10;
/// Срок, за который `unicode` узнаёт актуальную раскладку.
const PROBE: Duration = Duration::from_millis(150);

fn read<T: Clone>(l: &RwLock<T>) -> T {
    l.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn write<T>(l: &RwLock<T>, v: T) {
    *l.write().unwrap_or_else(|e| e.into_inner()) = v;
}

struct Ctx {
    focus: Arc<RwLock<Focus>>,
    current: Arc<RwLock<LayoutId>>,
}

impl ContextSource for Ctx {
    fn context(&self) -> Context {
        Context {
            app: AppId(read(&self.focus).0),
            layout: read(&self.current),
            secure_input: false,
            password_field: false,
        }
    }
}

/// Перехват evdev не отключается системой: включать заново нечего.
struct NoControl;

impl TapControl for NoControl {
    fn reenable(&self) {}
}

/// Окружение для `switch::select`: каждый вызов — на своём потоке со сроком из остатка бюджета.
struct EnvControl {
    desk: Arc<Desk>,
    specs: Vec<LayoutSpec>,
}

impl LayoutControl for EnvControl {
    fn switch_to(&self, index: usize, budget: Duration) -> bool {
        let desk = self.desk.clone();
        with_deadline(budget, move || desk.switch_to(index, budget)).unwrap_or(false)
    }

    fn current(&self, budget: Duration) -> Option<usize> {
        let (desk, specs) = (self.desk.clone(), self.specs.clone());
        with_deadline(budget, move || desk.current_index(&specs, budget)).flatten()
    }
}

struct EnvSwitcher {
    desk: Arc<Desk>,
    specs: Arc<RwLock<Vec<LayoutSpec>>>,
    current: Arc<RwLock<LayoutId>>,
    uinput: Arc<Uinput>,
}

impl EnvSwitcher {
    /// Замена возможна: есть виртуальная клавиатура и окружение умеет менять раскладку
    /// (иначе после стирания нечем было бы допечатать в нужной раскладке).
    fn can_post(&self) -> bool {
        self.uinput.ready() && self.desk.env.switch.is_some()
    }
}

impl Switcher for EnvSwitcher {
    fn post_allowed(&self) -> bool {
        self.can_post()
    }

    /// Команда и подтверждение — не дольше `SWITCH_TIMEOUT_MS` вместе; поздний ответ — неудача,
    /// и исполнитель печатает запасным путём по актуальной раскладке.
    fn select_layout(&self, id: &LayoutId) -> bool {
        let specs = read(&self.specs);
        let Some(index) = index_of(&specs, id) else {
            return false;
        };
        let control = EnvControl {
            desk: self.desk.clone(),
            specs,
        };
        let ok = select(&control, index, Duration::from_millis(SWITCH_TIMEOUT_MS));
        if ok {
            write(&self.current, id.clone());
        }
        ok
    }
}

struct Running {
    jobs: Sender<Job<LinuxEvent>>,
    nograb: Arc<NoGrab>,
    stats: Arc<ExecStats>,
}

struct Inner {
    env: Environment,
    desk: Arc<Desk>,
    specs: Arc<RwLock<Vec<LayoutSpec>>>,
    layouts: Arc<RwLock<LayoutSet>>,
    current: Arc<RwLock<LayoutId>>,
    focus: Arc<RwLock<Focus>>,
    shared: Arc<TapShared<LinuxEvent>>,
    uinput: Arc<Uinput>,
    switcher: Arc<Guarded<EnvSwitcher>>,
    open: Open,
    held: Arc<HeldMods>,
    watch: Arc<InputWatch>,
    input_denied: AtomicBool,
    xkb_warned: AtomicBool,
    run: Mutex<Option<Running>>,
}

impl Inner {
    /// Перечитывает список раскладок окружения; таблицы строятся заново только при изменении.
    fn refresh_layouts(&self) {
        let specs = self.desk.specs();
        if specs == read(&self.specs) && !read(&self.layouts).0.is_empty() {
            return;
        }
        let maps: Vec<_> = specs.iter().filter_map(xkb::layout_map).collect();
        if maps.len() < specs.len() && !self.xkb_warned.swap(true, Ordering::Relaxed) {
            log::warn!(
                "таблицы построены для {} раскладок из {} (нужна libxkbcommon.so.0)",
                maps.len(),
                specs.len()
            );
        }
        write(&self.layouts, LayoutSet(maps));
        write(&self.specs, specs);
    }

    /// Активная раскладка и окно с фокусом; `true` — сменилось приложение.
    fn refresh_state(&self) -> bool {
        let specs = read(&self.specs);
        if let Some(id) = self.desk.current(&specs, IO_TIMEOUT) {
            write(&self.current, id);
        }
        if !self.env.knows_focus() {
            return false;
        }
        let focus = self.desk.focused().unwrap_or_default();
        let changed = read(&self.focus).0 != focus.0;
        if changed {
            write(&self.focus, focus);
        }
        changed
    }

    fn sync_flags(&self) {
        self.shared
            .post_allowed
            .store(self.switcher.inner.can_post(), Ordering::Relaxed);
        let reading = !self
            .open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        self.shared.capturing.store(reading, Ordering::Relaxed);
    }

    fn access(&self) -> perms::Access {
        let reading = !self
            .open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        perms::Access {
            input: reading || input::readable(),
            uinput: self.uinput.ready()
                || std::fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/uinput")
                    .is_ok(),
        }
    }
}

fn fix_command() -> String {
    let script = std::env::current_exe().ok().and_then(|exe| {
        perms::script_candidates(&exe)
            .into_iter()
            .find(|p| p.exists())
    });
    perms::fix_command(script.as_deref())
}

fn autostart_file() -> Option<PathBuf> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    Some(autostart::dir(xdg.as_deref(), home.as_deref())?.join(autostart::FILE_NAME))
}

pub struct LinuxBackend {
    inner: Arc<Inner>,
}

impl Default for LinuxBackend {
    fn default() -> Self {
        LinuxBackend::new()
    }
}

impl LinuxBackend {
    pub fn new() -> LinuxBackend {
        let env = Environment::from_process();
        if let Some(reason) = env.unsupported_reason() {
            log::warn!("{reason}");
        }
        let desk = Arc::new(Desk::new(env.clone()));
        let specs = Arc::new(RwLock::new(Vec::new()));
        let layouts = Arc::new(RwLock::new(LayoutSet::default()));
        let current = Arc::new(RwLock::new(LayoutId::default()));
        let held = Arc::new(HeldMods::default());
        let watch = Arc::new(InputWatch::default());
        let probe: LayoutProbe = {
            let (desk, specs) = (desk.clone(), specs.clone());
            Box::new(move || {
                let (desk, specs) = (desk.clone(), read(&specs));
                with_deadline(PROBE, move || desk.current(&specs, PROBE)).flatten()
            })
        };
        let uinput = Arc::new(Uinput::new(layouts.clone(), current.clone(), probe));
        let switcher = Arc::new(Guarded::new(
            EnvSwitcher {
                desk: desk.clone(),
                specs: specs.clone(),
                current: current.clone(),
                uinput: uinput.clone(),
            },
            held.clone(),
            watch.clone(),
            RELEASE_WAIT_MS,
        ));
        let inner = Inner {
            env,
            desk,
            specs,
            layouts,
            current,
            focus: Arc::default(),
            shared: Arc::default(),
            uinput,
            switcher,
            open: Arc::default(),
            held,
            watch,
            input_denied: AtomicBool::new(false),
            xkb_warned: AtomicBool::new(false),
            run: Mutex::new(None),
        };
        inner.refresh_layouts();
        inner.refresh_state();
        LinuxBackend {
            inner: Arc::new(inner),
        }
    }
}

fn watch(inner: Arc<Inner>, logic: Arc<TapLogic<LinuxEvent>>, sink: Sink) {
    let mut n: u32 = 0;
    loop {
        std::thread::sleep(POLL);
        n = n.wrapping_add(1);
        if inner.refresh_state() {
            logic.reset(ResetReason::FocusChanged);
        }
        // Конец задания: всё, что виртуальная клавиатура нажала, отпущено.
        if !inner.shared.busy() {
            inner.uinput.release_all();
        }
        if n.is_multiple_of(RESCAN_EVERY) {
            let denied = input::scan(&inner.open, &sink);
            inner.input_denied.store(denied, Ordering::Relaxed);
            inner.uinput.ensure();
            inner.refresh_layouts();
            inner.sync_flags();
        }
    }
}

impl Backend for LinuxBackend {
    fn start(&self, handler: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        let inner = &self.inner;
        let mut run = inner.run.lock().unwrap_or_else(|e| e.into_inner());
        if run.is_some() {
            return Ok(());
        }
        inner.uinput.ensure();
        let selection: Option<SelectionReader> = (inner.env.session == Session::X11)
            .then(|| Box::new(x11::primary_text) as SelectionReader);
        let nograb = Arc::new(NoGrab::new(
            handler,
            inner.layouts.clone(),
            selection,
            inner.watch.clone(),
        ));
        {
            // Отказ перед первым Backspace: движок считает слово исправленным — сбросить.
            let nograb = nograb.clone();
            inner.watch.set_on_refuse(Box::new(move || {
                nograb.on_reset(ResetReason::External);
            }));
        }
        let ctx: Arc<dyn ContextSource> = Arc::new(Ctx {
            focus: inner.focus.clone(),
            current: inner.current.clone(),
        });
        let poster: Arc<dyn Poster<LinuxEvent>> = inner.uinput.clone();
        let switcher: Arc<dyn Switcher> = inner.switcher.clone();
        let exec = Executor::new(poster, switcher);
        let stats = exec.stats();
        let pipeline = Pipeline::new(nograb.clone(), ctx.clone(), inner.shared.clone(), exec);
        let jobs = spawn_executor(pipeline)?;
        let logic = Arc::new(TapLogic::new(
            nograb.clone(),
            ctx,
            inner.shared.clone(),
            jobs.clone(),
        ));
        let tracker = Arc::new(Mutex::new(Tracker::default()));
        let sink: Sink = {
            let logic = logic.clone();
            let held = inner.held.clone();
            let watch = inner.watch.clone();
            Arc::new(move |raw: Raw| {
                let mut tracker = tracker.lock().unwrap_or_else(|e| e.into_inner());
                match raw {
                    Raw::Lost => {
                        tracker.release_all();
                        held.set(Default::default());
                    }
                    Raw::Event { kind, code, value } => {
                        if let Some(ev) = tracker.input_event(kind, code, value, clock_ms()) {
                            held.set(tracker.mods());
                            let typed = match ev.kind {
                                InputKind::MouseDown => true,
                                InputKind::Key => ev.key.is_some_and(|k| k.dir == KeyDir::Down),
                                _ => false,
                            };
                            if typed {
                                watch.user_input(ev.time_ms);
                            }
                            let _ = logic.handle(&ev, &NoControl);
                        }
                    }
                }
            })
        };
        let denied = input::scan(&inner.open, &sink);
        inner.input_denied.store(denied, Ordering::Relaxed);
        inner.sync_flags();
        let watcher = inner.clone();
        std::thread::Builder::new()
            .name("ks-watch".to_owned())
            .spawn(move || watch(watcher, logic, sink))
            .map_err(|e| BackendError::Os(e.to_string()))?;
        *run = Some(Running {
            jobs,
            nograb,
            stats,
        });
        Ok(())
    }

    fn execute(&self, action: &Action) -> Result<(), BackendError> {
        let inner = &self.inner;
        let (jobs, action) = {
            let run = inner.run.lock().unwrap_or_else(|e| e.into_inner());
            let Some(r) = run.as_ref() else {
                return Err(BackendError::Os("перехват не запущен".to_owned()));
            };
            let action = match action {
                Action::ToggleAutocorrect => return Ok(()),
                Action::ConvertSelection => match r.nograb.convert_selection() {
                    Some(a) => a,
                    None => return Ok(()),
                },
                other => other.clone(),
            };
            (r.jobs.clone(), action)
        };
        if matches!(action, Action::Replace { .. }) {
            if !inner.switcher.inner.can_post() {
                return Err(BackendError::NoPermission);
            }
            inner.watch.arm(clock_ms());
        }
        submit(&inner.shared, &jobs, &action, true)
    }

    fn layouts(&self) -> LayoutSet {
        read(&self.inner.layouts)
    }

    fn current_layout(&self) -> LayoutId {
        read(&self.inner.current)
    }

    fn focused_app(&self) -> (AppId, String) {
        let (id, name) = read(&self.inner.focus);
        (AppId(id), name)
    }

    fn secure_input(&self) -> bool {
        false
    }

    fn permissions(&self) -> Vec<PermissionStatus> {
        perms::statuses(self.inner.access(), &fix_command())
    }

    fn request_permission(&self, _kind: PermissionKind) {
        log::info!(
            "права на устройства ввода выдаются командой: {}",
            fix_command()
        );
    }

    fn open_permission_settings(&self, kind: PermissionKind) {
        self.request_permission(kind);
    }

    fn set_autostart(&self, on: bool) -> Result<(), BackendError> {
        let path = autostart_file()
            .ok_or_else(|| BackendError::Os("не задан HOME: автозапуск не настроить".to_owned()))?;
        let os = |e: std::io::Error| BackendError::Os(format!("{}: {e}", path.display()));
        if on {
            let exe = std::env::current_exe().map_err(os)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(os)?;
            }
            std::fs::write(&path, autostart::entry(&exe)).map_err(os)
        } else {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(os(e)),
                _ => Ok(()),
            }
        }
    }

    fn autostart(&self) -> bool {
        autostart_file()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|c| autostart::is_enabled(&c))
    }

    fn health(&self) -> BackendHealth {
        let inner = &self.inner;
        let reading = inner.open.lock().unwrap_or_else(|e| e.into_inner()).len();
        let mut detail = vec![format!("окружение: {}", inner.env.describe())];
        if let Some(reason) = inner.env.unsupported_reason() {
            detail.push(reason);
        }
        detail.push(format!("устройств ввода читается: {reading}"));
        if inner.input_denied.load(Ordering::Relaxed) {
            detail.push("часть устройств ввода закрыта правами".to_owned());
        }
        detail.push(format!(
            "виртуальная клавиатура uinput: {}",
            if inner.uinput.ready() {
                "есть"
            } else {
                "нет"
            }
        ));
        let run = inner.run.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(summary) = run.as_ref().and_then(|r| r.stats.summary()) {
            detail.push(summary);
        }
        let failures = inner.shared.executor_failures.load(Ordering::Relaxed);
        if failures > 0 {
            detail.push(format!("сбои исполнителя замены: {failures}"));
        }
        BackendHealth {
            capturing: inner.shared.capturing.load(Ordering::Relaxed),
            layouts: read(&inner.specs).iter().map(LayoutSpec::label).collect(),
            secure_input: false,
            detail: detail.join("; "),
        }
    }
}
