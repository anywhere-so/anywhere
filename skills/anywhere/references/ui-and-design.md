# Web UI, UX and interface craft

Read this guide whenever building, styling, animating or reviewing a web app or static site on Anywhere. People open and share Anywhere apps on phones as often as on laptops, so every interface must feel mobile-ready, tactile, accessible and visually intentional on the first deploy.

Do not rely on generic defaults (`#f3f4f6` gray boxes, `1px solid #e5e7eb` around everything, four equal metric tiles, purple gradients or emoji icons). Start from the concrete CSS tokens, control recipes, Good/Bad patterns and critique rules below.

---

## 1. Anti-slop rules: what makes an interface look crafted vs. generated

Before writing layout or styles, avoid the failure modes that make AI-built web apps look generic:

| Fault | Bad (Generated tell) | Good (Crafted product) |
| --- | --- | --- |
| **4-equal-stat-cards template** | Opening every app with a 4-column grid of identical bordered metric cards (`Total`, `Active`, `Pending`, `Rate`) that compete with the workspace | Lead with the primary workspace (the list, editor, board or tool). If metrics matter, give **one** headline number visual weight or place compact inline stats inside the header bar |
| **Purple / neon glow slop** | Violet-to-pink gradients, gradient headings, colored `box-shadow` glows, or tinted glass plates over dark backgrounds | Warm neutral surfaces (`#f6f5f4` / `#ffffff` light, `#111113` / `#1a1a1d` dark), crisp neutral shadows, and **one** semantic accent color spent only on the active control or primary action |
| **Fake focus ring on hero cards** | Wrapping a primary card in a `1px` or `2px` accent border with a soft halo just to say "look here" | Elevate the primary surface with background contrast (`--surface` over `--canvas`), `--shadow-float`, or a small accent badge—save rings for actual `:focus-visible` states |
| **Tracked ALL-CAPS everywhere** | `text-transform: uppercase; letter-spacing: 0.08em` on every card title, table header, badge and button | **Sentence case** by default. Create hierarchy with whitespace, size (`13–14px` primary vs. `11–12px` metadata) and one font-weight step (`500` or `600`) |
| **Emoji or hand-drawn icons** | Using emoji (`✨`, `🚀`, `📁`, `⚡`) as UI icons, drawing custom multi-path SVGs from memory, or mixing thin outline icons with heavy filled ones | One icon set throughout (Lucide, Phosphor, Tabler or Radix), one stroke width (`1.5px` or `1.75px`), one optical size (`14–16px`), `flex: none` so icons never squish |
| **AI sparkle clichés** | Decorating AI or smart features with sparkles, magic wands, stars or purple gradients | Show what the feature actually does with a clear action label and domain icon |
| **Invented square logo** | Putting the app's first letter or a generic glyph inside a colored rounded square in the header | Use a clean typographic wordmark in `--text-strong` (or the user's real logo asset) |
| **Onion-skin translucent icons** | Coloring SVG icons with `rgba()` or translucent text tokens, which paints double-dark spots wherever strokes cross or join | Color icons with **opaque** tokens (`--icon: #707074` in light, `#9a9aa0` in dark); if fading an icon, put `opacity` on the `<svg>` element itself |
| **Over-stuffed list rows** | Packing an ID, hash, avatar, branch, timestamp, status pill and 3 action buttons onto every row with `justify-content: space-between` | Keep rows calm (3–4 aligned columns on a shared CSS Grid template). Group title + subtitle tightly (`4–6px` gap) inside `10–12px` row padding; move secondary metadata to a detail drawer or popover |
| **Near-tangents (`1–3px` gaps)** | Two borders, cards or pills sitting `1–3px` apart, or text sitting right against a divider | Either align edges on the exact same line (`0px` shared border) or separate them by at least `8px` (`12px+` inset for text inside cards/panels) |

---

## 2. Drop-in CSS token system (light & dark)

Instead of inventing ad-hoc hex values, start your stylesheet with this tuned token foundation. It pairs a warm paper-neutral light mode with a stepped-surface dark mode (where depth comes from lighter surfaces `--canvas` → `--surface` → `--raised`, not muddy drop shadows):

