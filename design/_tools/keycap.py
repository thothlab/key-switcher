"""Иконка приложения «колпачок A / Ф» и значок трея на её основе: SVG и PNG трея.

Запуск из каталога design/:  python3 -I -B _tools/keycap.py
Печатает JSON геометрии иконки приложения для pen (фреймы icon-master, icon-small, icon-tight, icon-small-tight).
"""
import json, math, os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from icons import (out, fmt, raster, write_png, hexrgba, rrect, circle, capsule, ring_arc, union, minus, tri, Poly)
import ttfglyph

# Буквы — контуры PT Sans Bold (ParaType, OFL 1.1, _tools/fonts/PTSans-OFL.txt)
FONT = ttfglyph.Font(os.path.join(os.path.dirname(os.path.abspath(__file__)), "fonts", "PTSans-Bold.ttf"))
GLYPH_A = FONT.glyph("A")[0]
GLYPH_F = FONT.glyph("Ф")[0]


def place(contours, cap, left=None, right=None, top=None, baseline=None):
    """Преобразование единиц шрифта (y вверх) в рисунок (y вниз): высота прописной cap, привязка к краю."""
    sc = cap / FONT.cap
    x0, y0, x1, y1 = ttfglyph.bbox(contours)
    dx = left - x0 * sc if left is not None else right - x1 * sc
    base = baseline if baseline is not None else top + FONT.cap * sc
    return lambda x, y: (dx + x * sc, base - y * sc)

# ---------------------------------------------------------------- иконка приложения, холст 1024
BODY = "#2F7D6D"
INK = "#1D5448"
FACE = "#F4FAF8"
TIGHT = 944 / 824


def app_spec(small=False, k=1.0):
    """Фигуры иконки в координатах 1024; k — плотная сетка Windows/Linux (масштаб вокруг центра)."""
    s = lambda v: 512 - (512 - v) * k
    d = lambda v: v * k
    shapes = [("rect", dict(x=s(100), y=s(100), w=d(824), h=d(824), r=d(184), fill=BODY))]
    if small:
        shapes.append(("rect", dict(x=s(188), y=s(188), w=d(648), h=d(648), r=d(128), fill=FACE)))
        ta = place(GLYPH_A, d(280), left=s(236), top=s(236))
        tf = place(GLYPH_F, d(280), right=s(792), baseline=s(772))
    else:
        shapes.append(("rect", dict(x=s(232), y=s(264), w=d(560), h=d(540), r=d(112), fill=INK)))
        shapes.append(("rect", dict(x=s(232), y=s(224), w=d(560), h=d(540), r=d(112), fill=FACE)))
        ta = place(GLYPH_A, d(190), left=s(296), top=s(280))
        tf = place(GLYPH_F, d(190), right=s(728), baseline=s(690))
    shapes.append(("fill", dict(d=ttfglyph.to_svg(GLYPH_A, ta, fmt), color=INK, name="Letter A")))
    shapes.append(("fill", dict(d=ttfglyph.to_svg(GLYPH_F, tf, fmt), color=INK, name="Letter Ф")))
    return shapes


def app_svg(shapes):
    body = []
    for kind, p in shapes:
        if kind == "rect":
            body.append(f'<rect x="{fmt(p["x"])}" y="{fmt(p["y"])}" width="{fmt(p["w"])}" height="{fmt(p["h"])}" rx="{fmt(p["r"])}" fill="{p["fill"]}"/>')
        else:
            body.append(f'<path d="{p["d"]}" fill="{p["color"]}"/>')
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n'
            + "\n".join(body) + "\n</svg>\n")


# ---------------------------------------------------------------- значок трея, холст 18×18
# Контурный стиль: колпачок — рамка 1.3 pt, буквы залиты; замок и черта контурные,
# бейджи внимания (4–6) закрашены. Вокруг бейджей прозрачный зазор.
GAP = 1.0
KEY = (1.5, 1.5, 16.5, 16.5, 3.5)
KEY_W = 1.0
TRAY_A = place(GLYPH_A, 5.4, left=3.7, top=3.6)
TRAY_F = place(GLYPH_F, 5.4, right=14.3, baseline=14.2)
BADGE_C, BADGE_R = (13.9, 4.1), 3.1
TRI = [(14.0, 1.0), (17.0, 7.0), (11.0, 7.0)]
TRI_GAP = [(14.0, -1.2), (19.0, 8.2), (9.0, 8.2)]
LOCK_BODY = (1.5, 12.5, 6.9, 16.5, 1.0)
LOCK_W = 1.0
LOCK_SHACKLE = (4.2, 12.2, 1.5, 1.0, 12.5)
LOCK_CUT = (-0.5, 9.6, 8.6, 18.6, 2.0)
SLASH = ((1.5, 16.5), (16.5, 1.5), 1.0)


