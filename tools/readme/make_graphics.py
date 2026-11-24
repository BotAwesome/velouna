"""Builds the README graphics (section banners, load-time chart, highlight
icons) as SVG into docs/readme/. Icons come from the Phosphor font that
egui-phosphor ships; run again after changing texts or numbers:

    python tools/readme/make_graphics.py
"""
import glob
import os

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "docs", "readme")

BG, RAISE, SURFACE, BORDER = "#0f0d10", "#151216", "#1a171b", "#2a2429"
TEXT, TEXT2, TEXT3 = "#f4f1f3", "#b4adb2", "#7d757b"
ACCENT, ACCENT_HI, ACCENT_DEEP = "#d83f5c", "#e8566f", "#a02e44"
FONT = "'Segoe UI', -apple-system, BlinkMacSystemFont, 'Helvetica Neue', Arial, sans-serif"

ICONS = {
    "lightning": 0xE2DE, "sparkle": 0xE6A2, "images": 0xE836, "download": 0xE20C,
    "code": 0xE1BC, "tree": 0xE67C, "hand_heart": 0xE810, "scales": 0xE750,
    "play_circle": 0xE3D2, "folders": 0xE260, "television": 0xE754, "hard_drives": 0xE2A0,
    "waveform": 0xE802, "puzzle": 0xE596, "mobile": 0xE1E0, "globe": 0xE288,
    "shield": 0xE40C,
}

_font = None


def font():
    global _font
    if _font is None:
        cands = glob.glob(os.path.expanduser("~/.cargo/registry/src/*/egui-phosphor-*/res/Phosphor.ttf"))
        if not cands:
            raise SystemExit("Phosphor.ttf not found (build Velouna once so cargo fetches egui-phosphor)")
        _font = TTFont(sorted(cands)[-1])
    return _font


def icon(name, x, y, size, color):
    """Phosphor glyph as a <path>, top-left at (x, y), size x size."""
    f = font()
    glyph = f.getBestCmap()[ICONS[name]]
    upm = f["head"].unitsPerEm
    gs = f.getGlyphSet()
    pen = SVGPathPen(gs)
    s = size / upm
    # Font y goes up from the baseline; Phosphor glyphs sit in 0..upm (descent ~ -0).
    asc = f["hhea"].ascent
    gs[glyph].draw(TransformPen(pen, (s, 0, 0, -s, x, y + asc * s)))
    return f'<path d="{pen.getCommands()}" fill="{color}"/>'


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def write(name, w, h, body):
    os.makedirs(OUT, exist_ok=True)
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
           f'font-family="{FONT}">\n{body}\n</svg>\n')
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as fh:
        fh.write(svg)


def velvet(x0, h, opacity):
    """The logo's two strokes, repeated as a faint pattern on the right."""
    out = []
    for i in range(7):
        x = x0 + i * 46
        out.append(f'<path d="M{x} {-10}Q{x + 18} {h * 0.5} {x + 40} {h + 10}" stroke="{ACCENT}" stroke-width="10" '
                   f'fill="none" stroke-linecap="round" opacity="{opacity * (0.35 + i * 0.1):.3f}"/>')
    return "\n".join(out)


# ------------------------------------------------------------ banners

BANNERS = [
    ("blazing-fast", "lightning", "Blazing fast", "From double-click to the first frame in about half a second"),
    ("highlights", "sparkle", "Highlights", "Files, servers, TV and radio, visualizers and plugins in one player"),
    ("screenshots", "images", "Screenshots", "Windows and Android, generated demo videos"),
    ("install", "download", "Install", "Windows, Linux and Android, updates built in"),
    ("build", "code", "Build from source", "Rust, egui and libmpv"),
    ("layout", "tree", "Project layout", "Where to find what"),
    ("credits", "hand_heart", "Credits", "The projects Velouna stands on"),
    ("license", "scales", "License", "Free software under the GPL-3.0"),
]


def banner(slug, ic, title, sub):
    w, h = 880, 104
    body = f"""<defs>
  <radialGradient id="g" cx="0.08" cy="0.5" r="0.6">
    <stop offset="0" stop-color="{ACCENT}" stop-opacity="0.30"/>
    <stop offset="1" stop-color="{ACCENT}" stop-opacity="0"/>
  </radialGradient>
  <clipPath id="c"><rect width="{w}" height="{h}" rx="18"/></clipPath>
</defs>
<g clip-path="url(#c)">
  <rect width="{w}" height="{h}" fill="{RAISE}"/>
  <rect width="{w}" height="{h}" fill="url(#g)"/>
  {velvet(600, h, 0.30)}
</g>
<rect x="0.5" y="0.5" width="{w - 1}" height="{h - 1}" rx="18" fill="none" stroke="{BORDER}"/>
<rect x="28" y="24" width="56" height="56" rx="16" fill="{ACCENT}" fill-opacity="0.16" stroke="{ACCENT}" stroke-opacity="0.45"/>
{icon(ic, 40, 36, 32, ACCENT_HI)}
<text x="106" y="50" font-size="27" font-weight="600" fill="{TEXT}">{esc(title)}</text>
<text x="107" y="76" font-size="15" fill="{TEXT2}">{esc(sub)}</text>"""
    write(f"banner-{slug}.svg", w, h, body)


