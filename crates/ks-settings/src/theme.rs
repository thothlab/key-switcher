//! Тема окна по `design/tokens.json`: палитра по ролям для светлой и тёмной темы, `Visuals`,
//! отступы и стили текста egui. Тема следует системной; свои виджеты берут цвета из [`palette`].

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme,
    Visuals, style::Selection, style::TextCursorStyle, style::WidgetVisuals, vec2,
};

/// Палитра по ролям; имена — как переменные в `design/key-switcher.pen`.
pub struct Palette {
    pub bg_window: Color32,
    pub bg_sidebar: Color32,
    pub bg_surface: Color32,
    pub bg_field: Color32,
    pub bg_faint: Color32,
    pub bg_hover: Color32,
    pub bg_pressed: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_2: Color32,
    pub text_3: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_text: Color32,
    pub accent_soft: Color32,
    pub accent_soft_text: Color32,
    pub warn: Color32,
    pub warn_bg: Color32,
    pub warn_border: Color32,
    pub error: Color32,
    pub error_bg: Color32,
    pub error_border: Color32,
    pub success: Color32,
    pub success_bg: Color32,
    pub keycap_bg: Color32,
    pub keycap_border: Color32,
    pub scroll: Color32,
    pub shadow: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const LIGHT: Palette = Palette {
    bg_window: rgb(0xF6F7F9),
    bg_sidebar: rgb(0xECEEF2),
    bg_surface: rgb(0xFFFFFF),
    bg_field: rgb(0xFFFFFF),
    bg_faint: rgb(0xF0F2F5),
    bg_hover: rgb(0xE3E6EB),
    bg_pressed: rgb(0xD5D9E0),
    border: rgb(0xDADEE4),
    border_strong: rgb(0xBFC5CE),
    text: rgb(0x1B1F24),
    text_2: rgb(0x59616C),
    text_3: rgb(0x757D88),
    accent: rgb(0x3550D0),
    accent_hover: rgb(0x2A42B5),
    accent_text: rgb(0xFFFFFF),
    accent_soft: rgb(0xE1E6FC),
    accent_soft_text: rgb(0x2238A8),
    warn: rgb(0x8F5300),
    warn_bg: rgb(0xFFF3DC),
    warn_border: rgb(0xEFCF94),
    error: rgb(0xB42318),
    error_bg: rgb(0xFDECEA),
    error_border: rgb(0xF3B9B3),
    success: rgb(0x18794E),
    success_bg: rgb(0xE3F4EA),
    keycap_bg: rgb(0xF3F4F7),
    keycap_border: rgb(0xC9CED6),
    scroll: rgb(0xC4C9D1),
    shadow: Color32::from_black_alpha(0x2E),
};

pub const DARK: Palette = Palette {
    bg_window: rgb(0x1C1E22),
    bg_sidebar: rgb(0x16181B),
    bg_surface: rgb(0x24272C),
    bg_field: rgb(0x121417),
    bg_faint: rgb(0x212429),
    bg_hover: rgb(0x2C3036),
    bg_pressed: rgb(0x363B42),
    border: rgb(0x33373E),
    border_strong: rgb(0x4A505A),
    text: rgb(0xE8EAED),
    text_2: rgb(0xA4ABB5),
    text_3: rgb(0x848B95),
    accent: rgb(0x7D93FF),
    accent_hover: rgb(0x97A8FF),
    accent_text: rgb(0x10142B),
    accent_soft: rgb(0x262E55),
    accent_soft_text: rgb(0xB7C3FF),
    warn: rgb(0xF0B762),
    warn_bg: rgb(0x352A15),
    warn_border: rgb(0x5C4720),
    error: rgb(0xFF8B7E),
    error_bg: rgb(0x3A1D1B),
    error_border: rgb(0x6A2D28),
    success: rgb(0x5FD394),
    success_bg: rgb(0x16301F),
    keycap_bg: rgb(0x30343B),
    keycap_border: rgb(0x4B515B),
    scroll: rgb(0x454A52),
    shadow: Color32::from_black_alpha(0x66),
};

/// Палитра темы, в которой рисуется `ui`.
pub fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

/// Семейство Medium (Golos Text Medium): заголовки, кнопки, выбранный пункт навигации.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn subheading() -> TextStyle {
    TextStyle::Name("Subheading".into())
}

