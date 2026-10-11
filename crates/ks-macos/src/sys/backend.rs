//! `MacBackend`: связывает поток перехвата, исполнителя замен и кэши, которые колбэк читает
//! без вызовов TIS (текущая раскладка, приложение в фокусе, безопасный ввод, право на ввод).

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::CFStringRef;
use dispatch2::DispatchQueue;
use ks_core::{Action, AppId, Context, LayoutId, LayoutSet};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus,
};
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSRunningApplication, NSWorkspace};

use super::post::CgPoster;
use super::system::{self, MainThreadMarker, on_main};
use super::tap_thread::{self, TapPort};
use crate::exec::{ExecStats, Executor, Switcher};
use crate::tap::{
    BUSY_TIMEOUT_MS, ContextSource, Job, MARK_OWN, Pipeline, TapLogic, TapShared, clock_ms,
};

/// Сколько исполнитель ждёт переключения раскладки на главном потоке; дольше — замена
/// уходит запасным путём (Unicode), чтобы стёртое слово было допечатано до срока
/// `BUSY_TIMEOUT_MS`.
const SWITCH_TIMEOUT: Duration = Duration::from_millis(500);

/// Как часто перечитывается право на эмуляцию ввода.
const PERMISSION_POLL: Duration = Duration::from_secs(2);

struct State {
    shared: Arc<TapShared>,
    layout: Mutex<LayoutId>,
    secure_input: AtomicBool,
    app: Mutex<(i32, AppId)>,
}

impl State {
    /// Зовётся с потока перехвата и исполнителя: вызовы ObjC — внутри своего autorelease pool.
    fn app_for(&self, pid: i32) -> AppId {
        let mut cached = self.app.lock().unwrap_or_else(|e| e.into_inner());
        if cached.0 != pid {
            // NSRunningApplication потокобезопасен (документация AppKit).
            let id = autoreleasepool(|_| {
                NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
                    .and_then(|app| app.bundleIdentifier())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            });
            *cached = (pid, AppId(id));
        }
        cached.1.clone()
    }

    fn set_layout(&self, id: LayoutId) {
        *self.layout.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }
}

impl ContextSource for State {
    fn context(&self, pid: i32) -> Context {
        Context {
            app: if pid == 0 {
                AppId::default()
            } else {
                self.app_for(pid)
            },
            layout: self
                .layout
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
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
            rx.recv_timeout(SWITCH_TIMEOUT).unwrap_or_else(|_| {
                abandoned.store(true, Ordering::Release);
                log::warn!(
                    "главный поток не переключил раскладку за {} мс",
                    SWITCH_TIMEOUT.as_millis()
                );
                false
            })
        };
        if ok {
            self.0.set_layout(id.clone());
        }
        ok
    }
}

/// Колбэк уведомления о смене раскладки; приходит на главный поток.
extern "C" fn on_layout_changed(
    _center: *mut c_void,
    observer: *mut c_void,
    _name: CFStringRef,
    _object: *const c_void,
    _info: CFDictionaryRef,
) {
    // observer — `Arc<State>`, отпущенный в `start` навсегда.
    let state = unsafe { &*(observer as *const State) };
    if let Some(mtm) = MainThreadMarker::new()
        && let Some(id) = system::current_layout(mtm)
    {
        state.set_layout(id);
    }
}

