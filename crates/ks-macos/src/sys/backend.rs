//! `MacBackend`: связывает поток перехвата, исполнителя замен (общая модель
//! `ks_platform::intercept`) и кэши, которые колбэк читает без системных вызовов: текущая
//! раскладка, приложение в фокусе (уведомление `NSWorkspace` об активации), безопасный ввод,
//! право на ввод, таблицы раскладок (сбрасываются уведомлением TIS о смене набора).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use block2::RcBlock;
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::CFStringRef;
use dispatch2::DispatchQueue;
use ks_core::{Action, AppId, Context, LayoutId, LayoutSet, ResetReason};
use ks_platform::exec::{ExecStats, Executor, SWITCH_TIMEOUT_MS, Switcher};
use ks_platform::intercept::{
    self, BUSY_TIMEOUT_MS, ContextSource, Job, Pipeline, TapLogic, TapShared,
};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus,
};
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidActivateApplicationNotification};
use objc2_foundation::NSNotification;

use super::clipboard::MacClipboard;
use super::post::CgPoster;
use super::system::{self, MainThreadMarker, on_main};
use super::tap_thread::{self, TapPort};
use crate::tap::{MARK_OWN, MacEvent};

fn launch_agent_file() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(crate::bundle::launch_agent_path(std::path::Path::new(
        &home,
    )))
}

/// Как часто перечитывается право на эмуляцию ввода.
const PERMISSION_POLL: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct State {
    shared: Arc<TapShared<MacEvent>>,
    layout: Mutex<LayoutId>,
    secure_input: AtomicBool,
    /// Приложение в фокусе и его имя: из `NSWorkspace`, не из события.
    app: Mutex<(AppId, String)>,
    /// Таблицы включённых раскладок; `None` — перестроить при следующем запросе.
    layouts: Mutex<Option<LayoutSet>>,
    /// Логика перехвата: через неё сброс по смене фокуса встаёт в очередь удержанного.
    logic: OnceLock<Arc<TapLogic<MacEvent>>>,
}

impl State {
    fn set_layout(&self, id: LayoutId) {
        *lock(&self.layout) = id;
    }

    /// `true` — сменилось приложение (не только имя).
    fn set_app(&self, app: (AppId, String)) -> bool {
        let mut cached = lock(&self.app);
        let changed = cached.0 != app.0;
        *cached = app;
        changed
    }
}

/// Приложение на переднем плане.
fn frontmost(_mtm: MainThreadMarker) -> (AppId, String) {
    autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| {
                let id = app.bundleIdentifier().map(|s| s.to_string());
                let name = app.localizedName().map(|s| s.to_string());
                (AppId(id.unwrap_or_default()), name.unwrap_or_default())
            })
            .unwrap_or_default()
    })
}

impl ContextSource for State {
    fn context(&self) -> Context {
        Context {
            app: lock(&self.app).0.clone(),
            layout: lock(&self.layout).clone(),
            secure_input: self.secure_input.load(Ordering::Relaxed),
            password_field: false,
        }
    }
}

struct SysSwitcher(Arc<State>);

impl Switcher for SysSwitcher {
    fn post_allowed(&self) -> bool {
        let allowed = system::post_allowed();
        self.0.shared.post_allowed.store(allowed, Ordering::Relaxed);
        allowed
    }

    fn select_layout(&self, id: &LayoutId) -> bool {
        let target = id.clone();
        let ok = if let Some(mtm) = MainThreadMarker::new() {
            system::select_layout(mtm, &target)
        } else {
            let (tx, rx) = mpsc::channel();
            let abandoned = Arc::new(AtomicBool::new(false));
            let flag = abandoned.clone();
            DispatchQueue::main().exec_async(move || {
                if flag.load(Ordering::Acquire) {
                    return;
                }
                let ok =
                    MainThreadMarker::new().is_some_and(|mtm| system::select_layout(mtm, &target));
                let _ = tx.send(ok);
            });
            rx.recv_timeout(Duration::from_millis(SWITCH_TIMEOUT_MS))
                .unwrap_or_else(|_| {
                    abandoned.store(true, Ordering::Release);
                    log::warn!("главный поток не переключил раскладку за {SWITCH_TIMEOUT_MS} мс");
                    false
                })
        };
        if ok {
            self.0.set_layout(id.clone());
        }
        ok
    }
}

/// Колбэк уведомления о смене текущей раскладки; приходит на главный поток.
extern "C" fn on_layout_changed(
    _center: *mut c_void,
    observer: *mut c_void,
    _name: CFStringRef,
    _object: *const c_void,
    _info: CFDictionaryRef,
) {
    // observer — `Arc<State>`, отпущенный в `start_executor` навсегда.
    let state = unsafe { &*(observer as *const State) };
    if let Some(mtm) = MainThreadMarker::new()
        && let Some(id) = system::current_layout(mtm)
    {
        state.set_layout(id);
    }
}

