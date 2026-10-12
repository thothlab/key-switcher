//! `WindowsBackend`: поток хука `WH_KEYBOARD_LL` + `WH_MOUSE_LL` со своим циклом сообщений,
//! исполнитель замен общей модели (`ks_platform::intercept`), контекст для движка. Колбэк хука
//! только переводит событие (`tap::Tracker`) и отдаёт решение `TapLogic`; замены, смену
//! раскладки и буфер обмена исполняет поток `ks-exec`, поэтому ОС не снимает хук по таймауту
//! (`analysis/platforms.md` §2.1). Снятый всё же хук ловит сторож на таймере потока хука и
//! ставит заново.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::ThreadId;
use std::time::Duration;

use ks_core::{Action, AppId, Context, LayoutId, LayoutSet, ResetReason};
use ks_platform::exec::{ExecStats, Executor, SWITCH_TIMEOUT_MS, Switcher};
use ks_platform::intercept::{
    self, BUSY_TIMEOUT_MS, ContextSource, Job, Pipeline, TapControl, TapLogic, TapShared, Verdict,
};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus,
};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DispatchMessageW, GetMessageW, HC_ACTION, HHOOK, HWND_MESSAGE,
    KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, MSG, MSLLHOOKSTRUCT, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN,
    WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDOWN,
};
use windows::core::{PCWSTR, w};

use super::system::{self, SendInputSink, WinClipOs};
use crate::clip::GuardedClipboard;
use crate::send::Typist;
use crate::tap::{HeldMods, KeyState, RawEvent, RawKind, Tracker, WinEvent, clock_ms};
use crate::{apps, layouts};

/// Шаг таймера потока хука: проверка смены окна на переднем плане и сторож хука.
const TICK_MS: u32 = 250;
/// Система отметила ввод, а хук его не видел дольше этого — хук снят по таймауту.
const MISSED_INPUT_MS: i32 = 1000;
/// Не чаще одной переустановки хука за этот срок (окна администратора хук не видит вовсе).
const REINSTALL_EVERY_MS: u64 = 10_000;
const START_TIMEOUT: Duration = Duration::from_secs(3);
/// Хук молчит дольше этого (зажатый модификатор шлёт автоповтор чаще) — модификаторы
/// пользователя сверяются с ОС: отпускание могло пройти мимо (Win+L, Ctrl+Alt+Del).
const IDLE_RESYNC_MS: u64 = 1000;
/// Тишина после пачки `SendInput`, после которой исполнитель сверяет модификаторы.
const SETTLE_QUIET: Duration = Duration::from_millis(50);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn os(e: windows::core::Error) -> BackendError {
    BackendError::Os(e.message())
}

struct State {
    shared: Arc<TapShared<WinEvent>>,
    /// Окно на переднем плане, его приложение и имя для меню.
    app: Mutex<(isize, AppId, String)>,
    /// Набор `HKL` и таблицы по нему; перестраиваются, когда набор в системе меняется.
    layouts: Mutex<Option<(Vec<isize>, LayoutSet)>>,
    logic: OnceLock<Arc<TapLogic<WinEvent>>>,
    /// Окно-владелец буфера обмена (создаётся потоком хука).
    clip_owner: Arc<AtomicIsize>,
    /// Модификаторы, которые уже применил хук: «зажатое сейчас» для пачек `SendInput`.
    held: Arc<HeldMods>,
}

impl State {
    /// Обновляет приложение, если окно на переднем плане сменилось.
    fn refresh_app(&self) {
        let fg = system::foreground();
        let hwnd = fg.map_or(0, |f| f.0);
        let mut app = lock(&self.app);
        if app.0 == hwnd && hwnd != 0 {
            return;
        }
        let (id, name) = fg
            .and_then(|(_, _, pid)| system::exe_path(pid))
            .map(|path| {
                let id = apps::app_id(&path);
                let name = id.0.strip_suffix(".exe").unwrap_or(&id.0).to_owned();
                (id, name)
            })
            .unwrap_or_default();
        *app = (hwnd, id, name);
    }
}

impl ContextSource for State {
    /// Раскладка — у окна с фокусом на каждом нажатии (риск «раскладка на поток»); приложение —
    /// из кэша, который обновляется при смене окна.
    fn context(&self) -> Context {
        self.refresh_app();
        Context {
            app: lock(&self.app).1.clone(),
            layout: system::current_hkl()
                .map(layouts::layout_id)
                .unwrap_or_default(),
            secure_input: false,
            password_field: false,
        }
    }
}

/// Особых прав на Windows нет; срок смены раскладки держит `system::select_layout`.
struct SysSwitcher;

