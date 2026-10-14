//! Шов оболочки: `Agent` + `EngineHandler` с настоящим движком поверх фальшивого `Backend`.
//! Статусы иконки, меню, команды меню и файл настроек, исключения приложений, таймер.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ks_app::agent::Agent;
use ks_app::status::{MenuEntry, menu_entries};
use ks_core::{
    Action, AppId, Config, Context, Disposition, KeyDir, KeyEvent, Lang, LayoutId, LayoutMap,
    LayoutSet, Level, Modifiers, PhysicalKey as K, ReplaceKind,
};
use ks_platform::{
    Backend, BackendError, BackendHealth, KeyHandler, PermissionKind, PermissionStatus,
    TrayCommand, TrayStatus,
};
use ks_settings::Section;

const US: &str = "com.apple.keylayout.US";
const RU: &str = "com.apple.keylayout.RussianWin";
const TEXTEDIT: &str = "com.apple.TextEdit";
const TERMINAL: &str = "com.apple.Terminal";
const GHBDTN: [K; 6] = [K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN];

fn fixture(file: &str, id: &str, lang: Lang) -> LayoutMap {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ks-core/tests/fixtures")
        .join(file);
    let text = std::fs::read_to_string(path).expect("фикстура");
    let mut map = LayoutMap::new(LayoutId(id.into()), lang, id);
    let levels = [Level::Base, Level::Shift, Level::Caps, Level::ShiftCaps];
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with(['#', '@']))
    {
        let mut cols = line.split('\t');
        let key: K = cols.next().unwrap().parse().expect("код W3C");
        for (level, cell) in levels.into_iter().zip(cols) {
            if let Some(ch) = cell.chars().next() {
                map.insert(key, level, ch);
            }
        }
    }
    map
}

struct FakeBackend {
    granted: AtomicBool,
    secure: AtomicBool,
    with_russian: bool,
    focused: Mutex<(AppId, String)>,
    executed: Mutex<Vec<Action>>,
    autostart_calls: Mutex<Vec<bool>>,
}

impl FakeBackend {
    fn new(with_russian: bool) -> Arc<FakeBackend> {
        Arc::new(FakeBackend {
            granted: AtomicBool::new(true),
            secure: AtomicBool::new(false),
            with_russian,
            focused: Mutex::new((AppId(TEXTEDIT.into()), "TextEdit".into())),
            executed: Mutex::new(Vec::new()),
            autostart_calls: Mutex::new(Vec::new()),
        })
    }
}

impl Backend for FakeBackend {
    fn start(&self, _h: Arc<dyn KeyHandler>) -> Result<(), BackendError> {
        Ok(())
    }
    fn execute(&self, a: &Action) -> Result<(), BackendError> {
        self.executed.lock().unwrap().push(a.clone());
        Ok(())
    }
    fn layouts(&self) -> LayoutSet {
        let mut v = vec![fixture("us.tsv", US, Lang::En)];
        if self.with_russian {
            v.push(fixture("ru_pc.tsv", RU, Lang::Ru));
        }
        LayoutSet(v)
    }
    fn current_layout(&self) -> LayoutId {
        LayoutId(US.into())
    }
    fn focused_app(&self) -> (AppId, String) {
        self.focused.lock().unwrap().clone()
    }
    fn secure_input(&self) -> bool {
        self.secure.load(Ordering::SeqCst)
    }
    fn permissions(&self) -> Vec<PermissionStatus> {
        let granted = self.granted.load(Ordering::SeqCst);
        [
            PermissionKind::InputMonitoring,
            PermissionKind::Accessibility,
        ]
        .into_iter()
        .map(|kind| PermissionStatus {
            kind,
            granted,
            explanation: "зачем".into(),
            fix_hint: "как".into(),
        })
        .collect()
    }
    fn request_permission(&self, _k: PermissionKind) {}
    fn open_permission_settings(&self, _k: PermissionKind) {}
    fn set_autostart(&self, on: bool) -> Result<(), BackendError> {
        self.autostart_calls.lock().unwrap().push(on);
        Ok(())
    }
    fn autostart(&self) -> bool {
        false
    }
    fn health(&self) -> BackendHealth {
        BackendHealth {
            capturing: true,
            ..BackendHealth::default()
        }
    }
}

