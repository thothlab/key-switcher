//! Окно на eframe/egui: разделы слева, содержимое справа. Каждое действие — `Settings::edit`
//! (перечитать файл, одна правка, записать); агент сам применяет записанный файл. Раз в полсекунды
//! окно смотрит просьбу второго окна подняться, внешнюю правку файла (команды меню), права и
//! (в разделе «Приложения-исключения») запущенные приложения.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align, Color32, Layout, RichText, TextWrapMode, vec2};
use ks_core::{AppCategory, AppId, Hotkey, config::MAX_PER_ACTION};
use ks_platform::{PermissionKind, PermissionStatus};

use crate::instance::{self, Guard};
use crate::model::{BuiltinRow, Exclusion, HotkeyAction, Settings};
use crate::record::{self, Recorded, Recorder};
use crate::{Host, Section, about};

const POLL: Duration = Duration::from_millis(500);

pub const EMPTY_APPS: &str = "Своих исключений нет — приложение добавляется здесь или пунктом \
                              меню „Не исправлять в …“";
pub const EMPTY_WORDS: &str = "Слов-исключений нет — слово попадает сюда после отмены исправления \
                               и пункта меню „Не исправлять …“ или здесь";

/// Высота строк-полей: поле сочетания, кнопки списков.
const ROW_H: f32 = 26.0;

pub(crate) fn show(section: Section, path: &Path, host: Host, guard: Guard) -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    let size = crate::shot::Shot::size().unwrap_or([820.0, 580.0]);
    #[cfg(not(debug_assertions))]
    let size = [820.0, 580.0];
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("KeySwitcher — настройки")
            .with_inner_size(size)
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    let window = Window::new(section, path, host, guard);
    eframe::run_native(
        "KeySwitcher",
        options,
        Box::new(move |cc| {
            crate::fonts::install(&cc.egui_ctx);
            Ok(Box::new(window))
        }),
    )
    .map_err(|e| anyhow::anyhow!("окно настроек не открылось: {e}"))
}

/// Разделы окна: «Разрешения» — только там, где они бывают (macOS, Linux).
fn visible_sections() -> impl Iterator<Item = Section> {
    Section::ALL
        .into_iter()
        .filter(|s| *s != Section::Permissions || !cfg!(windows))
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn kind_title(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::InputMonitoring => "Мониторинг ввода",
        PermissionKind::Accessibility => "Универсальный доступ",
        PermissionKind::LinuxInputDevices => "Доступ к клавиатуре (/dev/input)",
        PermissionKind::LinuxUinput => "Эмуляция ввода (/dev/uinput)",
    }
}

fn category_title(category: AppCategory) -> &'static str {
    match category {
        AppCategory::Terminal => "Терминалы",
        AppCategory::Ide => "Среды разработки",
        AppCategory::Editor => "Редакторы кода",
        AppCategory::Password => "Менеджеры паролей",
        AppCategory::Remote => "Удалённый доступ",
    }
}

const GRANTED: Color32 = Color32::from_rgb(0x2e, 0x9e, 0x4f);

/// Что сделали в строке раздела «Горячие клавиши».
enum HotkeyEvent {
    /// Флажок действия.
    Toggle(bool),
    /// Щелчок по полю: записать сочетание в ячейку.
    Record(usize),
    /// Готовое сочетание или жест из списка «…».
    Set(usize, Hotkey),
    Remove(usize),
}