impl Switcher for SysSwitcher {
    fn post_allowed(&self) -> bool {
        true
    }

    fn select_layout(&self, id: &LayoutId) -> bool {
        layouts::hkl_of(id)
            .is_some_and(|hkl| system::select_layout(hkl, Duration::from_millis(SWITCH_TIMEOUT_MS)))
    }
}

// --- поток хука -------------------------------------------------------------------------------

struct HookCtx {
    logic: Arc<TapLogic<WinEvent>>,
    state: Arc<State>,
    tracker: Tracker,
    /// Время (тики `GetTickCount`) последнего события, которое видел хук.
    last_seen: u32,
    /// Последнее событие клавиатуры по часам `clock_ms`.
    last_key_ms: u64,
    last_fg: isize,
    last_reinstall_ms: u64,
}

thread_local! {
    static CTX: RefCell<Option<HookCtx>> = const { RefCell::new(None) };
    static HOOKS: RefCell<(isize, isize)> = const { RefCell::new((0, 0)) };
}

fn last_input() -> Option<u32> {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe { GetLastInputInfo(&mut info) }
        .as_bool()
        .then_some(info.dwTime)
}

fn install() -> Result<(), BackendError> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(os)?;
    let module = Some(HINSTANCE(module.0));
    let kbd =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0) }.map_err(os)?;
    let mouse = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) } {
        Ok(h) => h.0 as isize,
        Err(e) => {
            log::warn!(
                "хук мыши не поставлен ({}): клик не сбросит слово",
                e.message()
            );
            0
        }
    };
    HOOKS.with(|h| *h.borrow_mut() = (kbd.0 as isize, mouse));
    Ok(())
}

fn uninstall() {
    let (kbd, mouse) = HOOKS.with(|h| std::mem::take(&mut *h.borrow_mut()));
    for hook in [kbd, mouse] {
        if hook != 0 {
            let _ = unsafe { UnhookWindowsHookEx(HHOOK(hook as *mut c_void)) };
        }
    }
}

/// Хук снят системой: ставим заново (вызывается на потоке хука).
struct Reinstall;

impl TapControl for Reinstall {
    fn reenable(&self) {
        uninstall();
        if let Err(e) = install() {
            log::error!("хук клавиатуры не поставлен заново: {e}");
        }
    }
}

fn with_ctx<R>(default: R, f: impl FnOnce(&mut HookCtx) -> R) -> R {
    CTX.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => match guard.as_mut() {
            Some(ctx) => f(ctx),
            None => default,
        },
        Err(_) => default,
    })
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: при HC_ACTION `lparam` — указатель на KBDLLHOOKSTRUCT от системы.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let kind = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => RawKind::KeyDown,
            _ => RawKind::KeyUp,
        };
        let raw = RawEvent {
            kind,
            vk: info.vkCode as u16,
            scan: info.scanCode,
            extended: info.flags.0 & LLKHF_EXTENDED.0 != 0,
            injected: info.flags.0 & LLKHF_INJECTED.0 != 0,
            extra: info.dwExtraInfo,
            time_ms: clock_ms(),
        };
        let verdict = with_ctx(Verdict::Pass, |ctx| {
            ctx.last_seen = info.time;
            ctx.last_key_ms = raw.time_ms;
            let ev = ctx.tracker.input_event(&raw);
            ctx.logic.handle(&ev, &Reinstall)
        });
        if verdict == Verdict::Swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: при HC_ACTION `lparam` — указатель на MSLLHOOKSTRUCT от системы.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let down = matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        );
        with_ctx((), |ctx| {
            // Движение тоже говорит сторожу, что хуки живы; в логику идут только нажатия.
            ctx.last_seen = info.time;
            if down {
                let ev = ctx
                    .tracker
                    .input_event(&RawEvent::other(RawKind::MouseDown, clock_ms()));
                ctx.logic.handle(&ev, &Reinstall);
            }
        });
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Таймер потока хука: смена окна на переднем плане → сброс слова; сторож снятого хука.
fn on_tick() {
    with_ctx((), |ctx| {
        let fg = system::foreground().map_or(0, |f| f.0);
        if fg != ctx.last_fg {
            ctx.last_fg = fg;
            ctx.state.refresh_app();
            log::debug!("окно на переднем плане сменилось");
            ctx.logic.reset(ResetReason::FocusChanged);
        }
        let Some(input) = last_input() else {
            return;
        };
        let now = clock_ms();
        if ctx.tracker.mods().0 != 0 && now.saturating_sub(ctx.last_key_ms) > IDLE_RESYNC_MS {
            ctx.tracker.resync_from_os();
        }
        if input.wrapping_sub(ctx.last_seen) as i32 > MISSED_INPUT_MS
            && now.saturating_sub(ctx.last_reinstall_ms) > REINSTALL_EVERY_MS
        {
            ctx.last_reinstall_ms = now;
            ctx.last_seen = input;
            let ev = ctx
                .tracker
                .input_event(&RawEvent::other(RawKind::Disabled, now));
            ctx.logic.handle(&ev, &Reinstall);
        }
    });
}