/// Каталог настроек теста; настоящий каталог KeySwitcher не трогается.
fn config_path(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ks-shell-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.toml")
}

fn agent(test: &str, backend: Arc<FakeBackend>, file: Option<&str>) -> (Arc<Agent>, PathBuf) {
    let path = config_path(test);
    match file {
        Some(text) => std::fs::write(&path, text).unwrap(),
        // классический набор жестов (⇧⇧, одиночный и оба Shift): на нём написаны сценарии;
        // умолчания macOS теперь как в Punto Switcher
        None => ks_core::Config {
            single_shift_switch: true,
            hotkeys: ks_core::HotkeyConfig::classic(ks_core::Os::current()),
            ..ks_core::Config::default()
        }
        .save(&path)
        .unwrap(),
    }
    (
        Arc::new(Agent::with_config_path(backend, Some(path.clone()))),
        path,
    )
}

fn ctx(app: &str, layout: &str) -> Context {
    Context {
        app: AppId(app.into()),
        layout: LayoutId(layout.into()),
        secure_input: false,
        password_field: false,
    }
}

struct Typist {
    t: u64,
}

impl Typist {
    fn key(&mut self, h: &dyn KeyHandler, c: &Context, key: K, dir: KeyDir) -> Vec<Action> {
        self.t += 60;
        let ev = KeyEvent {
            key,
            dir,
            mods: Modifiers {
                shift: key == K::ShiftLeft && dir == KeyDir::Down,
                ..Modifiers::default()
            },
            repeat: false,
            time_ms: self.t,
        };
        h.on_key(&ev, c).actions
    }

    /// Печатает клавиши (нажатие + отпускание); действия всех нажатий.
    fn type_keys(&mut self, h: &dyn KeyHandler, c: &Context, keys: &[K]) -> Vec<Action> {
        let mut all = Vec::new();
        for &k in keys {
            all.extend(self.key(h, c, k, KeyDir::Down));
            all.extend(self.key(h, c, k, KeyDir::Up));
        }
        all
    }
}

fn word_and_space() -> Vec<K> {
    let mut keys = GHBDTN.to_vec();
    keys.push(K::Space);
    keys
}

fn replaced(actions: &[Action], kind: ReplaceKind) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::Replace { kind: k, .. } if *k == kind))
}

fn has_word_item(model: &ks_platform::TrayModel) -> bool {
    menu_entries(model).iter().any(|e| {
        matches!(
            e,
            MenuEntry::Item {
                command: Some(TrayCommand::NeverWord(_)),
                ..
            }
        )
    })
}

#[test]
fn icon_status_follows_permissions_secure_input_layouts_config_and_autocorrect() {
    let backend = FakeBackend::new(true);
    let (a, _) = agent("status", backend.clone(), None);
    assert_eq!(a.refresh().status, TrayStatus::Working);

    backend.granted.store(false, Ordering::SeqCst);
    assert_eq!(a.refresh().status, TrayStatus::NeedsPermissions);
    backend.granted.store(true, Ordering::SeqCst);

    backend.secure.store(true, Ordering::SeqCst);
    assert_eq!(a.refresh().status, TrayStatus::SecureInput);
    backend.secure.store(false, Ordering::SeqCst);
    assert_eq!(a.refresh().status, TrayStatus::Working, "возобновление");

    a.command(&TrayCommand::ToggleAutocorrect);
    let model = a.refresh();
    assert_eq!(model.status, TrayStatus::AutocorrectOff);
    assert!(!model.autocorrect);

    let (no_pair, _) = agent("status-pair", FakeBackend::new(false), None);
    assert_eq!(no_pair.refresh().status, TrayStatus::NeedsLayouts);

    let (broken, _) = agent(
        "status-broken",
        FakeBackend::new(true),
        Some("version = [\n"),
    );
    let model = broken.refresh();
    assert_eq!(model.status, TrayStatus::ConfigError);
    assert!(model.config_error.is_some());
}