def outline_rrect(x0, y0, x1, y1, r, w):
    h = w / 2
    return minus(rrect(x0 - h, y0 - h, x1 + h, y1 + h, r + h), rrect(x0 + h, y0 + h, x1 - h, y1 - h, max(r - h, 0)))


def letters(with_f=True):
    fs = [Poly(ttfglyph.to_polys(GLYPH_A, TRAY_A))]
    if with_f:
        fs.append(Poly(ttfglyph.to_polys(GLYPH_F, TRAY_F)))
    return fs


def letters_d(with_f):
    return ttfglyph.to_svg(GLYPH_A, TRAY_A, fmt) + (ttfglyph.to_svg(GLYPH_F, TRAY_F, fmt) if with_f else "")


def bang(c):
    cx, cy = c
    return [rrect(cx - 0.55, cy - 2.15, cx + 0.55, cy + 0.55, 0.4), circle(cx, cy + 1.55, 0.6)]


def tray_shapes(state):
    base = [outline_rrect(*KEY, KEY_W)] + letters(state != "needs-layouts")
    cuts, badge, holes = [], [], []
    if state == "autocorrect-off":
        (a, b, w) = SLASH
        cuts.append(capsule(*a, *b, w + 2 * GAP))
        badge = [capsule(*a, *b, w)]
    elif state == "secure-input":
        cuts.append(rrect(*LOCK_CUT))
        badge = [outline_rrect(*LOCK_BODY, LOCK_W), ring_arc(*LOCK_SHACKLE)]
    elif state in ("needs-permissions", "needs-layouts"):
        cuts.append(circle(*BADGE_C, BADGE_R + GAP))
        badge = [circle(*BADGE_C, BADGE_R)]
        holes = bang(BADGE_C)
    elif state == "config-error":
        cuts.append(tri(TRI_GAP))
        badge = [tri(TRI)]
        holes = [rrect(13.48, 3.2, 14.52, 5.25, 0.4), circle(14.0, 6.05, 0.5)]
    return union(*base), cuts, badge, holes


def tray_layers(state, key_rgba, badge_rgba=None):
    key, cuts, badge, holes = tray_shapes(state)
    layers = [(minus(key, *cuts) if cuts else key, key_rgba)]
    if badge:
        layers.append((minus(union(*badge), *holes) if holes else union(*badge), badge_rgba or key_rgba))
    return layers