```css
:root {
  color-scheme: light dark;

  /* Surfaces (Light: warm canvas, crisp white cards) */
  --canvas: #f6f5f4;
  --surface: #ffffff;
  --raised: #ffffff;
  --chrome: #fbfbfa;

  /* Subtle neutral fills (hover, active, skeleton, subtle pills) */
  --ink-1: rgba(0, 0, 0, 0.035);
  --ink-2: rgba(0, 0, 0, 0.055);
  --ink-3: rgba(0, 0, 0, 0.08);
  --ink-4: rgba(0, 0, 0, 0.11);
  --ink-5: rgba(0, 0, 0, 0.16);
  /* Opaque equivalent of --ink-3 for overlapping items (avatars, stacked chips) */
  --fill: #ebebea;

  /* Hairlines */
  --line: rgba(0, 0, 0, 0.065);
  --line-faint: rgba(0, 0, 0, 0.04);

  /* Text & Opaque Icons (never use rgba on SVG strokes) */
  --text-strong: #1f1f22;
  --text: #65656a;
  --text-faint: #929298;
  --icon: #6e6e73;
  --icon-faint: #bcbcc2;

  /* Semantic accent + auto-darkened text for soft pills/links */
  --accent: #2563eb;
  --accent-hover: color-mix(in srgb, var(--accent) 88%, #000);
  --accent-soft: rgba(37, 99, 235, 0.11);
  --accent-text: color-mix(in srgb, var(--accent) 82%, #000);

  /* Status tokens (soft pill fill + high-contrast label) */
  --success: #16a34a;
  --success-soft: rgba(22, 163, 74, 0.11);
  --success-text: #15803d;
  --warning: #d97706;
  --warning-soft: rgba(217, 119, 6, 0.13);
  --warning-text: #b45309;
  --danger: #dc2626;
  --danger-soft: rgba(220, 38, 38, 0.1);
  --danger-text: #b91c1c;

  /* Shadows over borders for card & popover elevation */
  --shadow-card:
    0 0 0 1px rgba(0, 0, 0, 0.06),
    0 1px 2px -1px rgba(0, 0, 0, 0.06),
    0 2px 4px 0 rgba(0, 0, 0, 0.04);
  --shadow-card-hover:
    0 0 0 1px rgba(0, 0, 0, 0.09),
    0 2px 4px -1px rgba(0, 0, 0, 0.07),
    0 4px 8px 0 rgba(0, 0, 0, 0.05);
  --shadow-float:
    0 0 0 1px rgba(0, 0, 0, 0.08),
    0 4px 16px rgba(0, 0, 0, 0.1),
    0 12px 32px rgba(0, 0, 0, 0.06);

  /* Concentric radius scale (outerRadius = innerRadius + padding) */
  --radius-sm: 6px;     /* menu items, segmented thumb, small badges */
  --radius-md: 8px;     /* inputs, buttons, segmented track */
  --radius-lg: 12px;    /* inner panels, popovers, toasts */
  --radius-card: 16px;  /* outer cards (8px padding + 8px inner radius) */
  --radius-pill: 999px;

  /* Control density */
  --control-md: 36px;   /* desktop buttons & inputs (use 40-44px on touch) */
  --row-sm: 28px;       /* compact menu / segment item */

  --font-sans: system-ui, -apple-system, BlinkMacSystemFont, "Inter", sans-serif;
  --font-mono: "SF Mono", "SFMono-Regular", ui-monospace, Menlo, Consolas, monospace;
}

@media (prefers-color-scheme: dark) {
  :root {
    /* On dark, depth comes from stepping up surface lightness, not drop shadows */
    --canvas: #111113;
    --surface: #18181b;
    --raised: #232328;
    --chrome: #151518;

    --ink-1: rgba(255, 255, 255, 0.045);
    --ink-2: rgba(255, 255, 255, 0.07);
    --ink-3: rgba(255, 255, 255, 0.1);
    --ink-4: rgba(255, 255, 255, 0.14);
    --ink-5: rgba(255, 255, 255, 0.2);
    --fill: #2e2e33;

    --line: rgba(255, 255, 255, 0.08);
    --line-faint: rgba(255, 255, 255, 0.05);

    --text-strong: #ededf0;
    --text: #9d9da4;
    --text-faint: #6b6b72;
    --icon: #9898a0;
    --icon-faint: #4d4d54;

    --accent: #3b82f6;
    --accent-hover: color-mix(in srgb, var(--accent) 88%, #fff);
    --accent-soft: rgba(59, 130, 246, 0.18);
    --accent-text: color-mix(in srgb, var(--accent) 72%, #fff);

    --success-soft: rgba(34, 197, 94, 0.16);
    --success-text: #4ade80;
    --warning-soft: rgba(245, 158, 11, 0.18);
    --warning-text: #fbbf24;
    --danger-soft: rgba(239, 68, 68, 0.16);
    --danger-text: #f87171;

    --shadow-card: 0 0 0 1px rgba(255, 255, 255, 0.075);
    --shadow-card-hover: 0 0 0 1px rgba(255, 255, 255, 0.13);
    --shadow-float:
      0 0 0 1px rgba(255, 255, 255, 0.1),
      0 8px 24px rgba(0, 0, 0, 0.55);
  }
}
```

