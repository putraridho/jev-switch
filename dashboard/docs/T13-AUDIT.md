# T13 audit: packaging and UI polish

Scope: D9.1 and D1a word by word, against the UI as merged (three screens, one `Meter`
primitive). Presentation changes only: no behaviour, contract or dependency change.

## Delivered

- App icon set (`src-tauri/icons`): a 3x3 waffle (seven paper cells, one critical cell, one
  dim cell) on an ink tile. The tile carries its own background, so it reads the same on
  light and dark panels. Source `icon.svg`; PNG at 16, 32, 48, 64, 128, 256 px, plus
  `icon.png` (256). Checked at 16 px enlarged: the nine cells stay separate.
- `install.sh` installs all six sizes and `--uninstall` removes them. Retested in a temp
  prefix with `HOME` and every `XDG_*` variable overridden: 8 files written, desktop entry
  passes `desktop-file-validate`, uninstall leaves 0 files.
- `src/lib/design/base.css` (new, imported by `main.ts`): one focus ring (2 px accent, 2 px
  offset), quiet hairline buttons (hit area 32 px), styled number fields and select,
  tabular numerals, word wrapping, SVG that never exceeds its container, reduced motion.
- Screens: 48 px padding became 24/16 px with a 40 rem column (the old padding overflowed
  at 320 px); the Overview header wraps instead of squeezing the Refresh button; the big
  numerals use tabular figures.
- `e2e/screens/polish.spec.ts` (new): every route, both themes, 320 px and 200 % zoom
  (320 CSS px at DPR 2): zero axe violations, no horizontal scroll, reviewed screenshot
  baselines (`polish.spec.ts-snapshots`, 12 images).
- `e2e/screens/scaffold.spec.ts`: the heading it looked for no longer exists; it now looks
  for the Overview title. (It was failing before this task.)

Files touched: `scripts/install.sh`, `README.md` (icon sizes), `src-tauri/icons/*`,
`src/main.ts`, `src/lib/design/base.css`, `src/screens/{Overview,ProviderDetail,Settings}.svelte`,
`e2e/screens/{polish,scaffold}.spec.ts`, baselines.

Run: `pnpm lint`, `pnpm check`, `pnpm test` (56 tests) and `pnpm test:screens` (20 tests) pass.

## Second pass: the "detailed" gaps built

Authorised behaviour changes in the T07/T09/T10/T14 files (no new dependency, no IPC
contract change). Every item below has a test; the map is at the end.

- **Level 1** is a hover-and-focus disclosure (`MeterRow`), not `<details>`: it opens on
  pointer hover or keyboard focus with the same four lines (figure to one decimal, absolute
  reset, status, observed time), stays open while the pointer is over it, opens below the
  meter so it never covers the control that opened it, and closes on `Esc` without moving
  focus. `Esc` order is dialog, then disclosure, then back (`lib/escape.ts`); one press does
  one of these.
- **`aria-valuetext`** carries the whole detail: "86.0 percent used, resets in 2 hours 14
  minutes, today at 2:47 PM, status warning, seen at 12:32 PM". The meter is a tab stop, so
  Tab alone reaches it (the one approved `svelte-ignore`, `a11y_no_noninteractive_tabindex`).
