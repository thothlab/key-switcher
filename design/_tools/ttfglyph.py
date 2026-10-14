"""Контуры глифов из статичного TrueType-шрифта (glyf, cmap 4) — без fontTools.

glyph(path, char) → (contours, advance, units_per_em, cap_height): контур — список точек (x, y, on_curve)
в единицах шрифта, ось y вверх. Составные глифы не поддерживаются.
"""
import struct


class Font:
    def __init__(self, path):
        self.d = open(path, "rb").read()
        n = struct.unpack(">H", self.d[4:6])[0]
        self.t = {}
        for i in range(n):
            tag, _, off, ln = struct.unpack(">4sIII", self.d[12 + 16 * i:28 + 16 * i])
            self.t[tag.decode()] = (off, ln)
        h = self.t["head"][0]
        self.upm = struct.unpack(">H", self.d[h + 18:h + 20])[0]
        self.loc_long = struct.unpack(">h", self.d[h + 50:h + 52])[0] == 1
        o = self.t.get("OS/2")
        self.cap = None
        if o and o[1] >= 90:
            self.cap = struct.unpack(">h", self.d[o[0] + 88:o[0] + 90])[0]
        self.cmap = self._cmap()

    def _cmap(self):
        c = self.t["cmap"][0]
        n = struct.unpack(">H", self.d[c + 2:c + 4])[0]
        for i in range(n):
            pid, eid, off = struct.unpack(">HHI", self.d[c + 4 + 8 * i:c + 12 + 8 * i])
            s = c + off
            if struct.unpack(">H", self.d[s:s + 2])[0] == 4 and pid in (0, 3):
                seg = struct.unpack(">H", self.d[s + 6:s + 8])[0] // 2
                arr = lambda k: struct.unpack(f">{seg}H", self.d[s + 14 + k * 2 * seg + (2 if k >= 1 else 0):s + 14 + (k + 1) * 2 * seg + (2 if k >= 1 else 0)])
                ends, starts = arr(0), arr(1)
                deltas = struct.unpack(f">{seg}h", self.d[s + 16 + 4 * seg:s + 16 + 6 * seg])
                ro_off = s + 16 + 6 * seg
                ranges = struct.unpack(f">{seg}H", self.d[ro_off:ro_off + 2 * seg])
                m = {}
                for k in range(seg):
                    for cp in range(starts[k], ends[k] + 1):
                        if cp == 0xFFFF:
                            continue
                        if ranges[k] == 0:
                            g = (cp + deltas[k]) & 0xFFFF
                        else:
                            a = ro_off + 2 * k + ranges[k] + 2 * (cp - starts[k])
                            g = struct.unpack(">H", self.d[a:a + 2])[0]
                            g = (g + deltas[k]) & 0xFFFF if g else 0
                        m[cp] = g
                return m
        raise ValueError("нет cmap формата 4")

    def _loca(self, gid):
        l = self.t["loca"][0]
        if self.loc_long:
            return struct.unpack(">II", self.d[l + 4 * gid:l + 4 * gid + 8])
        a, b = struct.unpack(">HH", self.d[l + 2 * gid:l + 2 * gid + 4])
        return a * 2, b * 2

    def advance(self, gid):
        hh = self.t["hhea"][0]
        nm = struct.unpack(">H", self.d[hh + 34:hh + 36])[0]
        hm = self.t["hmtx"][0]
        i = min(gid, nm - 1)
        return struct.unpack(">H", self.d[hm + 4 * i:hm + 4 * i + 2])[0]

    def glyph(self, ch):
        gid = self.cmap[ord(ch)]
        a, b = self._loca(gid)
        g = self.t["glyf"][0] + a
        nc = struct.unpack(">h", self.d[g:g + 2])[0]
        if nc < 0:
            raise ValueError(f"составной глиф {ch!r}")
        ends = struct.unpack(f">{nc}H", self.d[g + 10:g + 10 + 2 * nc])
        npts = ends[-1] + 1
        p = g + 10 + 2 * nc
        il = struct.unpack(">H", self.d[p:p + 2])[0]
        p += 2 + il
        flags = []
        while len(flags) < npts:
            f = self.d[p]; p += 1
            flags.append(f)
            if f & 8:
                r = self.d[p]; p += 1
                flags += [f] * r

        def coords(short_bit, same_bit):
            nonlocal p
            v, out = 0, []
            for f in flags:
                if f & short_bit:
                    dv = self.d[p]; p += 1
                    v += dv if f & same_bit else -dv
                elif not f & same_bit:
                    v += struct.unpack(">h", self.d[p:p + 2])[0]; p += 2
                out.append(v)
            return out
        xs = coords(2, 16)
        ys = coords(4, 32)
        contours, s = [], 0
        for e in ends:
            contours.append([(xs[i], ys[i], bool(flags[i] & 1)) for i in range(s, e + 1)])
            s = e + 1
        return contours, self.advance(gid)


def segments(contour):
    """Квадратичный контур TrueType → список (start, [("L", p) | ("Q", c, p)])."""
    pts = contour
    n = len(pts)
    # начать с точки на кривой
    k = next((i for i, q in enumerate(pts) if q[2]), None)
    if k is None:
        a, b = pts[0], pts[1]
        start = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
        seq = pts[1:] + pts[:1]
    else:
        start = pts[k][:2]
        seq = pts[k + 1:] + pts[:k + 1]
    out, ctrl = [], None
    for x, y, on in seq:
        if on:
            out.append(("Q", ctrl, (x, y)) if ctrl else ("L", (x, y)))
            ctrl = None
        else:
            if ctrl:
                mid = ((ctrl[0] + x) / 2, (ctrl[1] + y) / 2)
                out.append(("Q", ctrl, mid))
            ctrl = (x, y)
    if ctrl:
        out.append(("Q", ctrl, start))
    return start, out


def to_svg(contours, tf, fmt):
    """tf(x, y) → (X, Y) в координатах рисунка."""
    d = []
    for c in contours:
        start, segs = segments(c)
        X, Y = tf(*start)
        d.append(f"M{fmt(X)} {fmt(Y)}")
        for s in segs:
            if s[0] == "L":
                X, Y = tf(*s[1]); d.append(f"L{fmt(X)} {fmt(Y)}")
            else:
                cx, cy = tf(*s[1]); X, Y = tf(*s[2])
                d.append(f"Q{fmt(cx)} {fmt(cy)} {fmt(X)} {fmt(Y)}")
        d.append("Z")
    return "".join(d)


def to_polys(contours, tf, steps=8):
    polys = []
    for c in contours:
        start, segs = segments(c)
        cur = start
        poly = [tf(*start)]
        for s in segs:
            if s[0] == "L":
                cur = s[1]; poly.append(tf(*cur))
            else:
                (cx, cy), (ex, ey) = s[1], s[2]
                for i in range(1, steps + 1):
                    t = i / steps
                    x = (1 - t) ** 2 * cur[0] + 2 * (1 - t) * t * cx + t * t * ex
                    y = (1 - t) ** 2 * cur[1] + 2 * (1 - t) * t * cy + t * t * ey
                    poly.append(tf(x, y))
                cur = (ex, ey)
        polys.append(poly)
    return polys


def bbox(contours):
    xs = [p[0] for c in contours for p in c]
    ys = [p[1] for c in contours for p in c]
    return min(xs), min(ys), max(xs), max(ys)
