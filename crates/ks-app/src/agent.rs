//! Агент: движок под мьютексом за `KeyHandler`, перехват бэкенда, таймер ручного управления,
//! слежение за файлом настроек, сокет для `--status` и `--selftest`, трей. Цикл событий — на
//! главном потоке (`tao`): он же обслуживает главную очередь, через которую бэкенд macOS зовёт
//! TIS, и раз в секунду опрашивает безопасный ввод.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ks_core::{Config, ConfigError, Engine, EngineStatus, Lexicon, Os, ResetReason};
use ks_platform::{Backend, KeyHandler, TrayCommand, TrayModel};

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

pub struct Agent {
    pub backend: Arc<dyn Backend>,
    pub handler: Arc<EngineHandler>,
    config_path: Option<PathBuf>,
    secure_input: AtomicBool,
    layout_ids: Mutex<Vec<String>>,
    watcher: Mutex<Option<crate::watch::Watcher>>,
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
        let engine = Engine::new(Config::default_for(Os::current()), lexicon, layouts);
        let handler = Arc::new(EngineHandler::new(engine, loaded, builtin));
        Agent {
            backend,
            handler,
            config_path,
            secure_input: AtomicBool::new(false),
            layout_ids: Mutex::new(layout_ids),
            watcher: Mutex::new(None),
        }
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
        self.handler.apply_config(Config::load(path))
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
            TrayCommand::OpenSettings => open_settings(),
            _ => {
                if self.handler.command(cmd) {
                    self.save();
                }
            }
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
        if self.handler.take_dirty() {
            self.save();
        }
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
                let ok = crate::selftest::run(out, &*self.handler, &*self.backend);
                let _ = writeln!(out, "RESULT: {}", if ok { "PASS" } else { "FAIL" });
            }
            _ => {
                let _ = writeln!(out, "неизвестная команда");
            }
        }
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

/// «Настройки…»: окно настроек — отдельный процесс `keyswitcher --settings`.
fn open_settings() {
    let spawned = std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).arg("--settings").spawn());
    if let Err(e) = spawned {
        log::error!("окно настроек не запущено: {e}");
    }
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

/// Linux: трей даёт `ks-linux` (задача Linux-бэкенда); пока агент без иконки.
#[cfg(target_os = "linux")]
pub fn run(backend: Arc<dyn Backend>) -> anyhow::Result<()> {
    let agent = Arc::new(Agent::new(backend));
    agent.serve()?;
    agent.start();
    loop {
        let _ = agent.refresh();
        std::thread::sleep(REFRESH);
    }
}