---

## 3. Copy-pasteable CSS control recipes & surface physics

### A. Inputs & fields with zero-shift focus rings

Use `inset` `box-shadow` instead of changing `border-width` so focusing or showing an error never shifts text by `1px`. Keep `font-size: 16px` (`1rem`) on mobile so iOS Safari never auto-zooms on focus:

```css
.field {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 6px;
}

.field-label {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--text-strong);
}

.input,
.select,
.textarea {
  width: 100%;
  min-height: 40px;
  padding: 0 11px;
  border: 0;
  border-radius: var(--radius-md);
  background-color: var(--surface); /* explicit bg/color keeps Windows dark <select> legible */
  color: var(--text-strong);
  box-shadow: inset 0 0 0 1px var(--ink-4), 0 0 0 0 transparent;
  font: inherit;
  font-size: 16px; /* prevents iOS Safari zoom on focus */
  transition-property: box-shadow, background-color;
  transition-duration: 160ms;
  transition-timing-function: ease-out;
}

@media (min-width: 768px) {
  .input,
  .select,
  .textarea {
    min-height: var(--control-md);
    font-size: 13.5px;
  }
}

.input::placeholder,
.textarea::placeholder {
  color: var(--text-faint);
}

.input:focus-visible,
.select:focus-visible,
.textarea:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3px var(--accent-soft);
}

.input[aria-invalid="true"] {
  box-shadow: inset 0 0 0 1px var(--danger), 0 0 0 3px var(--danger-soft);
}
```

### B. Tactile buttons, optical icon padding & zero-shift pending labels

- **Scale on press (`0.96`):** Never scale below `0.95`. Provide `.btn-static` when scale would distract.
- **Keep filled button color under pointer:** Step the accent color by `~10%`; never swap a colored primary button to gray on hover.
- **Optical icon padding:** When a button has text + icon, reduce padding on the icon side by `2px` (`padding-left: 14px; padding-right: 12px`) so the icon doesn't look pushed outward.
- **Zero-shift label stack:** Stack `"Save"` and `"Saving…"` in one CSS Grid cell (`grid-area: 1 / 1`) so the button width never jumps.

```css
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-height: 40px;
  padding: 0 14px;
  border: 0;
  border-radius: var(--radius-md);
  font-size: 13.5px;
  font-weight: 500;
  line-height: 1;
  cursor: pointer;
  user-select: none;
  touch-action: manipulation;
  transition-property: scale, background-color, box-shadow, opacity;
  transition-duration: 150ms;
  transition-timing-function: ease-out;
}

@media (min-width: 768px) {
  .btn { min-height: var(--control-md); }
}

.btn:active:not(:disabled):not(.btn-static) {
  scale: 0.96;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-primary {
  background: var(--accent);
  color: #ffffff;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.16), 0 1px 2px rgba(0, 0, 0, 0.12);
}

@media (hover: hover) {
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
}

.btn-secondary {
  background: var(--surface);
  color: var(--text-strong);
  box-shadow: var(--shadow-card);
}

@media (hover: hover) {
  .btn-secondary:hover:not(:disabled) {
    background: var(--ink-1);
    box-shadow: var(--shadow-card-hover);
  }
}

/* Optical balance when an icon sits on the leading or trailing side */
.btn-icon-left  { padding-left: 12px; padding-right: 14px; }
.btn-icon-right { padding-left: 14px; padding-right: 12px; }

/* Lock button width across "Save" -> "Saving…" or "Copy" -> "Copied" */
.btn-stack {
  display: inline-grid;
  align-items: center;
  justify-items: center;
}
.btn-stack > * { grid-area: 1 / 1; }
.btn-stack > .is-hidden { visibility: hidden; }
```

### C. Badges & status pills with cap-height centering

Plain line-boxes leave room for descenders (`g`, `y`, `p`), which makes a label inside a `22px` pill sit `2px` too low next to its dot or icon. Use `text-box: trim-both cap alphabetic`:

```css
.badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 22px;
  padding: 0 8px;
  border-radius: var(--radius-pill);
  background: var(--ink-2);
  color: var(--text-strong);
  font-size: 11.5px;
  font-weight: 500;
  white-space: nowrap;
}

.badge:has(.badge-dot) { padding-left: 7px; }

.badge-label {
  display: block;
  line-height: 1;
  text-box: trim-both cap alphabetic;
}

.badge-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  flex: none;
}

.badge[data-tone="accent"]  { background: var(--accent-soft);  color: var(--accent-text); }
.badge[data-tone="success"] { background: var(--success-soft); color: var(--success-text); }
.badge[data-tone="warning"] { background: var(--warning-soft); color: var(--warning-text); }
.badge[data-tone="danger"]  { background: var(--danger-soft);  color: var(--danger-text); }
```

### D. Segmented controls with a sliding raised thumb

```css
.segmented {
  position: relative;
  display: inline-grid;
  grid-template-columns: repeat(var(--segments, 2), minmax(0, 1fr));
  padding: 2px;
  border-radius: var(--radius-md);
  background: var(--ink-2);
}

.segmented-thumb {
  position: absolute;
  top: 2px;
  bottom: 2px;
  left: 2px;
  width: calc((100% - 4px) / var(--segments, 2));
  border-radius: var(--radius-sm);
  background: var(--raised);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08), 0 0 0 0.5px rgba(0, 0, 0, 0.06);
  transform: translateX(calc(var(--active-index, 0) * 100%));
  transition: transform 280ms cubic-bezier(0.22, 1, 0.36, 1);
  pointer-events: none;
}

.segmented-btn {
  position: relative;
  z-index: 1;
  height: var(--row-sm);
  padding: 0 12px;
  border: 0;
  background: transparent;
  color: var(--text);
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  transition: color 180ms ease;
}

.segmented-btn[aria-selected="true"] {
  color: var(--text-strong);
}
```

### E. Shared-column list rows & instant-in / eased-out hover

- **Every row shares one grid template:** Never use `justify-content: space-between` where badges and timestamps wiggle left and right depending on title length.
- **Instant-in, eased-out hover:** Native macOS and Linear menus highlight the instant the pointer enters (`transition-duration: 0s` on `:hover`) and fade out softly (`150ms ease`) when the pointer leaves.
- **Whitespace groups:** Keep title and subtitle `4px` apart so they read as one record inside `10–12px` row padding.

```css
/* Bad — leftover space-between makes columns zig-zag from row to row */
.bad-row {
  display: flex;
  justify-content: space-between;
}

/* Good — shared grid tracks align every column vertically down the list */
.list-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  transition: background-color 150ms ease;
}

@media (hover: hover) {
  .list-row:hover {
    background-color: var(--ink-1);
    transition-duration: 0s; /* instant highlight under pointer, soft fade out */
  }
}

.row-primary {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px; /* tight grouping between title and subtitle */
}

.row-title {
  font-size: 13.5px;
  font-weight: 500;
  color: var(--text-strong);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.row-subtitle {
  font-size: 12px;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
```

### F. Concentric radii, image outlines, hit areas & optical alignment

#### Concentric border radius
When nesting rounded elements with `24px` or less of padding between them, calculate `outerRadius = innerRadius + padding`:

```css
/* Good — concentric curves (12px inner + 8px padding = 20px outer) */
.card {
  border-radius: 20px;
  padding: 8px;
  background: var(--surface);
  box-shadow: var(--shadow-card);
}
.card-inner {
  border-radius: 12px;
}

/* Bad — same radius on both makes corners look pinched */
.card-bad { border-radius: 12px; padding: 8px; }
.card-bad .card-inner { border-radius: 12px; }
```

#### Subtle `1px` inset outline on images
Give every `<img>` explicit `width` and `height` plus a neutral inset outline so light images don't bleed into white cards and dark images don't vanish into dark cards:

```css
/* Good — pure black/white at 10% opacity, inset by 1px */
.media-frame {
  outline: 1px solid rgba(0, 0, 0, 0.1);
  outline-offset: -1px;
  border-radius: var(--radius-md);
}
@media (prefers-color-scheme: dark) {
  .media-frame {
    outline-color: rgba(255, 255, 255, 0.1);
  }
}
```

