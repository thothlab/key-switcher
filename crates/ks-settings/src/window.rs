//! Окно на eframe/egui: разделы слева, содержимое справа. Каждое действие — `Settings::edit`
//! (перечитать файл, одна правка, записать); агент сам применяет записанный файл. Раз в полсекунды
//! окно смотрит просьбу второго окна подняться, внешнюю правку файла (команды меню) и права.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Color32, RichText};
use ks_core::{AppId, Hotkey, config::MAX_PER_ACTION};
use ks_platform::{PermissionKind, PermissionStatus};

use crate::instance::{self, Guard};
use crate::model::{HotkeyAction, Settings};
use crate::record::{self, Recorded, Recorder};
use crate::{Host, Section, about};

const POLL: Duration = Duration::from_millis(500);

pub const EMPTY_APPS: &str = "Своих исключений нет — приложение добавляется пунктом меню \
                              „Не исправлять в …“ или здесь";
pub const EMPTY_WORDS: &str = "Слов-исключений нет — слово попадает сюда после отмены исправления \
                               и пункта меню „Не исправлять …“ или здесь";

pub(crate) fn show(section: Section, path: &Path, host: Host, guard: Guard) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("KeySwitcher — настройки")
            .with_inner_size([760.0, 540.0])
            .with_min_inner_size([560.0, 380.0]),
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

