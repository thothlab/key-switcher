//! Агент: движок под мьютексом за `KeyHandler`, перехват бэкенда, сокет для `--status` и
//! `--selftest`, трей. Цикл событий — на главном потоке (`tao`): он же обслуживает главную
//! очередь, через которую бэкенд macOS зовёт TIS.

use std::io::Write;
use std::sync::Arc;

use ks_core::{Config, Engine, Lexicon, Os};
use ks_platform::{Backend, KeyHandler, TrayModel};

use crate::handler::EngineHandler;
use crate::paths;
use crate::status::{Source, report, tray_status};

/// Настройки из файла; нет файла — умолчания, и файл создаётся. Ошибка — умолчания и текст ошибки.
pub fn load_config() -> (Config, Option<String>) {
    let default = Config::default_for(Os::current());
    let Some(path) = paths::config_file() else {
        return (default, None);
    };
    let existed = path.exists();
    match Config::load(&path) {
        Ok(config) => {
            if !existed {
                let saved = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| config.save(&path));
                if let Err(e) = saved {
                    log::warn!("файл настроек не создан: {e}");
                }
            }
            (config, None)
        }
        Err(e) => (default, Some(e.to_string())),
    }
}

pub struct Agent {
    pub backend: Arc<dyn Backend>,
    pub handler: Arc<EngineHandler>,
}

impl Agent {
    pub fn new(backend: Arc<dyn Backend>) -> Agent {
        let (config, config_error) = load_config();
        let engine = Engine::new(config.clone(), Lexicon::embedded(), backend.layouts());
        let handler = Arc::new(EngineHandler::new(engine, config, config_error));
        Agent { backend, handler }
    }

    /// Просит недостающие права (системный диалог) и поднимает перехват.
    pub fn start(&self) {
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
    }

    /// Раз в несколько секунд: перечитать раскладки (включили/убрали) и собрать модель иконки.
    pub fn tick(&self, layout_ids: &mut Vec<String>) -> TrayModel {
        let layouts = self.backend.layouts();
        let ids: Vec<String> = layouts.0.iter().map(|m| m.id.0.clone()).collect();
        if ids != *layout_ids {
            log::info!("раскладок в системе: {}", ids.len());
            *layout_ids = ids;
            self.handler.set_layouts(layouts);
        }
        let engine = self.handler.status();
        let status = tray_status(
            &self.backend.permissions(),
            self.backend.health().capturing,
            self.backend.secure_input(),
            &engine,
        );
        TrayModel {
            status,
            autocorrect: self.handler.autocorrect(),
            focused_app: None,
            undone_word: None,
            config_error: match engine {
                ks_core::EngineStatus::ConfigError(e) => Some(e),
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

const TICK: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(any(target_os = "macos", windows))]
pub fn run(backend: Arc<dyn Backend>) -> anyhow::Result<()> {
    use std::sync::Mutex;
    use std::time::Instant;

    use ks_platform::{TrayCommand, TrayHost};
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
    let mut layout_ids = Vec::new();
    let mut next_tick = Instant::now();
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
                if cmd == TrayCommand::Quit {
                    if let Some(path) = paths::agent_socket() {
                        let _ = std::fs::remove_file(path);
                    }
                    *control_flow = ControlFlow::Exit;
                    return;
                }
            }
        }
        if Instant::now() >= next_tick {
            let model = agent.tick(&mut layout_ids);
            if let Some((host, _)) = &tray {
                host.update(&model);
            }
            next_tick = Instant::now() + TICK;
        }
        *control_flow = ControlFlow::WaitUntil(next_tick);
    })
}

/// Linux: трей даёт `ks-linux` (задача Linux-бэкенда); пока агент без иконки.
#[cfg(target_os = "linux")]
pub fn run(backend: Arc<dyn Backend>) -> anyhow::Result<()> {
    let agent = Arc::new(Agent::new(backend));
    agent.serve()?;
    agent.start();
    let mut layout_ids = Vec::new();
    loop {
        let _ = agent.tick(&mut layout_ids);
        std::thread::sleep(TICK);
    }
}
