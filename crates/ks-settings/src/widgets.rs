//! Виджеты окна по `design/tokens.json → custom_widgets`. То, что ролями egui не выражается
//! (карточки, флажок с заливкой и частичным состоянием, колпачки клавиш, поле сочетания, кнопки
//! из значка, баннеры, плашки), рисуется здесь painter-ом; цвета — из [`palette`].

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Galley, Id, Label, Margin, Painter, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, TextEdit, TextWrapMode, Ui, pos2, vec2,
};

use crate::icons::{self, Icon};
use crate::record::{Cap, ModKey};
use crate::theme::{self, palette, radius};

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, theme::medium())
}

/// Межстрочный интервал макета: основной текст 14/20, пояснения 13/18.
pub const BODY_LINE: f32 = 20.0;
pub const HINT_LINE: f32 = 18.0;

fn galley(ui: &Ui, text: &str, font: FontId, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), font, color)
}

/// Заголовок раздела.
pub fn heading(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    ui.label(RichText::new(text).heading().color(p.text));
}

/// Заголовок группы внутри раздела («Свои исключения», «Встроенный список»).
pub fn subheading(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    ui.label(
        RichText::new(text)
            .text_style(theme::subheading())
            .color(p.text),
    );
}

/// Основной текст с переносом.
pub fn body(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    ui.add(
        Label::new(
            RichText::new(text)
                .color(p.text)
                .line_height(Some(BODY_LINE)),
        )
        .wrap_mode(TextWrapMode::Wrap),
    );
}

/// Пояснение 13 px вторичным цветом.
pub fn hint(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(theme::hint())
                .color(p.text_2)
                .line_height(Some(HINT_LINE)),
        )
        .wrap_mode(TextWrapMode::Wrap),
    );
}

/// Карточка: поверхность с рамкой, строки внутри разделены линией ([`row`]).
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui);
    Frame::new()
        .fill(p.bg_surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(radius::CARD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

/// Строка карточки с отступом `margin`; у всех, кроме первой, сверху линия во всю ширину.
pub fn row<R>(ui: &mut Ui, first: bool, margin: Margin, add: impl FnOnce(&mut Ui) -> R) -> R {
    if !first {
        let p = palette(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter()
            .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.border));
    }
    Frame::new()
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
            add(ui)
        })
        .inner
}

pub fn row_margin() -> Margin {
    Margin::symmetric(14, 10)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    Off,
    On,
    Partial,
}

impl Tick {
    pub fn of(on: bool) -> Tick {
        if on { Tick::On } else { Tick::Off }
    }
}

/// Флажок 18×18 в прямоугольнике `rect`: снят — рамка, отмечен — заливка акцентом и галочка,
/// частично — заливка и черта.
pub fn checkbox_at(ui: &Ui, rect: Rect, id: Id, tick: Tick) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    let p = palette(ui);
    let painter = ui.painter();
    let corner = CornerRadius::same(4);
    match tick {
        Tick::Off => {
            let edge = if resp.hovered() {
                p.text_3
            } else {
                p.border_strong
            };
            painter.rect(
                rect,
                corner,
                p.bg_field,
                Stroke::new(1.5, edge),
                StrokeKind::Inside,
            );
        }
        Tick::On | Tick::Partial => {
            let fill = if resp.hovered() {
                p.accent_hover
            } else {
                p.accent
            };
            painter.rect_filled(rect, corner, fill);
            let mark = if tick == Tick::On {
                Icon::Check
            } else {
                Icon::Dash
            };
            icons::paint(
                painter,
                Rect::from_center_size(rect.center(), vec2(16.0, 16.0)),
                mark,
                p.accent_text,
            );
        }
    }
    resp
}

pub fn checkbox(ui: &mut Ui, tick: Tick) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let id = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    checkbox_at(ui, rect, id, tick)
}

/// Флажок с подписью и пояснением под ней: щелчок по флажку или подписи переключает.
/// `Some(новое значение)` — переключили.
pub fn option(ui: &mut Ui, on: bool, title: &str, hint_text: &str) -> Option<bool> {
    let p = palette(ui);
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            ui.add_space(1.0);
            clicked |= checkbox(ui, Tick::of(on)).clicked();
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            clicked |= ui
                .add(
                    Label::new(
                        RichText::new(title)
                            .color(p.text)
                            .line_height(Some(BODY_LINE)),
                    )
                    .wrap_mode(TextWrapMode::Wrap)
                    .sense(Sense::click()),
                )
                .clicked();
            if !hint_text.is_empty() {
                hint(ui, hint_text);
            }
        });
    });
    clicked.then_some(!on)
}