const GRANTED: Color32 = Color32::from_rgb(0x2e, 0x9e, 0x4f);

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
    save_error: Option<String>,
    restart_msg: Option<String>,
    /// Идёт перезапуск: ответ потока, который ждёт новый агент.
    restarting: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    permissions: Vec<PermissionStatus>,
    mtime: Option<SystemTime>,
    next_poll: Instant,
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
            save_error: None,
            restart_msg: None,
            restarting: None,
            permissions,
            mtime: mtime(path),
            next_poll: Instant::now() + POLL,
            _guard: guard,
        }
    }

    /// Просьба второго окна подняться, внешняя правка файла, права.
    fn poll(&mut self, ctx: &egui::Context) {
        if Instant::now() < self.next_poll {
            return;
        }
        self.next_poll = Instant::now() + POLL;
        if let Some(section) = instance::take_raise(&self.dir) {
            self.section = section;
            self.recording = None;
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
        }
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

    fn general(&mut self, ui: &mut egui::Ui) {
        let config = self.settings.config().clone();
        let mut autocorrect = config.autocorrect;
        if ui
            .checkbox(&mut autocorrect, "Автоисправление на лету")
            .changed()
        {
            self.save(|s| {
                s.set_autocorrect(autocorrect);
                Ok(())
            });
        }
        let mut single = config.single_shift_switch;
        if ui
            .checkbox(&mut single, "Одиночный Shift переключает раскладку")
            .changed()
        {
            self.save(|s| {
                s.set_single_shift_switch(single);
                Ok(())
            });
        }
        let mut autostart = config.autostart;
        if ui
            .checkbox(&mut autostart, "Запускать KeySwitcher при входе в систему")
            .changed()
        {
            self.save(|s| {
                s.set_autostart(autostart);
                Ok(())
            });
        }
        ui.add_space(8.0);
        ui.weak("Изменения применяются сразу, без перезапуска.");
    }

    fn hotkeys(&mut self, ui: &mut egui::Ui) {
        ui.weak(
            "До двух назначений на действие. Сочетания записываются нажатием, жесты Shift и Pause \
             выбираются из списка. Одиночный Shift — флажок в «Общих».",
        );
        for action in HotkeyAction::ALL {
            ui.add_space(10.0);
            ui.strong(action.label());
            let current = self.settings.hotkeys(action).to_vec();
            if current.is_empty() {
                ui.weak("без горячей клавиши");
            }
            for (slot, hotkey) in current.iter().enumerate() {
                let mut record = false;
                let mut remove = false;
                ui.horizontal(|ui| {
                    if self.is_recording(action, slot) {
                        ui.label("Нажмите сочетание… (Esc — отмена)");
                    } else {
                        ui.monospace(record::label(hotkey));
                    }
                    record = ui.button("Записать").clicked();
                    remove = ui.button("Убрать").clicked();
                });
                if record {
                    self.start_recording(action, slot);
                }
                if remove {
                    self.hotkey_msg = None;
                    self.save(|s| {
                        s.remove_hotkey(action, slot);
                        Ok(())
                    });
                }
            }
            // Третье назначение окно не предлагает (PRD 31).
            if current.len() < MAX_PER_ACTION {
                let slot = current.len();
                let mut record = false;
                let mut chosen = None;
                ui.horizontal(|ui| {
                    if self.is_recording(action, slot) {
                        ui.label("Нажмите сочетание… (Esc — отмена)");
                    } else {
                        record = ui.button("Записать новое").clicked();
                    }
                    egui::ComboBox::from_id_salt(("gesture", action.label()))
                        .selected_text("Жест…")
                        .show_ui(ui, |ui| {
                            for g in record::gestures() {
                                if ui.selectable_label(false, record::label(&g)).clicked() {
                                    chosen = Some(g);
                                }
                            }
                        });
                });
                if record {
                    self.start_recording(action, slot);
                }
                if let Some(g) = chosen {
                    self.set_hotkey(action, slot, g);
                }
            }
            if let Some((a, msg)) = &self.hotkey_msg
                && *a == action
            {
                ui.colored_label(ui.visuals().error_fg_color, msg);
            }
        }
    }

    fn apps(&mut self, ui: &mut egui::Ui) {
        ui.strong("Свои исключения");
        let own = self.settings.config().apps.disabled.clone();
        if own.is_empty() {
            ui.weak(EMPTY_APPS);
        }
        let mut removed: Option<AppId> = None;
        for app in &own {
            ui.horizontal(|ui| {
                ui.monospace(&app.0);
                if ui.button("Удалить").clicked() {
                    removed = Some(app.clone());
                }
            });
        }
        if let Some(app) = removed {
            self.save(move |s| {
                s.remove_app(&app);
                Ok(())
            });
        }
        let mut add = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.app_input)
                    .hint_text("идентификатор, например com.apple.TextEdit"),
            );
            add = ui.button("Добавить").clicked();
        });
        if add {
            let id = self.app_input.clone();
            match self.edit(|s| s.add_app(&id).map_err(|e| e.to_string())) {
                Ok(()) => {
                    self.app_input.clear();
                    self.app_msg = None;
                }
                Err(e) => self.app_msg = Some(e),
            }
        }
        Self::error_line(ui, &self.app_msg);

        ui.add_space(14.0);
        ui.strong("Встроенный список");
        ui.weak(
            "Здесь автоматика выключена по умолчанию: терминалы, IDE, менеджеры паролей. \
             Флажок «включить здесь» возвращает автоисправление.",
        );
        let mut toggled: Option<(AppId, bool)> = None;
        for app in self.settings.builtin() {
            let mut on = self.settings.builtin_enabled(app);
            if ui
                .checkbox(&mut on, format!("{} — включить здесь", app.0))
                .changed()
            {
                toggled = Some((app.clone(), on));
            }
        }
        if let Some((app, on)) = toggled {
            self.save(move |s| {
                s.set_builtin_enabled(&app, on);
                Ok(())
            });
        }
    }

    fn words(&mut self, ui: &mut egui::Ui) {
        let words = self.settings.config().words.never.clone();
        if words.is_empty() {
            ui.weak(EMPTY_WORDS);
        }
        let mut removed: Option<String> = None;
        for word in &words {
            ui.horizontal(|ui| {
                ui.monospace(word);
                if ui.button("Удалить").clicked() {
                    removed = Some(word.clone());
                }
            });
        }
        if let Some(word) = removed {
            self.save(move |s| {
                s.remove_word(&word);
                Ok(())
            });
        }
        let mut add = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.word_input).hint_text("слово"));
            add = ui.button("Добавить").clicked();
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
        self.poll(&ctx);
        ctx.request_repaint_after(POLL);
        self.record(&ctx);
        egui::Panel::left("sections").show(ui, |ui| {
            ui.add_space(8.0);
            for s in visible_sections() {
                if ui.selectable_label(self.section == s, s.title()).clicked() {
                    self.section = s;
                    self.recording = None;
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.banner(ui);
            ui.heading(self.section.title());
            ui.add_space(6.0);
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
