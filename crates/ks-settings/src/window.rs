//! Окно на eframe/egui: разделы слева, содержимое справа. Каждое действие — `Settings::edit`
//! (перечитать файл, одна правка, записать); агент сам применяет записанный файл. Раз в полсекунды
//! окно смотрит просьбу второго окна подняться, внешнюю правку файла (команды меню), права и
//! (в разделе «Приложения-исключения») запущенные приложения. Вид — по `design/`: тема и
//! шрифты в [`crate::theme`], виджеты в [`crate::widgets`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Align2, ColorImage, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, pos2, vec2,
};
use ks_core::{AppCategory, AppId, Hotkey, HotkeyError, config::MAX_PER_ACTION};
use ks_platform::{PermissionKind, PermissionStatus};

use crate::icons::{self, Icon};
use crate::instance::{self, Guard};
use crate::model::{BuiltinRow, Exclusion, HotkeyAction, Settings};
use crate::record::{self, Cap, Recorded, Recorder};
use crate::theme::{self, palette};
use crate::widgets::{self as w, BannerKind, FieldState, Tick};
use crate::{Host, Section, about};

const POLL: Duration = Duration::from_millis(500);

pub const EMPTY_APPS: &str = "Своих исключений нет — приложение добавляется здесь или пунктом \
                              меню «Не исправлять в …».";
pub const EMPTY_WORDS: &str = "Слов-исключений нет — слово попадает сюда после отмены исправления \
                               и пункта меню «Не исправлять …» или здесь.";

const APP_ICON_64: &[u8] = include_bytes!("../assets/app-icon-64.png");
const APP_ICON_128: &[u8] = include_bytes!("../assets/app-icon-128.png");
#[cfg(not(target_os = "macos"))]
const WINDOW_ICON: &[u8] = include_bytes!("../assets/window-icon-128.png");

/// Ширина левой панели одинакова для 820 и 640.
const SIDEBAR_W: f32 = 188.0;
/// Уже этого область содержимого — строка горячей клавиши в два яруса, отступы меньше.
const NARROW_CONTENT: f32 = 520.0;
/// Поле сочетания и кнопки строки горячей клавиши.
const FIELD_W: f32 = 176.0;
const FIELD_H: f32 = 32.0;
const BUTTON: f32 = 28.0;
/// Поле «Добавить запущенное приложение…» и ввод идентификатора.
const PICKER_W: f32 = 290.0;

pub(crate) fn show(section: Section, path: &Path, host: Host, guard: Guard) -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    let size = crate::shot::Shot::size().unwrap_or([820.0, 580.0]);
    #[cfg(not(debug_assertions))]
    let size = [820.0, 580.0];
    #[allow(unused_mut)]
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("KeySwitcher — настройки")
        .with_inner_size(size)
        .with_min_inner_size([640.0, 420.0]);
    // macOS берёт значок из KeySwitcher.app
    #[cfg(not(target_os = "macos"))]
    if let Ok(icon) = eframe::icon_data::from_png_bytes(WINDOW_ICON) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let window = Window::new(section, path, host, guard);
    eframe::run_native(
        "KeySwitcher",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            #[cfg(debug_assertions)]
            if let Some(theme) = crate::shot::Shot::theme() {
                cc.egui_ctx.set_theme(theme);
            }
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

fn nav_label(section: Section) -> &'static str {
    match section {
        Section::Apps => "Приложения",
        Section::Words => "Слова",
        other => other.title(),
    }
}

