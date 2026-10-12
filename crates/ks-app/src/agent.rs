//! Агент: движок под мьютексом за `KeyHandler`, перехват бэкенда, таймер ручного управления,
//! слежение за файлом настроек, сокет для `--status` и `--selftest`, трей. Цикл событий — на
//! главном потоке (`tao`): он же обслуживает главную очередь, через которую бэкенд macOS зовёт
//! TIS, и раз в секунду опрашивает безопасный ввод. Окно настроек — отдельный процесс
//! `keyswitcher --settings`; агент открывает его из меню и сам — при старте без прав, ставит и
//! убирает автозапуск по `config.autostart` и перезапускает себя по команде `restart`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ks_core::{Config, ConfigError, Engine, EngineStatus, Lexicon, Os, ResetReason};
use ks_platform::{Backend, BackendError, KeyHandler, TrayCommand, TrayModel};
use ks_settings::Section;

use crate::handler::EngineHandler;
use crate::paths;
use crate::status::{Source, report, tray_status};

/// Настройки из файла. Нет файла — умолчания, и файл создаётся; испорчен — ошибка, файл не
/// трогается. Без каталога настроек — умолчания.
pub fn load_config(path: Option<&Path>) -> Result<Config, ConfigError> {
    let Some(path) = path else {
        return Ok(Config::default_for(Os::current()));
    };
    let existed = path.exists();
    let config = Config::load(path)?;
    if !existed && let Err(e) = config.save(path) {
        log::warn!("файл настроек не создан: {e}");
    }
    Ok(config)
}

/// Что сделать циклу событий после команды меню.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// Запуск окна настроек на разделе (`None` — по умолчанию). Возвращает процесс окна, если он
/// запущен; тесты подменяют запуск и процесс не создают.
pub type Launcher = Box<dyn Fn(Option<Section>) -> std::io::Result<Option<Child>> + Send + Sync>;

pub struct Agent {
    pub backend: Arc<dyn Backend>,
    pub handler: Arc<EngineHandler>,
    lexicon: Arc<Lexicon>,
    config_path: Option<PathBuf>,
    secure_input: AtomicBool,
    layout_ids: Mutex<Vec<String>>,
    watcher: Mutex<Option<crate::watch::Watcher>>,
    launcher: Launcher,
    /// Окна настроек, запущенные агентом; завершённые собираются в `refresh`.
    windows: Mutex<Vec<Child>>,
    /// Отказ «автозапуск не поддерживается» пишется в журнал один раз.
    autostart_unsupported_logged: AtomicBool,
    /// Агент перезапускается: цикл событий выходит.
    quit: AtomicBool,
}

impl Agent {
    pub fn new(backend: Arc<dyn Backend>) -> Agent {
        Agent::with_config_path(backend, paths::config_file())
    }

    /// Агент с явным путём файла настроек (тесты не трогают настоящий каталог настроек).
    pub fn with_config_path(backend: Arc<dyn Backend>, config_path: Option<PathBuf>) -> Agent {
        let loaded = load_config(config_path.as_deref());
        let lexicon = Lexicon::embedded();
        let builtin = lexicon.default_excluded_apps().to_vec();
        let layouts = backend.layouts();
        let layout_ids = layouts.0.iter().map(|m| m.id.0.clone()).collect();
        let engine = Engine::new(Config::default_for(Os::current()), lexicon.clone(), layouts);
        let handler = Arc::new(EngineHandler::new(engine, loaded, builtin));
        Agent {
            backend,
            handler,
            lexicon,
            config_path,
            secure_input: AtomicBool::new(false),
            layout_ids: Mutex::new(layout_ids),
            watcher: Mutex::new(None),
            launcher: Box::new(spawn_settings),
            windows: Mutex::new(Vec::new()),
            autostart_unsupported_logged: AtomicBool::new(false),
            quit: AtomicBool::new(false),
        }
    }

    /// Свой запуск окна настроек (тесты).
    pub fn with_launcher(mut self, launcher: Launcher) -> Agent {
        self.launcher = launcher;
        self
    }