# ------------------------------------------------------------ load-time chart

ROWS = [
    # label, cold start, (warm min, warm max)
    ("Video, 1080p H.264", 0.53, (0.12, 0.56)),
    ("Video, 4K HEVC 10-bit", 0.60, (0.16, 0.59)),
    ("Music, MP3", 0.40, (0.07, 0.45)),
]


def chart():
    w, h = 880, 330
    left, right = 220, w - 40
    span = 1.0  # seconds on the axis
    sx = lambda s: left + (right - left) * s / span
    top = 84
    row_h = 66
    parts = [f'<rect width="{w}" height="{h}" rx="18" fill="{RAISE}"/>',
             f'<rect x="0.5" y="0.5" width="{w - 1}" height="{h - 1}" rx="18" fill="none" stroke="{BORDER}"/>',
             f'<text x="32" y="42" font-size="19" font-weight="600" fill="{TEXT}">Double-click to first frame</text>',
             # legend
             f'<rect x="{w - 352}" y="30" width="14" height="14" rx="4" fill="{ACCENT}"/>',
             f'<text x="{w - 330}" y="42" font-size="13.5" fill="{TEXT2}">not running yet</text>',
             f'<rect x="{w - 212}" y="30" width="14" height="14" rx="4" fill="{ACCENT}" fill-opacity="0.3" stroke="{ACCENT}" stroke-opacity="0.6"/>',
             f'<text x="{w - 190}" y="42" font-size="13.5" fill="{TEXT2}">already open</text>']
    axis_bottom = top + row_h * len(ROWS) - 8
    for t in [0, 0.25, 0.5, 0.75, 1.0]:
        x = sx(t)
        parts.append(f'<line x1="{x:.1f}" y1="{top - 12}" x2="{x:.1f}" y2="{axis_bottom}" stroke="{BORDER}" stroke-dasharray="{"0" if t == 0 else "3 4"}"/>')
        label = "0" if t == 0 else f"{t:g} s"
        parts.append(f'<text x="{x:.1f}" y="{axis_bottom + 20}" font-size="12.5" fill="{TEXT3}" text-anchor="middle">{label}</text>')
    for i, (label, cold, (wmin, wmax)) in enumerate(ROWS):
        y = top + i * row_h
        parts.append(f'<text x="32" y="{y + 26}" font-size="15" fill="{TEXT}">{esc(label)}</text>')
        # cold start
        parts.append(f'<rect x="{left}" y="{y + 6}" width="{sx(cold) - left:.1f}" height="20" rx="6" fill="{ACCENT}"/>')
        parts.append(f'<text x="{sx(cold) + 10:.1f}" y="{y + 21}" font-size="14" font-weight="600" fill="{TEXT}">{cold:.2f} s</text>'.replace("0.60 s", "0.6 s").replace("0.40 s", "0.4 s"))
        # warm range
        parts.append(f'<rect x="{sx(wmin):.1f}" y="{y + 32}" width="{sx(wmax) - sx(wmin):.1f}" height="12" rx="4" fill="{ACCENT}" fill-opacity="0.3" stroke="{ACCENT}" stroke-opacity="0.6"/>')
        parts.append(f'<text x="{sx(wmax) + 10:.1f}" y="{y + 43}" font-size="12.5" fill="{TEXT2}">{wmin:.2f} – {wmax:.2f} s</text>')
    parts.append(f'<text x="32" y="{h - 22}" font-size="12.5" fill="{TEXT3}">Release build · AMD Ryzen AI 7 350 with Radeon 860M graphics · NVMe SSD</text>')
    write("load-times.svg", w, h, "\n".join(parts))


# ------------------------------------------------------------ highlight icons

HIGHLIGHTS = {
    "playback": "play_circle", "library": "folders", "tv-radio": "television", "servers": "hard_drives",
    "music": "waveform", "plugins": "puzzle", "android": "mobile", "everywhere": "globe",
}


def highlight_icon(slug, ic):
    s = 44
    body = (f'<rect x="0.5" y="0.5" width="{s - 1}" height="{s - 1}" rx="12" fill="{ACCENT}" fill-opacity="0.14" '
            f'stroke="{ACCENT}" stroke-opacity="0.45"/>\n{icon(ic, 10, 10, 24, ACCENT)}')
    write(f"icon-{slug}.svg", s, s, body)


if __name__ == "__main__":
    for b in BANNERS:
        banner(*b)
    chart()
    for slug, ic in HIGHLIGHTS.items():
        highlight_icon(slug, ic)
    print("written to", OUT)