#### Expanded `40×40px` hit area on small controls
Interactive elements need at least a `44×44px` (`40×40px` minimum) touch target. If a visible checkbox or icon button is smaller, expand its hit box with `::after` (ensuring neighboring hit areas never overlap):

```css
/* Small 20x20 control with expanded 40x40 touch target */
.icon-check {
  position: relative;
  width: 20px;
  height: 20px;
  flex: none;
}
.icon-check::after {
  content: "";
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 40px;
  height: 40px;
}
```

#### Optical centering for currency Hero numbers & asymmetric icons
- **Play triangles:** A play triangle's visual weight sits toward its flat left edge; shift the glyph `2px` right (`margin-left: 2px`) inside a circular button so it looks centered.
- **Centered currency/unit amounts:** `$45` centered as a single string looks pushed right because the `$` sign takes horizontal width. Center the digits box (`45`) and hang the currency sign outside (`position: absolute; right: 100%`):

```css
.hero-price {
  position: relative;
  display: inline-block;
  font-size: 36px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.02em;
}
.hero-price-currency {
  position: absolute;
  right: 100%;
  top: 0.18em;
  margin-right: 2px;
  font-size: 0.55em;
  color: var(--text);
}
```

---

## 4. Mobile-first (`375px`), zero overflow, zero layout shift & all 4 states

### A. Mobile-first viewport rules

- **Viewport and safe areas:** Include `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">` and `<meta name="theme-color" content="#f6f5f4">`. Never disable pinch-zoom (`user-scalable=no` or `maximum-scale=1`). Pad sticky headers, bottom bars and drawers with `env(safe-area-inset-top)` and `env(safe-area-inset-bottom)`.
- **Dynamic viewport height:** Use `100dvh` (`min-height: 100dvh`), never `100vh`, so mobile address bars never clip bottom toolbars or composers.
- **No hover-only actions:** Touch screens have no hover. Never hide edit, delete or secondary actions behind `:hover` alone; keep them reachable by tap or an explicit `⋯` menu, and wrap hover rules in `@media (hover: hover)`.
- **Scroll & focus discipline:** Do not set `autoFocus` on mobile viewports (it pops the virtual keyboard over half the screen on load). Put `overscroll-behavior: contain` on modals/drawers; if using `overscroll-behavior: none` on `<html>`, gate it inside `@media (pointer: fine)` so touch devices keep pull-to-refresh.

### B. Zero-overflow CSS patterns

- **Flex and Grid truncation trap:** Flex children default to `min-width: auto` and CSS Grid `1fr` means `minmax(auto, 1fr)`, causing long text or URLs to blow the container past `100vw`. Always set `min-width: 0` on flex children and `minmax(0, 1fr)` on grid tracks:

```css
/* Responsive grid that never overflows on 375px screens or unbroken strings */
.cards-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr));
  gap: 16px;
}
.cards-grid > * {
  min-width: 0;
  overflow-wrap: anywhere;
}

/* Bounded modal/sheet whose footer buttons never get pushed off-screen */
.modal {
  width: min(100vw - 2rem, 32rem);
  max-height: calc(100dvh - 2rem);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: var(--radius-card);
  background: var(--raised);
  box-shadow: var(--shadow-float);
}
.modal-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 16px;
}
```

### C. Zero layout shift (CLS) & natural skeletons

- **Best on Anywhere—inject initial data on first paint:** Because your Node/Python server and SQLite share the local disk, render initial items directly in the HTML or `<script id="initial-data" type="application/json">` so first paint has `0px` layout shift and zero fetch waterfall.
- **Stable scrollbars:** Set `scrollbar-gutter: stable` on `html` so filtering a list between 3 and 30 rows never shifts a centered layout horizontally by `15px`.
- **Natural skeleton rows (varied bar widths, never blank on refetch):**
  - Show skeletons **only** when `data === undefined` (initial load with no cached data). When searching, filtering or switching tabs, keep existing items visible (optionally at `opacity: 0.65`) until new data arrives.
  - Match skeleton row height and padding to real rows, and **vary placeholder bar widths** row to row (`60%`, `45%`, `70%`, `52%`) so the skeleton looks organic rather than a mechanical grid:

```css
.skeleton-bar {
  height: 10px;
  border-radius: 999px;
  background: var(--ink-2);
  animation: skeleton-pulse 1.6s ease-in-out infinite;
}
@media (prefers-reduced-motion: reduce) {
  .skeleton-bar { animation: none; }
}
@keyframes skeleton-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.45; }
}
```