fn run_hook(
    logic: Arc<TapLogic<WinEvent>>,
    state: Arc<State>,
    ready: Sender<Result<(), BackendError>>,
) {
    let shared = state.shared.clone();
    let owner = state.clip_owner.clone();
    let ctx = HookCtx {
        logic,
        tracker: Tracker::new(system::caps_on())
            .with_published(state.held.clone())
            .with_key_state(KeyState(Arc::new(system::async_mods))),
        last_seen: last_input().unwrap_or(0),
        last_key_ms: clock_ms(),
        last_fg: system::foreground().map_or(0, |f| f.0),
        last_reinstall_ms: 0,
        state,
    };
    let mut ctx = ctx;
    // Модификаторы, зажатые до установки хука, — из ОС.
    ctx.tracker.resync_from_os();
    CTX.with(|cell| *cell.borrow_mut() = Some(ctx));
    if let Err(e) = install() {
        let _ = ready.send(Err(e));
        return;
    }
    // Невидимое окно-владелец буфера обмена: сообщения ему разбирает цикл этого потока.
    match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("KeySwitcher"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
    } {
        Ok(hwnd) => owner.store(hwnd.0 as isize, Ordering::Release),
        Err(e) => log::warn!(
            "окно буфера обмена не создано ({}): конвертация выделения недоступна",
            e.message()
        ),
    }
    unsafe { SetTimer(None, 0, TICK_MS, None) };
    shared.capturing.store(true, Ordering::Release);
    let _ = ready.send(Ok(()));
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.message == WM_TIMER && msg.hwnd.0.is_null() {
            on_tick();
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    uninstall();
    shared.capturing.store(false, Ordering::Release);
}

fn spawn_hook(logic: Arc<TapLogic<WinEvent>>, state: Arc<State>) -> Result<(), BackendError> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("ks-hook".to_owned())
        .spawn(move || run_hook(logic, state, tx))
        .map_err(|e| BackendError::Os(e.to_string()))?;
    rx.recv_timeout(START_TIMEOUT).unwrap_or_else(|_| {
        Err(BackendError::Os(
            "хук клавиатуры не поднялся за 3 с".to_owned(),
        ))
    })
}

// --- бэкенд -----------------------------------------------------------------------------------

#[derive(Default)]
struct Runtime {
    jobs: Option<Sender<Job<WinEvent>>>,
    stats: Option<Arc<ExecStats>>,
    hook_up: bool,
}

pub struct WindowsBackend {
    state: Arc<State>,
    runtime: Mutex<Runtime>,
    /// Поток, создавший бэкенд (главный, цикл событий трея): с него `execute` не ждёт итога.
    owner: ThreadId,
}

impl Default for WindowsBackend {
    fn default() -> Self {
        WindowsBackend::new()
    }
}

impl WindowsBackend {
    pub fn new() -> WindowsBackend {
        WindowsBackend {
            state: Arc::new(State {
                shared: Arc::new(TapShared::default()),
                app: Mutex::new((0, AppId::default(), String::new())),
                layouts: Mutex::new(None),
                logic: OnceLock::new(),
                clip_owner: Arc::new(AtomicIsize::new(0)),
                held: Arc::default(),
            }),
            runtime: Mutex::new(Runtime::default()),
            owner: std::thread::current().id(),
        }
    }

