"""Контраст пар токенов (WCAG 2.x) в обеих темах; читает tokens.json."""
import json, os, sys
T = json.load(open(os.path.join(os.path.dirname(__file__), "..", "tokens.json")))
def lum(h):
    h = h.lstrip("#")[:6]
    c = [int(h[i:i+2], 16) / 255 for i in (0, 2, 4)]
    c = [x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
def ratio(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)
PAIRS = [("text", "bg-window", 4.5), ("text", "bg-surface", 4.5), ("text", "bg-sidebar", 4.5), ("text", "bg-field", 4.5),
         ("text-2", "bg-window", 4.5), ("text-2", "bg-surface", 4.5), ("text-2", "bg-sidebar", 4.5),
         ("text-3", "bg-window", 3.0), ("text-3", "bg-surface", 3.0), ("text-3", "bg-field", 3.0),
         ("accent", "bg-surface", 4.5), ("accent", "bg-window", 4.5), ("accent-text", "accent", 4.5),
         ("accent-soft-text", "accent-soft", 4.5), ("error", "bg-surface", 4.5), ("error", "bg-window", 4.5),
         ("text", "error-bg", 4.5), ("warn", "warn-bg", 4.5), ("text", "warn-bg", 4.5), ("success", "success-bg", 4.5),
         ("error", "error-bg", 4.5), ("border-strong", "bg-surface", 1.5)]
bad = 0
for theme in ("light", "dark"):
    c = T["color"][theme]
    for a, b, need in PAIRS:
        r = ratio(c[a], c[b])
        flag = "" if r >= need else "  <-- ниже " + str(need)
        bad += bool(flag)
        print(f"{theme:5} {a:17} на {b:12} {r:5.2f}{flag}")
sys.exit(1 if bad else 0)