### D. Build all 4 states for every data view

Never write a view that only handles `items.map(...)`. Explicitly build all four states:

1. **Initial loading (`isLoading && !items`):** 4 geometry-matched skeleton rows with varied bar widths (`60%`, `45%`, `70%`, `52%`).
2. **Empty database (`items.length === 0 && !hasActiveFilter`):** A calm card with an icon, a specific heading (`"No notes yet"`), one sentence explaining what lives here, and a primary CTA button (`"New note"`) that opens the creation flow immediately.
3. **Empty search or filter (`items.length === 0 && hasActiveFilter`):** Distinct from empty-database onboarding! State what matched nothing (`No notes matching “${query}”`) with a one-click `"Clear filters"` button.
4. **Error (`error`):** Keep stale data on screen if present and show an inline error banner; if no data loaded, render an inline error card stating what failed with a `"Retry"` button—never a raw stack trace or blank screen.

---

## 5. Typography & copy polish

```css
/* Good — root font smoothing on macOS, balanced headings, pretty paragraphs */
html {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  scrollbar-gutter: stable;
}

h1, h2, h3 {
  text-wrap: balance; /* balances headings up to 6 lines */
  letter-spacing: -0.015em;
}

p, li, figcaption, blockquote {
  text-wrap: pretty; /* prevents lone orphan words on the last line */
}

.numeric, time, .count, .price, td[data-numeric] {
  font-variant-numeric: tabular-nums; /* equal-width digits prevent jitter */
}
```

| Scenario | Rule |
| --- | --- |
| Headings (`h1`–`h3`, card titles, empty-state headers) | `text-wrap: balance` |
| Short-to-medium prose (`p`, descriptions, list items, captions) | `text-wrap: pretty` |
| Long prose (`10+` lines), `<pre>`, `<code>` | Default wrapping (neither `balance` nor `pretty`) |
| Counters, timers, live prices, table number columns | `font-variant-numeric: tabular-nums` |
| Monospace font (`--font-mono`) | Only for code, hashes, IDs, paths and terminal logs—never as a lazy substitute for UI labels |
| Ellipsis, quotes, units | Real `…` (`&hellip;`), curly quotes `“ ”`, non-breaking space in `10\u00A0MB` and `⌘\u00A0K` |
| Button labels | Specific active verbs (`"Create note"`, `"Revoke key"`), never vague `"Submit"` or `"Continue"` |

---

## 6. Animations, tactile motion & Storyboard choreography

### A. Transition specificity & GPU compositing

Never use `transition: all` (or bare Tailwind `transition`). List exact properties (`transform`, `opacity`, `scale`, `background-color`, `box-shadow`).

```css
/* Good — explicit compositor properties */
.card-interactive {
  transition-property: transform, box-shadow, background-color;
  transition-duration: 180ms;
  transition-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
}

/* Bad — watches every property, causes layout transition bugs */
.card-bad {
  transition: all 180ms ease;
}
```

| Property | GPU-compositable | Worth `will-change` (only if fixing first-frame stutter) |
| --- | --- | --- |
| `transform`, `scale`, `translate`, `rotate` | Yes | Yes |
| `opacity` | Yes | Yes |
| `filter` (`blur`, `brightness`) | Yes | Yes |
| `clip-path` | Yes | Yes |
| `top`, `left`, `width`, `height`, `padding`, `background` | No | Never (`will-change: all` is forbidden) |

### B. Contextual icon swaps (Play/Pause, Copy/Check) & `initial={false}`

When swapping stateful icons inside a button or toggle, animate `scale(0.25 → 1)`, `opacity(0 → 1)` and `filter: blur(4px → 0px)` with **zero bounce**, and skip mount animation on initial page load:

```tsx
// React + Motion: contextual icon swap that does not animate on initial page load
<AnimatePresence initial={false} mode="popLayout">
  <motion.span
    key={copied ? "check" : "copy"}
    initial={{ opacity: 0, scale: 0.25, filter: "blur(4px)" }}
    animate={{ opacity: 1, scale: 1, filter: "blur(0px)" }}
    exit={{ opacity: 0, scale: 0.25, filter: "blur(4px)" }}
    transition={{ type: "spring", duration: 0.3, bounce: 0 }}
  >
    {copied ? <CheckIcon /> : <CopyIcon />}
  </motion.span>
</AnimatePresence>
```

