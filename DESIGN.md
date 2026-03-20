# Design System — RustyClinic

## Product Context
- **What this is:** An offline-first EMR and health platform built for low-resource settings
- **Who it's for:** Nurses, pharmacists, lab technicians, CHWs, billing clerks, and district health officers in Rwanda and sub-Saharan Africa
- **Space/industry:** Healthcare / EMR — competing with OpenMRS, DHIS2, Bahmni, CommCare, and ClinikEHR
- **Project type:** Clinical web application (PWA) running on tablets, laptops, and Raspberry Pis

## Aesthetic Direction
- **Direction:** Industrial/Utilitarian with Warmth
- **Decoration level:** Minimal — typography and color do the work. No decorative illustrations, blobs, or gradients. Every pixel earns its place.
- **Mood:** The clarity of a well-organized pharmacy shelf, not the coldness of a corporate dashboard. Function-first, but warm enough that a nurse in rural Rwanda feels this was made for them — not adapted from a US hospital system.
- **Reference sites:** Bahmni (warm health palette), Linear (slate sidebar), DHIS2 (data density reference — we intentionally chose the opposite direction: spacious)

## Typography
- **Display/Hero:** Source Sans 3 (800 weight) — Excellent readability at all sizes, designed for UI, free, supports Latin and extended scripts. Not overused in health tech.
- **Body:** Source Sans 3 (400/500 weight) — Same family for consistency. Clean at 16px+ body size.
- **UI/Labels:** Source Sans 3 (600 weight, uppercase for section labels)
- **Data/Tables:** Source Sans 3 with `font-variant-numeric: tabular-nums` — Aligned columns for clinical data (patient IDs, vitals, timestamps).
- **Code:** JetBrains Mono — For admin CLI contexts and terminal output.
- **Loading:** Google Fonts CDN: `Source+Sans+3:wght@300;400;500;600;700;800` + `JetBrains+Mono:wght@400;500`. One font family = smallest possible download for low-bandwidth settings.
- **Scale:**
  - xs: 11px (rarely used — badge text only)
  - sm: 13px (captions, hints, metadata)
  - base: 16px (body text, form inputs — minimum for readability)
  - lg: 18px (emphasized body, card titles)
  - xl: 22px (page titles)
  - 2xl: 24px (subtitles)
  - 3xl: 32px (stat values)
  - 4xl: 48px (hero/display — used sparingly)
- **Line heights:** Body 1.6, headings 1.1-1.25, data/tables 1.4
- **Minimum body text:** 16px — never smaller for primary content. Semi-literate users and sunlight readability require this.

## Color

### Approach: Restrained — Neutral Primary + Warm Accent
The UI is calm and professional by default (warm slate). Burnt orange accent draws attention exactly where it matters — active states, CTAs, and important indicators. Most of the screen is neutral, so the accent carries maximum signal.

### Palette

#### Primary (Warm Slate)
| Token | Hex | Usage |
|-------|-----|-------|
| primary | `#334155` | Default buttons, sidebar text, secondary actions |
| primary-light | `#475569` | Hover states on primary elements |
| primary-dark | `#1E293B` | Sidebar background, pressed states |
| primary-50 | `#F8FAFC` | Hover backgrounds on table rows |
| primary-100 | `#F1F5F9` | Focus rings, subtle backgrounds |

#### Accent (Burnt Orange)
| Token | Hex | Usage |
|-------|-----|-------|
| accent | `#C2410C` | Primary CTAs, active nav, important actions |
| accent-light | `#EA580C` | Hover on accent buttons |
| accent-dark | `#9A3412` | Pressed state on accent buttons |

#### Neutrals (Stone Scale — warm grays)
| Token | Hex | Usage |
|-------|-----|-------|
| bg | `#FFFFFF` | Card backgrounds, input backgrounds |
| bg-surface | `#F8F6F4` | Page background |
| bg-muted | `#F1EFEC` | Disabled backgrounds, subtle fills |
| border | `#E0DDD9` | Primary borders |
| border-subtle | `#EBE9E6` | Card borders, table row dividers |
| text | `#1C1917` | Primary text |
| text-secondary | `#57534E` | Secondary text, descriptions |
| text-muted | `#A8A29E` | Placeholder text, metadata |

#### Semantic
| Token | Hex | Background | Usage |
|-------|-----|------------|-------|
| success | `#15803D` | `#F0FDF4` | Successful registration, synced status, dispensed |
| warning | `#B45309` | `#FFFBEB` | Credential expiring, stale sync, drug interaction caution |
| error | `#B91C1C` | `#FEF2F2` | Failed submission, drug interaction alert, validation errors |
| info | `#0369A1` | `#F0F9FF` | Pending lab results, system notifications |

#### Dark Mode Strategy
- Surfaces use elevation (darker = lower, lighter = higher) — not lightness inversion
- Text is off-white (`#F5F5F4`), not pure white
- Accent desaturated slightly: `#EA580C` in dark mode
- Primary becomes light slate: `#94A3B8`
- Sidebar deepens to `#0F172A`
- `color-scheme: dark` on html element