#[test]
fn word_item_appears_only_after_undo_and_disappears_after_the_next_correction() {
    let (a, path) = agent("word-item", FakeBackend::new(true), None);
    let before = std::fs::read(&path).unwrap();
    let h = &*a.handler;
    let mut typist = Typist { t: 1000 };
    assert!(!has_word_item(&a.refresh()), "до первой отмены пункта нет");

    let auto = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(replaced(&auto, ReplaceKind::Auto), "«ghbdtn » исправлено");
    assert!(!has_word_item(&a.refresh()));

    let undo = typist.type_keys(h, &ctx(TEXTEDIT, RU), &[K::ShiftLeft, K::ShiftLeft]);
    assert!(replaced(&undo, ReplaceKind::Undo), "⇧⇧ отменил исправление");
    let model = a.refresh();
    assert_eq!(model.undone_word.as_deref(), Some("ghbdtn"));
    let entries = menu_entries(&model);
    assert!(entries.iter().any(|e| matches!(e,
        MenuEntry::Item { label, command: Some(TrayCommand::NeverWord(w)), .. }
            if label == "Не исправлять „ghbdtn“" && w == "ghbdtn")));

    typist.t += 1000;
    // Другое слово: отменённое «ghbdtn» в этом сеансе снова не исправляется (PRD 18c).
    let ckjdj = [K::KeyC, K::KeyK, K::KeyJ, K::KeyD, K::KeyJ, K::Space];
    let again = typist.type_keys(h, &ctx(TEXTEDIT, US), &ckjdj);
    assert!(replaced(&again, ReplaceKind::Auto));
    assert!(
        !has_word_item(&a.refresh()),
        "следующее исправление убрало пункт"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "сеанс набора с исправлением и отменой не изменил файл настроек"
    );
}

#[test]
fn never_word_from_the_menu_is_written_and_the_word_is_left_alone() {
    let (a, path) = agent("never", FakeBackend::new(true), None);
    let h = &*a.handler;
    let mut typist = Typist { t: 1000 };
    typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    typist.type_keys(h, &ctx(TEXTEDIT, RU), &[K::ShiftLeft, K::ShiftLeft]);
    a.command(&TrayCommand::NeverWord("ghbdtn".into()));
    let saved = ks_core::Config::load(&path).unwrap();
    assert_eq!(saved.words.never, ["ghbdtn"]);
    assert!(!has_word_item(&a.refresh()));
    typist.t += 1000;
    let again = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(!replaced(&again, ReplaceKind::Auto), "слово в «никогда»");
}

#[test]
fn menu_commands_change_config_file() {
    let (a, path) = agent("commands", FakeBackend::new(true), None);
    a.command(&TrayCommand::ToggleAutocorrect);
    a.command(&TrayCommand::ToggleApp(AppId(TEXTEDIT.into())));
    a.command(&TrayCommand::ToggleApp(AppId(TERMINAL.into())));
    let saved = ks_core::Config::load(&path).unwrap();
    assert!(!saved.autocorrect);
    assert_eq!(saved.apps.disabled, [AppId(TEXTEDIT.into())]);
    assert_eq!(saved.apps.enabled, [AppId(TERMINAL.into())]);
}

