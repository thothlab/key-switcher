"""Источник всех иконок KeySwitcher: пишет SVG (ui, app, tray) и растрирует значки трея в PNG.

Запуск из каталога design/:  python3 -I _tools/icons.py
Геометрия знака «ЯR» задаётся в единицах 0..100 (высота прописной), общая ножка в центре.
"""
import json, math, os, struct, sys, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "_tools"))


def out(path, text):
    full = os.path.join(ROOT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        f.write(text)


def fmt(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s == "-0" else s


# ---------------------------------------------------------------- знак «ЯR»
# Подпуть: (start, [(kind, end, sweep)]) — kind "L" отрезок или "A" полуокружность.

def glyph_parts(s=14.0, b=54.0, ext=95.0):
    ro = b / 2
    ri = ro - s
    xc = ext - ro
    h = s / 2
    lw = s * 1.08
    tl = xc - lw * 0.55
    ly = b - s / 2
    right = [
        ((50, 0), [("L", (xc, 0), 0), ("A", (xc, b), 1), ("L", (50, b), 0)]),  # чаша R
        ((50 + h, s), [("L", (50 + h, b - s), 0), ("L", (xc, b - s), 0),
                       ("A", (xc, s), 0)]),  # просвет чаши (обратный обход)
        ((tl, ly), [("L", (tl + lw, ly), 0), ("L", (ext, 100), 0),
                    ("L", (ext - lw, 100), 0)]),  # ножка R
    ]
    stem = [((50 - h, 0), [("L", (50 + h, 0), 0), ("L", (50 + h, 100), 0), ("L", (50 - h, 100), 0)])]
    left = [reverse(mirror(p)) for p in right]
    return {"stem": stem, "right": right, "left": left}


def mirror(sp):
    (sx, sy), segs = sp
    return ((100 - sx, sy), [(k, (100 - x, y), 1 - w if k == "A" else w) for k, (x, y), w in segs])


def reverse(sp):
    start, segs = sp
    pts = [start] + [e for _, e, _ in segs]
    # замыкающий отрезок Pn→P0 обходится первым, затем сегменты в обратном порядке
    new = [("L", pts[-1], 0)]
    for i in range(len(segs) - 1, 0, -1):
        k, _, w = segs[i]
        new.append((k, pts[i], 1 - w if k == "A" else w))
    k, _, w = segs[0]
    new.append((k, pts[0], 1 - w if k == "A" else w))
    return (start, new)


def glyph(s=14.0, b=54.0, halves=("left", "right")):
    p = glyph_parts(s, b)
    subs = list(p["stem"])
    for hname in halves:
        subs += p[hname]
    return subs


def to_svg_d(subs, k=1.0, dx=0.0, dy=0.0):
    out_ = []
    for (sx, sy), segs in subs:
        d = [f"M{fmt(sx * k + dx)} {fmt(sy * k + dy)}"]
        prev = (sx, sy)
        for kind, (x, y), w in segs:
            if kind == "L":
                d.append(f"L{fmt(x * k + dx)} {fmt(y * k + dy)}")
            else:
                r = math.dist(prev, (x, y)) / 2 * k
                d.append(f"A{fmt(r)} {fmt(r)} 0 0 {w} {fmt(x * k + dx)} {fmt(y * k + dy)}")
            prev = (x, y)
        d.append("Z")
        out_.append("".join(d))
    return "".join(out_)


def flatten(subs, k=1.0, dx=0.0, dy=0.0, steps=24):
    polys = []
    for (sx, sy), segs in subs:
        pts = [(sx, sy)]
        prev = (sx, sy)
        for kind, (x, y), w in segs:
            if kind == "A":
                cx, cy = (prev[0] + x) / 2, (prev[1] + y) / 2
                r = math.dist(prev, (x, y)) / 2
                a0 = math.atan2(prev[1] - cy, prev[0] - cx)
                da = math.pi if w == 1 else -math.pi
                for i in range(1, steps + 1):
                    a = a0 + da * i / steps
                    pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
            else:
                pts.append((x, y))
            prev = (x, y)
        polys.append([(px * k + dx, py * k + dy) for px, py in pts])
    return polys


# ---------------------------------------------------------------- растеризатор
def winding(polys, x, y):
    wn = 0
    for poly in polys:
        n = len(poly)
        for i in range(n):
            x0, y0 = poly[i]
            x1, y1 = poly[(i + 1) % n]
            if y0 <= y:
                if y1 > y and (x1 - x0) * (y - y0) - (x - x0) * (y1 - y0) > 0:
                    wn += 1
            elif y1 <= y and (x1 - x0) * (y - y0) - (x - x0) * (y1 - y0) < 0:
                wn -= 1
    return wn


class Poly:
    def __init__(self, polys):
        self.polys = polys
        xs = [p[0] for q in polys for p in q]
        ys = [p[1] for q in polys for p in q]
        self.bb = (min(xs), min(ys), max(xs), max(ys))

    def __call__(self, x, y):
        a, b, c, d = self.bb
        if x < a or x > c or y < b or y > d:
            return False
        return winding(self.polys, x, y) != 0


def circle(cx, cy, r):
    return lambda x, y: (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def rrect(x0, y0, x1, y1, r):
    def f(x, y):
        if x < x0 or x > x1 or y < y0 or y > y1:
            return False
        qx = max(x0 + r - x, 0, x - (x1 - r))
        qy = max(y0 + r - y, 0, y - (y1 - r))
        return qx * qx + qy * qy <= r * r
    return f


def capsule(ax, ay, bx, by, w):
    def f(x, y):
        vx, vy = bx - ax, by - ay
        t = max(0, min(1, ((x - ax) * vx + (y - ay) * vy) / (vx * vx + vy * vy)))
        px, py = ax + vx * t, ay + vy * t
        return (x - px) ** 2 + (y - py) ** 2 <= (w / 2) ** 2
    return f


def ring_arc(cx, cy, r, w, ymax):
    """Дужка замка: кольцо толщиной w выше ymax."""
    return lambda x, y: y <= ymax and abs(math.hypot(x - cx, y - cy) - r) <= w / 2


def union(*fs):
    return lambda x, y: any(f(x, y) for f in fs)


def minus(a, *bs):
    return lambda x, y: a(x, y) and not any(b(x, y) for b in bs)


def tri(p):
    return Poly([p])


def raster(layers, size, design=18.0, ss=6):
    """layers: [(shape_fn, rgba)] в координатах design; рисуются по порядку (source-over)."""
    k = design / size
    px = []
    offs = [(i + 0.5) / ss for i in range(ss)]
    for j in range(size):
        row = []
        for i in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]  # premultiplied
            for fn, (r, g, b, a) in layers:
                cov = 0
                for oy in offs:
                    for ox in offs:
                        if fn((i + ox) * k, (j + oy) * k):
                            cov += 1
                if cov:
                    al = a / 255 * cov / (ss * ss)
                    for c, v in enumerate((r, g, b)):
                        acc[c] = v / 255 * al + acc[c] * (1 - al)
                    acc[3] = al + acc[3] * (1 - al)
            a = acc[3]
            if a > 0:
                row.append(tuple(round(min(1, acc[c] / a) * 255) for c in range(3)) + (round(a * 255),))
            else:
                row.append((0, 0, 0, 0))
        px.append(row)
    return px


def write_png(path, px):
    full = os.path.join(ROOT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    h, w = len(px), len(px[0])
    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in px)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    with open(full, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def hexrgba(h, a=255):
    h = h.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


# ---------------------------------------------------------------- значок трея
# Холст 18×18 (pt). Знак 14 по высоте: масштаб 0.14, сдвиг (2, 2). Жирный вариант знака.
TS, TDX, TDY = 0.14, 2.0, 2.0
TRAY_GLYPH = dict(s=18.0, b=58.0)
GAP = 1.25

BADGE_C = (14.25, 14.25)
BADGE_R = 3.75
LEFT_BADGE_C = (3.75, 14.25)


def tray_shapes(state):
    """→ (основа, вырезы, бейдж-заливка, вырезы-в-бейдже) в координатах 18×18."""
    halves = ("right",) if state == "needs-layouts" else ("left", "right")
    g = Poly(flatten(glyph(halves=halves, **TRAY_GLYPH), TS, TDX, TDY))
    cuts, badge, holes = [], [], []
    if state == "autocorrect-off":
        cuts = [capsule(1.2, 1.2, 16.8, 16.8, 1.6 + 2 * GAP)]
        badge = [capsule(1.2, 1.2, 16.8, 16.8, 1.6)]
    elif state == "secure-input":
        cuts = [rrect(9.25, 8.0, 18.5, 18.5, 2.2)]
        badge = [rrect(10.75, 13.0, 17.75, 17.75, 1.2), ring_arc(14.25, 12.5, 2.0, 1.4, 13.2)]
        holes = [circle(14.25, 15.3, 0.75)]
    elif state in ("needs-permissions", "needs-layouts"):
        c = BADGE_C if state == "needs-permissions" else LEFT_BADGE_C
        cuts = [circle(*c, BADGE_R + GAP)]
        badge = [circle(*c, BADGE_R)]
        holes = [rrect(c[0] - 0.65, c[1] - 2.45, c[0] + 0.65, c[1] + 0.7, 0.5),
                 circle(c[0], c[1] + 1.85, 0.72)]
    elif state == "config-error":
        t = [(14.25, 9.9), (18.4, 17.6), (10.1, 17.6)]
        t_gap = [(14.25, 7.4), (20.6, 19.0), (7.9, 19.0)]
        cuts = [tri(t_gap)]
        badge = [rrect_tri(t)]
        holes = [rrect(13.6, 12.4, 14.9, 15.3, 0.5), circle(14.25, 16.45, 0.7)]
    return g, cuts, badge, holes


def rrect_tri(t):
    return tri(t)


def tray_svg(state, color="#000000", view="0 0 18 18", size=18, extra=""):
    halves = ("right",) if state == "needs-layouts" else ("left", "right")
    gd = to_svg_d(glyph(halves=halves, **TRAY_GLYPH), TS, TDX, TDY)
    cut, badge, holes = tray_svg_parts(state)
    mask = ""
    if cut:
        mask = (f'<mask id="m" maskUnits="userSpaceOnUse" x="-2" y="-2" width="22" height="22">'
                f'<rect x="-2" y="-2" width="22" height="22" fill="#fff"/>{cut}</mask>')
    bmask = ""
    if holes:
        bmask = (f'<mask id="h" maskUnits="userSpaceOnUse" x="-2" y="-2" width="22" height="22">'
                 f'<rect x="-2" y="-2" width="22" height="22" fill="#fff"/>{holes}</mask>')
    body = f'<path d="{gd}" fill="{color}"' + (' mask="url(#m)"' if cut else "") + "/>"
    if badge:
        body += f'<g fill="{color}"' + (' mask="url(#h)"' if holes else "") + f">{badge}</g>"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="{view}"{extra}>'
            f"<defs>{mask}{bmask}</defs>{body}</svg>\n")


def tray_svg_parts(state):
    k = "#000"
    if state == "autocorrect-off":
        w = 1.6 + 2 * GAP
        cut = f'<line x1="1.2" y1="1.2" x2="16.8" y2="16.8" stroke="{k}" stroke-width="{w}" stroke-linecap="round"/>'
        badge = '<line x1="1.2" y1="1.2" x2="16.8" y2="16.8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>'
        return cut, badge.replace("currentColor", "inherit"), ""
    if state == "secure-input":
        cut = f'<rect x="9.25" y="8" width="9.25" height="10.5" rx="2.2" fill="{k}"/>'
        badge = ('<rect x="10.75" y="13" width="7" height="4.75" rx="1.2"/>'
                 '<path d="M12.25 13.2V12.5a2 2 0 0 1 4 0v0.7" fill="none" stroke-width="1.4" stroke="inherit"/>')
        holes = f'<circle cx="14.25" cy="15.3" r="0.75" fill="{k}"/>'
        return cut, badge, holes
    if state in ("needs-permissions", "needs-layouts"):
        cx, cy = BADGE_C if state == "needs-permissions" else LEFT_BADGE_C
        cut = f'<circle cx="{cx}" cy="{cy}" r="{BADGE_R + GAP}" fill="{k}"/>'
        badge = f'<circle cx="{cx}" cy="{cy}" r="{BADGE_R}"/>'
        holes = (f'<rect x="{fmt(cx - 0.65)}" y="{fmt(cy - 2.45)}" width="1.3" height="3.15" rx="0.5" fill="{k}"/>'
                 f'<circle cx="{cx}" cy="{fmt(cy + 1.85)}" r="0.72" fill="{k}"/>')
        return cut, badge, holes
    if state == "config-error":
        cut = f'<path d="M14.25 7.4L20.6 19H7.9Z" fill="{k}"/>'
        badge = '<path d="M14.25 9.9L18.4 17.6H10.1Z"/>'
        holes = (f'<rect x="13.6" y="12.4" width="1.3" height="2.9" rx="0.5" fill="{k}"/>'
                 f'<circle cx="14.25" cy="16.45" r="0.7" fill="{k}"/>')
        return cut, badge, holes
    return "", "", ""


STATES = ["working", "autocorrect-off", "secure-input", "needs-permissions", "needs-layouts", "config-error"]
ATTENTION = {"needs-permissions": "#F5A524", "needs-layouts": "#F5A524", "config-error": "#E5484D"}


def tray_layers(state, glyph_rgba, badge_rgba=None):
    g, cuts, badge, holes = tray_shapes(state)
    layers = [(minus(g, *cuts) if cuts else g, glyph_rgba)]
    if badge:
        layers.append((minus(union(*badge), *holes) if holes else union(*badge), badge_rgba or glyph_rgba))
    return layers


def build_tray():
    for st in STATES:
        # macOS: шаблон — только чёрный и альфа
        svg = tray_svg(st)
        svg = svg.replace('stroke="inherit"', 'stroke="#000000"')
        out(f"icons/tray/macos/{st}.svg", svg)
        layers = tray_layers(st, hexrgba("#000000"))
        write_png(f"icons/tray/macos/{st}.png", raster(layers, 18))
        write_png(f"icons/tray/macos/{st}@2x.png", raster(layers, 36))
        # Windows: светлая панель — тёмный знак, тёмная — белый; бейджи внимания цветные
        for theme, col in (("light", "#1C1C1C"), ("dark", "#FFFFFF")):
            badge = hexrgba(ATTENTION[st]) if st in ATTENTION else None
            layers = tray_layers(st, hexrgba(col), badge)
            for size in (16, 20, 24, 32):
                write_png(f"icons/tray/windows/{st}-{theme}-{size}.png", raster(layers, size))
        # Linux: символьный SVG 22×22 (перекрашивается темой) и цветные PNG
        sym = tray_svg(st, color="#2e3436", view="-1 -1 20 20", size=22)
        sym = sym.replace('stroke="inherit"', 'stroke="#2e3436"')
        out(f"icons/tray/linux/{st}-symbolic.svg", sym)
        layers = tray_layers(st, hexrgba("#4C66E6"), hexrgba(ATTENTION[st]) if st in ATTENTION else None)
        for size in (24, 48):
            write_png(f"icons/tray/linux/{st}-{size}.png", raster(layers, size))


# ---------------------------------------------------------------- иконки интерфейса 20×20, обводка 1.5
UI = {
    "section-general": '<path d="M3 6h7.5M14.5 6H17M3 14h2.5M9.5 14H17"/><circle cx="12.5" cy="6" r="2"/><circle cx="7.5" cy="14" r="2"/>',
    "section-hotkeys": '<rect x="2.5" y="4.75" width="15" height="10.5" rx="2"/><path d="M5.75 8h.5M9.75 8h.5M13.75 8h.5M7 12h6"/>',
    "section-apps": '<rect x="3" y="3" width="5.5" height="5.5" rx="1.25"/><rect x="11.5" y="3" width="5.5" height="5.5" rx="1.25"/><rect x="3" y="11.5" width="5.5" height="5.5" rx="1.25"/><circle cx="14.25" cy="14.25" r="3"/><path d="M12.5 14.25h3.5"/>',
    "section-words": '<path d="M3 5h14M3 10h8M3 15h5"/><circle cx="14.25" cy="14.25" r="3"/><path d="M12.5 14.25h3.5"/>',
    "section-permissions": '<path d="M10 2.75 15.75 5v4.6c0 3.55-2.45 6.15-5.75 7.65-3.3-1.5-5.75-4.1-5.75-7.65V5Z"/><path d="m7.6 9.9 1.65 1.65 3.2-3.3"/>',
    "section-about": '<circle cx="10" cy="10" r="7.25"/><path d="M10 9.25v4.5"/><circle cx="10" cy="6.6" r=".4" fill="currentColor"/>',
    "delete": '<path d="M3.75 5.75h12.5M8 5.75V4.25h4v1.5M5.5 5.75l.75 10.5h7.5l.75-10.5M8.5 9v4.25M11.5 9v4.25"/>',
    "add": '<path d="M10 4.25v11.5M4.25 10h11.5"/>',
    "more": '<circle cx="5" cy="10" r="1.25" fill="currentColor" stroke="none"/><circle cx="10" cy="10" r="1.25" fill="currentColor" stroke="none"/><circle cx="15" cy="10" r="1.25" fill="currentColor" stroke="none"/>',
    "status-granted": '<circle cx="10" cy="10" r="7.25"/><path d="m6.9 10.1 2.15 2.15 4.05-4.25"/>',
    "status-denied": '<circle cx="10" cy="10" r="7.25"/><path d="m7.4 7.4 5.2 5.2M12.6 7.4l-5.2 5.2"/>',
    "status-warning": '<path d="M10 3.25 17.25 16H2.75Z"/><path d="M10 8.25v3.5"/><circle cx="10" cy="13.85" r=".4" fill="currentColor"/>',
    "chevron-down": '<path d="m5.5 8 4.5 4.5L14.5 8"/>',
    "chevron-right": '<path d="m8 5.5 4.5 4.5L8 14.5"/>',
    "check": '<path d="m5 10.25 3.25 3.25L15 6.75"/>',
    "dash": '<path d="M5.5 10h9"/>',
    "external": '<path d="M11.5 3.75h4.75v4.75M16.25 3.75 9.5 10.5M14 11.75v3.5a1 1 0 0 1-1 1H4.75a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h3.5"/>',
    "restart": '<path d="M16.25 10A6.25 6.25 0 1 1 14.4 5.6"/><path d="M14.75 2.75v3.25H11.5"/>',
    "copy": '<rect x="7" y="7" width="9.25" height="9.25" rx="1.5"/><path d="M13 7V5.25a1.5 1.5 0 0 0-1.5-1.5H5.25a1.5 1.5 0 0 0-1.5 1.5v6.25A1.5 1.5 0 0 0 5.25 13H7"/>',
    "key-cmd": '<path d="M13 4.5v11a2.5 2.5 0 1 0 2.5-2.5h-11a2.5 2.5 0 1 0 2.5 2.5v-11a2.5 2.5 0 1 0-2.5 2.5h11a2.5 2.5 0 1 0-2.5-2.5"/>',
    "key-option": '<path d="M3 5.5h4.5l5 9H17M12 5.5h5"/>',
    "key-shift": '<path d="M10 3.25 17 10.5h-3.75v6H6.75v-6H3Z"/>',
    "key-control": '<path d="m5 11.5 5-5 5 5"/>',
    "key-win": '<rect x="3.5" y="3.5" width="13" height="13" rx="1"/><path d="M10 3.5v13M3.5 10h13"/>',
    "key-space": '<path d="M3.5 9v3.75h13V9"/>',
}


def build_ui():
    for name, body in UI.items():
        out(f"icons/ui/{name}.svg",
            '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20" fill="none" '
            'stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">'
            f"{body}</svg>\n")


# ---------------------------------------------------------------- иконка приложения
APP = dict(
    body=(100, 100, 824, 185.4), body_top="#4D68F0", body_bottom="#2B3DB5",
    lip=(222, 262, 580, 540, 116), lip_color="#1C2A8A",
    face=(222, 226, 580, 536, 116), face_top="#FFFFFF", face_bottom="#E6EBFF",
    glyph_color="#2B3DB5", glyph_h=340,
)


def app_glyph_d(cx, cy, h, s=14.0, b=54.0):
    k = h / 100
    return to_svg_d(glyph(s=s, b=b), k, cx - 50 * k, cy - 50 * k)


def build_app():
    x, y, w, r = APP["body"]
    lx, ly, lw, lh, lr = APP["lip"]
    fx, fy, fw, fh, fr = APP["face"]
    gcx, gcy = fx + fw / 2, fy + fh / 2
    gd = app_glyph_d(gcx, gcy, APP["glyph_h"])
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
<defs>
<linearGradient id="body" x1="0" y1="{y}" x2="0" y2="{y + w}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{APP["body_top"]}"/><stop offset="1" stop-color="{APP["body_bottom"]}"/></linearGradient>
<linearGradient id="face" x1="0" y1="{fy}" x2="0" y2="{fy + fh}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{APP["face_top"]}"/><stop offset="1" stop-color="{APP["face_bottom"]}"/></linearGradient>
</defs>
<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="{r}" fill="url(#body)"/>
<rect x="{lx}" y="{ly}" width="{lw}" height="{lh}" rx="{lr}" fill="{APP["lip_color"]}"/>
<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" rx="{fr}" fill="url(#face)"/>
<path d="{gd}" fill="{APP["glyph_color"]}"/>
</svg>
'''
    out("icons/app/concept-yar/icon.svg", svg)
    # упрощённый рисунок для 16–32 px: без клавиши, жирный белый знак на теле иконки
    sd = app_glyph_d(512, 512, 540, s=18, b=58)
    small = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="{r}" fill="{APP["body_bottom"]}"/>
<path d="{sd}" fill="#FFFFFF"/>
</svg>
'''
    out("icons/app/concept-yar/icon-small.svg", small)
    return {"glyph": gd, "small_glyph": sd}


if __name__ == "__main__":
    build_ui()
    geo = build_app()
    geo["tray_glyph_full"] = to_svg_d(glyph(**TRAY_GLYPH))
    geo["tray_glyph_right"] = to_svg_d(glyph(halves=("right",), **TRAY_GLYPH))
    geo["glyph_regular"] = to_svg_d(glyph())
    # трей «ЯR» — прежняя концепция; текущий трей строит keycap.py
    if "--yar-tray" in sys.argv:
        build_tray()
    out("_tools/geometry.json", json.dumps(geo, indent=1))
    print("ok")
