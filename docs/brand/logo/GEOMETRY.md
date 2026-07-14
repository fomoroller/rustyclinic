# RustyClinic logo — shared geometry

| Constant | Value | Notes |
|----------|------:|-------|
| BLOCK | 134 | Module size |
| RX | 22–24 | Corner radius |
| GAP | 98 | Hollow cell |
| ROT | **3°** | Matches `logo.png` / `block-cross-03.jpg` (not 12°) |
| SLATE | `#1E293B` | Top, left, bottom only — **no right bridge** |
| ORANGE | `#C2410C` | Docking module / tittle |
| BG | `#F8F6F4` | Background |

## Mark

Winner: **exact** (IoU vs logo.png = 0.974)

Three identical slate rounded squares + orange module at 3° CW, docked into the open right cell.
No vertical slate strip between hollow and orange.

Bitmap lattice: top (189,74), left (73,189), bottom (189,306), orange center (371,255), rot 3°.

## Wordmark (`wordmark.svg`)

- Typeface: **Source Sans 3** (the repo's own `source-sans-3.woff2`), variable axis
  instanced at **wght 740**, letter-spacing −0.012 em — matches DESIGN.md display usage.
- Glyphs are shaped with HarfBuzz (kerning on) and exported as outlines, so the SVG
  needs no font at render time.
- The first "i" of *clinic* is a dotless ı; its tittle is the mark's **orange docking
  module** (same rx ratio 22/134, same 3° rotation) at 1.30× the natural dot height.
- The second "i" keeps its natural slate dot. **Rule: the orange tittle appears only
  when the mark is absent** — in the lockup, all i-dots are slate because the mark
  already carries the accent.
- Margins: 110 px optical ink margin left/right; text block vertically centered.

## Lockup (`lockup.svg`)

Mark at 0.703× (same rects as `logo.svg`) + wordmark text (wght 740, slate dots only),
baseline aligned so the text's ink midline sits on the mark's vertical center.
Left and right margins equal by construction.

Both files are generated — regenerate with the fontTools/uharfbuzz script rather than
hand-editing path data.