def tray_svg(state, color="#000000", view="0 0 18 18", size=18):
    k = "#000"
    rect = lambda x0, y0, x1, y1, r, extra: f'<rect x="{fmt(x0)}" y="{fmt(y0)}" width="{fmt(x1 - x0)}" height="{fmt(y1 - y0)}" rx="{fmt(r)}" {extra}/>'
    cut, badge, holes = "", "", ""
    if state == "autocorrect-off":
        (a, b, w) = SLASH
        cut = f'<path d="M{a[0]} {a[1]}L{b[0]} {b[1]}" stroke="{k}" stroke-width="{fmt(w + 2 * GAP)}" stroke-linecap="round"/>'
        badge = f'<path d="M{a[0]} {a[1]}L{b[0]} {b[1]}" fill="none" stroke="{color}" stroke-width="{w}" stroke-linecap="round"/>'
    elif state == "secure-input":
        cut = rect(*LOCK_CUT, f'fill="{k}"')
        cx, cy, rr, w, _ = LOCK_SHACKLE
        badge = (rect(*LOCK_BODY, f'fill="none" stroke="{color}" stroke-width="{LOCK_W}"')
                 + f'<path d="M{fmt(cx - rr)} 12.5V{cy}a{rr} {rr} 0 0 1 {fmt(2 * rr)} 0V12.5" fill="none" stroke="{color}" stroke-width="{w}"/>')
    elif state in ("needs-permissions", "needs-layouts"):
        cx, cy = BADGE_C
        cut = f'<circle cx="{cx}" cy="{cy}" r="{fmt(BADGE_R + GAP)}" fill="{k}"/>'
        badge = f'<circle cx="{cx}" cy="{cy}" r="{BADGE_R}" fill="{color}"/>'
        holes = (f'<rect x="{fmt(cx - 0.55)}" y="{fmt(cy - 2.15)}" width="1.1" height="2.7" rx="0.4" fill="{k}"/>'
                 f'<circle cx="{cx}" cy="{fmt(cy + 1.55)}" r="0.6" fill="{k}"/>')
    elif state == "config-error":
        pts = lambda t: "M" + "L".join(f"{fmt(x)} {fmt(y)}" for x, y in t) + "Z"
        cut = f'<path d="{pts(TRI_GAP)}" fill="{k}"/>'
        badge = f'<path d="{pts(TRI)}" fill="{color}"/>'
        holes = (f'<rect x="13.48" y="3.2" width="1.04" height="2.05" rx="0.4" fill="{k}"/>'
                 f'<circle cx="14" cy="6.05" r="0.5" fill="{k}"/>')
    mask = lambda i, inner: (f'<mask id="{i}" maskUnits="userSpaceOnUse" x="-2" y="-2" width="22" height="22">'
                             f'<rect x="-2" y="-2" width="22" height="22" fill="#fff"/>{inner}</mask>')
    defs = (mask("m", cut) if cut else "") + (mask("h", holes) if holes else "")
    base = (rect(*KEY, f'fill="none" stroke="{color}" stroke-width="{KEY_W}"')
            + f'<path d="{letters_d(state != "needs-layouts")}" fill="{color}"/>')
    mm = ' mask="url(#m)"' if cut else ""
    mh = ' mask="url(#h)"' if holes else ""
    body = f"<g{mm}>{base}</g>"
    if badge:
        body += f"<g{mh}>{badge}</g>"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="{view}">'
            + (f"<defs>{defs}</defs>" if defs else "") + f"{body}</svg>\n")


STATES = ["working", "autocorrect-off", "secure-input", "needs-permissions", "needs-layouts", "config-error"]
ATTENTION = {"needs-permissions": "#F5A524", "needs-layouts": "#F5A524", "config-error": "#E5484D"}
LINUX_COLOR = "#2F8F7B"


def shifted(layers):
    """Рисунок занимает 1..17 холста 18 — для 16/20/24/32/48 px берём эту область целиком."""
    return [((lambda f: (lambda x, y: f(x + 1, y + 1)))(fn), c) for fn, c in layers]


def grid(layers, size):
    return raster(shifted(layers), size, design=16.0)


def build_tray():
    for st in STATES:
        out(f"icons/tray/macos/{st}.svg", tray_svg(st))
        layers = tray_layers(st, hexrgba("#000000"))
        write_png(f"icons/tray/macos/{st}.png", raster(layers, 18))
        write_png(f"icons/tray/macos/{st}@2x.png", raster(layers, 36))
        write_png(f"_tools/preview/macos-dark-{st}.png", raster(tray_layers(st, hexrgba("#FFFFFF")), 18))
        for theme, col in (("light", "#1C1C1C"), ("dark", "#FFFFFF")):
            layers = tray_layers(st, hexrgba(col), hexrgba(ATTENTION[st]) if st in ATTENTION else None)
            for size in (16, 20, 24, 32):
                write_png(f"icons/tray/windows/{st}-{theme}-{size}.png", grid(layers, size))
        out(f"icons/tray/linux/{st}-symbolic.svg", tray_svg(st, color="#2e3436", view="1 1 16 16", size=22))
        layers = tray_layers(st, hexrgba(LINUX_COLOR), hexrgba(ATTENTION[st]) if st in ATTENTION else None)
        for size in (24, 48):
            write_png(f"icons/tray/linux/{st}-{size}.png", grid(layers, size))

if __name__ == "__main__":
    specs = {name: app_spec(small, k) for name, small, k in
             (("icon-master", False, 1.0), ("icon-small", True, 1.0), ("icon-tight", False, TIGHT), ("icon-small-tight", True, TIGHT))}
    out("icons/app/icon.svg", app_svg(specs["icon-master"]))
    out("icons/app/icon-small.svg", app_svg(specs["icon-small"]))
    if "--no-tray" not in sys.argv:
        build_tray()
    out("_tools/app_geometry.json", json.dumps(specs))
    print(json.dumps(specs, separators=(",", ":")))