struct Window {
    path: PathBuf,
    dir: PathBuf,
    settings: Settings,
    host: Host,
    section: Section,
    word_input: String,
    app_input: String,
    word_msg: Option<String>,
    app_msg: Option<String>,
    hotkey_msg: Option<(HotkeyAction, String)>,
    recording: Option<(HotkeyAction, usize, Recorder)>,
    /// Назначения действий, у которых сняли флажок: флажок обратно — они возвращаются.
    parked: HashMap<HotkeyAction, Vec<Hotkey>>,
    /// Запущенные приложения (идентификатор, имя) для выбора своего исключения.
    running: Vec<(AppId, String)>,
    /// Имена установленных приложений по идентификатору: спрашиваются у системы один раз.
    names: HashMap<AppId, Option<String>>,
    /// Поле «идентификатор вручную» раскрыто.
    manual_app: bool,
    save_error: Option<String>,
    restart_msg: Option<String>,
    /// Идёт перезапуск: ответ потока, который ждёт новый агент.
    restarting: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    permissions: Vec<PermissionStatus>,
    mtime: Option<SystemTime>,
    next_poll: Instant,
    #[cfg(debug_assertions)]
    shot: Option<crate::shot::Shot>,
    _guard: Guard,
}

impl Window {
    fn new(section: Section, path: &Path, host: Host, guard: Guard) -> Window {
        let settings = Settings::load(path, host.builtin.clone());
        let permissions = host
            .backend
            .as_ref()
            .map(|b| b.permissions())
            .unwrap_or_default();
        let running = host
            .backend
            .as_ref()
            .map(|b| b.running_apps())
            .unwrap_or_default();
        Window {
            path: path.to_path_buf(),
            dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            settings,
            host,
            section,
            word_input: String::new(),
            app_input: String::new(),
            word_msg: None,
            app_msg: None,
            hotkey_msg: None,
            recording: None,
            parked: HashMap::new(),
            manual_app: running.is_empty(),
            running,
            names: HashMap::new(),
            save_error: None,
            restart_msg: None,
            restarting: None,
            permissions,
            mtime: mtime(path),
            next_poll: Instant::now() + POLL,
            #[cfg(debug_assertions)]
            shot: crate::shot::Shot::from_env(),
            _guard: guard,
        }
    }