    /// Один раз на процесс: исполнитель и логика перехвата.
    fn logic(
        &self,
        rt: &mut Runtime,
        handler: Arc<dyn KeyHandler>,
    ) -> Result<Arc<TapLogic<WinEvent>>, BackendError> {
        if let Some(logic) = self.state.logic.get() {
            return Ok(logic.clone());
        }
        // Сверка модификаторов после конца задания: по тишине после последней пачки.
        let (notify, quiet) = mpsc::channel::<()>();
        let typist =
            Arc::new(Typist::new(self.state.held.clone(), SendInputSink).with_notify(notify));
        let settler = typist.clone();
        let spawned = std::thread::Builder::new()
            .name("ks-mods".to_owned())
            .spawn(move || {
                while quiet.recv().is_ok() {
                    loop {
                        match quiet.recv_timeout(SETTLE_QUIET) {
                            Ok(()) => {}
                            Err(RecvTimeoutError::Timeout) => {
                                settler.settle();
                                if !settler.owes() {
                                    break;
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("поток сверки модификаторов не запущен: {e}");
        }
        let exec = Executor::new(typist, Arc::new(SysSwitcher)).with_clipboard(Arc::new(
            GuardedClipboard::new(WinClipOs {
                owner: self.state.clip_owner.clone(),
            }),
        ));
        rt.stats = Some(exec.stats());
        let pipeline = Pipeline::new(
            handler.clone(),
            self.state.clone(),
            self.state.shared.clone(),
            exec,
        );
        let jobs = intercept::spawn_executor(pipeline)?;
        let logic = Arc::new(TapLogic::new(
            handler,
            self.state.clone(),
            self.state.shared.clone(),
            jobs.clone(),
        ));
        let _ = self.state.logic.set(logic.clone());
        rt.jobs = Some(jobs);
        Ok(logic)
    }
}

impl Backend for WindowsBackend {
    /// Идемпотентен: при живом хуке повторный вызов ничего не делает.
    fn start(&self, handler: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        let mut rt = lock(&self.runtime);
        if rt.hook_up && self.state.shared.capturing.load(Ordering::Acquire) {
            log::info!("перехват уже поднят");
            return Ok(());
        }
        // Особого права на SendInput нет; окна администратора UIPI отсекает молча.
        self.state
            .shared
            .post_allowed
            .store(true, Ordering::Relaxed);
        self.state.refresh_app();
        let logic = self.logic(&mut rt, handler)?;
        spawn_hook(logic, self.state.clone())?;
        rt.hook_up = true;
        Ok(())
    }

    /// Через ту же очередь исполнителя, что и замены из хука, под тем же удержанием.
    fn execute(&self, action: &Action) -> Result<(), BackendError> {
        let Some(jobs) = lock(&self.runtime).jobs.clone() else {
            return Err(BackendError::Os("бэкенд не запущен".to_owned()));
        };
        let wait = std::thread::current().id() != self.owner;
        intercept::submit(&self.state.shared, &jobs, action, wait)
    }

    fn layouts(&self) -> LayoutSet {
        let list = system::layout_list();
        let mut cached = lock(&self.state.layouts);
        if let Some((ids, set)) = cached.as_ref()
            && *ids == list
        {
            return set.clone();
        }
        let set = system::build_layouts(&list);
        *cached = Some((list, set.clone()));
        set
    }

    fn current_layout(&self) -> LayoutId {
        system::current_hkl()
            .map(layouts::layout_id)
            .unwrap_or_default()
    }

    fn focused_app(&self) -> (AppId, String) {
        self.state.refresh_app();
        let app = lock(&self.state.app);
        (app.1.clone(), app.2.clone())
    }

    /// Общесистемного флага безопасного ввода на Windows нет (`analysis/platforms.md` §2.6).
    fn secure_input(&self) -> bool {
        false
    }

    /// Особых прав нет.
    fn permissions(&self) -> Vec<PermissionStatus> {
        Vec::new()
    }

    fn request_permission(&self, _kind: PermissionKind) {}

    fn open_permission_settings(&self, _kind: PermissionKind) {}

    fn set_autostart(&self, on: bool) -> Result<(), BackendError> {
        system::set_autostart(on).map_err(BackendError::Os)
    }

    fn autostart(&self) -> bool {
        system::autostart()
    }

    fn health(&self) -> BackendHealth {
        let layouts = self
            .layouts()
            .0
            .iter()
            .map(|m| format!("{} ({:?}, {})", m.name, m.lang, m.id.0))
            .collect();
        let shared = &self.state.shared;
        let mut detail = vec![
            format!(
                "хук ставился заново после снятия системой: {}",
                shared.reenabled.load(Ordering::Relaxed)
            ),
            format!(
                "сбоев исполнителя замены (паника или срок {BUSY_TIMEOUT_MS} мс): {}{}",
                shared.executor_failures.load(Ordering::Relaxed),
                if shared.stalled() {
                    " — сейчас завис, исправления на паузе"
                } else {
                    ""
                }
            ),
            "окна, запущенные от администратора, не поддерживаются".to_owned(),
        ];
        if let Some(summary) = lock(&self.runtime).stats.as_ref().and_then(|s| s.summary()) {
            detail.push(summary);
        }
        BackendHealth {
            capturing: shared.capturing.load(Ordering::Acquire),
            layouts,
            secure_input: false,
            detail: detail.join("\n"),
        }
    }
}
