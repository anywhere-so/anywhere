# Web UI, UX and interface craft

Read this guide whenever building, styling, animating or reviewing a web app or static site on Anywhere. People open and share Anywhere apps on phones as often as on laptops, so every interface must feel mobile-ready, tactile, accessible and visually intentional on the first deploy.

## 1. Mobile-first and responsive layout

- **Viewport and safe areas:** Include `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">`. Never disable pinch-zoom with `user-scalable=no` or `maximum-scale=1`. Pad sticky headers, bottom bars and full-bleed drawers with `env(safe-area-inset-top)` and `env(safe-area-inset-bottom)` so controls never collide with notches or home indicators.
- **Dynamic viewport height:** Use `100dvh` (`min-height: 100dvh`), not `100vh`, so mobile browser address bars never clip bottom toolbars or chat/composer inputs.
- **375px first, fluid up to desktop:** Design for a `375px` phone screen first, then expand with fluid CSS Grid (`grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr))`) or `@media (min-width: 768px)` breakpoints.
- **No horizontal page scroll:** Never let fixed widths or wide tables push the page wider than `100vw`. Collapse desktop sidebars and multi-button toolbars into a compact header, bottom bar or drawer (`overscroll-behavior: contain`) on narrow viewports. Wrap wide tables in `overflow-x: auto` containers or stack rows as cards on mobile.
- **Prevent iOS Safari input auto-zoom:** Set `font-size: 16px` (`1rem`) or larger on all `<input>`, `<textarea>` and `<select>` elements; anything smaller forces iOS Safari to zoom the page on focus.
- **Touch-first controls and hit areas:** Interactive elements need at least a `44×44px` (`40×40px` minimum) hit target and `touch-action: manipulation` (prevents double-tap zoom delay). If a visible control is smaller (such as a `20×20px` checkbox or icon), expand its hit target with an `::after` pseudo-element centered at `40×40px`, ensuring adjacent hit targets never overlap.
- **No hover-only actions:** Touch screens have no hover. Never hide edit, delete or secondary actions behind `:hover` alone; keep them reachable by tap or an explicit menu, and gate hover visuals behind `@media (hover: hover)`.
- **Scroll and focus discipline on mobile:** Avoid `autoFocus` on mobile viewports so the virtual keyboard does not cover half the screen on load. Use `overscroll-behavior: contain` inside modals, drawers and sheets; if suppressing page rubber-banding, apply `overscroll-behavior: none` on `<html>` only inside `@media (pointer: fine)` so coarse-pointer touch devices keep pull-to-refresh and swipe navigation.

## 2. Overflow, layout shift, skeletons and the 4 view states

General UI guides rarely show the concrete CSS and state logic needed to prevent overflow, layout jumps and blank loading screens. Apply these patterns in every Anywhere web app:

### A. Zero-overflow CSS rules

- **Flex and Grid truncation trap:** Flex children default to `min-width: auto` and CSS Grid `1fr` means `minmax(auto, 1fr)`, so `text-overflow: ellipsis` silently fails and long text stretches the container past `100vw`. Always set `min-width: 0` on flex children and use `minmax(0, 1fr)` instead of bare `1fr`:
  ```css
  /* Flex row with truncating title and fixed action buttons */
  .row { display: flex; align-items: center; gap: 12px; }
  .row-main { flex: 1; min-width: 0; }
  .row-title { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-actions { flex-shrink: 0; }

  /* Responsive grid that never blows out on long unbroken content */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr));
    gap: 16px;
  }
  .grid > * { min-width: 0; overflow-wrap: anywhere; }
  ```
- **Bounded modals, drawers and sheets:** Never let a modal grow taller than the viewport and push its Save/Cancel buttons off-screen. Cap the shell and scroll only the body:
  ```css
  .modal {
    width: min(100vw - 2rem, 32rem);
    max-height: calc(100dvh - 2rem);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .modal-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  ```
- **Unbroken tokens and pill rows:** Put `overflow-wrap: anywhere` on URLs, filenames, emails, IDs and logs. On filter/tag bars, use `flex-wrap: wrap` or a horizontal scroll strip (`overflow-x: auto; scrollbar-width: none`) with `flex-shrink: 0` on each pill.