pub fn hint() -> TextStyle {
    TextStyle::Name("Hint".into())
}

/// Радиусы скругления из `tokens.json → radius`.
pub mod radius {
    pub const WIDGET: u8 = 6;
    pub const KEYCAP: u8 = 5;
    pub const NAV_ITEM: u8 = 7;
    pub const BANNER: u8 = 8;
    pub const MENU: u8 = 8;
    pub const CARD: u8 = 10;
    pub const PILL: u8 = 10;
}

/// Шрифты, обе темы, отступы и стили текста. Какая тема видна — решает система.
pub fn install(ctx: &egui::Context) {
    crate::fonts::install(ctx);
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, false));
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, true));
    ctx.all_styles_mut(|style| {
        let s = &mut style.spacing;
        s.item_spacing = vec2(8.0, 8.0);
        s.window_margin = Margin::ZERO;
        s.button_padding = vec2(12.0, 6.0);
        s.menu_margin = Margin::same(6);
        s.indent = 22.0;
        s.interact_size = vec2(28.0, 28.0);
        s.combo_width = 290.0;
        s.combo_height = 320.0;
        s.text_edit_width = 260.0;
        s.icon_width = 18.0;
        s.icon_width_inner = 10.0;
        s.icon_spacing = 10.0;
        s.tooltip_width = 320.0;
        s.menu_width = 236.0;
        s.menu_spacing = 4.0;
        s.extra_text_line_spacing = 0.0;
        s.scroll.floating = true;
        s.scroll.bar_width = 6.0;
        s.scroll.floating_width = 6.0;
        s.scroll.bar_inner_margin = 4.0;
        s.scroll.bar_outer_margin = 4.0;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, medium())),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, medium())),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
            (subheading(), FontId::new(15.0, medium())),
            (hint(), FontId::new(13.0, FontFamily::Proportional)),
        ]
        .into();
    });
}

fn widget(
    p: &Palette,
    fill: Color32,
    weak: Color32,
    stroke: Stroke,
    fg_width: f32,
) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: weak,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(radius::WIDGET),
        fg_stroke: Stroke::new(fg_width, p.text),
        expansion: 0.0,
    }
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: p.shadow,
    };
    v.override_text_color = None;
    v.weak_text_color = Some(p.text_2);
    v.hyperlink_color = p.accent;
    v.faint_bg_color = p.bg_faint;
    v.extreme_bg_color = p.bg_field;
    v.text_edit_bg_color = Some(p.bg_field);
    v.code_bg_color = p.bg_faint;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;
    v.window_fill = p.bg_window;
    v.panel_fill = p.bg_window;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(radius::CARD);
    v.menu_corner_radius = CornerRadius::same(radius::MENU);
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.selection = Selection {
        bg_fill: p.accent_soft,
        stroke: Stroke::new(1.0, p.accent_soft_text),
    };
    v.text_cursor = TextCursorStyle {
        stroke: Stroke::new(1.5, p.accent),
        ..v.text_cursor
    };
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.disabled_alpha = 0.45;
    let w = &mut v.widgets;
    w.noninteractive = widget(p, p.bg_window, p.bg_window, Stroke::new(1.0, p.border), 1.0);
    w.inactive = widget(
        p,
        p.bg_field,
        p.bg_surface,
        Stroke::new(1.0, p.border_strong),
        1.5,
    );
    w.hovered = widget(p, p.bg_hover, p.bg_hover, Stroke::new(1.0, p.text_3), 1.5);
    w.active = widget(
        p,
        p.bg_pressed,
        p.bg_pressed,
        Stroke::new(1.0, p.text_3),
        1.5,
    );
    w.open = widget(
        p,
        p.bg_pressed,
        p.bg_pressed,
        Stroke::new(1.5, p.accent),
        1.5,
    );
    v
}