```css
/* Pure CSS alternative (no library needed): keep both icons in a stacked grid cell */
.icon-swap {
  display: inline-grid;
  place-items: center;
}
.icon-swap > svg {
  grid-area: 1 / 1;
  transition:
    opacity 240ms cubic-bezier(0.2, 0, 0, 1),
    transform 240ms cubic-bezier(0.2, 0, 0, 1),
    filter 240ms cubic-bezier(0.2, 0, 0, 1);
}
.icon-swap > svg[data-hidden="true"] {
  opacity: 0;
  transform: scale(0.25);
  filter: blur(4px);
  pointer-events: none;
}
```

### C. Modal, toast, row-insertion & chart choreography (Zero-dependency CSS + DOM)

Do not snap modals (`display: none`), toasts (`toast.remove()`), newly created list rows (`innerHTML = ...`), or bar charts in 1 frame. Use `@starting-style` + `data-closing="true"` for entrances/exits, tag newly inserted rows with `data-entering="true"`, and reuse chart bar DOM nodes so CSS height transitions interpolate:

```css
/* 1. Modal & Toast enter (@starting-style) + softer exit (data-closing="true") */
.modal-backdrop {
  opacity: 1;
  transition: opacity 180ms cubic-bezier(0.22, 1, 0.36, 1);
  @starting-style { opacity: 0; }
}
.modal-backdrop[data-closing="true"] { opacity: 0; pointer-events: none; }

.modal, .toast {
  opacity: 1;
  transform: translateY(0) scale(1);
  filter: blur(0px);
  transition-property: opacity, transform, filter;
  transition-duration: 210ms;
  transition-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
  @starting-style {
    opacity: 0;
    transform: translateY(8px) scale(0.96);
    filter: blur(4px);
  }
}
.modal-backdrop[data-closing="true"] .modal,
.toast[data-closing="true"] {
  opacity: 0;
  transform: translateY(-4px) scale(0.97);
  filter: blur(2px);
  transition-duration: 150ms;
}

/* 2. Newly inserted list row: glide in + brief accent highlight pulse */
@keyframes row-enter {
  0%   { opacity: 0; transform: translateY(-8px) scale(0.99); background-color: var(--accent-soft); }
  35%  { opacity: 1; transform: translateY(0) scale(1); background-color: var(--accent-soft); }
  100% { opacity: 1; transform: translateY(0) scale(1); }
}
.list-row[data-entering="true"] {
  animation: row-enter 900ms cubic-bezier(0.22, 1, 0.36, 1) both;
}

/* 3. Detail inspector selection switch & live counter bump */
@keyframes inspector-settle {
  from { opacity: 0.55; transform: translateY(3px); }
  to   { opacity: 1;    transform: translateY(0); }
}
.inspector[data-switching="true"] {
  animation: inspector-settle 180ms cubic-bezier(0.22, 1, 0.36, 1);
}
```

```js
// Reuse bar DOM nodes instead of overwriting innerHTML so `transition: height 240ms` runs:
while (barsEl.children.length < series.length) {
  const bar = document.createElement("div");
  bar.className = "history-bar";
  bar.style.height = "16%";
  barsEl.appendChild(bar);
}
series.forEach((pt, i) => {
  barsEl.children[i].dataset.tone = pt.tone;
  barsEl.children[i].style.height = `${pt.pct}%`;
});
```

### D. The Storyboard Pattern for multi-stage animations

Whenever building a staged entrance, onboarding flow, interactive showcase or multi-step transition in React, structure the component as a **Storyboard** at the top of the file—zero magic numbers inline in JSX:

```tsx
/* ─────────────────────────────────────────────────────────
 * ANIMATION STORYBOARD
 *
 * Read top-to-bottom. Each `at` value is ms after trigger.
 *
 *    0ms   waiting for trigger (mount or scroll into view)
 *  150ms   shell card fades in, scale 0.96 → 1.0
 *  350ms   header & primary stat resolve (blur 4px → 0px)
 *  550ms   list rows slide up (staggered 80ms per row)
 *  850ms   action bar settles in
 * ───────────────────────────────────────────────────────── */

const TIMING = {
  cardAppear:  150,  // shell card fades and scales in
  headerShow:  350,  // header & primary metric resolve
  rowsStagger: 550,  // list rows begin staggering up
  actionBar:   850,  // bottom or header action bar settles
};

const CARD = {
  initialScale: 0.96,
  finalScale:   1.0,
  spring: { type: "spring" as const, stiffness: 300, damping: 30 }, // smooth container settle
};

const ROWS = {
  staggerMs: 80,
  offsetY:   10,
  spring: { type: "spring" as const, stiffness: 350, damping: 28 }, // balanced slide
};
```

