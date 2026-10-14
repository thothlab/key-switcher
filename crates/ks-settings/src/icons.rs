//! Иконки интерфейса из `design/icons/ui`: контуры 20×20 с обводкой 1.5, цвет задаёт код.
//! Контур SVG (`design/_tools/ui_paths.json`) разбирается в ломаные один раз и рисуется
//! обводкой egui в любом масштабе — без растра и без библиотеки SVG.

use std::sync::OnceLock;

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    SectionGeneral,
    SectionHotkeys,
    SectionApps,
    SectionWords,
    SectionPermissions,
    SectionAbout,
    Delete,
    Add,
    More,
    StatusGranted,
    StatusDenied,
    StatusWarning,
    ChevronDown,
    ChevronRight,
    Check,
    Dash,
    KeyCmd,
    KeyOption,
    KeyShift,
    KeyControl,
}

const ALL: [Icon; 20] = [
    Icon::SectionGeneral,
    Icon::SectionHotkeys,
    Icon::SectionApps,
    Icon::SectionWords,
    Icon::SectionPermissions,
    Icon::SectionAbout,
    Icon::Delete,
    Icon::Add,
    Icon::More,
    Icon::StatusGranted,
    Icon::StatusDenied,
    Icon::StatusWarning,
    Icon::ChevronDown,
    Icon::ChevronRight,
    Icon::Check,
    Icon::Dash,
    Icon::KeyCmd,
    Icon::KeyOption,
    Icon::KeyShift,
    Icon::KeyControl,
];

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Icon::SectionGeneral => {
                "M3 6h7.5M14.5 6H17M3 14h2.5M9.5 14H17M10.5 6a2 2 0 1 0 4 0a2 2 0 1 0 -4 0Z\
                 M5.5 14a2 2 0 1 0 4 0a2 2 0 1 0 -4 0Z"
            }
            Icon::SectionHotkeys => {
                "M4.5 4.75h11a2 2 0 0 1 2 2v6.5a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-6.5\
                 a2 2 0 0 1 2 -2ZM5.75 8h.5M9.75 8h.5M13.75 8h.5M7 12h6"
            }
            Icon::SectionApps => {
                "M4.25 3h3a1.25 1.25 0 0 1 1.25 1.25v3a1.25 1.25 0 0 1 -1.25 1.25h-3\
                 a1.25 1.25 0 0 1 -1.25 -1.25v-3a1.25 1.25 0 0 1 1.25 -1.25Z\
                 M12.75 3h3a1.25 1.25 0 0 1 1.25 1.25v3a1.25 1.25 0 0 1 -1.25 1.25h-3\
                 a1.25 1.25 0 0 1 -1.25 -1.25v-3a1.25 1.25 0 0 1 1.25 -1.25Z\
                 M4.25 11.5h3a1.25 1.25 0 0 1 1.25 1.25v3a1.25 1.25 0 0 1 -1.25 1.25h-3\
                 a1.25 1.25 0 0 1 -1.25 -1.25v-3a1.25 1.25 0 0 1 1.25 -1.25Z\
                 M11.25 14.25a3 3 0 1 0 6 0a3 3 0 1 0 -6 0ZM12.5 14.25h3.5"
            }
            Icon::SectionWords => {
                "M3 5h14M3 10h8M3 15h5M11.25 14.25a3 3 0 1 0 6 0a3 3 0 1 0 -6 0Z\
                 M12.5 14.25h3.5"
            }
            Icon::SectionPermissions => {
                "M10 2.75 15.75 5v4.6c0 3.55-2.45 6.15-5.75 7.65-3.3-1.5-5.75-4.1-5.75-7.65V5Z\
                 M7.6 9.9l1.65 1.65 3.2-3.3"
            }
            Icon::SectionAbout => {
                "M2.75 10a7.25 7.25 0 1 0 14.5 0a7.25 7.25 0 1 0 -14.5 0ZM10 9.25v4.5\
                 M9.99 6.6a0.01 0.01 0 1 0 0.02 0a0.01 0.01 0 1 0 -0.02 0Z"
            }
            Icon::Delete => {
                "M3.75 5.75h12.5M8 5.75V4.25h4v1.5M5.5 5.75l.75 10.5h7.5l.75-10.5\
                 M8.5 9v4.25M11.5 9v4.25"
            }
            Icon::Add => "M10 4.25v11.5M4.25 10h11.5",
            Icon::More => {
                "M4.5 10a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0Z\
                 M9.5 10a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0Z\
                 M14.5 10a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0Z"
            }
            Icon::StatusGranted => {
                "M2.75 10a7.25 7.25 0 1 0 14.5 0a7.25 7.25 0 1 0 -14.5 0Z\
                 M6.9 10.1l2.15 2.15 4.05-4.25"
            }
            Icon::StatusDenied => {
                "M2.75 10a7.25 7.25 0 1 0 14.5 0a7.25 7.25 0 1 0 -14.5 0Z\
                 M7.4 7.4l5.2 5.2M12.6 7.4l-5.2 5.2"
            }
            Icon::StatusWarning => {
                "M10 3.25 17.25 16H2.75ZM10 8.25v3.5\
                 M9.99 13.85a0.01 0.01 0 1 0 0.02 0a0.01 0.01 0 1 0 -0.02 0Z"
            }
            Icon::ChevronDown => "M5.5 8l4.5 4.5L14.5 8",
            Icon::ChevronRight => "M8 5.5l4.5 4.5L8 14.5",
            Icon::Check => "M5 10.25l3.25 3.25L15 6.75",
            Icon::Dash => "M5.5 10h9",
            Icon::KeyCmd => {
                "M13 4.5v11a2.5 2.5 0 1 0 2.5-2.5h-11a2.5 2.5 0 1 0 2.5 2.5v-11\
                 a2.5 2.5 0 1 0-2.5 2.5h11a2.5 2.5 0 1 0-2.5-2.5"
            }
            Icon::KeyOption => "M3 5.5h4.5l5 9H17M12 5.5h5",
            Icon::KeyShift => "M10 3.25 17 10.5h-3.75v6H6.75v-6H3Z",
            Icon::KeyControl => "M5 11.5l5-5 5 5",
        }
    }

    fn subpaths(self) -> &'static [Subpath] {
        static PARSED: OnceLock<Vec<Vec<Subpath>>> = OnceLock::new();
        let all = PARSED.get_or_init(|| ALL.iter().map(|i| parse(i.path())).collect());
        let index = ALL.iter().position(|i| *i == self).unwrap_or(0);
        &all[index]
    }
}