fn nav_icon(section: Section) -> Icon {
    match section {
        Section::General => Icon::SectionGeneral,
        Section::Hotkeys => Icon::SectionHotkeys,
        Section::Apps => Icon::SectionApps,
        Section::Words => Icon::SectionWords,
        Section::Permissions => Icon::SectionPermissions,
        Section::About => Icon::SectionAbout,
    }
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

fn texture(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(png).ok()?;
    let image =
        ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

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

/// Отказ назначить сочетание: в какую ячейку, что пытались поставить, текст.
struct HotkeyMsg {
    action: HotkeyAction,
    slot: usize,
    attempted: Option<Hotkey>,
    text: String,
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
    hotkey_msg: Option<HotkeyMsg>,
    recording: Option<(HotkeyAction, usize, Recorder)>,
    /// Назначения действий, у которых сняли флажок: флажок обратно — они возвращаются.
    parked: HashMap<HotkeyAction, Vec<Hotkey>>,
    /// Запущенные приложения (идентификатор, имя) для выбора своего исключения.
    running: Vec<(AppId, String)>,
    /// Имена установленных приложений по идентификатору: спрашиваются у системы один раз.
    names: HashMap<AppId, Option<String>>,
    /// Поле «идентификатор вручную» раскрыто.
    manual_app: bool,
    /// Свёрнутые категории встроенного списка.
    collapsed: HashSet<AppCategory>,
    save_error: Option<String>,
    restart_msg: Option<String>,
    /// Идёт перезапуск: ответ потока, который ждёт новый агент.
    restarting: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    permissions: Vec<PermissionStatus>,
    mtime: Option<SystemTime>,
    next_poll: Instant,
    /// Значок приложения: 64 — в панели, 128 — в «О программе».
    icons: Option<(Option<TextureHandle>, Option<TextureHandle>)>,
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
            collapsed: HashSet::new(),
            save_error: None,
            restart_msg: None,
            restarting: None,
            permissions,
            mtime: mtime(path),
            next_poll: Instant::now() + POLL,
            icons: None,
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

    fn needs_permissions(&self) -> bool {
        self.permissions.iter().any(|p| !p.granted)
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

    /// Текст отказа: у повтора — какому действию сочетание уже назначено.
    fn refusal(&self, action: HotkeyAction, hotkey: &Hotkey, error: &HotkeyError) -> String {
        let shown = record::label(hotkey);
        match error {
            HotkeyError::Duplicate(_) => {
                let holder = HotkeyAction::ALL
                    .into_iter()
                    .find(|a| *a != action && self.settings.hotkeys(*a).contains(hotkey));
                match holder {
                    Some(a) => format!(
                        "{shown} уже назначено действию «{}». Выберите другое сочетание.",
                        a.label()
                    ),
                    None => format!(
                        "{shown} уже назначено другому действию. Выберите другое сочетание."
                    ),
                }
            }
            e => format!("{shown}: {e}"),
        }
    }

    fn set_hotkey(&mut self, action: HotkeyAction, slot: usize, hotkey: Hotkey) {
        let mut refused = None;
        let result = self.edit(|s| {
            s.set_hotkey(action, slot, hotkey).map_err(|e| {
                let text = e.to_string();
                refused = Some(e);
                text
            })
        });
        self.hotkey_msg = match (result, refused) {
            (Ok(()), _) => {
                self.parked.remove(&action);
                None
            }
            (Err(_), Some(e)) => Some(HotkeyMsg {
                action,
                slot,
                attempted: Some(hotkey),
                text: self.refusal(action, &hotkey, &e),
            }),
            (Err(text), None) => Some(HotkeyMsg {
                action,
                slot,
                attempted: None,
                text,
            }),
        };
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
                self.hotkey_msg = Some(HotkeyMsg {
                    action,
                    slot,
                    attempted: None,
                    text: text.to_owned(),
                });
            }
            Recorded::Done(hotkey) => {
                self.recording = None;
                self.set_hotkey(action, slot, hotkey);
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let ctx = ui.ctx().clone();
        let icons = self.icons.get_or_insert_with(|| {
            (
                texture(&ctx, "app-icon-64", APP_ICON_64),
                texture(&ctx, "app-icon-128", APP_ICON_128),
            )
        });
        let logo = icons.0.clone();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            if let Some(logo) = &logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(24.0, 24.0)));
            }
            ui.label(
                RichText::new("KeySwitcher")
                    .font(w::medium(15.0))
                    .color(p.text),
            );
        });
        ui.add_space(16.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let attention = self.needs_permissions();
        for s in visible_sections() {
            let item = w::nav_item(
                ui,
                nav_icon(s),
                nav_label(s),
                self.section == s,
                s == Section::Permissions && attention,
            );
            if item.clicked() {
                self.go(s);
            }
        }
    }

    /// Баннер испорченного файла настроек — над заголовком любого раздела.
    fn config_banner(&mut self, ui: &mut egui::Ui) {
        let Some(error) = self.settings.error().map(str::to_owned) else {
            return;
        };
        let text = format!(
            "Файл настроек не прочитан: {error}. Работают значения по умолчанию, файл не \
             перезаписан."
        );
        if w::banner(ui, BannerKind::Error, &text, "Записать умолчания") {
            self.save_error = self.settings.reset_to_defaults(&self.path).err();
            self.mtime = mtime(&self.path);
        }
        ui.add_space(20.0);
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        if self.needs_permissions() {
            if w::banner(
                ui,
                BannerKind::Warn,
                "KeySwitcher не видит клавиатуру: не выданы разрешения.",
                "Открыть «Разрешения»",
            ) {
                self.go(Section::Permissions);
            }
            ui.add_space(16.0);
        }
        let config = self.settings.config().clone();
        let undo = if cfg!(target_os = "macos") {
            "Cmd+Z"
        } else {
            "Ctrl+Z"
        };
        let mut autocorrect = None;
        let mut autostart = None;
        let mut updates = None;
        w::card(ui, |ui| {
            w::row(ui, true, Margin::symmetric(14, 12), |ui| {
                autocorrect = w::option(
                    ui,
                    config.autocorrect,
                    "Исправлять раскладку на лету",
                    &format!(
                        "Слово, набранное не в той раскладке, исправляется после пробела: \
                         «ghbdtn» → «привет». Отменить исправление — сразу {undo} или горячей \
                         клавишей."
                    ),
                );
            });
            w::row(ui, false, Margin::symmetric(14, 12), |ui| {
                autostart = w::option(
                    ui,
                    config.autostart,
                    "Запускать KeySwitcher при входе в систему",
                    "Значок в строке меню появится сам после входа.",
                );
            });
            w::row(ui, false, Margin::symmetric(14, 12), |ui| {
                updates = w::option(
                    ui,
                    config.check_updates,
                    "Проверять обновления",
                    "Раз в сутки KeySwitcher спрашивает GitHub, не вышла ли новая версия, и \
                     предлагает её в меню значка. Ставится только по вашему нажатию.",
                );
            });
        });
        if let Some(on) = autocorrect {
            self.save(|s| {
                s.set_autocorrect(on);
                Ok(())
            });
        }
        if let Some(on) = updates {
            self.save(|s| {
                s.set_check_updates(on);
                Ok(())
            });
        }
        if let Some(on) = autostart {
            self.save(|s| {
                s.set_autostart(on);
                Ok(())
            });
        }
        ui.add_space(16.0);
        let p = palette(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), rect, Icon::Check, p.text_3);
            ui.label(
                RichText::new("Изменения применяются сразу, без перезапуска.")
                    .text_style(theme::hint())
                    .color(p.text_3),
            );
        });
    }

    fn hotkeys(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let hint = if cfg!(target_os = "macos") {
            "Щёлкните поле и нажмите сочетание, Esc — отмена. Сочетание из одних модификаторов \
             срабатывает, когда их отпускают. Сочетания, которые забирает система (Cmd + \
             Пробел), жесты Shift и Pause — в меню «…»."
        } else {
            "Щёлкните поле и нажмите сочетание, Esc — отмена. Сочетание из одних модификаторов \
             срабатывает, когда их отпускают. Сочетания, которые забирает система, жесты Shift \
             и Pause — в меню «…»."
        };
        w::hint(ui, hint);
        ui.add_space(16.0);
        let mut events = Vec::new();
        w::card(ui, |ui| {
            for (i, action) in HotkeyAction::ALL.into_iter().enumerate() {
                w::row(ui, i == 0, w::row_margin(), |ui| {
                    if let Some(event) = self.hotkey_row(ui, action, narrow) {
                        events.push((action, event));
                    }
                });
            }
        });
        for (action, event) in events {
            self.hotkey_event(action, event);
        }
        ui.add_space(16.0);
        let single = self.settings.config().single_shift_switch;
        let mut toggled = None;
        w::card(ui, |ui| {
            w::row(ui, true, Margin::symmetric(14, 12), |ui| {
                toggled = w::option(
                    ui,
                    single,
                    "Сменить раскладку одиночным Shift",
                    "Короткое нажатие Shift без других клавиш переключает раскладку.",
                );
            });
        });
        if let Some(on) = toggled {
            self.save(|s| {
                s.set_single_shift_switch(on);
                Ok(())
            });
        }
    }

    /// Строка действия как в Punto Switcher: подпись, флажок, поле сочетания, меню «…», «+».
    /// Второе назначение — под первым полем. Узкое окно — подпись сверху, управление под ней.
    fn hotkey_row(
        &self,
        ui: &mut egui::Ui,
        action: HotkeyAction,
        narrow: bool,
    ) -> Option<HotkeyEvent> {
        let p = palette(ui);
        let current = self.settings.hotkeys(action).to_vec();
        let adding = !current.is_empty() && self.is_recording(action, current.len());
        let slots = current.len().max(1) + usize::from(adding);
        let msg = self.hotkey_msg.as_ref().filter(|m| m.action == action);
        let row_w = ui.available_width();
        let gap = 12.0;
        let controls_w = 18.0 + gap + FIELD_W + 8.0 + BUTTON + 8.0 + BUTTON;
        let label_w = if narrow {
            row_w
        } else {
            (row_w - controls_w - gap).max(120.0)
        };
        let mut job = egui::text::LayoutJob::simple(
            action.label().to_owned(),
            w::font(14.0),
            p.text,
            label_w,
        );
        job.sections[0].format.line_height = Some(w::BODY_LINE);
        let label = ui.painter().layout_job(job);
        let controls_h = slots as f32 * FIELD_H + (slots - 1) as f32 * 8.0;
        let (row_h, label_y, controls_y) = if narrow {
            let top = label.size().y + 8.0;
            (top + controls_h, 0.0, top)
        } else {
            let h = label.size().y.max(controls_h);
            (h, (h - label.size().y) / 2.0, (h - controls_h) / 2.0)
        };
        let (rect, _) = ui.allocate_exact_size(vec2(row_w, row_h), Sense::hover());
        ui.painter()
            .galley(pos2(rect.left(), rect.top() + label_y), label, p.text);
        let (field_x, field_w) = if narrow {
            let x = rect.left() + 18.0 + gap;
            (x, rect.right() - x - 8.0 - BUTTON - 8.0 - BUTTON)
        } else {
            (rect.right() - controls_w + 18.0 + gap, FIELD_W)
        };
        let id = Id::new(("hotkey", action as u8));
        let mut event = None;
        for slot in 0..slots {
            let y = rect.top() + controls_y + slot as f32 * (FIELD_H + 8.0);
            let field = Rect::from_min_size(pos2(field_x, y), vec2(field_w, FIELD_H));
            if slot == 0 {
                let on = !current.is_empty() || self.is_recording(action, 0);
                let check = Rect::from_min_size(
                    pos2(field_x - gap - 18.0, field.center().y - 9.0),
                    vec2(18.0, 18.0),
                );
                if w::checkbox_at(ui, check, id.with("check"), Tick::of(on)).clicked() {
                    event = Some(HotkeyEvent::Toggle(!on));
                }
            }
            let attempted = msg
                .filter(|m| m.slot == slot)
                .and_then(|m| m.attempted)
                .map(|h| record::caps(&h));
            let assigned: Option<Vec<Cap>> = current.get(slot).map(record::caps);
            let state = if self.is_recording(action, slot) {
                FieldState::Recording
            } else if let Some(caps) = attempted.as_deref().or(assigned.as_deref()) {
                FieldState::Caps(caps)
            } else {
                FieldState::Empty
            };
            let error = msg.is_some_and(|m| m.slot == slot);
            let resp = w::hotkey_field_at(ui, field, id.with(("field", slot)), &state, error);
            let resp = match current.get(slot) {
                Some(h) => resp.on_hover_text(record::label(h)),
                None => resp.on_hover_text("Щёлкните и нажмите сочетание"),
            };
            if resp.clicked() {
                event = Some(HotkeyEvent::Record(slot));
            }
            let more_rect = Rect::from_min_size(
                pos2(field.right() + 8.0, field.center().y - BUTTON / 2.0),
                vec2(BUTTON, BUTTON),
            );
            let more_id = id.with(("more", slot));
            let open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
            let more = w::icon_button_at(ui, more_rect, more_id, Icon::More, open)
                .on_hover_text("Готовые сочетания и жесты");
            let chosen = egui::Popup::menu(&more)
                .width(236.0)
                .frame(menu_frame(ui))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                .show(|ui| self.hotkey_menu(ui, current.get(slot)))
                .and_then(|r| r.inner);
            if let Some(choice) = chosen {
                event = Some(match choice {
                    Some(h) => HotkeyEvent::Set(slot, h),
                    None => HotkeyEvent::Remove(slot),
                });
            }
            // Третье назначение окно не предлагает (PRD 31).
            if slot == 0 && current.len() == 1 && !adding && MAX_PER_ACTION > 1 {
                let plus = Rect::from_min_size(
                    pos2(more_rect.right() + 8.0, more_rect.top()),
                    vec2(BUTTON, BUTTON),
                );
                if w::icon_button_at(ui, plus, id.with("add"), Icon::Add, false)
                    .on_hover_text("Добавить второе сочетание")
                    .clicked()
                {
                    event = Some(HotkeyEvent::Record(1));
                }
            }
        }
        if let Some(m) = msg {
            ui.add_space(6.0);
            w::error_line(ui, &m.text);
        }
        event
    }

    /// Меню «…»: готовые сочетания, жесты, «Убрать». `Some(Some(h))` — выбрано сочетание,
    /// `Some(None)` — «Убрать».
    fn hotkey_menu(&self, ui: &mut egui::Ui, assigned: Option<&Hotkey>) -> Option<Option<Hotkey>> {
        let p = palette(ui);
        let mut chosen = None;
        w::menu_title(ui, "Готовые сочетания");
        for preset in record::presets().into_iter().chain(record::gestures()) {
            let note = match preset {
                Hotkey::DoubleShift => Some("Двойной Shift"),
                Hotkey::BothShifts => Some("Оба Shift"),
                _ => None,
            };
            let caps = record::caps(&preset);
            let checked = assigned == Some(&preset);
            let item = w::menu_item(ui, |ui, rect| {
                w::paint_caps(ui, ui.painter(), rect.left(), rect.center().y, &caps);
                let mut right = rect.right();
                if checked {
                    let tick = Rect::from_min_size(
                        pos2(right - 16.0, rect.center().y - 8.0),
                        vec2(16.0, 16.0),
                    );
                    icons::paint(ui.painter(), tick, Icon::Check, p.accent);
                    right -= 24.0;
                }
                if let Some(note) = note {
                    ui.painter().text(
                        pos2(right, rect.center().y),
                        Align2::RIGHT_CENTER,
                        note,
                        w::font(13.0),
                        p.text_2,
                    );
                }
            });
            let item = item.on_hover_text(record::label(&preset));
            if item.clicked() {
                chosen = Some(Some(preset));
            }
        }
        if assigned.is_some() {
            w::menu_separator(ui);
            let remove = w::menu_item(ui, |ui, rect| {
                let icon =
                    Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(16.0, 16.0));
                icons::paint(ui.painter(), icon, Icon::Delete, p.error);
                ui.painter().text(
                    pos2(icon.right() + 8.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "Убрать",
                    w::font(13.0),
                    p.error,
                );
            });
            if remove.clicked() {
                chosen = Some(None);
            }
        }
        chosen
    }

    fn hotkey_event(&mut self, action: HotkeyAction, event: HotkeyEvent) {
        match event {
            HotkeyEvent::Toggle(false) => {
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
            HotkeyEvent::Toggle(true) => match self.parked.remove(&action) {
                Some(parked) => {
                    let result =
                        self.edit(|s| s.restore_hotkeys(action, parked).map_err(|e| e.to_string()));
                    if let Err(text) = result {
                        self.hotkey_msg = Some(HotkeyMsg {
                            action,
                            slot: 0,
                            attempted: None,
                            text,
                        });
                        self.recording = Some((action, 0, Recorder::new()));
                    }
                }
                None => self.start_recording(action, 0),
            },
            HotkeyEvent::Record(slot) => self.start_recording(action, slot),
            HotkeyEvent::Set(slot, hotkey) => {
                self.recording = None;
                self.set_hotkey(action, slot, hotkey);
            }
            HotkeyEvent::Remove(slot) => {
                self.hotkey_msg = None;
                self.save(|s| {
                    s.remove_hotkey(action, slot);
                    Ok(())
                });
            }
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

    /// Строка своего исключения: значок-буква, название, идентификатор мелко, «Удалить».
    fn list_row(ui: &mut egui::Ui, first: bool, title: &str, id: Option<&str>) -> bool {
        let p = palette(ui);
        let mut remove = false;
        w::row(
            ui,
            first,
            Margin {
                left: 14,
                right: 8,
                top: 8,
                bottom: 8,
            },
            |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(28.0);
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if let Some(id) = id {
                        w::app_badge(ui, title, id);
                    }
                    let subtitle = id.filter(|id| *id != title);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if subtitle.is_none() {
                            // одна строка — по центру строки высотой 28, как «Удалить»
                            ui.add_space(5.0);
                        }
                        ui.label(RichText::new(title).color(p.text));
                        if let Some(id) = subtitle {
                            ui.label(RichText::new(id).font(w::font(12.0)).color(p.text_3));
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        remove = w::quiet_button(ui, Icon::Delete, "Удалить", p.text_2).clicked();
                    });
                });
            },
        );
        remove
    }

    fn apps(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let p = palette(ui);
        w::body(
            ui,
            "В этих приложениях KeySwitcher не исправляет слова на лету. Горячие клавиши \
             работают везде.",
        );
        ui.add_space(16.0);
        w::subheading(ui, "Свои исключения");
        ui.add_space(4.0);
        let own = self.settings.config().apps.disabled.clone();
        let mut removed: Option<AppId> = None;
        if own.is_empty() {
            w::empty_state(ui, EMPTY_APPS);
        } else {
            let rows: Vec<(AppId, String)> = own
                .iter()
                .map(|app| (app.clone(), self.app_title(app)))
                .collect();
            w::card(ui, |ui| {
                for (i, (app, title)) in rows.iter().enumerate() {
                    if Self::list_row(ui, i == 0, title, Some(&app.0)) {
                        removed = Some(app.clone());
                    }
                }
            });
        }
        if let Some(app) = removed {
            self.save(move |s| {
                s.remove_app(&app);
                Ok(())
            });
        }
        ui.add_space(12.0);

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
        if !self.running.is_empty() {
            // узкое окно: выбор во всю ширину, ссылка строкой ниже
            let picker_w = if narrow {
                ui.available_width()
            } else {
                PICKER_W
            };
            let layout = if narrow {
                Layout::top_down(Align::Min)
            } else {
                Layout::left_to_right(Align::Center)
            };
            ui.with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing = vec2(16.0, 10.0);
                let picker = self.app_picker(ui, picker_w);
                egui::Popup::menu(&picker)
                    .width(picker_w)
                    .frame(menu_frame(ui))
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                    .show(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                if candidates.is_empty() {
                                    w::hint(ui, "Все запущенные приложения уже в исключениях");
                                }
                                for (id, name) in &candidates {
                                    let item = w::menu_item(ui, |ui, rect| {
                                        let badge = Rect::from_min_size(
                                            pos2(rect.left(), rect.center().y - 12.0),
                                            vec2(24.0, 24.0),
                                        );
                                        w::paint_badge(ui, badge, name, &id.0);
                                        ui.painter().text(
                                            pos2(badge.right() + 12.0, rect.center().y),
                                            Align2::LEFT_CENTER,
                                            name,
                                            w::font(14.0),
                                            p.text,
                                        );
                                    });
                                    if item.on_hover_text(&id.0).clicked() {
                                        chosen = Some(id.clone());
                                    }
                                }
                            });
                    });
                let toggle = if self.manual_app {
                    "Скрыть ввод вручную"
                } else {
                    "Ввести идентификатор…"
                };
                if w::link(ui, toggle).clicked() {
                    self.manual_app = !self.manual_app;
                }
            });
        }
        if let Some(id) = chosen {
            self.add_app(id.0);
        }
        if self.manual_app {
            let placeholder = if cfg!(target_os = "macos") {
                "bundle id, например com.apple.TextEdit"
            } else if cfg!(windows) {
                "имя exe, например notepad.exe"
            } else {
                "app_id или WM_CLASS, например org.gnome.TextEditor"
            };
            ui.add_space(8.0);
            let mut add = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let id = Id::new("app-id-input");
                let width = if narrow {
                    ui.available_width() - 16.0 - 96.0
                } else {
                    PICKER_W
                };
                let field = w::text_field(
                    ui,
                    id,
                    &mut self.app_input,
                    placeholder,
                    width,
                    self.app_msg.is_some(),
                );
                add = w::primary_button(ui, "Добавить").clicked()
                    || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            });
            if add {
                self.add_app(self.app_input.clone());
            }
        }
        if let Some(msg) = self.app_msg.clone() {
            ui.add_space(6.0);
            w::error_line(ui, &msg);
        }

        ui.add_space(20.0);
        w::subheading(ui, "Встроенный список");
        ui.add_space(2.0);
        w::hint(
            ui,
            "Отмечено — автоисправление здесь выключено, так по умолчанию. Снимите флажок, \
             чтобы KeySwitcher работал в приложении.",
        );
        ui.add_space(8.0);
        let rows = self.settings.builtin_rows();
        let mut categories: Vec<AppCategory> = rows.iter().map(|r| r.category).collect();
        categories.dedup();
        let mut toggled: Option<(BuiltinRow, bool)> = None;
        let mut fold: Option<AppCategory> = None;
        w::card(ui, |ui| {
            for (i, category) in categories.iter().enumerate() {
                let open = !self.collapsed.contains(category);
                let first = i == 0;
                w::row(
                    ui,
                    first,
                    Margin {
                        left: 10,
                        right: 14,
                        top: 10,
                        bottom: 10,
                    },
                    |ui| {
                        let header = ui
                            .horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                w::chevron(ui, rect, open, p.text_2);
                                ui.label(
                                    RichText::new(category_title(*category))
                                        .font(w::medium(14.0))
                                        .color(p.text),
                                );
                            })
                            .response;
                        let click = ui.interact(
                            header.rect,
                            Id::new(("category", category_title(*category))),
                            Sense::click(),
                        );
                        if click.clicked() {
                            fold = Some(*category);
                        }
                        if !open {
                            return;
                        }
                        ui.add_space(4.0);
                        for row in rows.iter().filter(|r| r.category == *category) {
                            let state = self.settings.exclusion(row);
                            let tick = match state {
                                Exclusion::All => Tick::On,
                                Exclusion::Partly => Tick::Partial,
                                _ => Tick::Off,
                            };
                            let ids = row
                                .ids
                                .iter()
                                .map(|id| id.0.as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.horizontal_top(|ui| {
                                ui.add_space(22.0);
                                ui.spacing_mut().item_spacing.x = 10.0;
                                ui.vertical(|ui| {
                                    ui.add_space(4.0);
                                    if w::checkbox(ui, tick).on_hover_text(&ids).clicked() {
                                        toggled = Some((row.clone(), state != Exclusion::All));
                                    }
                                });
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.add_space(4.0);
                                    let title = ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(row.title).color(p.text),
                                            )
                                            .sense(Sense::click()),
                                        )
                                        .on_hover_text(&ids);
                                    if title.clicked() {
                                        toggled = Some((row.clone(), state != Exclusion::All));
                                    }
                                    if !row.members.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(row.members.join(", "))
                                                    .font(w::font(12.0))
                                                    .color(p.text_3),
                                            )
                                            .wrap_mode(egui::TextWrapMode::Wrap),
                                        );
                                    }
                                });
                            });
                        }
                    },
                );
            }
        });
        if let Some(category) = fold
            && !self.collapsed.remove(&category)
        {
            self.collapsed.insert(category);
        }
        if let Some((row, excluded)) = toggled {
            self.save(move |s| {
                s.set_excluded(&row, excluded);
                Ok(())
            });
        }
    }

    /// Поле-кнопка «Добавить запущенное приложение…» с шевроном.
    fn app_picker(&self, ui: &mut egui::Ui, width: f32) -> egui::Response {
        let p = palette(ui);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), resp.id.with("popup"));
        let edge = if open {
            Stroke::new(1.5, p.accent)
        } else if resp.hovered() {
            Stroke::new(1.0, p.text_3)
        } else {
            Stroke::new(1.0, p.border_strong)
        };
        ui.painter().rect(
            rect,
            theme::radius::WIDGET,
            p.bg_field,
            edge,
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(rect.left() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            "Добавить запущенное приложение…",
            w::font(14.0),
            p.text,
        );
        let chevron = Rect::from_min_size(
            pos2(rect.right() - 26.0, rect.center().y - 8.0),
            vec2(16.0, 16.0),
        );
        icons::paint(ui.painter(), chevron, Icon::ChevronDown, p.text_2);
        resp
    }

    fn words(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        w::body(
            ui,
            "Эти слова KeySwitcher не исправляет никогда. Слово из одной-двух букв добавить \
             нельзя: короткие слова решаются по соседним.",
        );
        ui.add_space(16.0);
        let words = self.settings.config().words.never.clone();
        let mut removed: Option<String> = None;
        if words.is_empty() {
            w::empty_state(ui, EMPTY_WORDS);
        } else {
            w::card(ui, |ui| {
                for (i, word) in words.iter().enumerate() {
                    if Self::list_row(ui, i == 0, word, None) {
                        removed = Some(word.clone());
                    }
                }
            });
        }
        if let Some(word) = removed {
            self.save(move |s| {
                s.remove_word(&word);
                Ok(())
            });
        }
        ui.add_space(16.0);
        let mut add = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let field = w::text_field(
                ui,
                Id::new("word-input"),
                &mut self.word_input,
                "слово",
                260.0,
                self.word_msg.is_some(),
            );
            add = w::primary_button(ui, "Добавить").clicked()
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
        if let Some(msg) = self.word_msg.clone() {
            ui.add_space(6.0);
            w::error_line(ui, &msg);
        }
        if !words.is_empty() {
            ui.add_space(16.0);
            let mut clear = false;
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                clear = w::quiet_button(ui, Icon::Delete, "Очистить всё", p.text_2).clicked();
            });
            if clear {
                self.save(|s| {
                    s.clear_words();
                    Ok(())
                });
            }
        }
    }

    fn permissions(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let Some(backend) = self.host.backend.clone() else {
            w::body(
                ui,
                "Права видны только окну, открытому из KeySwitcher (пункт меню «Настройки…»).",
            );
            return;
        };
        if self.permissions.is_empty() {
            w::body(ui, "Особых разрешений не нужно.");
        }
        for perm in self.permissions.clone() {
            w::card(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            let (icon, color) = if perm.granted {
                                (Icon::StatusGranted, p.success)
                            } else {
                                (Icon::StatusDenied, p.error)
                            };
                            icons::paint(ui.painter(), rect, icon, color);
                            w::subheading(ui, kind_title(perm.kind));
                            if perm.granted {
                                w::pill(ui, "выдано", p.success, p.success_bg);
                            } else {
                                w::pill(ui, "не выдано", p.error, p.error_bg);
                            }
                        });
                        w::hint(ui, &perm.explanation);
                        if !perm.granted {
                            w::mono_box(ui, &perm.fix_hint);
                        }
                        if cfg!(target_os = "macos")
                            && w::button(ui, "Открыть системные настройки").clicked()
                        {
                            backend.open_permission_settings(perm.kind);
                        }
                    });
            });
            ui.add_space(16.0);
        }
        ui.add_space(8.0);
        w::hint(
            ui,
            if cfg!(target_os = "macos") {
                "Права применяются только к новому процессу: после выдачи KeySwitcher \
                 перезапустится сам. Если этого не случилось, перезапустите его кнопкой ниже."
            } else {
                "Права применяются только к новому процессу: после выдачи перезапустите \
                 KeySwitcher."
            },
        );
        ui.add_space(8.0);
        if let Some(rx) = &self.restarting {
            match rx.try_recv() {
                Ok(Ok(())) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                Ok(Err(e)) => {
                    self.restart_msg = Some(e);
                    self.restarting = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.add(egui::Spinner::new().size(16.0).color(p.accent));
                        ui.label(
                            RichText::new("Перезапуск: ждём, пока KeySwitcher поднимется заново…")
                                .color(p.text),
                        );
                    });
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.restart_msg = Some("перезапуск прервался".to_owned());
                    self.restarting = None;
                }
            }
        } else if let Some(restart) = self.host.restart.clone()
            && w::button(ui, "Перезапустить KeySwitcher").clicked()
        {
            self.restart_msg = None;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(restart());
            });
            self.restarting = Some(rx);
        }
        if let Some(msg) = self.restart_msg.clone() {
            ui.add_space(6.0);
            w::error_line(ui, &msg);
        }
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let icon = self.icons.as_ref().and_then(|i| i.1.clone());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            if let Some(icon) = &icon {
                ui.add(egui::Image::new(icon).fit_to_exact_size(vec2(64.0, 64.0)));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(8.0);
                ui.label(
                    RichText::new("KeySwitcher")
                        .font(w::medium(22.0))
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format!("Версия {}", about::VERSION))
                        .font(w::font(14.0))
                        .color(p.text_2),
                );
            });
        });
        ui.add_space(16.0);
        w::body(
            ui,
            "Автопереключатель русской и английской раскладки. Единственный сетевой запрос — \
             проверка новой версии (можно выключить в «Общих»), набранный текст не сохраняется.",
        );
        ui.add_space(16.0);
        w::collapsing_card(
            ui,
            Id::new("about-data"),
            "Словари и данные",
            true,
            |ui| {
                license(ui, about::THIRD_PARTY);
            },
        );
        for (name, text) in about::NOTICES {
            ui.add_space(12.0);
            w::collapsing_card(
                ui,
                Id::new(("about-notice", *name)),
                &format!("Заимствованный код и шрифты: {name}"),
                false,
                |ui| license(ui, text),
            );
        }
    }
}