/// Пункт навигации: значок раздела, подпись; выбранный — заливка `accent-soft`, требующий
/// внимания — точка цвета `warn` справа.
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, selected: bool, attention: bool) -> Response {
    let p = palette(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, radius::NAV_ITEM, p.accent_soft);
    } else if resp.hovered() {
        painter.rect_filled(rect, radius::NAV_ITEM, p.bg_hover);
    }
    let (fg, icon_color, text_font) = if selected {
        (p.accent_soft_text, p.accent_soft_text, medium(14.0))
    } else {
        (p.text, p.text_2, font(14.0))
    };
    let icon_rect = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.center().y - 10.0),
        vec2(20.0, 20.0),
    );
    icons::paint(painter, icon_rect, icon, icon_color);
    painter.text(
        pos2(icon_rect.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        text_font,
        fg,
    );
    if attention {
        painter.circle_filled(pos2(rect.right() - 12.0, rect.center().y), 4.0, p.warn);
    }
    resp
}

fn mod_icon(m: ModKey) -> Option<Icon> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    Some(match m {
        ModKey::Meta => Icon::KeyCmd,
        ModKey::Alt => Icon::KeyOption,
        ModKey::Shift => Icon::KeyShift,
        ModKey::Ctrl => Icon::KeyControl,
    })
}

const TWICE: &str = "дважды";

fn cap_width(ui: &Ui, cap: &Cap) -> f32 {
    let p = palette(ui);
    match cap {
        Cap::Mod(m) if mod_icon(*m).is_some() => 24.0,
        Cap::Mod(m) => (galley(ui, m.word(), medium(12.0), p.text).size().x + 12.0).max(24.0),
        Cap::Key(k) => (galley(ui, k, medium(12.0), p.text).size().x + 12.0).max(24.0),
        Cap::Twice => galley(ui, TWICE, font(12.0), p.text_2).size().x,
    }
}

/// Колпачки клавиш в строку, левый край — `left`, по вертикали — центр `center_y`. Колпачок:
/// высота 22, нижняя грань 2 px, модификаторы macOS — значками.
pub fn paint_caps(ui: &Ui, painter: &Painter, left: f32, center_y: f32, caps: &[Cap]) {
    let p = palette(ui);
    let mut x = left;
    for cap in caps {
        let w = cap_width(ui, cap);
        if *cap == Cap::Twice {
            painter.text(
                pos2(x, center_y),
                Align2::LEFT_CENTER,
                TWICE,
                font(12.0),
                p.text_2,
            );
            x += w + 4.0;
            continue;
        }
        let rect = Rect::from_min_size(pos2(x, center_y - 12.0), vec2(w, 22.0));
        painter.rect_filled(
            rect.translate(vec2(0.0, 2.0)),
            radius::KEYCAP,
            p.keycap_border,
        );
        painter.rect(
            rect,
            radius::KEYCAP,
            p.keycap_bg,
            Stroke::new(1.0, p.keycap_border),
            StrokeKind::Inside,
        );
        match cap {
            Cap::Mod(m) => match mod_icon(*m) {
                Some(icon) => icons::paint(
                    painter,
                    Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
                    icon,
                    p.text,
                ),
                None => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        m.word(),
                        medium(12.0),
                        p.text,
                    );
                }
            },
            Cap::Key(k) => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    k,
                    medium(12.0),
                    p.text,
                );
            }
            Cap::Twice => {}
        }
        x += w + 4.0;
    }
}

/// Что показывает поле сочетания.
pub enum FieldState<'a> {
    Empty,
    Caps(&'a [Cap]),
    Recording,
}