#[test]
fn broken_config_runs_on_defaults_and_is_never_rewritten() {
    let text = "version = 1\n[words]\nnever = [\"rjvg\"\nautocorrect = maybe\n";
    let (a, path) = agent("broken", FakeBackend::new(true), Some(text));
    assert!(a.handler.autocorrect(), "умолчания");
    assert_eq!(a.refresh().status, TrayStatus::ConfigError);
    a.command(&TrayCommand::ToggleAutocorrect);
    a.command(&TrayCommand::NeverWord("ghbdtn".into()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "байт в байт");
    assert!(!a.reload_config() || a.refresh().status == TrayStatus::ConfigError);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn edited_config_is_applied_to_the_engine_without_restart() {
    let (a, path) = agent("reload", FakeBackend::new(true), None);
    let h = &*a.handler;
    let mut typist = Typist { t: 1000 };
    std::fs::write(&path, "version = 1\n[words]\nnever = [\"ghbdtn\"]\n").unwrap();
    assert!(a.reload_config());
    let out = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(
        !replaced(&out, ReplaceKind::Auto),
        "новое правило уже действует"
    );
    assert!(
        !a.reload_config(),
        "те же настройки повторно не применяются"
    );

    std::fs::write(&path, "version = 1\nautocorrect = false\n").unwrap();
    a.reload_config();
    assert_eq!(a.refresh().status, TrayStatus::AutocorrectOff);
}

#[test]
fn config_file_change_is_picked_up_by_the_watcher() {
    let (a, path) = agent("watch", FakeBackend::new(true), None);
    a.watch_config();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(&path, "version = 1\nautocorrect = false\n").unwrap();
    let start = Instant::now();
    while a.handler.autocorrect() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !a.handler.autocorrect(),
        "правка файла применена без перезапуска"
    );
}

#[test]
fn builtin_excluded_terminal_is_not_corrected_until_enabled_from_the_menu() {
    let backend = FakeBackend::new(true);
    let (a, _) = agent("terminal", backend.clone(), None);
    let h = &*a.handler;
    let mut typist = Typist { t: 1000 };
    let out = typist.type_keys(h, &ctx(TERMINAL, US), &word_and_space());
    assert!(
        !replaced(&out, ReplaceKind::Auto),
        "терминал во встроенном списке"
    );

    *backend.focused.lock().unwrap() = (AppId(TERMINAL.into()), "Terminal".into());
    let model = a.refresh();
    let (_, name, excluded) = model.focused_app.clone().unwrap();
    assert!(excluded);
    assert!(menu_entries(&model).iter().any(
        |e| matches!(e, MenuEntry::Item { label, .. } if label == &format!("Исправлять в {name}"))
    ));
    a.command(&TrayCommand::ToggleApp(AppId(TERMINAL.into())));
    typist.t += 1000;
    let out = typist.type_keys(h, &ctx(TERMINAL, US), &word_and_space());
    assert!(replaced(&out, ReplaceKind::Auto), "включено вручную");
}

#[test]
fn app_excluded_from_the_menu_is_not_corrected_and_comes_back_when_enabled() {
    let (a, _) = agent("textedit", FakeBackend::new(true), None);
    let h = &*a.handler;
    let mut typist = Typist { t: 1000 };
    let model = a.refresh();
    assert!(menu_entries(&model).iter().any(
        |e| matches!(e, MenuEntry::Item { label, .. } if label == "Не исправлять в TextEdit")
    ));
    a.command(&TrayCommand::ToggleApp(AppId(TEXTEDIT.into())));
    let out = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(!replaced(&out, ReplaceKind::Auto), "выключено вручную");
    assert_eq!(
        a.refresh().status,
        TrayStatus::Working,
        "автоматика в целом включена"
    );
    a.command(&TrayCommand::ToggleApp(AppId(TEXTEDIT.into())));
    typist.t += 1000;
    let out = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(
        replaced(&out, ReplaceKind::Auto),
        "«Исправлять в …» вернуло"
    );
}

#[test]
fn single_shift_switches_layout_by_timer_without_the_next_key() {
    let backend = FakeBackend::new(true);
    let (a, _) = agent("timer", backend.clone(), None);
    let h: &dyn KeyHandler = &*a.handler;
    let c = ctx(TEXTEDIT, US);
    let release = 1000;
    for (dir, t) in [(KeyDir::Down, release - 50), (KeyDir::Up, release)] {
        let ev = KeyEvent {
            key: K::ShiftLeft,
            dir,
            mods: Modifiers {
                shift: dir == KeyDir::Down,
                ..Modifiers::default()
            },
            repeat: false,
            time_ms: t,
        };
        assert!(h.on_key(&ev, &c).actions.is_empty(), "ждём, не двойной ли");
    }
    // T_double = 400 мс (PRD «Shift-жесты»): срок — отпускание + 400, действует строго после.
    let deadline = release + 400;
    assert_eq!(h.next_deadline(), Some(deadline));
    assert_eq!(ks_app::timer::run_due(h, &*backend, deadline), 0);
    assert!(backend.executed.lock().unwrap().is_empty());
    assert_eq!(ks_app::timer::run_due(h, &*backend, deadline + 1), 1);
    assert_eq!(
        *backend.executed.lock().unwrap(),
        [Action::SwitchLayout(LayoutId(RU.into()))]
    );
    assert_eq!(h.next_deadline(), None);
}

/// Ревью 06: кэш срока для потока таймера идёт за движком после каждого изменения (здесь —
/// сброса): таймер не крутится по устаревшему сроку, а после срабатывания спит до `max_wait`.
#[test]
fn after_reset_the_timer_follows_the_engine_and_then_sleeps_until_max_wait() {
    let backend = FakeBackend::new(true);
    let (a, _) = agent("deadline", backend.clone(), None);
    let h = &*a.handler;
    let c = ctx(TEXTEDIT, US);
    let mut typist = Typist {
        t: ks_platform::clock_ms(),
    };
    typist.key(h, &c, K::ShiftLeft, KeyDir::Down);
    typist.key(h, &c, K::ShiftLeft, KeyDir::Up);
    assert!(h.next_deadline().is_some(), "одиночный Shift ждёт срок");
    h.on_reset(ks_core::ResetReason::MouseClick);
    assert_eq!(
        h.next_deadline(),
        None,
        "сброс снимает срок одиночного Shift"
    );
    let start = Instant::now();
    let due = h.wait_deadline(&ks_platform::clock_ms, Duration::from_millis(900));
    assert!(!due, "срока нет — tick не нужен");
    assert!(
        start.elapsed() >= Duration::from_millis(850),
        "таймер спал до max_wait, а не до старого срока Shift"
    );
    assert_eq!(
        ks_app::timer::run_due(h, &*backend, ks_platform::clock_ms()),
        0
    );
    assert!(
        backend.executed.lock().unwrap().is_empty(),
        "после сброса одиночный Shift раскладку не переключает"
    );
}

/// Ревью 06: команда меню перечитывает файл перед записью — внешняя правка не затирается.
#[test]
fn menu_command_keeps_an_external_edit_not_yet_reloaded() {
    let (a, path) = agent("merge", FakeBackend::new(true), None);
    std::fs::write(&path, "version = 1\n[words]\nnever = [\"rjvg\"]\n").unwrap();
    a.command(&TrayCommand::ToggleApp(AppId(TEXTEDIT.into())));
    let saved = ks_core::Config::load(&path).unwrap();
    assert_eq!(saved.words.never, ["rjvg"], "правка окна настроек цела");
    assert_eq!(saved.apps.disabled, [AppId(TEXTEDIT.into())]);
}

/// Файл испортили руками, а агент ещё не успел это перечитать: команда меню не должна
/// записать поверх него последние рабочие настройки.
#[test]
fn menu_command_never_rewrites_a_file_broken_since_the_last_reload() {
    let (a, path) = agent("broken-edit", FakeBackend::new(true), None);
    let broken = "version = 1\n[words\nnever = [\"rjvg\"\n";
    std::fs::write(&path, broken).unwrap();
    a.command(&TrayCommand::ToggleApp(AppId(TEXTEDIT.into())));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        broken,
        "битый файл перезаписан"
    );
}