- **Shortcuts** (`lib/shortcuts.ts`): `R`, `,`, `?` (Settings, Behaviour, which lists every
  key), `1` to `6` in the fixed provider order (a hidden provider's digit does nothing),
  `Esc`, and Ctrl plus, minus and zero for zoom (100 to 200 percent in 25s). Single keys act
  only when no text field has focus and can be switched off (Settings, Behaviour); `Esc`
  and the Ctrl combinations always work.
- **One polite live region** (`LiveRegion` mounted in `App`): level changes and provider state
  changes are derived from consecutive snapshots (`stores/changes.ts`), worded like the
  notification (`copy.levelChange`); user actions say "Updated 12:33", "Saved"; changes within
  2 s merge and the same sentence is not repeated within 60 s. A number that moves within a
  level announces nothing.
- **Header** is mounted (`Flashboard`, `Settings`), so Overview reaches Settings. One
  document title per view; focus moves to the view title on navigation, to a named section
  when asked (`?`), and back to the control that opened the view on `Esc`.
- **Names**: provider names from `copy.providerName` ("Claude", "DeepSeek"), plans as "Max
  5x", windows as "5-hour"; a card button's name is its visible text ("Claude Max 5x").
- **Copy and format** are real: `lib/format` (every case of `format-vectors.json`),
  `lib/copy.ts` (one sentence per `Unavailable`, `Setup`, stale state, unsupported provider),
  so timestamps follow the 12/24 h setting instead of `toLocaleString()`.
- **Overview** orders cards by severity then provider order, restores keyboard focus when a
  card moves, shows providers without a usage source as one quiet line, and has no card for a
  hidden provider.
- **Tokens**: `faint` is now 3.54 and 3.16 on paper and chip (light), 3.81 and 3.38 (dark);
  the dark `alert` is 5.47 and 4.85. `lib/design/contrast.test.ts` computes every pair from
  `tokens.css`.
- **Harness**: outside Tauri the built UI runs on a named mock script (`#fixture=mixed`,
  `lib/ipc/select.ts`); inside Tauri it is always the real client. The mock makes no request.
  The page clock is frozen at 12:33 Mon 14 Dec 2026 (Asia/Jakarta, en-US), and
  `expectNoAxeViolations` runs axe.
- **Polish (impeccable, better-ui)**: first dot aligned optically to the text edge; text
  buttons no longer sit 1 px right of the headings; the four Settings fields share one
  column; press feedback is exactly `scale(0.96)` and absent under reduced motion; elapsed
  windows say "Reset today 12:23 PM · waiting for your next request"; the number is "—", never
  invented.

## Audit by the four words of D1a

**Minimal: met.** No sidebar, cards, shadows or decorative icons; hairlines; two inks plus
one alert colour that appears only at warn and critical. One hierarchy per provider: number,
waffle, one quiet line. The header is one line.

**Detailed: met.** Level 0: provider, plan, and per window the percentage, the waffle, time
to reset, freshness. Level 1 is identical on hover and focus and is in the accessible text.
Level 2 is built: the history chart (a step line with threshold lines, reset hairlines and
gaps as gaps, a described image, a slider crosshair and a hidden data table), the diagnostics
in Settings, Copy and Retry, the first-run screen and every window on each overview card.

**Beautiful: met.** Serif numerals and titles, DM Sans UI text, one 4 px spacing scale,
hairlines, both themes reviewed in the baselines, motion limited to a 120 ms hover tint and
the press scale, absent under reduced motion. There is nothing nested, so concentric radii
do not apply yet. The impeccable detector's one warning is Instrument Serif, the face of the
chosen Waffle design (the brief wins).

**Accessible: met for what exists.** Zero axe violations of any impact on every fixture x
screen x theme (110 runs at 360 px) and on every route in both themes at 320 px and at 200 %
zoom; the findings below are fixed or listed.

## Findings

| #   | Finding                                                                                                             | Status                                                                                |
| --- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| 1   | No visible focus ring on any control                                                                                | Fixed (base.css)                                                                      |
| 2   | Padding 48 px made 320 px overflow                                                                                  | Fixed                                                                                 |
| 3   | Refresh button squeezed at 320 px                                                                                   | Fixed                                                                                 |
| 4   | Controls had default browser styling, hit area under 32 px                                                          | Fixed                                                                                 |
| 5   | Big numerals not tabular                                                                                            | Fixed                                                                                 |
| 6   | `faint` on `chip` 2.7:1 and 2.8:1                                                                                   | Fixed (tokens), tested                                                                |
| 7   | Dark `alert` on `chip` 4.47:1                                                                                       | Fixed (tokens), tested                                                                |
| 8   | Meter `aria-valuetext` lacked reset time, status and observed time                                                  | Fixed                                                                                 |
| 9   | Level-1 disclosure was `<details>`                                                                                  | Fixed (hover and focus, `Esc`)                                                        |
| 10  | Provider names lower case, raw window ids                                                                           | Fixed (`copy.providerName`, `windowName`)                                             |
| 11  | No shortcuts, zoom keys or live-region sentences                                                                    | Fixed                                                                                 |
| 12  | `Header` not mounted                                                                                                | Fixed                                                                                 |
| 13  | Screen tests reached only the pending state; axe was a stub                                                         | Fixed (fixtures in the built UI, frozen clock, real axe)                              |
| 14  | Timestamps used `toLocaleString()`                                                                                  | Fixed (`format`)                                                                      |
| 15  | Zoom field allowed 50 to 200                                                                                        | Fixed: 100 to 200 in steps of 25 (the shell snaps to those)                           |
| 16  | History chart and Diagnostics are placeholders                                                                      | Fixed (`HistoryChart`, `HistorySection`, `DiagnosticsPanel`; D9.4, D9.6)              |
| 17  | No Copy button for a login command, so no "Copied" announcement; no Retry on a failing card                         | Fixed (`CopyButton`, `CommandLine`, Retry; announced through the live region)         |
| 18  | No first-run screen                                                                                                 | Fixed (`FirstRun`, shown while `first_run_seen` is false; D9.5)                       |
| 19  | Overview shows the first window of each provider only                                                               | Fixed: every window, number and quiet line beside the waffle from 34 rem              |
| 20  | No validation text next to a threshold field                                                                        | Fixed: sentence tied by `aria-describedby`/`aria-invalid`, value restored on blur     |
| 21  | Focus kept on reorder and a level change in the live region cannot be driven in the built UI (the mock cannot push) | Logic unit-tested (`overview`, `changes`, `stores`); manual in the screen-reader pass |
| 22  | `forced-colors` is not tested                                                                                       | Fixed (`history.spec.ts`: axe, visible focus, no overflow, chart line drawn)          |
| 23  | The Claude live check had no switch or confirmation                                                                 | Fixed (Settings, native dialog, `Esc` closes it first)                                |
| 24  | WebKitGTK showed a hidden table wider than the window (a 1 px table still lays out at full width)                   | Fixed (the table sits in a clipped container); found by the 320 px test               |

Copy tone: short, plain, no exclamation marks; one table (`copy.ts`) for every failure.

Contrast (computed by `contrast.test.ts`, light and dark): ink 14.2 on paper and 12.6 on
chip; muted 6.2 and 5.5; alert 5.6 and 5.0 (light), 5.5 and 4.9 (dark); stale 5.6 and 5.0
(light), 7.6 and 6.8 (dark); faint (waffle cells, borders) 3.5 and 3.2 (light), 3.8 and 3.4
(dark); critical fill 3.7 and 3.3 (light).

## Files touched in the second pass

`src/App.svelte`, `src/main.ts`, `src/screens/{Overview,ProviderDetail,Settings}.svelte`,
`src/lib/{copy,escape,actions,overview,shortcuts}.ts`, `src/lib/router.svelte.ts`,
`src/lib/format/index.ts`, `src/lib/ipc/{fixtures,select}.ts`,
`src/lib/stores/{usage.svelte,clock.svelte,changes}.ts`,
`src/lib/components/primitives/{Meter,MeterRow}.svelte`,
`src/lib/components/shell/Header.svelte`, `src/lib/design/{tokens,base}.css`,
`vitest.config.ts` (two projects), `package.json` (lint and type-check caches),
`e2e/screens/{_harness/index.ts,polish.spec.ts,states.spec.ts,detail.spec.ts}`,
`playwright.config.ts` (time zone, locale), the baselines, and the new `*.test.ts` files.

Third pass (the remaining gaps): `src/lib/chart.ts`, `src/lib/diagnostics.ts`,
`src/lib/thresholds.ts`, `src/lib/stores/queries.ts`, `src/lib/test-support.ts`;
`src/lib/components/primitives/{HistoryChart,CopyButton,CommandLine}.svelte`,
`src/lib/components/shell/{HistorySection,DiagnosticsPanel,FirstRun}.svelte`;
changed: `src/App.svelte`, `src/screens/{Overview,ProviderDetail,Settings}.svelte`,
`src/lib/components/primitives/MeterRow.svelte`, `src/lib/{actions,copy}.ts`,
`src/lib/stores/{index,settings.svelte}.ts`, `src/lib/ipc/fixtures.ts` (a day of history and
sample diagnostics), `e2e/screens/history.spec.ts` and its baselines, the other baselines
(the Overview and detail layouts changed), and the new `*.test.ts` files. No dependency,
IPC type or Rust file changed.

Run: `pnpm lint`, `pnpm check`, `pnpm test` (coverage above every threshold of
`vitest.config.ts`) and `pnpm test:screens` (162 tests) pass.

The real-binary WebDriver run (`e2e/webdriver/webdriver.py`, owned by T12) needs two edits
now that the first-run screen exists and the threshold field corrects itself on blur: press
Continue before reading the overview, and type into the field instead of `clear`ing it
(WebDriver's `clear` blurs the field, which restores the saved value mid-edit). With both,
it passes in the real WebKitGTK view, with axe clean on the overview, detail (chart
included) and settings.

## D9.10 map

| D9.10 bullet            | Checked by                                                                                                                |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Contrast                | `lib/design/contrast.test.ts` (every text and graphic pair, both themes, paper and chip); axe in every screen spec        |
| Not colour alone        | The critical glyph in `Meter` (`Meter.test.ts`); level in words in the disclosure (`MeterRow.test.ts`)                    |
| Keyboard                | `shortcuts.test.ts` (every key, text fields, off switch, `Esc` order); `detail.spec.ts` (Tab order, every shortcut)       |
| Focus                   | Ring in `base.css`; `detail.spec.ts` (focus to the title, to Behaviour, back to the opener); axe in `states.spec.ts`      |
| Names and roles         | `detail.spec.ts` (one main, one h1, an h2 per card, meter name); `Meter.test.ts`; axe                                     |
| Label in name           | `detail.spec.ts` ("Claude Max 5x", "Codex Plus", "Claude 5-hour"); an automated all-controls comparison is not written    |
| Status messages         | `announce.test.ts` (merge, 60 s), `changes.test.ts` (level and state), `stores.test.ts`, `detail.spec.ts` ("Updated")     |
| Pointer (targets 32 px) | `base.css` (`--hit-min`); manual                                                                                          |
| Reflow and zoom         | `polish.spec.ts`: 320 CSS px and 200 % zoom, no horizontal scroll, all routes                                             |
| Forms                   | `Settings.test.ts` (sentence, `aria-describedby`, `aria-invalid`, value restored); `history.spec.ts` (invalid state, axe) |
| Updating content        | Manual: script 1.8; no element moves except a card reordering (restores focus)                                            |
| Page (lang, title)      | `scaffold.spec.ts` (`lang`); `detail.spec.ts` (title per view)                                                            |
| Motion                  | `base.css` reduced-motion block; `polish.spec.ts` runs with `reducedMotion: reduce`                                       |
| Forced colours          | `history.spec.ts` "forced colours": axe, visible focus outline, no overflow, chart line stroke; palette in `HistoryChart` |
| Disclosure              | `MeterRow.test.ts` and `detail.spec.ts`: hover, focus, stays over it, `Esc` closes without moving focus                   |
| Focus kept              | `Overview.svelte` restores focus after a reorder; logic in `overview.test.ts`; manual walk-through                        |

## README check against D8

Checked the privacy section against D8.3, D8.6 and D8.7: the files read, the two status
commands, the single DeepSeek host over HTTPS without proxy or redirect, no telemetry,
updater, runtime assets or link opening, and the honest limits match. The README does not
list `claude` as an opt-in child that talks to Anthropic (D8.7 point 3) or the
`MISE_*` constants by name; it says only that wrappers are set offline. No file says
"Jev Dashboard" or `dev.jev.dashboard`; the identifier is `dev.jev.flashboard`.
The binary and the `User-Agent` are still named `jev-dashboard`, which D8.7 specifies.

## For the screen-reader pass

`docs/screen-reader-script.md` was updated for the second pass. Lines 2.2 to 2.6 (chart
description, crosshair, range, meter selector) and 3.3 (validation text) now describe built
features. Lines 1.8, 1.9 and
2.4 cover what the built UI cannot drive (a level change in the live region, focus kept on
reorder): run them against a live gateway.
