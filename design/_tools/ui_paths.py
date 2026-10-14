"""SVG иконок интерфейса → один контур на иконку (для предпросмотра в pen)."""
import json, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from icons import UI, fmt

def attrs(s):
    return dict(re.findall(r'([\w-]+)="([^"]*)"', s))

def rect(a):
    x, y, w, h = (float(a[k]) for k in ("x", "y", "width", "height"))
    r = float(a.get("rx", 0))
    if not r:
        return f"M{fmt(x)} {fmt(y)}h{fmt(w)}v{fmt(h)}h{fmt(-w)}Z"
    return (f"M{fmt(x + r)} {fmt(y)}h{fmt(w - 2*r)}a{fmt(r)} {fmt(r)} 0 0 1 {fmt(r)} {fmt(r)}v{fmt(h - 2*r)}"
            f"a{fmt(r)} {fmt(r)} 0 0 1 {fmt(-r)} {fmt(r)}h{fmt(-(w - 2*r))}a{fmt(r)} {fmt(r)} 0 0 1 {fmt(-r)} {fmt(-r)}"
            f"v{fmt(-(h - 2*r))}a{fmt(r)} {fmt(r)} 0 0 1 {fmt(r)} {fmt(-r)}Z")

def circle(a):
    cx, cy, r = (float(a[k]) for k in ("cx", "cy", "r"))
    if "fill" in a and a.get("stroke") == "none":  # закрашенная точка → кружок под обводкой 1.5
        r = max(r - 0.75, 0.01)
    elif "fill" in a:
        r = 0.01
    return f"M{fmt(cx - r)} {fmt(cy)}a{fmt(r)} {fmt(r)} 0 1 0 {fmt(2*r)} 0a{fmt(r)} {fmt(r)} 0 1 0 {fmt(-2*r)} 0Z"

res = {}
for name, body in UI.items():
    d = []
    for tag, a in re.findall(r"<(\w+)([^>]*)/>", body):
        a = attrs(a)
        d.append(re.sub(r"^m\s*([-\d.]+)[ ,]?([-\d.]+)\s*", r"M\1 \2l", a["d"]) if tag == "path" else rect(a) if tag == "rect" else circle(a))
    res[name] = "".join(d)
json.dump(res, open(os.path.join(os.path.dirname(__file__), "ui_paths.json"), "w"), indent=0)
print(json.dumps(res))
