#!/usr/bin/env python3
"""Generate wordmark.svg and lockup.svg for RustyClinic from the repo's
Source Sans 3 variable woff2, instanced at wght=740 (per brand.html worksheet).
"""
import io, math
import uharfbuzz as hb
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.boundsPen import BoundsPen
from fontTools.misc.transform import Transform

HERE = __import__("pathlib").Path(__file__).resolve().parent
WOFF2 = str(HERE / "../../../crates/rustyclinic-web/static/fonts/source-sans-3.woff2")
OUT_DIR = str(HERE)
SLATE, ORANGE, BG = "#1E293B", "#C2410C", "#F8F6F4"
WGHT = 740
LS_EM = -0.012            # letter-spacing from brand.html
RX_RATIO = 22 / 134       # corner radius ratio of the mark's modules
ROT = 3                   # docking-module rotation, matches logo.svg

font = TTFont(WOFF2)
instantiateVariableFont(font, {"wght": WGHT}, inplace=True)
upem = font["head"].unitsPerEm
xheight = font["OS/2"].sxHeight

buf_ttf = io.BytesIO()
font.flavor = None
font.save(buf_ttf)

hbface = hb.Face(buf_ttf.getvalue())
hbfont = hb.Font(hbface)
glyph_order = font.getGlyphOrder()
glyphset = font.getGlyphSet()


def shape(text):
    """Return [(glyphname, x_fontunits)] and total advance, letter-spaced."""
    buf = hb.Buffer()
    buf.add_str(text)
    buf.guess_segment_properties()
    hb.shape(hbfont, buf, {"kern": True, "liga": True})
    ls = LS_EM * upem
    x = 0.0
    out = []
    infos, poss = buf.glyph_infos, buf.glyph_positions
    for i, (gi, gp) in enumerate(zip(infos, poss)):
        out.append((glyph_order[gi.codepoint], x + gp.x_offset))
        x += gp.x_advance + (ls if i < len(infos) - 1 else 0)
    return out, x


def contours_of(gname):
    rec = DecomposingRecordingPen(glyphset)
    glyphset[gname].draw(rec)
    conts, cur = [], []
    for op, args in rec.value:
        cur.append((op, args))
        if op in ("closePath", "endPath"):
            conts.append(cur)
            cur = []
    if cur:
        conts.append(cur)
    return conts


def contour_bbox(cont):
    xs, ys = [], []
    for _, args in cont:
        for pt in args:
            if pt is not None:
                xs.append(pt[0]); ys.append(pt[1])
    return min(xs), min(ys), max(xs), max(ys)


def glyph_bbox(gname):
    bp = BoundsPen(glyphset)
    glyphset[gname].draw(bp)
    return bp.bounds


# natural i-dot metrics (font units)
dot = next(c for c in contours_of("i") if contour_bbox(c)[1] > xheight * 0.7)
dxmin, dymin, dxmax, dymax = contour_bbox(dot)
dot_h = dymax - dymin
dot_gap = dymin - xheight        # gap between x-height top and dot bottom


def path_for(gname, penx, baseline, s):
    spen = SVGPathPen(glyphset, ntos=lambda v: f"{v:.1f}")
    t = Transform(s, 0, 0, -s, penx, baseline)
    glyphset[gname].draw(TransformPen(spen, t))
    return spen.getCommands()


def text_paths(shaped, x0, baseline, s):
    ds = [path_for(g, x0 + gx * s, baseline, s) for g, gx in shaped]
    return " ".join(ds)


def ink_bbox_text(shaped, x0, baseline, s):
    xs, ys = [], []
    for g, gx in shaped:
        b = glyph_bbox(g)
        if b is None:
            continue
        xs += [x0 + (gx + b[0]) * s, x0 + (gx + b[2]) * s]
        ys += [baseline - b[3] * s, baseline - b[1] * s]
    return min(xs), min(ys), max(xs), max(ys)


def tittle_svg(cx, cy, side):
    rx = side * RX_RATIO
    return (f'  <g transform="translate({cx:.1f} {cy:.1f}) rotate({ROT})">\n'
            f'    <rect x="{-side/2:.1f}" y="{-side/2:.1f}" width="{side:.1f}" '
            f'height="{side:.1f}" rx="{rx:.1f}" fill="{ORANGE}"/>\n  </g>')


# ---------------- wordmark ----------------
FS = 230.0
S = FS / upem
MARGIN = 110.0
CANVAS_H = 420

shaped_wm, adv = shape("rustyclınic")   # dotless first i of "clinic"
# tittle geometry (svg px)
side = dot_h * S * 1.30
gap = dot_gap * S
ib = glyph_bbox("dotlessi")
dotless = next((g, gx) for g, gx in shaped_wm if g in ("dotlessi", "uni0131", "idotless"))