### B. Zero layout shift (CLS) from data loading and state changes

- **Best on Anywhere—inject initial data on first paint:** Because your Node or Python server and SQLite live on the same disk, render the initial items into the HTML (or embed `<script id="initial-data" type="application/json">`) so the first paint has zero client fetch waterfall and `0px` layout shift.
- **Lock button width across state changes:** When a button changes from `"Save"` to `"Saving…"` or `"Copy"` to `"Copied"`, stack both labels in a single CSS Grid cell so the button's width never jumps and shifts neighboring controls:
  ```css
  .btn-stack { display: inline-grid; align-items: center; justify-items: center; }
  .btn-stack > * { grid-area: 1 / 1; }
  .btn-stack > .is-hidden { visibility: hidden; }
  ```
- **Stable scrollbars, numbers and media:**
  - Set `scrollbar-gutter: stable` on `html` so filtering a list between 3 and 30 rows never shifts a centered layout horizontally by `15px`.
  - Apply `font-variant-numeric: tabular-nums` on counts, badges, timers and table columns so `9` → `10` never shifts adjacent text.
  - Give every `<img>`, avatar, video and chart container an explicit `width`/`height`, `aspect-ratio` or `min-height` before its content loads.

### C. Skeletons and stale-while-revalidate

- **Match skeleton geometry to real rows/cards:** Never render a `20px` `"Loading…"` line that drops the page `400px` when data arrives. Render skeleton placeholders using the exact same grid, card height, padding and border-radius as the loaded items:
  ```css
  .skeleton {
    background: rgba(128, 128, 128, 0.12);
    border-radius: 6px;
    animation: skeleton-pulse 1.6s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .skeleton { animation: none; }
  }
  @keyframes skeleton-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }
  ```
- **Never blank the screen on refetch, search or tab switch:** Show skeletons *only* when `data === undefined` (initial load with no cached items). When searching, filtering, switching tabs or refreshing after a mutation, keep the current rows visible (optionally at `opacity: 0.65` with a small inline spinner) until the new data replaces them.

### D. Build all 4 states for every data view

Never write a list or dashboard view that only handles `items.map(...)`. Explicitly handle all four states:

1. **Initial loading (`isLoading && !items`):** Render 3–5 geometry-matched skeleton rows/cards inside the real container frame.
2. **Empty data (`items.length === 0 && !hasActiveFilter`):** Render a calm empty-state card with an icon, a clear heading (`"No notes yet"`), one sentence explaining what appears here, and a primary CTA button (`"Create your first note"`) that opens the creation flow immediately.
3. **Empty search or filter (`items.length === 0 && hasActiveFilter`):** Keep this separate from the empty-database onboarding! State what matched nothing (`No notes matching "${query}"`) with a one-click `"Clear filters"` button.
4. **Error (`error`):** If stale data is already on screen, keep it visible and show a non-destructive error banner; if no data loaded, render an inline error card naming what failed with a `"Retry"` button—never a blank screen or raw stack trace.

## 3. Visual design and surfaces

- **Intentional color and theming:** Use one neutral surface scale (page background, elevated card, subtle divider) and one semantic accent color. Set `color-scheme: light` or `color-scheme: dark` (or `light dark`) on `<html>` so native scrollbars and form controls match, set `<meta name="theme-color">` to the page background, and give native `<select>` elements explicit `background-color` and `color` so Windows dark mode stays legible.
- **Concentric border radius:** When nesting rounded elements with `24px` or less of padding between them, match the curves with `outerRadius = innerRadius + padding`:
  ```css
  .card {
    border-radius: 20px; /* 12px inner + 8px padding */
    padding: 8px;
  }
  .card-inner {
    border-radius: 12px;
  }
  ```
