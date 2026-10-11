//! `KeyHandler` поверх `Engine` под мьютексом. `ToggleAutocorrect` исполняется здесь и
//! из `Outcome` вычищается: бэкенд получает только действия ОС.

use std::sync::{Mutex, MutexGuard};

use ks_core::{
    Action, Config, Context, Engine, EngineStatus, KeyEvent, LayoutSet, Outcome, ResetReason,
};
use ks_platform::KeyHandler;

pub struct EngineHandler {
    engine: Mutex<Engine>,
    config: Mutex<Config>,
    config_error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl EngineHandler {
    pub fn new(engine: Engine, config: Config, config_error: Option<String>) -> EngineHandler {
        EngineHandler {
            engine: Mutex::new(engine),
            config: Mutex::new(config),
            config_error,
        }
    }

    pub fn status(&self) -> EngineStatus {
        match &self.config_error {
            Some(e) => EngineStatus::ConfigError(e.clone()),
            None => lock(&self.engine).status(),
        }
    }

    pub fn autocorrect(&self) -> bool {
        lock(&self.config).autocorrect
    }

    pub fn set_layouts(&self, layouts: LayoutSet) {
        lock(&self.engine).set_layouts(layouts);
    }
}

impl KeyHandler for EngineHandler {
    fn on_key(&self, ev: &KeyEvent, ctx: &Context) -> Outcome {
        let mut engine = lock(&self.engine);
        let mut out = engine.on_key(ev, ctx);
        if out
            .actions
            .iter()
            .any(|a| matches!(a, Action::ToggleAutocorrect))
        {
            out.actions
                .retain(|a| !matches!(a, Action::ToggleAutocorrect));
            let mut config = lock(&self.config);
            config.autocorrect = !config.autocorrect;
            engine.set_config(config.clone());
            log::info!("автоисправление: {}", config.autocorrect);
        }
        out
    }

    fn on_reset(&self, why: ResetReason) {
        lock(&self.engine).on_reset(why);
    }

    fn convert_text(&self, text: &str) -> Option<String> {
        lock(&self.engine).convert_text(text)
    }
}