### Color Usage Rules
- No color-only encoding — always pair color with icon, label, or pattern
- No red/green only combinations (8% of men have red-green color deficiency)
- Semantic colors are consistent across the entire app: green = success, red = error, amber = warning, blue = info
- The accent (burnt orange) is reserved for interactive elements — never used for decoration

## Spacing
- **Base unit:** 8px
- **Density:** Comfortable — generous padding for touch targets in clinical environments
- **Scale:** 2xs(2px) xs(4px) sm(8px) md(16px) lg(24px) xl(32px) 2xl(48px) 3xl(64px)
- **Touch targets:** Minimum 48px height for all interactive elements (buttons, inputs, links, nav items). This exceeds the WCAG 44px minimum and accounts for use on tablets with imprecise touch.
- **Section spacing:** 24px between related groups, 48px between distinct sections
- **Card padding:** 20px (comfortable for content density)
- **Table row height:** Minimum 48px (touch-friendly rows)

## Layout
- **Approach:** Grid-disciplined
- **Grid:** Sidebar (240px fixed) + content area (fluid). Content uses 12-column grid at desktop.
- **Max content width:** 1200px on desktop, full-width on tablet
- **Border radius:**
  - sm: 4px (inputs, small elements)
  - md: 8px (buttons, cards, dropdowns)
  - lg: 12px (modal dialogs, large cards)
  - full: 9999px (badges, pills, avatars)
- **Sidebar:** Dark background (`#1E293B`) with icon + text nav items. Active item highlighted with accent color. Always visible on desktop/tablet, hidden behind hamburger on mobile.
- **Forms:** Full-width within their container. Two-column layout for related fields (name + surname, date + sex). Single column on mobile.
- **Data tables:** Full-width, hover highlight on rows, sortable columns where applicable.

## Motion
- **Approach:** Minimal-functional — only transitions that aid comprehension
- **Easing:** enter(ease-out) exit(ease-in) move(ease-in-out)
- **Duration:**
  - micro: 50-100ms (button press, toggle switch)
  - short: 150ms (hover states, focus rings)
  - medium: 250ms (dropdown open, sidebar collapse)
  - long: 400ms (modal entry/exit, page transitions)
- **`prefers-reduced-motion`:** Respected. All transitions disabled when reduced motion is preferred.
- **Rules:**
  - No `transition: all` — properties listed explicitly
  - Only `transform` and `opacity` animated — never layout properties (width, height, top, left)
  - No decorative animation. Every motion communicates a state change.
  - No animation on page load — content appears immediately

## Component Patterns

### Status Badges
Queue states use pill badges with semantic dot + label:
- Waiting: amber background + dot
- Called: blue background + dot
- In Service: green background + dot
- Completed: gray background + dot

### Alerts
Always include: semantic color + icon + specific message + action when applicable.
Never: generic "An error occurred" without context.

### Empty States
Warm message + icon + primary action button. Never just "No items." — explain what will appear and how to create it.

### Forms
- Labels above inputs (not placeholder-only)
- Hint text below label in muted color for complex fields
- Validation errors appear below the input with error color
- 16px minimum font size on all inputs (prevents iOS zoom on focus)

## Anti-Patterns (never use)
- Purple/violet gradients
- 3-column feature grid with icons in colored circles
- Centered everything with uniform spacing
- Uniform bubbly border-radius on all elements
- Decorative blobs, floating circles, wavy SVG dividers
- Emoji as design elements
- Colored left-border cards
- Generic hero copy ("Welcome to...", "Unlock the power of...")

## Accessibility
- WCAG AA minimum: body text 4.5:1 contrast, large text 3:1, UI components 3:1
- Touch targets: 48px minimum (exceeds 44px WCAG standard)
- Focus visible: `focus-visible` ring on all interactive elements, never `outline: none`
- No color-only encoding
- Keyboard navigable: all workflows must be completable without a mouse
- Screen reader: semantic HTML, ARIA labels on icon-only buttons
- Sunlight readability: high-contrast neutrals chosen specifically for outdoor use on tablets

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-03-20 | Initial design system created | Created by /design-consultation based on competitive research across OpenMRS, DHIS2, Bahmni, CommCare, Streamline, and ClinikEHR |
| 2026-03-20 | Teal primary rejected | User found teal (#0D7377) unappealing. Replaced with warm slate + burnt orange. |
| 2026-03-20 | Warm Slate + Burnt Orange chosen | Neutral primary (#334155) with warm accent (#C2410C). Most distinctive in the health EMR space — no competitor uses this combination. Calm UI with intentional warmth. |
| 2026-03-20 | Single font family (Source Sans 3) | One font = fastest load on 2G connections. Weight variation provides hierarchy without additional font downloads. |
| 2026-03-20 | 48px minimum touch targets | Exceeds WCAG 44px. Accounts for shared devices, imprecise touch in clinical settings, and outdoor use. |
| 2026-03-20 | Deliberately spacious layout | Counter to competitor density. Readability over information density — users in this context need clarity, not dashboards crammed with data. |