- **Shadows over borders for elevation:** For cards, buttons and floating containers, prefer layered transparent `box-shadow` over solid borders so depth adapts to any background. Keep `1px` solid borders for layout dividers (`border-bottom`, table cells, form inputs):
  ```css
  :root {
    --shadow-border:
      0px 0px 0px 1px rgba(0, 0, 0, 0.06),
      0px 1px 2px -1px rgba(0, 0, 0, 0.06),
      0px 2px 4px 0px rgba(0, 0, 0, 0.04);
    --shadow-border-hover:
      0px 0px 0px 1px rgba(0, 0, 0, 0.08),
      0px 1px 2px -1px rgba(0, 0, 0, 0.08),
      0px 2px 4px 0px rgba(0, 0, 0, 0.06);
  }
  /* Dark mode: single crisp ring */
  @media (prefers-color-scheme: dark) {
    :root {
      --shadow-border: 0 0 0 1px rgba(255, 255, 255, 0.08);
      --shadow-border-hover: 0 0 0 1px rgba(255, 255, 255, 0.13);
    }
  }
  ```
- **Optical over geometric alignment:**
  - *Buttons with text + icon:* reduce padding on the icon side by `2px` (`padding-left: 16px; padding-right: 14px`) so the icon does not look pushed outward.
  - *Play triangles and asymmetric icons:* shift play triangles `2px` right (`margin-left: 2px`) or adjust the SVG `viewBox` so visual weight is centered, and use icons from a single family/stroke weight.
- **Image outlines and dimensions:** Give every `<img>` explicit `width` and `height` (prevents layout shift), `loading="lazy"` below the fold (`fetchpriority="high"` for critical hero images), and a subtle inset neutral outline—pure black `rgba(0, 0, 0, 0.1)` in light mode and pure white `rgba(255, 255, 255, 0.1)` in dark mode (`outline: 1px solid ...; outline-offset: -1px`), never a tinted palette color.

## 4. Typography, copy and content resilience

- **Root font smoothing:** Apply `-webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale;` once on `html` so macOS text renders crisply.
- **Text wrapping:**
  - `h1, h2, h3`: `text-wrap: balance` (balances lines up to 6 lines; do not apply to long paragraphs).
  - `p, li, figcaption, blockquote`: `text-wrap: pretty` (prevents lone orphan words on the last line of short-to-medium copy).
- **Tabular numerals:** Apply `font-variant-numeric: tabular-nums` to counters, timers, live prices, dashboards and numeric table columns so digits align vertically and never jitter when values change.
- **Typographic and copy polish:**
  - Use the ellipsis character `…` instead of `...`, curly quotes `“` `”` instead of straight quotes in prose, and non-breaking spaces (`10\u00A0MB`, `⌘\u00A0K`) where units or shortcuts must stay together.
  - End loading/pending states with `…` (`"Loading…"`, `"Saving…"`).
  - Write active-voice, specific button labels (`"Save API Key"` or `"Create Note"`, not `"Continue"` or `"Submit"`), use numerals for counts (`"8 deploys"`), and make error messages state both what went wrong and how to fix it.
  - Format dates, times, numbers and currency with `Intl.DateTimeFormat` and `Intl.NumberFormat` rather than hardcoded strings, and wrap brand names, code tokens and identifiers with `translate="no"`.

## 5. Animations and tactile motion

- **Interruptible CSS transitions vs. keyframes:** Use CSS `transition` for interactive state changes (hover, toggle, drawer open/close) so reversing mid-interaction retargets smoothly. Reserve `@keyframes` for one-shot staged sequences.
- **Transition only what changes:** Never use `transition: all` (or bare Tailwind `transition`). List exact compositor properties (`transition-property: transform, opacity, box-shadow`). Use `will-change` (`transform`, `opacity`, `filter`) only when fixing a real first-frame stutter, and never `will-change: all`.
- **Scale on press:** Give interactive buttons tactile feedback with `scale: 0.96` on `:active` (`transition: scale 150ms ease-out`). Never scale below `0.95`, and support a `static` variant where motion would be distracting.
- **Split and stagger entrances; keep exits subtle:**
  - Break entering content into semantic groups (heading, description, actions) staggered by `~100ms`, combining `opacity: 0 → 1`, `translateY(12px → 0)` and `filter: blur(4px → 0px)`.
  - Make exits faster and softer than entrances (`150ms ease-in`, `opacity: 0`, small fixed `translateY(-12px)` rather than full container height).
  - On default-state controls (tabs, toggles, icon swaps), skip entrance animation on initial page load (`initial={false}` on `AnimatePresence`).