/// Окно настроек KeySwitcher: набор в его полях и запись сочетания агент не трогает.
#[test]
fn own_settings_window_is_never_corrected() {
    let (a, _) = agent("own", FakeBackend::new(true), None);
    let mut typist = Typist { t: 1000 };
    let out = typist.type_keys(
        &*a.handler,
        &ctx("com.keyswitcher.app", US),
        &word_and_space(),
    );
    assert!(out.is_empty(), "{out:?}");
}

/// Ревью 06: файл перечитывается после тишины не короче паузы, а не через паузу от первого
/// события (окно настроек может писать файл в несколько приёмов).
#[test]
fn config_is_reread_only_after_a_quiet_settle() {
    let (tx, rx) = std::sync::mpsc::channel();
    let start = Instant::now();
    let sender = std::thread::spawn(move || {
        for _ in 0..4 {
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(600));
    });
    rx.recv().unwrap();
    assert!(ks_app::watch::wait_quiet(&rx, Duration::from_millis(250)));
    let waited = start.elapsed();
    assert!(waited >= Duration::from_millis(530), "{waited:?}");
    sender.join().unwrap();
}

/// Перед каждым `--selftest` агент начинает сеанс движка заново (текущие настройки и
/// раскладки): отменённые раньше слова снова исправляются, файл настроек не трогается.
#[test]
fn fresh_engine_session_forgets_undone_words_without_touching_the_file() {
    let (a, path) = agent("fresh", FakeBackend::new(true), None);
    let before = std::fs::read(&path).unwrap();
    let h = &*a.handler;
    let mut typist = Typist { t: 1000 };
    let auto = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(replaced(&auto, ReplaceKind::Auto), "«ghbdtn » исправлено");
    let undo = typist.type_keys(h, &ctx(TEXTEDIT, RU), &[K::ShiftLeft, K::ShiftLeft]);
    assert!(replaced(&undo, ReplaceKind::Undo), "⇧⇧ отменил исправление");

    typist.t += 1000;
    h.on_reset(ks_core::ResetReason::External);
    let again = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(
        !replaced(&again, ReplaceKind::Auto),
        "отменённое слово в этом сеансе не исправляется"
    );

    a.fresh_engine();
    typist.t += 1000;
    let fresh = typist.type_keys(h, &ctx(TEXTEDIT, US), &word_and_space());
    assert!(
        replaced(&fresh, ReplaceKind::Auto),
        "в новом сеансе слово исправляется снова"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "новый сеанс движка не пишет файл"
    );
}