Drive stages with a single integer `stage` state (`stage >= 1`, `stage >= 2`) inside one `useEffect` that cleans up its `setTimeout` list on unmount, and disable or fast-forward to the final stage when `window.matchMedia("(prefers-reduced-motion: reduce)").matches` is true.

---

## 7. Accessibility, effortless forms & URL state

- **Semantic HTML first:** Use `<button type="button">` for actions, `<a href="...">` for navigation (preserving Cmd/Ctrl-click; never `<div onClick>`), `<label>` for form fields, and `<table>` for tabular data.
- **ARIA & focus visibility:** Every icon-only button needs `aria-label`; decorative SVGs get `aria-hidden="true"`; toast/status regions get `aria-live="polite"`. Never use `outline: none` without a visible `:focus-visible` ring (`:focus-within` on compound inputs).
- **Forms that feel effortless:**
  - Pair every input with a clickable `<label>` (wrap checkboxes/radios and their text in one `<label>` so there is no dead gap between box and words).
  - Set the right `type` (`email`, `url`, `tel`, `number`), `inputmode`, `autocomplete`, and `spellcheck="false"` on emails, codes, URLs and identifiers.
  - Never block paste (`onPaste` + `preventDefault`).
  - Keep the submit button enabled until submission starts; on submit, focus the first invalid field and show inline field errors (cleared as soon as the user fixes that field).
- **URL state & safe destructive actions:** Reflect active tabs, search queries, filters and selected items in URL query parameters (`?tab=...&q=...`) using `history.replaceState` so refreshing or sharing an Anywhere URL opens the exact same view. Require an inline confirmation step or an Undo toast before destructive actions (deleting items, revoking keys).

---

## 8. Senior design critique & visual verification (before finishing)

Never declare a web app done based only on `curl` returning `200` or the code compiling. Inspect the rendered UI at both desktop (`1280×800`) and mobile (`375×812`) viewports using this severity-ordered critique:

### 1. Structural (Information architecture & focusing mechanism)
- **Focusing mechanism:** What does the eye land on in the first second? If the sidebar, a row of 4 metric cards, a filter bar and the table all have the same visual weight, quieten the supporting chrome (`--canvas` vs `--surface`) and remove redundant card containers so the primary workspace leads.
- **Category bar:** Ask *"What would Linear, Stripe, Raycast or Apple Notes do on this screen?"* Does this layout feel tailored to this specific tool, or could it be any generic CRUD template?
- **Progressive disclosure:** Count the columns/data points per row and the buttons in the header. If a row has more than 4 data points or the header has more than 2 primary controls, move secondary metadata and rare actions into a detail panel, popover or `⋯` menu.

### 2. Behavioral (States, flow & uncommon care)
- **All 4 data states verified:** Check Initial Loading (varied-width skeletons), Empty Database (helpful explanation + primary create CTA), Empty Search/Filter (`"Clear filters"` action), and Error (inline retry, stale data preserved).
- **Psychological framing ("uncommon care"):** Frame progress and status encouragingly—for example, `"Phase 1 of 4"` or a collapsing completed group rather than a demoralizing `"10 / 47 tasks complete"` wall. Ensure empty states and error messages tell the user *how* to proceed in one click.
- **Mobile (`375px`) & keyboard check:** Verify zero horizontal scroll (`document.documentElement.scrollWidth <= window.innerWidth`), `16px` form inputs, `40×40px+` tap targets, and complete keyboard/focus-visible navigation.

### 3. Visual (Surface, spacing & icon precision)
- **Count background colors & borders:** Stick to `--canvas`, `--surface` and `--raised` plus one `--accent`. Remove nested `1px` borders inside cards where whitespace or a single `--line` divider does the job.
- **Zero near-tangents & fat rows:** Check that no two edges sit `1–3px` apart, text keeps at least `12px` inset from card edges, buttons keep `≥8px` clearance from fields above them, and list rows share a single aligned grid template.
- **Icons & badges:** Verify every icon comes from one family at one stroke weight (`14–16px`, `flex: none`), colored with opaque `--icon` tokens (never emoji or hand-drawn marks), and pills center their labels with `text-box: trim-both cap alphabetic`.