/// Поле сочетания: пусто — «не задано», назначено — колпачки, запись — акцентная рамка и
/// «Нажмите сочетание…», ошибка — рамка цвета ошибки.
pub fn hotkey_field_at(ui: &Ui, rect: Rect, id: Id, state: &FieldState, error: bool) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    let p = palette(ui);
    let painter = ui.painter();
    if let FieldState::Recording = state {
        painter.rect(
            rect,
            radius::WIDGET,
            p.accent_soft,
            Stroke::new(2.0, p.accent),
            StrokeKind::Inside,
        );
        let dot = pos2(rect.left() + 14.0, rect.center().y);
        painter.circle_filled(dot, 4.0, p.accent);
        painter.text(
            pos2(dot.x + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            "Нажмите сочетание…",
            medium(13.0),
            p.accent_soft_text,
        );
        return resp;
    }
    let edge = if error {
        Stroke::new(1.5, p.error)
    } else if resp.hovered() {
        Stroke::new(1.0, p.text_3)
    } else {
        Stroke::new(1.0, p.border_strong)
    };
    painter.rect(rect, radius::WIDGET, p.bg_field, edge, StrokeKind::Inside);
    match state {
        FieldState::Empty => {
            painter.text(
                pos2(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                "не задано",
                font(14.0),
                p.text_3,
            );
        }
        FieldState::Caps(caps) => {
            let clipped = painter.with_clip_rect(rect.shrink(2.0));
            paint_caps(ui, &clipped, rect.left() + 5.0, rect.center().y, caps);
        }
        FieldState::Recording => {}
    }
    resp
}

/// Кнопка-значок 28×28: «…» и «+». `pressed` — открыто её меню.
pub fn icon_button_at(ui: &Ui, rect: Rect, id: Id, icon: Icon, pressed: bool) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    let p = palette(ui);
    let fill = if pressed || resp.is_pointer_button_down_on() {
        p.bg_pressed
    } else if resp.hovered() {
        p.bg_hover
    } else {
        p.bg_surface
    };
    ui.painter().rect(
        rect,
        radius::WIDGET,
        fill,
        Stroke::new(1.0, p.border_strong),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
        icon,
        p.text_2,
    );
    resp
}

/// Заголовок меню («Готовые сочетания»).
pub fn menu_title(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    Frame::new()
        .inner_margin(Margin {
            left: 10,
            right: 10,
            top: 6,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(medium(12.0)).color(p.text_3));
        });
}

/// Пункт меню высотой 30: наведение — заливка `bg-hover`; содержимое рисует `paint`
/// (прямоугольник пункта без отступов).
pub fn menu_item(ui: &mut Ui, paint: impl FnOnce(&Ui, Rect)) -> Response {
    let p = palette(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5, p.bg_hover);
    }
    paint(ui, rect.shrink2(vec2(10.0, 0.0)));
    resp
}

pub fn menu_separator(ui: &mut Ui) {
    let p = palette(ui);
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.border));
    ui.add_space(4.0);
}

/// Тихая кнопка без рамки: значок 16 и подпись 13 («Удалить», «Очистить всё»).
pub fn quiet_button(ui: &mut Ui, icon: Icon, text: &str, color: Color32) -> Response {
    let p = palette(ui);
    let label = galley(ui, text, font(13.0), color);
    let size = vec2(8.0 + 16.0 + 6.0 + label.size().x + 8.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, radius::WIDGET, p.bg_hover);
    }
    let icon_rect = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.center().y - 8.0),
        vec2(16.0, 16.0),
    );
    icons::paint(ui.painter(), icon_rect, icon, color);
    ui.painter().galley(
        pos2(
            icon_rect.right() + 6.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        color,
    );
    resp
}

/// Основная кнопка: заливка акцентом («Добавить»).
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    let label = galley(ui, text, medium(14.0), p.accent_text);
    let size = vec2(label.size().x + 28.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        p.accent_hover
    } else {
        p.accent
    };
    ui.painter().rect_filled(rect, radius::WIDGET, fill);
    ui.painter()
        .galley(rect.center() - label.size() / 2.0, label, p.accent_text);
    resp
}

/// Обычная кнопка: поверхность с рамкой.
pub fn button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    let label = galley(ui, text, medium(14.0), p.text);
    let size = vec2(label.size().x + 24.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        p.bg_pressed
    } else if resp.hovered() {
        p.bg_hover
    } else {
        p.bg_surface
    };
    ui.painter().rect(
        rect,
        radius::WIDGET,
        fill,
        Stroke::new(1.0, p.border_strong),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - label.size() / 2.0, label, p.text);
    resp
}

/// Ссылка-действие акцентным цветом («Ввести идентификатор…»).
pub fn link(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(Label::new(RichText::new(text).font(medium(14.0)).color(p.accent)).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Однострочное поле ввода в своей рамке: фокус — акцентная рамка, ошибка — рамка ошибки.
pub fn text_field(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    placeholder: &str,
    width: f32,
    error: bool,
) -> Response {
    let p = palette(ui);
    let focused = ui.memory(|m| m.has_focus(id));
    let edge = if error {
        Stroke::new(1.5, p.error)
    } else if focused {
        Stroke::new(1.5, p.accent)
    } else {
        Stroke::new(1.0, p.border_strong)
    };
    Frame::new()
        .fill(p.bg_field)
        .stroke(edge)
        .corner_radius(radius::WIDGET)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(text)
                    .id(id)
                    .frame(Frame::NONE)
                    .margin(Margin::ZERO)
                    .font(font(14.0))
                    .text_color(p.text)
                    .hint_text(RichText::new(placeholder).color(p.text_3))
                    .desired_width(width - 20.0),
            )
        })
        .inner
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Warn,
    Error,
}