/// Рамка всплывающего меню в теме `ui`.
fn menu_frame(ui: &egui::Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.bg_surface)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(theme::radius::MENU)
        .shadow(ui.visuals().popup_shadow)
        .inner_margin(6)
}

/// Текст лицензии из Markdown: заголовки — 14 Medium, абзацы и пункты — 13 вторичным цветом,
/// блоки кода — моноширинным 12. Первый заголовок уже стоит в шапке карточки.
fn license(ui: &mut egui::Ui, text: &str) {
    let p = palette(ui);
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut code = false;
    let mut block = String::new();
    let flush = |ui: &mut egui::Ui, block: &mut String, code: bool| {
        if block.trim().is_empty() {
            block.clear();
            return;
        }
        let rich = if code {
            RichText::new(block.trim_end())
                .font(egui::FontId::monospace(12.0))
                .color(p.text_2)
        } else {
            RichText::new(plain(block.trim()))
                .text_style(theme::hint())
                .color(p.text_2)
                .line_height(Some(w::HINT_LINE))
        };
        ui.add(
            egui::Label::new(rich)
                .selectable(true)
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
        block.clear();
    };
    for (i, line) in text.lines().enumerate() {
        if line.starts_with("```") {
            flush(ui, &mut block, code);
            code = !code;
            continue;
        }
        if code {
            block.push_str(line);
            block.push('\n');
            continue;
        }
        if let Some(title) = line.strip_prefix('#') {
            flush(ui, &mut block, false);
            if i > 0 {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(plain(title.trim_start_matches('#').trim()))
                        .font(w::medium(14.0))
                        .color(p.text),
                );
            }
            continue;
        }
        if line.trim().is_empty() || line.starts_with("- ") || line.starts_with("* ") {
            flush(ui, &mut block, false);
        }
        if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            block.push_str("• ");
            block.push_str(item);
        } else {
            if !block.is_empty() {
                block.push(' ');
            }
            block.push_str(line.trim());
        }
    }
    flush(ui, &mut block, code);
}