    /// Просьба второго окна подняться, внешняя правка файла, права, запущенные приложения.
    fn poll(&mut self, ctx: &egui::Context) {
        if Instant::now() < self.next_poll {
            return;
        }
        self.next_poll = Instant::now() + POLL;
        if let Some(section) = instance::take_raise(&self.dir) {
            self.go(section);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let now = mtime(&self.path);
        if now != self.mtime {
            self.mtime = now;
            self.settings.reload(&self.path);
        }
        if let Some(backend) = &self.host.backend {
            self.permissions = backend.permissions();
            if self.section == Section::Apps {
                self.running = backend.running_apps();
            }
        }
    }

    fn go(&mut self, section: Section) {
        self.section = section;
        self.recording = None;
    }

    /// Одна правка файла; ошибку показывает вызывающий там, где она понятна.
    fn edit(
        &mut self,
        change: impl FnOnce(&mut Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        let result = self.settings.edit(&self.path, change);
        self.mtime = mtime(&self.path);
        result
    }

    /// Правка без своего места для ошибки: ошибка — под разделом.
    fn save(&mut self, change: impl FnOnce(&mut Settings) -> Result<(), String>) {
        self.save_error = self.edit(change).err();
    }

    fn set_hotkey(&mut self, action: HotkeyAction, slot: usize, hotkey: Hotkey) {
        let shown = record::label(&hotkey);
        let result = self.edit(|s| {
            s.set_hotkey(action, slot, hotkey)
                .map_err(|e| format!("«{shown}»: {e}"))
        });
        if result.is_ok() {
            self.parked.remove(&action);
        }
        self.hotkey_msg = result.err().map(|e| (action, e));
    }

    fn is_recording(&self, action: HotkeyAction, slot: usize) -> bool {
        matches!(&self.recording, Some((a, s, _)) if *a == action && *s == slot)
    }

    fn start_recording(&mut self, action: HotkeyAction, slot: usize) {
        self.hotkey_msg = None;
        self.recording = Some((action, slot, Recorder::new()));
    }

    /// Во время записи события клавиатуры забираются у виджетов: Space и Tab не нажимают кнопки.
    fn record(&mut self, ctx: &egui::Context) {
        let Some((action, slot, recorder)) = self.recording.as_mut() else {
            return;
        };
        let (action, slot) = (*action, *slot);
        // ⌘Пробел во время записи открывает Spotlight, окно уходит в фон: запись снимается, а не
        // висит «Нажмите сочетание…». Только по событию потери фокуса: флаг `focused` egui-winit
        // на macOS до первого события ложный (winit #4371), по нему запись бы не началась.
        let lost_focus = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if lost_focus {
            self.recording = None;
            return;
        }
        let (events, held) = ctx.input_mut(|i| (std::mem::take(&mut i.events), i.modifiers));
        match recorder.feed(&events, held) {
            Recorded::Pending => {}
            Recorded::Cancelled => self.recording = None,
            Recorded::Refused(text) => {
                self.recording = None;
                self.hotkey_msg = Some((action, text.to_owned()));
            }
            Recorded::Done(hotkey) => {
                self.recording = None;
                self.set_hotkey(action, slot, hotkey);
            }
        }
    }

    fn banner(&mut self, ui: &mut egui::Ui) {
        let Some(error) = self.settings.error().map(str::to_owned) else {
            return;
        };
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!(
                "Файл настроек не прочитан: {error}. Работают значения по умолчанию, файл не \
                 перезаписан."
            ),
        );
        if ui.button("Записать умолчания").clicked() {
            self.save_error = self.settings.reset_to_defaults(&self.path).err();
            self.mtime = mtime(&self.path);
        }
        ui.separator();
    }

    fn error_line(ui: &mut egui::Ui, text: &Option<String>) {
        if let Some(text) = text {
            ui.colored_label(ui.visuals().error_fg_color, text);
        }
    }

    /// Флажок с пояснением под ним; `Some` — флажок переключили.
    fn option(ui: &mut egui::Ui, on: bool, title: &str, hint: &str) -> Option<bool> {
        let mut value = on;
        let changed = ui.checkbox(&mut value, title).changed();
        ui.indent(title, |ui| {
            ui.add(egui::Label::new(RichText::new(hint).weak()).wrap_mode(TextWrapMode::Wrap));
        });
        ui.add_space(10.0);
        changed.then_some(value)
    }

    /// Строка списка: название, под ним подпись мелко, справа «Удалить». `true` — удалить.
    fn list_row(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) -> bool {
        let mut remove = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(title);
                if let Some(sub) = subtitle {
                    ui.label(RichText::new(sub).small().weak());
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                remove = ui.button("Удалить").clicked();
            });
        });
        ui.separator();
        remove
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        if self.permissions.iter().any(|p| !p.granted) {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "KeySwitcher не видит клавиатуру: не выданы разрешения.",
                );
                if ui.button("Открыть «Разрешения»").clicked() {
                    self.go(Section::Permissions);
                }
            });
            ui.add_space(10.0);
        }
        let config = self.settings.config().clone();
        if let Some(on) = Self::option(
            ui,
            config.autocorrect,
            "Исправлять раскладку на лету",
            if cfg!(target_os = "macos") {
                "Слово, набранное не в той раскладке, исправляется после пробела: «ghbdtn» → \
                 «привет». Отменить исправление — сразу Cmd+Z или горячей клавишей."
            } else {
                "Слово, набранное не в той раскладке, исправляется после пробела: «ghbdtn» → \
                 «привет». Отменить исправление — сразу Ctrl+Z или горячей клавишей."
            },
        ) {
            self.save(|s| {
                s.set_autocorrect(on);
                Ok(())
            });
        }
        if let Some(on) = Self::option(
            ui,
            config.autostart,
            "Запускать KeySwitcher при входе в систему",
            "Значок в строке меню появится сам после входа.",
        ) {
            self.save(|s| {
                s.set_autostart(on);
                Ok(())
            });
        }
        ui.weak("Изменения применяются сразу, без перезапуска.");
    }

    fn hotkeys(&mut self, ui: &mut egui::Ui) {
        let hint = if cfg!(target_os = "macos") {
            "Щёлкните поле и нажмите сочетание, Esc — отмена. Сочетание из одних модификаторов \
             (Cmd + Option + Shift) срабатывает, когда их отпускают. Сочетания, которые забирает \
             система (Cmd + Пробел), жесты Shift и Pause — в списке «…» рядом с полем."
        } else {
            "Щёлкните поле и нажмите сочетание, Esc — отмена. Сочетание из одних модификаторов \
             (Ctrl + Shift) срабатывает, когда их отпускают. Сочетания, которые забирает система \
             (Win + Пробел), жесты Shift и Pause — в списке «…» рядом с полем."
        };
        ui.add(egui::Label::new(hint).wrap_mode(TextWrapMode::Wrap));
        ui.add_space(12.0);
        let field_w = (ui.available_width() * 0.34).clamp(150.0, 230.0);
        for action in HotkeyAction::ALL {
            self.hotkey_row(ui, action, field_w);
            ui.add_space(8.0);
        }
        ui.separator();
        ui.add_space(4.0);
        let single = self.settings.config().single_shift_switch;
        if let Some(on) = Self::option(
            ui,
            single,
            "Сменить раскладку одиночным Shift",
            "Короткое нажатие Shift без других клавиш переключает раскладку.",
        ) {
            self.save(|s| {
                s.set_single_shift_switch(on);
                Ok(())
            });
        }
    }

    /// Строка действия как в Punto Switcher: название, флажок, поле сочетания, список «…». Второе
    /// назначение (классический набор: двойной Shift и Pause) — второй строкой под первым.
    fn hotkey_row(&mut self, ui: &mut egui::Ui, action: HotkeyAction, field_w: f32) {
        let current = self.settings.hotkeys(action).to_vec();
        let adding = !current.is_empty() && self.is_recording(action, current.len());
        let rows = current.len().max(1) + usize::from(adding);
        let spacing = ui.spacing().item_spacing.x;
        let side = ui.spacing().interact_size.y + field_w + 2.0 * 28.0 + 4.0 * spacing;
        let label_w = (ui.available_width() - side).max(120.0);
        let mut event = None;
        for slot in 0..rows {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(label_w, ROW_H),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_width(label_w);
                        if slot == 0 {
                            ui.add(
                                egui::Label::new(format!("{}:", action.label()))
                                    .wrap_mode(TextWrapMode::Wrap),
                            );
                        }
                    },
                );
                if slot == 0 {
                    let mut on = !current.is_empty() || self.is_recording(action, 0);
                    if ui.add(egui::Checkbox::without_text(&mut on)).changed() {
                        event = Some(HotkeyEvent::Toggle(on));
                    }
                } else {
                    ui.add_space(ui.spacing().interact_size.y);
                }
                let recording = self.is_recording(action, slot);
                let text = match (recording, current.get(slot)) {
                    (true, _) => RichText::new("Нажмите сочетание…"),
                    (false, Some(h)) => RichText::new(record::label(h)),
                    (false, None) => RichText::new("не задано").weak(),
                };
                let field = ui
                    .add(
                        egui::Button::new(text)
                            .min_size(vec2(field_w, ROW_H))
                            .selected(recording)
                            .truncate(),
                    )
                    .on_hover_text("Щёлкните и нажмите сочетание");
                if field.clicked() {
                    event = Some(HotkeyEvent::Record(slot));
                }
                let assigned = current.get(slot).is_some();
                ui.menu_button("…", |ui| {
                    ui.label(RichText::new("Готовые сочетания").weak());
                    for preset in record::presets().into_iter().chain(record::gestures()) {
                        if ui.button(record::label(&preset)).clicked() {
                            event = Some(HotkeyEvent::Set(slot, preset));
                        }
                    }
                    if assigned {
                        ui.separator();
                        if ui.button("Убрать").clicked() {
                            event = Some(HotkeyEvent::Remove(slot));
                        }
                    }
                })
                .response
                .on_hover_text("Готовые сочетания и жесты");
                // Третье назначение окно не предлагает (PRD 31).
                if slot == 0
                    && current.len() == 1
                    && !adding
                    && MAX_PER_ACTION > 1
                    && ui
                        .small_button("+")
                        .on_hover_text("Добавить второе сочетание")
                        .clicked()
                {
                    event = Some(HotkeyEvent::Record(1));
                }
            });
        }
        if let Some((a, msg)) = &self.hotkey_msg
            && *a == action
        {
            ui.horizontal(|ui| {
                ui.add_space(label_w + spacing);
                ui.colored_label(ui.visuals().error_fg_color, msg);
            });
        }
        match event {
            Some(HotkeyEvent::Toggle(false)) => {
                self.hotkey_msg = None;
                if matches!(&self.recording, Some((a, _, _)) if *a == action) {
                    self.recording = None;
                }
                let mut parked = Vec::new();
                self.save(|s| {
                    parked = s.clear_hotkeys(action);
                    Ok(())
                });
                if !parked.is_empty() {
                    self.parked.insert(action, parked);
                }
            }
            Some(HotkeyEvent::Toggle(true)) => match self.parked.remove(&action) {
                Some(parked) => {
                    let result =
                        self.edit(|s| s.restore_hotkeys(action, parked).map_err(|e| e.to_string()));
                    if let Err(e) = result {
                        self.hotkey_msg = Some((action, e));
                        self.recording = Some((action, 0, Recorder::new()));
                    }
                }
                None => self.start_recording(action, 0),
            },
            Some(HotkeyEvent::Record(slot)) => self.start_recording(action, slot),
            Some(HotkeyEvent::Set(slot, hotkey)) => {
                self.recording = None;
                self.set_hotkey(action, slot, hotkey);
            }
            Some(HotkeyEvent::Remove(slot)) => {
                self.hotkey_msg = None;
                self.save(|s| {
                    s.remove_hotkey(action, slot);
                    Ok(())
                });
            }
            None => {}
        }
    }

    /// Название приложения для списка: встроенный список, запущенные, система; иначе — id.
    fn app_title(&mut self, app: &AppId) -> String {
        if let Some(name) = self.settings.builtin_name(app) {
            return name.to_owned();
        }
        if let Some((_, name)) = self.running.iter().find(|(id, _)| id == app) {
            return name.clone();
        }
        let backend = self.host.backend.clone();
        self.names
            .entry(app.clone())
            .or_insert_with(|| backend.and_then(|b| b.app_name(app)))
            .clone()
            .unwrap_or_else(|| app.0.clone())
    }

    fn add_app(&mut self, id: String) {
        match self.edit(|s| s.add_app(&id).map_err(|e| e.to_string())) {
            Ok(()) => {
                self.app_input.clear();
                self.app_msg = None;
            }
            Err(e) => self.app_msg = Some(e),
        }
    }

    fn apps(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::Label::new(
                "В этих приложениях KeySwitcher не исправляет слова на лету. Горячие клавиши \
                 работают везде.",
            )
            .wrap_mode(TextWrapMode::Wrap),
        );
        ui.add_space(12.0);
        ui.strong("Свои исключения");
        ui.add_space(4.0);
        let own = self.settings.config().apps.disabled.clone();
        if own.is_empty() {
            ui.weak(EMPTY_APPS);
            ui.add_space(4.0);
        }
        let mut removed: Option<AppId> = None;
        for app in &own {
            let title = self.app_title(app);
            let subtitle = (title != app.0).then_some(app.0.as_str());
            if Self::list_row(ui, &title, subtitle) {
                removed = Some(app.clone());
            }
        }
        if let Some(app) = removed {
            self.save(move |s| {
                s.remove_app(&app);
                Ok(())
            });
        }

        let candidates: Vec<(AppId, String)> = self
            .running
            .iter()
            .filter(|(id, _)| {
                !self
                    .settings
                    .config()
                    .apps
                    .excludes(id, self.settings.builtin())
            })
            .cloned()
            .collect();
        let mut chosen: Option<AppId> = None;
        ui.horizontal(|ui| {
            if !self.running.is_empty() {
                egui::ComboBox::from_id_salt("add-app")
                    .selected_text("Добавить запущенное приложение…")
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if candidates.is_empty() {
                            ui.weak("Все запущенные приложения уже в исключениях");
                        }
                        for (id, name) in &candidates {
                            if ui
                                .selectable_label(false, name)
                                .on_hover_text(&id.0)
                                .clicked()
                            {
                                chosen = Some(id.clone());
                            }
                        }
                    });
                let toggle = if self.manual_app {
                    "Скрыть ввод вручную"
                } else {
                    "Ввести идентификатор…"
                };
                if ui.small_button(toggle).clicked() {
                    self.manual_app = !self.manual_app;
                }
            }
        });
        if let Some(id) = chosen {
            self.add_app(id.0);
        }
        if self.manual_app {
            let hint = if cfg!(target_os = "macos") {
                "bundle id, например com.apple.TextEdit"
            } else if cfg!(windows) {
                "имя exe, например notepad.exe"
            } else {
                "app_id или WM_CLASS, например org.gnome.TextEditor"
            };
            let mut add = false;
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.app_input)
                        .hint_text(hint)
                        .desired_width(260.0),
                );
                add = ui.button("Добавить").clicked()
                    || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            });
            if add {
                self.add_app(self.app_input.clone());
            }
        }
        Self::error_line(ui, &self.app_msg);

        ui.add_space(16.0);
        ui.strong("Встроенный список");
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Отмечено — автоисправление здесь выключено, так по умолчанию. Снимите флажок, \
                     чтобы KeySwitcher работал в приложении.",
                )
                .weak(),
            )
            .wrap_mode(TextWrapMode::Wrap),
        );
        ui.add_space(4.0);
        let rows = self.settings.builtin_rows();
        let mut toggled: Option<(BuiltinRow, bool)> = None;
        let mut categories: Vec<AppCategory> = rows.iter().map(|r| r.category).collect();
        categories.dedup();
        for category in categories {
            let in_category: Vec<&BuiltinRow> =
                rows.iter().filter(|r| r.category == category).collect();
            egui::CollapsingHeader::new(category_title(category))
                .id_salt(("builtin", category_title(category)))
                .default_open(true)
                .show(ui, |ui| {
                    for row in in_category {
                        let state = self.settings.exclusion(row);
                        let mut excluded = state == Exclusion::All;
                        let ids = row
                            .ids
                            .iter()
                            .map(|id| id.0.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let check = ui
                            .add(
                                egui::Checkbox::new(&mut excluded, row.title)
                                    .indeterminate(state == Exclusion::Partly),
                            )
                            .on_hover_text(ids);
                        if check.changed() {
                            toggled = Some((row.clone(), excluded));
                        }
                        if !row.members.is_empty() {
                            ui.indent(("members", row.title), |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(row.members.join(", ")).small().weak(),
                                    )
                                    .wrap_mode(TextWrapMode::Wrap),
                                );
                            });
                        }
                    }
                });
        }
        if let Some((row, excluded)) = toggled {
            self.save(move |s| {
                s.set_excluded(&row, excluded);
                Ok(())
            });
        }
    }

    fn words(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::Label::new(
                "Эти слова KeySwitcher не исправляет никогда. Слово из одной-двух букв \
                 добавить нельзя: короткие слова решаются по соседним.",
            )
            .wrap_mode(TextWrapMode::Wrap),
        );
        ui.add_space(12.0);
        let words = self.settings.config().words.never.clone();
        if words.is_empty() {
            ui.weak(EMPTY_WORDS);
            ui.add_space(4.0);
        }
        let mut removed: Option<String> = None;
        for word in &words {
            if Self::list_row(ui, word, None) {
                removed = Some(word.clone());
            }
        }
        if let Some(word) = removed {
            self.save(move |s| {
                s.remove_word(&word);
                Ok(())
            });
        }
        let mut add = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.word_input)
                    .hint_text("слово")
                    .desired_width(260.0),
            );
            add = ui.button("Добавить").clicked()
                || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        });
        if add {
            let word = self.word_input.clone();
            match self.edit(|s| s.add_word(&word).map_err(|e| e.to_string())) {
                Ok(()) => {
                    self.word_input.clear();
                    self.word_msg = None;
                }
                Err(e) => self.word_msg = Some(e),
            }
        }
        Self::error_line(ui, &self.word_msg);
        if !words.is_empty() {
            ui.add_space(8.0);
            if ui.button("Очистить всё").clicked() {
                self.save(|s| {
                    s.clear_words();
                    Ok(())
                });
            }
        }
    }

    fn permissions(&mut self, ui: &mut egui::Ui) {
        let Some(backend) = self.host.backend.clone() else {
            ui.label(
                "Права видны только окну, открытому из KeySwitcher (пункт меню «Настройки…»).",
            );
            return;
        };
        if self.permissions.is_empty() {
            ui.label("Особых разрешений не нужно.");
        }
        for p in self.permissions.clone() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.strong(kind_title(p.kind));
                if p.granted {
                    ui.colored_label(GRANTED, "выдано");
                } else {
                    ui.colored_label(ui.visuals().error_fg_color, "не выдано");
                }
            });
            ui.label(&p.explanation);
            if !p.granted {
                ui.add(egui::Label::new(RichText::new(&p.fix_hint).monospace()).selectable(true));
            }
            if cfg!(target_os = "macos") && ui.button("Открыть системные настройки").clicked()
            {
                backend.open_permission_settings(p.kind);
            }
        }
        ui.add_space(14.0);
        ui.label(
            "Права применяются только к новому процессу: после выдачи перезапустите KeySwitcher.",
        );
        if let Some(rx) = &self.restarting {
            match rx.try_recv() {
                Ok(Ok(())) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                Ok(Err(e)) => {
                    self.restart_msg = Some(e);
                    self.restarting = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Перезапуск: ждём, пока KeySwitcher поднимется заново…");
                    });
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.restart_msg = Some("перезапуск прервался".to_owned());
                    self.restarting = None;
                }
            }
        } else if let Some(restart) = self.host.restart.clone()
            && ui.button("Перезапустить KeySwitcher").clicked()
        {
            self.restart_msg = None;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(restart());
            });
            self.restarting = Some(rx);
        }
        Self::error_line(ui, &self.restart_msg);
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("KeySwitcher {}", about::VERSION));
        ui.label(
            "Автопереключатель раскладки RU↔EN. Сетевых запросов нет, набранный текст не \
             сохраняется.",
        );
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Словари и данные")
            .default_open(true)
            .show(ui, |ui| {
                ui.add(egui::Label::new(about::THIRD_PARTY).selectable(true));
            });
        for (name, text) in about::NOTICES {
            egui::CollapsingHeader::new(format!("Заимствованный код и шрифты: {name}"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(*text).selectable(true));
                });
        }
    }
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(debug_assertions)]
        if let Some(shot) = &mut self.shot {
            shot.frame(&ctx);
        }
        self.poll(&ctx);
        ctx.request_repaint_after(POLL);
        self.record(&ctx);
        egui::Panel::left("sections")
            .resizable(false)
            .exact_size(190.0)
            .show(ui, |ui| {
                ui.add_space(10.0);
                for s in visible_sections() {
                    let mut title = RichText::new(s.title());
                    if s == Section::Permissions && self.permissions.iter().any(|p| !p.granted) {
                        title = title.color(ui.visuals().warn_fg_color);
                    }
                    let item = egui::Button::selectable(self.section == s, title)
                        .min_size(vec2(ui.available_width(), ROW_H));
                    if ui.add(item).clicked() {
                        self.go(s);
                    }
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.banner(ui);
            ui.heading(self.section.title());
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    match self.section {
                        Section::General => self.general(ui),
                        Section::Hotkeys => self.hotkeys(ui),
                        Section::Apps => self.apps(ui),
                        Section::Words => self.words(ui),
                        Section::Permissions => self.permissions(ui),
                        Section::About => self.about(ui),
                    }
                    Self::error_line(ui, &self.save_error);
                });
        });
    }
}