    /// Агент просил цикл событий завершиться (перезапуск).
    pub fn quit_requested(&self) -> bool {
        self.quit.load(Ordering::SeqCst)
    }

    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Просит недостающие права (системный диалог), поднимает перехват, таймер ручного
    /// управления и слежение за файлом настроек.
    pub fn start(self: &Arc<Self>) {
        for p in self.backend.permissions() {
            if !p.granted {
                log::warn!(
                    "нет права {:?}: замена отключена до выдачи и перезапуска",
                    p.kind
                );
                self.backend.request_permission(p.kind);
            }
        }
        let handler: Arc<dyn KeyHandler> = self.handler.clone();
        if let Err(e) = self.backend.start(handler) {
            log::error!("перехват клавиатуры не поднят: {e}");
        }
        crate::timer::spawn(self.handler.clone(), self.backend.clone());
        self.watch_config();
        self.on_start();
    }

    /// Шаги старта без перехвата и потоков: автозапуск по настройкам и окно на «Разрешениях»,
    /// если какого-то права нет (права TCC действуют только на новый процесс — окно объясняет и
    /// перезапускает).
    pub fn on_start(&self) {
        self.sync_autostart();
        if self.backend.permissions().iter().any(|p| !p.granted) {
            self.open_settings(Some(Section::Permissions));
        }
    }

    /// Запись автозапуска следует `config.autostart`. Битый файл её не трогает: в памяти тогда
    /// умолчания, а не выбор пользователя.
    fn sync_autostart(&self) {
        if self.handler.config_error().is_some() {
            return;
        }
        match self.backend.set_autostart(self.handler.config().autostart) {
            Ok(()) => {}
            Err(BackendError::Unsupported(why)) => {
                if !self
                    .autostart_unsupported_logged
                    .swap(true, Ordering::Relaxed)
                {
                    log::info!("автозапуск не поставлен: {why}");
                }
            }
            Err(e) => log::warn!("автозапуск не изменён: {e}"),
        }
    }

    /// Окно настроек; второе окно само поднимает первое и выходит (`ks_settings::instance`).
    fn open_settings(&self, section: Option<Section>) {
        match (self.launcher)(section) {
            Ok(Some(child)) => lock(&self.windows).push(child),
            Ok(None) => {}
            Err(e) => log::error!("окно настроек не запущено: {e}"),
        }
    }