/// Колбэк уведомления о смене набора включённых раскладок: таблицы перестроятся по запросу.
extern "C" fn on_enabled_layouts_changed(
    _center: *mut c_void,
    observer: *mut c_void,
    _name: CFStringRef,
    _object: *const c_void,
    _info: CFDictionaryRef,
) {
    let state = unsafe { &*(observer as *const State) };
    *lock(&state.layouts) = None;
}

/// Подписка на активацию приложения (`NSWorkspace`, главный поток). Смена приложения → кэш
/// контекста и `ResetReason::FocusChanged` через логику перехвата.
fn observe_focus(_mtm: MainThreadMarker, state: Arc<State>) {
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if state.set_app(frontmost(mtm)) {
            log::debug!("приложение в фокусе сменилось");
            if let Some(logic) = state.logic.get() {
                logic.reset(ResetReason::FocusChanged);
            }
        }
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: блок живёт, пока жив наблюдатель; наблюдатель не снимается до конца процесса.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &block,
        )
    };
    std::mem::forget(token);
}

/// Что поднято `start`: исполнитель и логика перехвата (по одной на процесс).
#[derive(Default)]
struct Runtime {
    jobs: Option<Sender<Job<MacEvent>>>,
    logic: Option<Arc<TapLogic<MacEvent>>>,
    stats: Option<Arc<ExecStats>>,
    tap_up: bool,
}

pub struct MacBackend {
    state: Arc<State>,
    port: TapPort,
    runtime: Mutex<Runtime>,
}

impl Default for MacBackend {
    fn default() -> Self {
        MacBackend::new()
    }
}

impl MacBackend {
    pub fn new() -> MacBackend {
        MacBackend {
            state: Arc::new(State {
                shared: Arc::new(TapShared::default()),
                layout: Mutex::new(LayoutId::default()),
                secure_input: AtomicBool::new(false),
                app: Mutex::new((AppId::default(), String::new())),
                layouts: Mutex::new(None),
                logic: OnceLock::new(),
            }),
            port: TapPort::default(),
            runtime: Mutex::new(Runtime::default()),
        }
    }

    fn runtime(&self) -> MutexGuard<'_, Runtime> {
        lock(&self.runtime)
    }

    fn refresh_caches(&self) {
        let state = self.state.clone();
        on_main(move |mtm| {
            if let Some(id) = system::current_layout(mtm) {
                state.set_layout(id);
            }
            state
                .secure_input
                .store(system::secure_input(mtm), Ordering::Relaxed);
            state.set_app(frontmost(mtm));
        });
        let allowed = system::post_allowed();
        self.state
            .shared
            .post_allowed
            .store(allowed, Ordering::Relaxed);
    }

    /// Один раз на процесс: подписки (раскладки, фокус), опрос прав, исполнитель, логика.
    fn start_executor(
        &self,
        rt: &mut Runtime,
        handler: Arc<dyn KeyHandler>,
    ) -> Result<(), BackendError> {
        let observer = Arc::into_raw(self.state.clone()) as *const c_void as usize;
        let state = self.state.clone();
        on_main(move |mtm| {
            // SAFETY: observer — `Arc<State>`, намеренно не отпускаемый до конца процесса.
            unsafe {
                let observer = observer as *const c_void;
                system::observe_layout_changes(mtm, observer, on_layout_changed, false);
                system::observe_layout_changes(mtm, observer, on_enabled_layouts_changed, true);
            }
            observe_focus(mtm, state);
        });
        let shared = self.state.shared.clone();
        let _ = std::thread::Builder::new()
            .name("ks-permissions".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(PERMISSION_POLL);
                    let now = system::post_allowed();
                    if shared.post_allowed.swap(now, Ordering::Relaxed) != now {
                        log::info!("право на эмуляцию ввода: {now}");
                    }
                }
            });
        let exec = Executor::new(
            Arc::new(CgPoster { mark: MARK_OWN }),
            Arc::new(SysSwitcher(self.state.clone())),
        )
        .with_clipboard(Arc::new(MacClipboard));
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
        rt.logic = Some(logic);
        Ok(())
    }
}

impl Backend for MacBackend {
    /// Идемпотентен: перехват и исполнитель поднимаются один раз; повторный вызов при живом
    /// перехвате ничего не делает (обработчик первого вызова остаётся), при упавшем — пробует
    /// поднять перехват снова.
    fn start(&self, handler: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        self.refresh_caches();
        let mut rt = self.runtime();
        if rt.tap_up && self.state.shared.capturing.load(Ordering::Acquire) {
            log::info!("перехват уже поднят");
            return Ok(());
        }
        if rt.logic.is_none() {
            self.start_executor(&mut rt, handler)?;
        }
        let Some(logic) = rt.logic.clone() else {
            return Err(BackendError::Os("исполнитель не поднят".to_owned()));
        };
        tap_thread::spawn(logic, self.state.shared.clone(), self.port.clone())?;
        rt.tap_up = true;
        Ok(())
    }

