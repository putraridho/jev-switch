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

## Audit by the four words of D1a

**Minimal: met.** No sidebar, cards, shadows or decorative icons; hairline rules; two
inks plus one alert colour (the alert colour appears only for warn and critical). One
hierarchy per provider: number, waffle, one quiet line.

**Detailed: not met, listed for the lead.** Level 0 is present. Level 1 is a native
`<details>` ("More details"), not the specified hover-or-focus disclosure with `Esc`;
the meter's `aria-valuetext` carries only "N% used", not the full detail sentence
(D9.10). Level 2 exists as a screen but History says "not available yet" and Diagnostics is
a placeholder. These are feature gaps in T09/T10, not polish; changing them changes
behaviour.

**Beautiful: met after polish.** Serif numerals and titles, DM Sans UI text, one 4 px
spacing scale, hairlines, both themes verified in screenshots. Gaps: radii are only on
controls (nothing nested, so concentric radii do not apply yet); no motion beyond a 120 ms
hover tint, absent under reduced motion.

**Accessible: met for what exists, with the findings below.**

## Findings

| #   | Finding                                                                                                                                                                | Status                                                                                         |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| 1   | No visible focus ring on any control                                                                                                                                   | Fixed (base.css)                                                                               |
| 2   | Padding 48 px made 320 px overflow                                                                                                                                     | Fixed                                                                                          |
| 3   | Refresh button squeezed at 320 px                                                                                                                                      | Fixed                                                                                          |
| 4   | Controls had default browser styling, hit area under 32 px                                                                                                             | Fixed                                                                                          |
| 5   | Big numerals not tabular                                                                                                                                               | Fixed                                                                                          |
| 6   | `faint` on `chip` is 2.7:1 (light) and 2.8:1 (dark), under the 3:1 needed for waffle cells                                                                             | Listed: the waffle sits on `paper` (3.0 and 3.2), so no failure today; do not put it on `chip` |
| 7   | Dark `alert` text on `chip` is 4.47:1, under 4.5:1                                                                                                                     | Listed: `alert` text is only on `paper` today (5.05)                                           |
| 8   | Meter `aria-valuetext` lacks reset time, status and observed time                                                                                                      | Listed (needs a component contract change, T14)                                                |
| 9   | Level-1 disclosure is `<details>`, not hover and focus with `Esc`                                                                                                      | Listed (T09)                                                                                   |
| 10  | Provider names show lower case ("claude", "Hide claude") and raw ids ("5h")                                                                                            | Listed: needs display labels from `copy.ts` (T09/T10)                                          |
| 11  | No shortcuts (`R`, `,`, `1`-`6`, `?`), no zoom keys, no live-region sentences for level changes                                                                        | Listed (T07/T09)                                                                               |
| 12  | `Header.svelte` is not mounted by `App.svelte`, so there is no way back to Settings from Overview                                                                      | Listed (T07)                                                                                   |
| 13  | Screen tests reach only the pending state: the mock fixtures are not selectable in the built UI (the harness ignores `fixture`), and `expectNoAxeViolations` is a stub | Listed (T07)                                                                                   |
| 14  | Timestamps use `toLocaleString()` rather than the 12/24 h setting                                                                                                      | Listed                                                                                         |

Copy tone: short, plain, no exclamation marks. "Checking…", "Offline" and "Not logged in"
state the fact without blame. Provider detail says "Unavailable: server error"; the Overview
uses sentence case labels for the same reason, so the two should share one table (finding 10).

Contrast (computed from tokens): ink 14.2 and 14.2, muted 6.2 and 6.2, alert 5.6 and 5.1
on `paper` (light, dark); critical fill 3.7 and 5.1.

## D9.10 map

| D9.10 bullet            | Checked by                                                                          |
| ----------------------- | ----------------------------------------------------------------------------------- |
| Contrast                | Computed above; the automated contrast test is not written (listed, finding 13)     |
| Not colour alone        | Manual: the critical marker glyph in `Meter`; screen-reader script 4.x              |
| Keyboard                | Manual keyboard walk-through (script section 5); shortcuts missing (finding 11)     |
| Focus                   | `polish.spec.ts` axe run; ring in `base.css`; manual check                          |
| Names and roles         | axe in `polish.spec.ts`; screen-reader script 0.x and 1.x                           |
| Label in name           | Manual: script 1.4 and 1.5; automated comparison not written                        |
| Status messages         | Screen-reader script 1.7 to 1.9; live-region test not written                       |
| Pointer (targets 32 px) | `base.css` (`--hit-min`); manual                                                    |
| Reflow and zoom         | `polish.spec.ts`: 320 CSS px and 200 % zoom, no horizontal scroll, all routes       |
| Forms                   | Manual: script 3.2 to 3.4; error text not built (settings show no validation text)  |
| Updating content        | Manual: script 1.8                                                                  |
| Page (lang, title)      | `scaffold.spec.ts` checks `lang="en"`; per-view document titles not built           |
| Motion                  | `base.css` reduced-motion block; `polish.spec.ts` runs with `reducedMotion: reduce` |
| Forced colours          | Manual only (not tested)                                                            |
| Disclosure              | Manual: script 1.x; not built to spec (finding 9)                                   |
| Focus kept              | Manual: script 2.4; card reordering not built                                       |

## README check against D8

Checked the privacy section against D8.3, D8.6 and D8.7: the files read, the two status
commands, the single DeepSeek host over HTTPS without proxy or redirect, no telemetry,
updater, runtime assets or link opening, and the honest limits match. The README does not
list `claude` as an opt-in child that talks to Anthropic (D8.7 point 3) or the
`MISE_*` constants by name; it says only that wrappers are set offline. No file says
"Jev Dashboard" or `dev.jev.dashboard`; the identifier is `dev.jev.flashboard`.
The binary and the `User-Agent` are still named `jev-dashboard`, which D8.7 specifies.

## For the screen-reader pass

`docs/screen-reader-script.md` is unchanged. Lines 2.2, 2.3, 3.3 and 4.x describe features
that are not built yet (chart description, validation text, stale and unreachable
messages), so they will be recorded as findings until T09 and T10 add them.