/// Markdown без разметки: `**жирный**` и `` `код` `` — как текст, `[ссылка](url)` — подпись.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find("](").and_then(|mid| {
            after[mid + 2..]
                .find(')')
                .map(|end| (mid, mid + 2 + end + 1))
        }) {
            Some((mid, end)) => {
                out.push_str(&after[..mid]);
                rest = &after[end..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
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
        let p = palette(ui);
        egui::Panel::left("sections")
            .resizable(false)
            .exact_size(SIDEBAR_W)
            .frame(
                Frame::new()
                    .fill(p.bg_sidebar)
                    .inner_margin(Margin::symmetric(8, 14)),
            )
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg_window))
            .show(ui, |ui| {
                let narrow = ui.available_width() - 56.0 < NARROW_CONTENT;
                let pad = if narrow {
                    Margin {
                        left: 22,
                        right: 22,
                        top: 20,
                        bottom: 28,
                    }
                } else {
                    Margin {
                        left: 28,
                        right: 28,
                        top: 24,
                        bottom: 32,
                    }
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Frame::new().inner_margin(pad).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.config_banner(ui);
                            w::heading(ui, self.section.title());
                            ui.add_space(16.0);
                            match self.section {
                                Section::General => self.general(ui),
                                Section::Hotkeys => self.hotkeys(ui, narrow),
                                Section::Apps => self.apps(ui, narrow),
                                Section::Words => self.words(ui),
                                Section::Permissions => self.permissions(ui),
                                Section::About => self.about(ui),
                            }
                            if let Some(e) = self.save_error.clone() {
                                ui.add_space(12.0);
                                w::error_line(ui, &e);
                            }
                        });
                    });
            });
    }
}