    /// Через ту же очередь исполнителя, что и замены из колбэка, под тем же удержанием.
    /// С главного потока итог не ждётся (исполнителю нужен главный поток для TIS).
    fn execute(&self, action: &Action) -> Result<(), BackendError> {
        let Some(jobs) = self.runtime().jobs.clone() else {
            return Err(if system::post_allowed() {
                BackendError::Os("бэкенд не запущен".to_owned())
            } else {
                BackendError::NoPermission
            });
        };
        let wait = MainThreadMarker::new().is_none();
        intercept::submit(&self.state.shared, &jobs, action, wait)
    }

    /// Из кэша; таблицы строятся заново только после смены набора раскладок в системе.
    fn layouts(&self) -> LayoutSet {
        if let Some(cached) = lock(&self.state.layouts).clone() {
            return cached;
        }
        let built = on_main(|mtm| system::enabled_layouts(mtm, system::machine_kbd_type(mtm)));
        *lock(&self.state.layouts) = Some(built.clone());
        built
    }

    fn current_layout(&self) -> LayoutId {
        let state = self.state.clone();
        on_main(move |mtm| {
            let id = system::current_layout(mtm).unwrap_or_default();
            state.set_layout(id.clone());
            id
        })
    }

    /// Из кэша, который ведёт уведомление об активации; пустой кэш заполняется запросом.
    fn focused_app(&self) -> (AppId, String) {
        let cached = lock(&self.state.app).clone();
        if !cached.0.0.is_empty() {
            return cached;
        }
        let app = on_main(frontmost);
        self.state.set_app(app.clone());
        app
    }

    fn secure_input(&self) -> bool {
        let on = on_main(system::secure_input);
        self.state.secure_input.store(on, Ordering::Relaxed);
        on
    }

    fn permissions(&self) -> Vec<PermissionStatus> {
        let post = system::post_allowed();
        self.state
            .shared
            .post_allowed
            .store(post, Ordering::Relaxed);
        vec![
            PermissionStatus {
                kind: PermissionKind::InputMonitoring,
                granted: system::listen_allowed(),
                explanation: "Мониторинг ввода: видеть нажатия клавиш, чтобы распознать слово, \
                              набранное не в той раскладке."
                    .to_owned(),
                fix_hint: "Системные настройки → Конфиденциальность и безопасность → Мониторинг \
                           ввода: включите KeySwitcher и перезапустите его."
                    .to_owned(),
            },
            PermissionStatus {
                kind: PermissionKind::Accessibility,
                granted: post,
                explanation: "Универсальный доступ: стереть слово и напечатать исправленное, \
                              придержать клавишу на время замены."
                    .to_owned(),
                fix_hint: "Системные настройки → Конфиденциальность и безопасность → \
                           Универсальный доступ: включите KeySwitcher и перезапустите его."
                    .to_owned(),
            },
        ]
    }

    fn request_permission(&self, kind: PermissionKind) {
        match kind {
            PermissionKind::InputMonitoring => system::request_listen(),
            PermissionKind::Accessibility => system::request_post(),
            _ => {}
        }
    }

    /// Раздел «Конфиденциальность и безопасность» Системных настроек — через `/usr/bin/open`, без
    /// главного потока: окно настроек зовёт это со своего главного потока.
    fn open_permission_settings(&self, kind: PermissionKind) {
        let pane = match kind {
            PermissionKind::InputMonitoring => "Privacy_ListenEvent",
            PermissionKind::Accessibility => "Privacy_Accessibility",
            _ => return,
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
        if let Err(e) = std::process::Command::new("/usr/bin/open")
            .arg(url)
            .status()
        {
            log::warn!("системные настройки не открыты: {e}");
        }
    }

    /// Запись `~/Library/LaunchAgents/com.keyswitcher.app.plist` (`crate::bundle`); только для
    /// процесса из KeySwitcher.app.
    fn set_autostart(&self, on: bool) -> Result<(), BackendError> {
        let exe = std::env::current_exe().map_err(|e| BackendError::Os(e.to_string()))?;
        if crate::bundle::bundle_of(&exe).is_none() {
            return Err(BackendError::Unsupported(
                "автозапуск ставится только для KeySwitcher.app".to_owned(),
            ));
        }
        let file = launch_agent_file()
            .ok_or_else(|| BackendError::Os("не задан домашний каталог".to_owned()))?;
        match crate::bundle::apply_launch_agent(&file, &exe, on) {
            Ok(true) => {
                log::info!(
                    "автозапуск {}",
                    if on {
                        "включён"
                    } else {
                        "выключен"
                    }
                );
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(e) => Err(BackendError::Os(format!("запись автозапуска: {e}"))),
        }
    }

    fn autostart(&self) -> bool {
        launch_agent_file().is_some_and(|f| f.is_file())
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
                "перехват включался заново после отключения системой: {}",
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
        ];
        if let Some(summary) = self.runtime().stats.as_ref().and_then(|s| s.summary()) {
            detail.push(summary);
        }
        BackendHealth {
            capturing: shared.capturing.load(Ordering::Acquire) && self.port.enabled(),
            layouts,
            secure_input: self.secure_input(),
            detail: detail.join("\n"),
        }
    }
}