/// Окна настроек, которые агент открыл бы, — по разделам; процесс не запускается.
fn recording_launcher() -> (ks_app::agent::Launcher, Arc<Mutex<Vec<Option<Section>>>>) {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let log = opened.clone();
    let launcher: ks_app::agent::Launcher = Box::new(move |section| {
        log.lock().unwrap().push(section);
        Ok(None)
    });
    (launcher, opened)
}

/// app-shell «Права не выданы»: агент без прав при старте сам открывает окно на «Разрешениях» —
/// один раз; с правами окно не открывается.
#[test]
fn start_without_permissions_opens_the_window_on_permissions_once() {
    let backend = FakeBackend::new(true);
    backend.granted.store(false, Ordering::SeqCst);
    let (launcher, opened) = recording_launcher();
    let a =
        Agent::with_config_path(backend, Some(config_path("perm-missing"))).with_launcher(launcher);
    a.on_start();
    assert_eq!(*opened.lock().unwrap(), vec![Some(Section::Permissions)]);

    let (launcher, opened) = recording_launcher();
    let a = Agent::with_config_path(FakeBackend::new(true), Some(config_path("perm-granted")))
        .with_launcher(launcher);
    a.on_start();
    assert!(
        opened.lock().unwrap().is_empty(),
        "права есть — окно не открывается"
    );
}

/// «Настройки…» из меню открывает окно на разделе по умолчанию через тот же запуск.
#[test]
fn menu_settings_command_launches_the_window() {
    let (launcher, opened) = recording_launcher();
    let a = Arc::new(
        Agent::with_config_path(FakeBackend::new(true), Some(config_path("menu-settings")))
            .with_launcher(launcher),
    );
    a.command(&TrayCommand::OpenSettings);
    assert_eq!(*opened.lock().unwrap(), vec![None]);
}

/// app-shell «Выключение автозапуска»: окно пишет только `autostart` в файл, а запись входа в
/// систему ставит и убирает агент — при старте и после перечитывания файла.
#[test]
fn autostart_follows_the_settings_file() {
    let backend = FakeBackend::new(true);
    let (launcher, _) = recording_launcher();
    let (a, path) = agent("autostart", backend.clone(), None);
    let a = Arc::try_unwrap(a).ok().unwrap().with_launcher(launcher);
    a.on_start();
    assert_eq!(
        *backend.autostart_calls.lock().unwrap(),
        vec![true],
        "по умолчанию включён"
    );

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("autostart = true"));
    std::fs::write(&path, text.replace("autostart = true", "autostart = false")).unwrap();
    assert!(a.reload_config());
    assert_eq!(*backend.autostart_calls.lock().unwrap(), vec![true, false]);

    std::fs::write(&path, "это не toml [").unwrap();
    a.reload_config();
    assert_eq!(
        *backend.autostart_calls.lock().unwrap(),
        vec![true, false],
        "битый файл автозапуск не трогает"
    );
}