# provisional layout at x0=0, baseline=0 to measure
x0, base = 0.0, 0.0
tb = ink_bbox_text(shaped_wm, x0, base, S)
stem_cx = x0 + (dotless[1] + (ib[0] + ib[2]) / 2) * S
t_cy = base - xheight * S - gap - side / 2
ink = (min(tb[0], stem_cx - side / 2 - 4), min(tb[1], t_cy - side / 2 - 4),
       max(tb[2], stem_cx + side / 2 + 4), tb[3])
ink_w, ink_h = ink[2] - ink[0], ink[3] - ink[1]
canvas_w = round(ink_w + 2 * MARGIN)
dx = MARGIN - ink[0]
dy = (CANVAS_H - ink_h) / 2 - ink[1]

d = text_paths(shaped_wm, x0 + dx, base + dy, S)
tit = tittle_svg(stem_cx + dx, t_cy + dy, side)
wordmark = f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {canvas_w} {CANVAS_H}" width="{canvas_w}" height="{CANVAS_H}" role="img" aria-label="rustyclinic">
  <rect width="{canvas_w}" height="{CANVAS_H}" fill="{BG}"/>
  <path d="{d}" fill="{SLATE}"/>
{tit}
</svg>
'''
open(f"{OUT_DIR}/wordmark.svg", "w").write(wordmark)
print(f"wordmark: canvas {canvas_w}x{CANVAS_H}, ink {ink_w:.0f}x{ink_h:.0f}, "
      f"baseline {base+dy:.1f}, tittle side {side:.1f}, natural dot {dot_h*S:.1f}")

# ---------------- lockup ----------------
# mark: same geometry as logo.svg, scaled
MSCALE = 0.703125
MX, MY = 70.0, 40.0
CANVAS_H_L = 440
mark_ink_x0, mark_ink_x1 = 73, 440.6   # mark ink bounds in logo.svg coords
mark_ink_y0, mark_ink_y1 = 74, 440
m_left = MX + mark_ink_x0 * MSCALE
m_cy = MY + (mark_ink_y0 + mark_ink_y1) / 2 * MSCALE
GAP_MT = 115.0

FSL = 240.0
SL = FSL / upem
shaped_lk, advl = shape("rustyclınic")       # dotless first i, orange tittle like the wordmark
tbl = ink_bbox_text(shaped_lk, 0.0, 0.0, SL)
text_x = MX + mark_ink_x1 * MSCALE + GAP_MT - tbl[0]
ink_cy = (tbl[1] + tbl[3]) / 2
baseline_l = m_cy - ink_cy
dl = text_paths(shaped_lk, text_x, baseline_l, SL)
text_right = text_x + tbl[2]
canvas_w_l = round(text_right + m_left)      # right margin == left margin

side_l = dot_h * SL * 1.30
gap_l = dot_gap * SL
dotless_l = next((g, gx) for g, gx in shaped_lk if g in ("dotlessi", "uni0131", "idotless"))
stem_cx_l = text_x + (dotless_l[1] + (ib[0] + ib[2]) / 2) * SL
t_cy_l = baseline_l - xheight * SL - gap_l - side_l / 2
tittle_l = tittle_svg(stem_cx_l, t_cy_l, side_l)

mark_rects = f'''  <g transform="translate({MX} {MY}) scale({MSCALE})">
    <rect x="189" y="74" width="134" height="134" rx="22" fill="{SLATE}"/>
    <rect x="73" y="189" width="134" height="134" rx="22" fill="{SLATE}"/>
    <rect x="189" y="306" width="134" height="134" rx="22" fill="{SLATE}"/>
    <g transform="translate(371 255) rotate({ROT})">
      <rect x="-67" y="-67" width="134" height="134" rx="22" fill="{ORANGE}"/>
    </g>
  </g>'''
lockup = f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {canvas_w_l} {CANVAS_H_L}" width="{canvas_w_l}" height="{CANVAS_H_L}" role="img" aria-label="RustyClinic">
  <rect width="{canvas_w_l}" height="{CANVAS_H_L}" fill="{BG}"/>
{mark_rects}
  <path d="{dl}" fill="{SLATE}"/>
{tittle_l}
</svg>
'''
open(f"{OUT_DIR}/lockup.svg", "w").write(lockup)
print(f"lockup: canvas {canvas_w_l}x{CANVAS_H_L}, baseline {baseline_l:.1f}, "
      f"text x {text_x:.1f}..{text_right:.1f}, mark cy {m_cy:.1f}")