/// Баннер во всю ширину: значок, текст и кнопка под ним. `true` — нажата кнопка.
pub fn banner(ui: &mut Ui, kind: BannerKind, text: &str, action: &str) -> bool {
    let p = palette(ui);
    let (fill, edge, accent) = match kind {
        BannerKind::Warn => (p.warn_bg, p.warn_border, p.warn),
        BannerKind::Error => (p.error_bg, p.error_border, p.error),
    };
    let mut clicked = false;
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, edge))
        .corner_radius(radius::BANNER)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                icons::paint(ui.painter(), rect, Icon::StatusWarning, accent);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    body(ui, text);
                    clicked = button(ui, action).clicked();
                });
            });
        });
    clicked
}

/// Строка ошибки: значок предупреждения 16 и текст 13 цветом ошибки.
pub fn error_line(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
        icons::paint(
            ui.painter(),
            Rect::from_min_size(rect.min + vec2(0.0, 1.0), vec2(16.0, 16.0)),
            Icon::StatusWarning,
            p.error,
        );
        ui.add(
            Label::new(
                RichText::new(text)
                    .text_style(theme::hint())
                    .color(p.error)
                    .line_height(Some(HINT_LINE)),
            )
            .wrap_mode(TextWrapMode::Wrap),
        );
    });
}

/// Плашка-статус («выдано», «не выдано»).
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    let label = galley(ui, text, medium(12.0), fg);
    let size = label.size() + vec2(16.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, radius::PILL, bg);
    ui.painter()
        .galley(rect.center() - label.size() / 2.0, label, fg);
}

/// Путь или команда в рамке, моноширинным, с выделением.
pub fn mono_box(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    Frame::new()
        .fill(p.bg_faint)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(radius::WIDGET)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                Label::new(RichText::new(text).monospace().color(p.text))
                    .selectable(true)
                    .wrap_mode(TextWrapMode::Wrap),
            );
        });
}

/// Пустой список: подсказка в бледной рамке.
pub fn empty_state(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    Frame::new()
        .fill(p.bg_faint)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(radius::CARD)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            hint(ui, text);
        });
}

/// Цвета квадратов-заглушек вместо иконки приложения.
const BADGE_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x2F, 0x80, 0xED),
    Color32::from_rgb(0x2A, 0xA8, 0xE0),
    Color32::from_rgb(0x2B, 0x2F, 0x36),
    Color32::from_rgb(0xE8, 0xA3, 0x17),
    Color32::from_rgb(0x4A, 0x15, 0x4B),
    Color32::from_rgb(0x6B, 0x72, 0x80),
    Color32::from_rgb(0xF5, 0x9E, 0x0B),
    Color32::from_rgb(0x00, 0x7A, 0xCC),
];

/// Квадрат 24 с первой буквой названия: иконку приложения система отдаёт не везде.
pub fn app_badge(ui: &mut Ui, name: &str, id: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    paint_badge(ui, rect, name, id);
}

pub fn paint_badge(ui: &Ui, rect: Rect, name: &str, id: &str) {
    let hash = id
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let color = BADGE_COLORS[hash as usize % BADGE_COLORS.len()];
    ui.painter().rect_filled(rect, 6, color);
    let letter: String = name
        .chars()
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        medium(12.0),
        Color32::WHITE,
    );
}

/// Шеврон раскрытия 16.
pub fn chevron(ui: &Ui, rect: Rect, open: bool, color: Color32) {
    let icon = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    icons::paint(ui.painter(), rect, icon, color);
}

/// Разворачиваемая карточка: шеврон и заголовок 14 Medium, тело под линией.
pub fn collapsing_card(
    ui: &mut Ui,
    id: Id,
    title: &str,
    default_open: bool,
    body_ui: impl FnOnce(&mut Ui),
) {
    let p = palette(ui);
    let mut open = ui.data_mut(|d| *d.get_temp_mut_or(id, default_open));
    card(ui, |ui| {
        let header = Frame::new()
            .inner_margin(Margin {
                left: 10,
                right: 14,
                top: 10,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    chevron(ui, rect, open, p.text_2);
                    ui.label(RichText::new(title).font(medium(14.0)).color(p.text));
                });
            });
        let click = ui.interact(header.response.rect, id.with("header"), Sense::click());
        if click.clicked() {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        if open {
            row(
                ui,
                false,
                Margin {
                    left: 32,
                    right: 14,
                    top: 12,
                    bottom: 14,
                },
                body_ui,
            );
        }
    });
}
