//! `KeyHandler` поверх `Engine` под мьютексом. Здесь же — то, чего движок не знает:
//! `ToggleAutocorrect`, пункт «Не исправлять „…“», подпись исключения приложения для меню и
//! срок таймера ручного управления. Вето исключённого приложения — в движке
//! (`AppRules::excludes`, одно определение). Порядок блокировок: `engine`, затем `state`.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use ks_core::{
    Action, AppId, Config, ConfigError, Context, Engine, EngineStatus, KeyEvent, LayoutSet, Os,
    Outcome, ReplaceKind, ResetReason, WordRules,
};
use ks_platform::{KeyHandler, TrayCommand};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct State {
    /// Настройки пользователя: из файла или умолчания, если файл испорчен.
    config: Config,
    config_error: Option<String>,
    /// Слово для пункта «Не исправлять „…“»: ставится отменой, снимается следующим исправлением.
    undone_word: Option<String>,
    /// Настройки поменялись хоткеем и ещё не записаны в файл.
    dirty: bool,
}

pub struct EngineHandler {
    engine: Mutex<Engine>,
    state: Mutex<State>,
    builtin_excluded: Vec<AppId>,
    deadline: Mutex<Option<u64>>,
    deadline_changed: Condvar,
}

impl EngineHandler {
    /// `builtin_excluded` — встроенный список приложений без автоматики (`Lexicon`).
    pub fn new(
        engine: Engine,
        loaded: Result<Config, ConfigError>,
        builtin_excluded: Vec<AppId>,
    ) -> EngineHandler {
        let handler = EngineHandler {
            engine: Mutex::new(engine),
            state: Mutex::new(State {
                config: Config::default_for(Os::current()),
                config_error: None,
                undone_word: None,
                dirty: false,
            }),
            builtin_excluded,
            deadline: Mutex::new(None),
            deadline_changed: Condvar::new(),
        };
        handler.apply_config(loaded);
        handler
    }

    /// Применяет прочитанный файл настроек. Ошибка — умолчания и текст ошибки (файл не
    /// трогается). `true` — что-то изменилось; те же настройки повторно не применяются, чтобы
    /// своя же запись файла не сбрасывала слово в работе.
    pub fn apply_config(&self, loaded: Result<Config, ConfigError>) -> bool {
        let mut engine = lock(&self.engine);
        let mut state = lock(&self.state);
        let (config, error) = match loaded {
            Ok(config) => (config, None),
            Err(e) => (Config::default_for(Os::current()), Some(e.to_string())),
        };
        if config == state.config && error == state.config_error {
            return false;
        }
        match &error {
            Some(_) => log::warn!("файл настроек не прочитан: работа на умолчаниях"),
            None => log::info!("настройки применены"),
        }
        state.config = config;
        state.config_error = error;
        engine.set_config(state.config.clone());
        true
    }

    pub fn status(&self) -> EngineStatus {
        let engine = lock(&self.engine).status();
        let state = lock(&self.state);
        match (&state.config_error, engine) {
            (Some(e), _) => EngineStatus::ConfigError(e.clone()),
            (None, EngineStatus::NeedsLayouts) => EngineStatus::NeedsLayouts,
            (None, _) if !state.config.autocorrect => EngineStatus::AutocorrectOff,
            (None, _) => EngineStatus::Working,
        }
    }

    pub fn autocorrect(&self) -> bool {
        lock(&self.state).config.autocorrect
    }

    pub fn config(&self) -> Config {
        lock(&self.state).config.clone()
    }

    pub fn config_error(&self) -> Option<String> {
        lock(&self.state).config_error.clone()
    }

    pub fn undone_word(&self) -> Option<String> {
        lock(&self.state).undone_word.clone()
    }

    /// Автоматика выключена в приложении `app` (встроенный список или выбор пользователя) —
    /// для подписи в меню; то же правило движок применяет как вето.
    pub fn excludes(&self, app: &AppId) -> bool {
        lock(&self.state)
            .config
            .apps
            .excludes(app, &self.builtin_excluded)
    }

    pub fn set_layouts(&self, layouts: LayoutSet) {
        lock(&self.engine).set_layouts(layouts);
    }