    /// Следит за файлом настроек: правка применяется без перезапуска.
    pub fn watch_config(self: &Arc<Self>) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let agent = Arc::downgrade(self);
        match crate::watch::watch(&path, move || {
            if let Some(agent) = agent.upgrade() {
                agent.reload_config();
            }
        }) {
            Ok(w) => *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(w),
            Err(e) => log::warn!("слежение за файлом настроек не поднято: {e}"),
        }
    }

    /// Перечитывает файл настроек и применяет к движку; `true` — что-то изменилось.
    pub fn reload_config(&self) -> bool {
        let Some(path) = &self.config_path else {
            return false;
        };
        self.apply_loaded(Config::load(path))
    }

    /// Прочитанный файл → движок и запись автозапуска. Один путь для слежения за файлом и для
    /// перечитывания перед записью.
    fn apply_loaded(&self, loaded: Result<Config, ConfigError>) -> bool {
        let changed = self.handler.apply_config(loaded);
        if changed {
            self.sync_autostart();
        }
        changed
    }

    /// Любая запись агента: файл перечитывается (правка окна настроек, до которой слежение ещё
    /// не дошло, не затирается), правка ложится поверх, результат пишется. Битый файл не
    /// трогается — правка остаётся в памяти.
    fn edit_and_save(&self, change: impl FnOnce(&EngineHandler) -> bool) {
        if let Some(path) = &self.config_path
            && let Ok(config) = Config::load(path)
        {
            self.apply_loaded(Ok(config));
        }
        if change(&self.handler) {
            self.save();
        }
    }

    /// Новый сеанс движка с текущими настройками и раскладками (перед `--selftest`).
    pub fn fresh_engine(&self) {
        let engine = Engine::new(
            Config::default_for(Os::current()),
            self.lexicon.clone(),
            self.backend.layouts(),
        );
        self.handler.renew(engine);
        log::info!("новый сеанс движка");
    }

    /// Пишет настройки в файл, если он не испорчен.
    fn save(&self) {
        let (Some(path), Some(config)) = (&self.config_path, self.handler.config_to_save()) else {
            if self.handler.config_error().is_some() {
                log::warn!("файл настроек испорчен: изменение применено, но не записано");
            }
            return;
        };
        if let Err(e) = config.save(path) {
            log::error!("файл настроек не записан: {e}");
        }
    }

    /// Команда меню: правила пишутся в файл, «Настройки…» — отдельный процесс.
    pub fn command(&self, cmd: &TrayCommand) -> Flow {
        match cmd {
            TrayCommand::Quit => return Flow::Quit,
            TrayCommand::OpenSettings => self.open_settings(None),
            _ => self.edit_and_save(|h| h.command(cmd)),
        }
        Flow::Continue
    }

    /// Раз в секунду: безопасный ввод, раскладки, запись переключения хоткеем, модель иконки.
    pub fn refresh(&self) -> TrayModel {
        let secure = self.backend.secure_input();
        if self.secure_input.swap(secure, Ordering::Relaxed) != secure {
            log::info!("безопасный ввод: {secure}");
            if secure {
                self.handler.on_reset(ResetReason::SecureInput);
            }
        }
        let layouts = self.backend.layouts();
        let ids: Vec<String> = layouts.0.iter().map(|m| m.id.0.clone()).collect();
        {
            let mut known = self.layout_ids.lock().unwrap_or_else(|e| e.into_inner());
            if ids != *known {
                log::info!("раскладок в системе: {}", ids.len());
                *known = ids;
                self.handler.set_layouts(layouts);
            }
        }
        if let Some(on) = self.handler.take_autocorrect_toggle() {
            self.edit_and_save(|h| {
                h.set_autocorrect(on);
                true
            });
        }
        lock(&self.windows).retain_mut(|w| matches!(w.try_wait(), Ok(None)));
        self.model(secure)
    }

    fn model(&self, secure: bool) -> TrayModel {
        let engine = self.handler.status();
        let status = tray_status(
            &self.backend.permissions(),
            self.backend.health().capturing,
            secure,
            &engine,
        );
        let (app, name) = self.backend.focused_app();
        let focused_app = (!app.0.is_empty()).then(|| {
            let excluded = self.handler.excludes(&app);
            let name = if name.is_empty() { app.0.clone() } else { name };
            (app, name, excluded)
        });
        TrayModel {
            status,
            autocorrect: self.handler.autocorrect(),
            focused_app,
            undone_word: self.handler.undone_word(),
            config_error: match engine {
                EngineStatus::ConfigError(e) => Some(e),
                _ => None,
            },
        }
    }

    /// Ответ на команду из сокета.
    pub fn respond(&self, command: &str, out: &mut dyn Write) {
        match command {
            "status" => {
                let text = report(
                    &*self.backend,
                    Some(&self.handler.status()),
                    Source::Agent {
                        pid: std::process::id(),
                    },
                );
                let _ = out.write_all(text.as_bytes());
            }
            #[cfg(target_os = "macos")]
            "selftest" => {
                // Память отменённых слов живёт весь сеанс: без нового сеанса сценарии, которые
                // ждут исправления, упали бы на словах, отменённых раньше.
                self.fresh_engine();
                let ok = crate::selftest::run(out, &*self.handler, &*self.backend);
                let _ = writeln!(out, "RESULT: {}", if ok { "PASS" } else { "FAIL" });
            }
            "pid" => {
                let _ = writeln!(out, "{}", std::process::id());
            }
            "restart" => match self.restart() {
                Ok(()) => {
                    let _ = writeln!(out, "OK");
                }
                Err(e) => {
                    let _ = writeln!(out, "перезапуск невозможен: {e}");
                }
            },
            _ => {
                let _ = writeln!(out, "неизвестная команда");
            }
        }
    }

    /// «Перезапустить KeySwitcher» из окна: отвязанный процесс ждёт выхода агента и открывает
    /// KeySwitcher.app заново, цикл событий агента выходит.
    #[cfg(target_os = "macos")]
    fn restart(&self) -> Result<(), String> {
        let bundle =
            ks_macos::bundle::current_bundle().ok_or("агент запущен не из KeySwitcher.app")?;
        ks_macos::bundle::relaunch_after_exit(std::process::id(), &bundle)
            .map_err(|e| e.to_string())?;
        log::info!("перезапуск по просьбе окна настроек");
        self.quit.store(true, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn restart(&self) -> Result<(), String> {
        Err("перезапуск из окна есть только на macOS".to_owned())
    }

    #[cfg(unix)]
    fn serve(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(path) = paths::agent_socket() else {
            return Ok(());
        };
        let agent = self.clone();
        crate::ipc::serve(&path, move |cmd, stream| agent.respond(cmd, stream))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn serve(self: &Arc<Self>) -> anyhow::Result<()> {
        Ok(())
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Окно настроек — отдельный процесс `keyswitcher --settings [--section <имя>]`.
fn spawn_settings(section: Option<Section>) -> std::io::Result<Option<Child>> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--settings");
    if let Some(section) = section {
        cmd.args(["--section", section.name()]);
    }
    cmd.spawn().map(Some)
}

/// Как часто опрашивается безопасный ввод и обновляется иконка.
const REFRESH: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(any(target_os = "macos", windows))]
pub fn run(backend: Arc<dyn Backend>) -> anyhow::Result<()> {
    use std::time::Instant;

    use ks_platform::TrayHost;
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};

    use crate::tray::TrayIconHost;

    let agent = Arc::new(Agent::new(backend));
    agent.serve()?;
    agent.start();

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<()>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = Mutex::new(event_loop.create_proxy());
    let mut wake = Some(move || {
        if let Ok(p) = proxy.lock() {
            let _ = p.send_event(());
        }
    });
    let mut tray = None;
    let mut next_refresh = Instant::now();
    event_loop.run(move |event, _target, control_flow| {
        if let Event::NewEvents(StartCause::Init) = event
            && let Some(wake) = wake.take()
        {
            match TrayIconHost::new(wake) {
                Ok(host) => {
                    let commands = host.commands();
                    tray = Some((host, commands));
                }
                Err(e) => log::error!("иконка в строке меню не создана: {e}"),
            }
        }
        if agent.quit_requested() {
            if let Some(path) = paths::agent_socket() {
                let _ = std::fs::remove_file(path);
            }
            *control_flow = ControlFlow::Exit;
            return;
        }
        if let Some((_, commands)) = &tray {
            while let Ok(cmd) = commands.try_recv() {
                if agent.command(&cmd) == Flow::Quit {
                    if let Some(path) = paths::agent_socket() {
                        let _ = std::fs::remove_file(path);
                    }
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                next_refresh = Instant::now();
            }
        }
        if Instant::now() >= next_refresh {
            let model = agent.refresh();
            if let Some((host, _)) = &tray {
                host.update(&model);
            }
            next_refresh = Instant::now() + REFRESH;
        }
        *control_flow = ControlFlow::WaitUntil(next_refresh);
    })
}

/// Linux: трей StatusNotifierItem из `ks-linux` (`ksni`, без GTK). Нет хоста трея или шины
/// D-Bus — агент работает без значка, состояние видно в `--status`.
#[cfg(target_os = "linux")]
pub fn run(backend: Arc<dyn Backend>) -> anyhow::Result<()> {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Instant;

    use ks_platform::TrayHost;

    let agent = Arc::new(Agent::new(backend));
    agent.serve()?;
    agent.start();
    let tray = match ks_linux::LinuxTray::new(Box::new(linux_tray_view)) {
        Ok(tray) => Some(tray),
        Err(e) => {
            log::error!("значок в трее не создан: {e}");
            None
        }
    };
    let commands = tray.as_ref().map(TrayHost::commands);
    let mut next_refresh = Instant::now();
    while !agent.quit_requested() {
        if Instant::now() >= next_refresh {
            let model = agent.refresh();
            if let Some(tray) = &tray {
                tray.update(&model);
            }
            next_refresh = Instant::now() + REFRESH;
        }
        let wait = next_refresh.saturating_duration_since(Instant::now());
        match commands.as_ref().map(|rx| rx.recv_timeout(wait)) {
            Some(Ok(cmd)) => {
                if agent.command(&cmd) == Flow::Quit {
                    break;
                }
                next_refresh = Instant::now();
            }
            Some(Err(RecvTimeoutError::Timeout)) => {}
            Some(Err(RecvTimeoutError::Disconnected)) | None => std::thread::sleep(wait),
        }
    }
    if let Some(path) = paths::agent_socket() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Меню и подсказка трея Linux — из тех же `status::tooltip` и `status::menu_entries`, что у
/// macOS и Windows.
#[cfg(target_os = "linux")]
fn linux_tray_view(model: &TrayModel) -> ks_linux::tray::TrayView {
    use ks_linux::tray::{TrayEntry, TrayView};

    use crate::status::{MenuEntry, menu_entries, tooltip};

    TrayView {
        tooltip: tooltip(model.status),
        entries: menu_entries(model)
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Separator => TrayEntry::Separator,
                MenuEntry::Item {
                    label,
                    command,
                    checked,
                } => TrayEntry::Item {
                    label,
                    command,
                    checked,
                },
            })
            .collect(),
    }
}