- **Contextual icon swaps:** When swapping state icons (play/pause, copy/check), animate `scale(0.25 → 1)`, `opacity(0 → 1)` and `filter: blur(4px → 0px)` with `{ type: "spring", duration: 0.3, bounce: 0 }` (bounce must be `0`). If no motion library is installed, keep both icons in the DOM (one `position: absolute`) and cross-fade them with CSS `transition: opacity 300ms cubic-bezier(0.2, 0, 0, 1), transform 300ms cubic-bezier(0.2, 0, 0, 1), filter 300ms cubic-bezier(0.2, 0, 0, 1)` rather than adding a dependency.
- **Storyboard pattern for multi-stage animations:** When building a multi-step choreography, put an ASCII storyboard comment at the top of the file with right-aligned `ms` timestamps, a single `const TIMING = { ... }` object holding every stage delay in milliseconds, uppercase config objects (`CARD`, `ROWS`) holding scales/offsets/springs, and a single integer `stage` state driven by `useEffect` timers (`stage >= 1`, `stage >= 2`).
- **Reduced motion:** Always honor `@media (prefers-reduced-motion: reduce)` by disabling or simplifying non-essential motion and stopping decorative loops.

## 6. Accessibility, focus, forms and URL state

- **Semantic HTML first:** Use `<button type="button">` for actions, `<a href="...">` for navigation (preserving Cmd/Ctrl-click and middle-click; never `<div onClick>`), `<label>` for form fields, and `<table>` for tabular data. Keep headings hierarchical (`<h1>`–`<h6>`) with `scroll-margin-top` on anchored headings.
- **ARIA and focus rings:** Give icon-only buttons an `aria-label`, decorative icons `aria-hidden="true"`, and async status updates/toasts `aria-live="polite"`. Provide a visible `:focus-visible` ring on every interactive control (`:focus-within` for compound inputs)—never `outline: none` without a visible replacement—and ensure sticky bars never obscure the focused element.
- **Forms that feel effortless:**
  - Pair every input with a clickable `<label>` (for checkboxes/radios, wrap label and control in one hit target with no dead gap).
  - Set the right `type` (`email`, `tel`, `url`, `number`), `inputmode`, `autocomplete` (`autocomplete="off"` on non-auth fields to avoid password-manager popups), and `spellcheck="false"` on emails, codes and usernames.
  - Never block paste (`onPaste` + `preventDefault`).
  - Keep the submit button enabled until submission starts, then disable and show `"Saving…"` or a spinner; render validation errors inline next to the field and focus the first invalid field on submit.
  - Warn before navigating away from unsaved form changes (`beforeunload`).
- **URL state and destructive actions:** Reflect active tabs, filters, search queries and open panels in URL query parameters (`?tab=...`, `?q=...`) so refreshing or sharing an Anywhere URL opens the same view. Always require a confirmation modal or an undo toast before destructive actions (deleting items, clearing data).
- **Rendering performance:** Never read layout properties (`getBoundingClientRect`, `offsetHeight`, `scrollTop`) during render, and use `content-visibility: auto` or virtualization on lists over 50 items.

## 7. Interface critique checklist (before deploying)

Before finishing a UI change, review the screen through five lenses:

1. **Focusing mechanism:** Is it immediately obvious where the user should look and act first, or do multiple panels/cards/buttons compete at equal visual weight?
2. **Progressive disclosure & density:** Can secondary metadata, rarely used filters or admin settings move into a drawer, popover or collapsed section so the primary task stays calm?
3. **Overflow, layout shift & all 4 states:** Do flex/grid children use `min-width: 0` / `minmax(0, 1fr)` and `overflow-wrap: anywhere` on long strings? Are modals bounded with internal scroll? Does loading use geometry-matched skeletons (or SSR initial data) with `0px` jump, and are Empty DB, Empty Search/Filter and Error states all built?
4. **Emotional & user context ("uncommon care"):** Does progress framing feel encouraging rather than overwhelming? Are empty states, loading states and error messages helpful and specific?
5. **Mobile & keyboard check:** Does the page work cleanly at `375px` width with zero horizontal overflow, `16px+` inputs, `44×44px` tap targets, and full keyboard/focus-visible navigation?