/// Что поднято `start`: исполнитель (один на процесс) и признак живого перехвата.
#[derive(Default)]
struct Runtime {
    jobs: Option<Sender<Job>>,
    handler: Option<Arc<dyn KeyHandler>>,
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
                app: Mutex::new((0, AppId::default())),
            }),
            port: TapPort::default(),
            runtime: Mutex::new(Runtime::default()),
        }
    }

    fn runtime(&self) -> MutexGuard<'_, Runtime> {
        self.runtime.lock().unwrap_or_else(|e| e.into_inner())
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
        });
        let allowed = system::post_allowed();
        self.state
            .shared
            .post_allowed
            .store(allowed, Ordering::Relaxed);
    }

    /// Один раз на процесс: подписка на смену раскладки, опрос прав, поток исполнителя.
    fn start_executor(
        &self,
        rt: &mut Runtime,
        handler: Arc<dyn KeyHandler>,
    ) -> Result<(), BackendError> {
        let observer = Arc::into_raw(self.state.clone()) as *const c_void as usize;
        on_main(move |mtm| {
            // SAFETY: observer — `Arc<State>`, намеренно не отпускаемый до конца процесса.
            unsafe {
                system::observe_layout_changes(mtm, observer as *const c_void, on_layout_changed)
            }
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
        );
        rt.stats = Some(exec.stats());
        let pipeline = Pipeline {
            handler: handler.clone(),
            ctx: self.state.clone(),
            shared: self.state.shared.clone(),
            exec,
        };
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("ks-exec".to_owned())
            .spawn(move || {
                for job in jobs_rx {
                    // Паника в задании не убивает исполнителя; удержание снимает Pipeline.
                    if catch_unwind(AssertUnwindSafe(|| pipeline.run(job))).is_err() {
                        log::error!("задание исполнителя завершилось паникой");
                    }
                }
            })
            .map_err(|e| BackendError::Os(e.to_string()))?;
        rt.jobs = Some(jobs_tx);
        rt.handler = Some(handler);
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
        if rt.jobs.is_none() {
            self.start_executor(&mut rt, handler)?;
        }
        let (Some(jobs), Some(handler)) = (rt.jobs.clone(), rt.handler.clone()) else {
            return Err(BackendError::Os("исполнитель не поднят".to_owned()));
        };
        let logic = TapLogic::new(handler, self.state.clone(), self.state.shared.clone(), jobs);
        tap_thread::spawn(logic, self.state.shared.clone(), self.port.clone())?;
        rt.tap_up = true;
        Ok(())
    }

    /// Через ту же очередь исполнителя, что и замены из колбэка, под тем же удержанием.
    /// С главного потока итог не ждётся (исполнителю нужен главный поток для TIS): задание
    /// ставится в очередь, ответ — `Ok(())`, ошибка уходит в лог.
    fn execute(&self, action: &Action) -> Result<(), BackendError> {
        let Some(jobs) = self.runtime().jobs.clone() else {
            return Err(if system::post_allowed() {
                BackendError::Os("бэкенд не запущен".to_owned())
            } else {
                BackendError::NoPermission
            });
        };
        let shared = &self.state.shared;
        let Some(generation) = shared.begin(clock_ms()) else {
            return Err(BackendError::Os(
                "исполнитель замены не отвечает".to_owned(),
            ));
        };
        let on_main_thread = MainThreadMarker::new().is_some();
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job {
            actions: vec![action.clone()],
            then_post: None,
            generation,
            reply: (!on_main_thread).then_some(reply_tx),
        };
        if jobs.send(job).is_err() {
            shared.abort(generation);
            return Err(BackendError::Os("исполнитель не запущен".to_owned()));
        }
        if on_main_thread {
            return Ok(());
        }
        reply_rx
            .recv_timeout(Duration::from_millis(2 * BUSY_TIMEOUT_MS))
            .unwrap_or_else(|_| Err(BackendError::Os("исполнитель не ответил".to_owned())))
    }

    fn layouts(&self) -> LayoutSet {
        on_main(|mtm| system::enabled_layouts(mtm, system::machine_kbd_type(mtm)))
    }

    fn current_layout(&self) -> LayoutId {
        let state = self.state.clone();
        on_main(move |mtm| {
            let id = system::current_layout(mtm).unwrap_or_default();
            state.set_layout(id.clone());
            id
        })
    }

    fn focused_app(&self) -> (AppId, String) {
        on_main(|_mtm| {
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

    fn open_permission_settings(&self, _kind: PermissionKind) {
        log::info!("раздел «Разрешения» появится в окне настроек (задача 07)");
    }

    fn set_autostart(&self, _on: bool) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(
            "автозапуск ещё не реализован".to_owned(),
        ))
    }

    fn autostart(&self) -> bool {
        false
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