/// Ломаная в координатах холста 20×20.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub points: Vec<Pos2>,
    pub closed: bool,
}

/// Иконка в квадрате `rect` (холст 20×20 растягивается на его сторону), обводка 1.5 холста.
/// Точки (дуги нулевого радиуса) — кружки диаметром в обводку, концы линий скруглены.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let scale = rect.width() / 20.0;
    let width = 1.5 * scale;
    let at = |p: Pos2| rect.min + vec2(p.x * scale, p.y * scale);
    for sub in icon.subpaths() {
        let points: Vec<Pos2> = sub.points.iter().copied().map(at).collect();
        let (lo, hi) = bounds(&sub.points);
        if (hi - lo).length() < 0.2 {
            painter.circle_filled(at(lo + (hi - lo) / 2.0), width / 2.0, color);
            continue;
        }
        let stroke = Stroke::new(width, color);
        if sub.closed {
            painter.add(Shape::closed_line(points, stroke));
        } else {
            for end in [points[0], points[points.len() - 1]] {
                painter.circle_filled(end, width / 2.0, color);
            }
            painter.add(Shape::line(points, stroke));
        }
    }
}

fn bounds(points: &[Pos2]) -> (Pos2, Pos2) {
    points.iter().fold(
        (pos2(f32::MAX, f32::MAX), pos2(f32::MIN, f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

/// Разбор контура SVG: команды M L H V A C Z в обоих регистрах, повтор координат без команды.
pub fn parse(d: &str) -> Vec<Subpath> {
    let mut tokens = Tokens::new(d);
    let mut out: Vec<Subpath> = Vec::new();
    let mut current = Subpath {
        points: Vec::new(),
        closed: false,
    };
    let mut pen = pos2(0.0, 0.0);
    let mut start = pen;
    let mut command = 'M';
    while let Some(next) = tokens.command_or_number() {
        if let Some(c) = next {
            command = c;
            if matches!(c, 'Z' | 'z') {
                current.closed = true;
                pen = start;
                flush(&mut out, &mut current);
                continue;
            }
        }
        let rel = command.is_ascii_lowercase();
        let base = if rel { pen } else { pos2(0.0, 0.0) };
        match command.to_ascii_uppercase() {
            'M' => {
                let p = base + tokens.vec();
                flush(&mut out, &mut current);
                current.points.push(p);
                pen = p;
                start = p;
                // координаты после M без команды — L той же относительности
                command = if rel { 'l' } else { 'L' };
            }
            'L' => {
                pen = base + tokens.vec();
                current.points.push(pen);
            }
            'H' => {
                let x = tokens.number();
                pen = pos2(if rel { pen.x + x } else { x }, pen.y);
                current.points.push(pen);
            }
            'V' => {
                let y = tokens.number();
                pen = pos2(pen.x, if rel { pen.y + y } else { y });
                current.points.push(pen);
            }
            'C' => {
                let c1 = base + tokens.vec();
                let c2 = base + tokens.vec();
                let end = base + tokens.vec();
                for i in 1..=12 {
                    let t = i as f32 / 12.0;
                    let u = 1.0 - t;
                    let p = pen.to_vec2() * u * u * u
                        + c1.to_vec2() * 3.0 * u * u * t
                        + c2.to_vec2() * 3.0 * u * t * t
                        + end.to_vec2() * t * t * t;
                    current.points.push(p.to_pos2());
                }
                pen = end;
            }
            'A' => {
                let (rx, ry) = (tokens.number(), tokens.number());
                let rotation = tokens.number();
                let large = tokens.flag();
                let sweep = tokens.flag();
                let end = base + tokens.vec();
                arc(
                    &mut current.points,
                    pen,
                    end,
                    rx,
                    ry,
                    rotation,
                    large,
                    sweep,
                );
                pen = end;
            }
            _ => break,
        }
    }
    flush(&mut out, &mut current);
    out
}

fn flush(out: &mut Vec<Subpath>, current: &mut Subpath) {
    if !current.points.is_empty() {
        out.push(std::mem::replace(
            current,
            Subpath {
                points: Vec::new(),
                closed: false,
            },
        ));
    }
    current.closed = false;
}

/// Дуга эллипса по концам (SVG 1.1, F.6.5) — точками через каждые ~10°.
#[allow(clippy::too_many_arguments)]
fn arc(
    points: &mut Vec<Pos2>,
    from: Pos2,
    to: Pos2,
    rx: f32,
    ry: f32,
    rotation_deg: f32,
    large: bool,
    sweep: bool,
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || from == to {
        points.push(to);
        return;
    }
    let phi = rotation_deg.to_radians();
    let (sin, cos) = phi.sin_cos();
    let d = (from - to) / 2.0;
    let x1 = cos * d.x + sin * d.y;
    let y1 = -sin * d.x + cos * d.y;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let cx1 = k * rx * y1 / ry;
    let cy1 = -k * ry * x1 / rx;
    let mid = (from.to_vec2() + to.to_vec2()) / 2.0;
    let center = pos2(cos * cx1 - sin * cy1 + mid.x, sin * cx1 + cos * cy1 + mid.y);
    let angle = |ux: f32, uy: f32| uy.atan2(ux);
    let theta1 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let theta2 = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let mut delta = theta2 - theta1;
    let tau = std::f32::consts::TAU;
    if sweep && delta < 0.0 {
        delta += tau;
    } else if !sweep && delta > 0.0 {
        delta -= tau;
    }
    let steps = ((delta.abs() / 10f32.to_radians()).ceil() as usize).max(2);
    for i in 1..=steps {
        let t = theta1 + delta * i as f32 / steps as f32;
        let (st, ct) = t.sin_cos();
        let (x, y) = (rx * ct, ry * st);
        points.push(pos2(
            center.x + cos * x - sin * y,
            center.y + sin * x + cos * y,
        ));
    }
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Tokens<'a> {
    fn new(d: &'a str) -> Self {
        Tokens {
            s: d.as_bytes(),
            i: 0,
        }
    }

    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\n' | b'\t') {
            self.i += 1;
        }
    }

    /// `Some(Some(c))` — команда, `Some(None)` — дальше число (повтор прошлой команды),
    /// `None` — конец.
    fn command_or_number(&mut self) -> Option<Option<char>> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() {
            self.i += 1;
            Some(Some(c as char))
        } else {
            Some(None)
        }
    }

    fn number(&mut self) -> f32 {
        self.skip();
        let start = self.i;
        let mut seen_dot = false;
        while let Some(&c) = self.s.get(self.i) {
            let sign_ok = (c == b'-' || c == b'+') && self.i == start;
            let dot_ok = c == b'.' && !seen_dot;
            if !(c.is_ascii_digit() || sign_ok || dot_ok) {
                break;
            }
            seen_dot |= c == b'.';
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0.0)
    }

    /// Флаг дуги: одна цифра, может идти вплотную к следующему числу.
    fn flag(&mut self) -> bool {
        self.skip();
        let c = self.s.get(self.i).copied();
        self.i += 1;
        c == Some(b'1')
    }

    fn vec(&mut self) -> eframe::egui::Vec2 {
        vec2(self.number(), self.number())
    }
}