/// «Перезапустить KeySwitcher» вне KeySwitcher.app: агент отвечает отказом и продолжает работать.
#[test]
fn restart_outside_the_app_bundle_is_refused_and_the_agent_keeps_running() {
    let (a, _) = agent("restart", FakeBackend::new(true), None);
    let mut out = Vec::new();
    a.respond("restart", &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(!text.lines().any(|l| l == "OK"), "ответ: {text}");
    assert!(!a.quit_requested());
}

fn event(key: K, dir: KeyDir, mods: Modifiers, time_ms: u64) -> KeyEvent {
    KeyEvent {
        key,
        dir,
        mods,
        repeat: false,
        time_ms,
    }
}

/// Ревью 07: переключение «оба Shift» записывается поверх правки файла, которую агент ещё не
/// перечитал, а применённая при этом правка идёт тем же путём, что слежение (с автозапуском).
#[test]
fn both_shifts_toggle_is_saved_over_an_external_edit_not_yet_reloaded() {
    let backend = FakeBackend::new(true);
    let (a, path) = agent("both-shifts", backend.clone(), None);
    let h = &*a.handler;
    let mut external = Config::load(&path).unwrap();
    assert!(external.autocorrect && external.single_shift_switch && external.autostart);
    external.single_shift_switch = false;
    external.autostart = false;
    external.save(&path).unwrap();

    let c = ctx(TEXTEDIT, US);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for ev in [
        event(K::ShiftLeft, KeyDir::Down, shift, 1_000),
        event(K::ShiftRight, KeyDir::Down, shift, 1_030),
        event(K::ShiftRight, KeyDir::Up, shift, 1_080),
        event(K::ShiftLeft, KeyDir::Up, Modifiers::default(), 1_100),
    ] {
        h.on_key(&ev, &c);
    }
    assert!(!h.autocorrect(), "оба Shift выключили автоисправление");

    a.refresh();
    let saved = Config::load(&path).unwrap();
    assert!(!saved.autocorrect, "переключение записано");
    assert!(!saved.single_shift_switch, "внешняя правка не затёрта");
    assert!(!saved.autostart, "внешняя правка не затёрта");
    assert_eq!(
        backend.autostart_calls.lock().unwrap().last(),
        Some(&false),
        "перечитанный файл применён вместе с автозапуском"
    );
}

/// Ревью 07: назначенное сочетание, нажатое в окне самого KeySwitcher (запись хоткея), действие
/// не запускает; в другом приложении то же сочетание срабатывает.
#[test]
fn assigned_hotkey_pressed_in_own_window_does_nothing() {
    let (a, path) = agent("own-hotkey", FakeBackend::new(true), None);
    let mut config = Config::load(&path).unwrap();
    config.hotkeys.convert_selection = vec!["Ctrl+Alt+KeyK".parse().unwrap()];
    config.save(&path).unwrap();
    assert!(a.reload_config());
    let mods = Modifiers {
        ctrl: true,
        alt: true,
        ..Modifiers::default()
    };
    let press = |app: &str, t: u64| {
        let c = ctx(app, US);
        let down = a.handler.on_key(&event(K::KeyK, KeyDir::Down, mods, t), &c);
        a.handler
            .on_key(&event(K::KeyK, KeyDir::Up, mods, t + 30), &c);
        down
    };
    let elsewhere = press(TEXTEDIT, 1_000);
    assert_eq!(
        elsewhere.actions,
        vec![Action::ConvertSelection],
        "контроль"
    );
    let own = press("com.keyswitcher.app", 2_000);
    assert!(own.actions.is_empty(), "{:?}", own.actions);
    assert_eq!(own.disposition, Disposition::Pass, "нажатие уходит в окно");
}