    /// Команда меню, меняющая настройки. `true` — настройки изменились и их надо записать.
    pub fn command(&self, cmd: &TrayCommand) -> bool {
        let mut engine = lock(&self.engine);
        let mut state = lock(&self.state);
        match cmd {
            TrayCommand::ToggleAutocorrect => {
                state.config.autocorrect = !state.config.autocorrect;
            }
            TrayCommand::ToggleApp(app) => {
                if app.0.is_empty() {
                    return false;
                }
                state.config.apps.toggle(app, &self.builtin_excluded);
            }
            TrayCommand::NeverWord(word) => {
                if !WordRules::accepts(word) || !state.config.words.add(word) {
                    return false;
                }
                state.undone_word = None;
            }
            TrayCommand::OpenSettings | TrayCommand::Quit => return false,
        }
        engine.set_config(state.config.clone());
        true
    }

    /// Настройки для записи в файл; `None`, пока файл испорчен: молча перезаписать его нельзя.
    pub fn config_to_save(&self) -> Option<Config> {
        let state = lock(&self.state);
        state.config_error.is_none().then(|| state.config.clone())
    }

    /// Снимает отметку «изменено хоткеем»; `true` — была.
    pub fn take_dirty(&self) -> bool {
        std::mem::take(&mut lock(&self.state).dirty)
    }

    /// Ждёт, пока срок таймера не пройдёт (строго после `next_deadline`) по часам `now`;
    /// не дольше `max_wait`. `true` — пора звать `tick`.
    pub fn wait_deadline(&self, now: &dyn Fn() -> u64, max_wait: Duration) -> bool {
        let mut at = lock(&self.deadline);
        let started = now();
        loop {
            let t = now();
            let waited = Duration::from_millis(t.saturating_sub(started));
            if waited >= max_wait {
                return at.is_some_and(|d| t > d);
            }
            let left = max_wait - waited;
            let wait = match *at {
                Some(d) if t > d => return true,
                Some(d) => Duration::from_millis(d + 1 - t).min(left),
                None => left,
            };
            at = self
                .deadline_changed
                .wait_timeout(at, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Учитывает действия движка: `ToggleAutocorrect` исполняется здесь и вычищается,
    /// отмена показывает пункт слова, любое другое исправление его убирает.
    fn absorb(&self, engine: &mut Engine, state: &mut State, actions: &mut Vec<Action>) {
        for action in actions.iter() {
            match action {
                Action::ToggleAutocorrect => {
                    state.config.autocorrect = !state.config.autocorrect;
                    state.dirty = true;
                    engine.set_config(state.config.clone());
                    log::info!("автоисправление: {}", state.config.autocorrect);
                }
                Action::Replace {
                    kind: ReplaceKind::Undo,
                    ..
                } => state.undone_word = engine.last_undone().map(str::to_owned),
                Action::Replace { .. } => state.undone_word = None,
                _ => {}
            }
        }
        actions.retain(|a| !matches!(a, Action::ToggleAutocorrect));
        let deadline = engine.next_deadline();
        let mut at = lock(&self.deadline);
        if *at != deadline {
            *at = deadline;
            self.deadline_changed.notify_all();
        }
    }
}

impl KeyHandler for EngineHandler {
    fn on_key(&self, ev: &KeyEvent, ctx: &Context) -> Outcome {
        let mut engine = lock(&self.engine);
        let mut state = lock(&self.state);
        let mut out = engine.on_key(ev, ctx);
        self.absorb(&mut engine, &mut state, &mut out.actions);
        out
    }

    fn on_reset(&self, why: ResetReason) {
        lock(&self.engine).on_reset(why);
    }

    fn convert_text(&self, text: &str) -> Option<String> {
        lock(&self.engine).convert_text(text)
    }

    fn tick(&self, now_ms: u64) -> Vec<Action> {
        let mut engine = lock(&self.engine);
        let mut state = lock(&self.state);
        let mut actions = engine.tick(now_ms);
        self.absorb(&mut engine, &mut state, &mut actions);
        actions
    }

    fn next_deadline(&self) -> Option<u64> {
        lock(&self.engine).next_deadline()
    }
}
