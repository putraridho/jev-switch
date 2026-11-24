# Screen-reader pass (Orca, installed app, real WebKitGTK view)

Run by the user with the lead on the **installed** app (`install.sh`, then launch from the
desktop entry). Start Orca (`orca`), then the app without `--hidden`. For each line press
the keys, tick the box if Orca says what is written, otherwise write what it said.

Fixture: use the isolated launch (stub `codex`/`claude`, temp `HOME`) so the numbers are
known. Fill in the expected numbers from that fixture where a line says `<n>`.

Every difference is a finding and goes to the lead. v1 is not done until none is open.

## 0. The D16 assumption: `role="meter"`

| #   | Keys                       | Orca must say                                                          | Said instead |
| --- | -------------------------- | ---------------------------------------------------------------------- | ------------ |
| 0.1 | Tab to the first meter row | "Claude 5-hour", then a meter or progress bar role                     | [ ]          |
| 0.2 | Stay on it                 | "<n> percent used, resets in …, today at …, status allowed, seen at …" | [ ]          |
| 0.3 | If 0.1 or 0.2 are silent   | Fallback needed: same text in a hidden element via `aria-describedby`  | [ ]          |

## 1. Overview

| #   | Keys                       | Orca must say                                                           | Said instead |
| --- | -------------------------- | ----------------------------------------------------------------------- | ------------ |
| 1.1 | Launch                     | Window title "Flashboard: Overview"                                     | [ ]          |
| 1.2 | `Tab` (first stop)         | Refresh button, named in words, not "button" alone                      | [ ]          |
| 1.3 | `Tab`                      | Settings button, named in words                                         | [ ]          |
| 1.4 | `Tab`                      | A card header: begins "Claude, Max 5x" then the summary                 | [ ]          |
| 1.5 | `Tab` through the meters   | Each meter: provider and window name, then the full detail (as 0.2)     | [ ]          |
| 1.6 | `Tab` order                | Matches the reading order top to bottom; no stop is skipped or repeated | [ ]          |
| 1.7 | `R`                        | "Updated <time>" once                                                   | [ ]          |
| 1.8 | Wait for 20 number updates | Nothing is announced while no level changes                             | [ ]          |
| 1.9 | Cause a level change       | One sentence naming provider and new level, once                        | [ ]          |

## 2. Provider detail

| #   | Keys                        | Orca must say                                                    | Said instead |
| --- | --------------------------- | ---------------------------------------------------------------- | ------------ |
| 2.1 | `Enter` on a card header    | Title "Flashboard: <provider>", focus on the view heading        | [ ]          |
| 2.2 | `Tab` to the chart          | A description of the history (range, lowest, highest, latest)    | [ ]          |
| 2.3 | Arrow keys on the crosshair | The value and time at each step                                  | [ ]          |
| 2.4 | `Esc`                       | Back on Overview, focus on the card you came from                | [ ]          |
| 2.5 | `1` to `6`                  | Opens the provider in the fixed order; a hidden one does nothing | [ ]          |

## 3. Settings

| #   | Keys                        | Orca must say                                                             | Said instead |
| --- | --------------------------- | ------------------------------------------------------------------------- | ------------ |
| 3.1 | `,`                         | Title "Flashboard: Settings"                                              | [ ]          |
| 3.2 | `Tab` to the Warn field     | Label "Warn", its value, and its allowed range                            | [ ]          |
| 3.3 | Type a value above Critical | An error read with the field: "Warn must be below critical (95)"; invalid | [ ]          |
| 3.4 | Correct it, `Tab` away      | "Saved" once                                                              | [ ]          |
| 3.5 | `Tab` through the toggles   | Each says its label and on or off                                         | [ ]          |
| 3.6 | `?`                         | Opens Settings at Behaviour; every shortcut is listed as text             | [ ]          |
| 3.7 | Copy the login command      | "Copied" once                                                             | [ ]          |
| 3.8 | `Esc`                       | Back one view; with a dialog open, closes the dialog first                | [ ]          |

## 4. States

| #   | Setup                              | Orca must say                                                        | Said instead |
| --- | ---------------------------------- | -------------------------------------------------------------------- | ------------ |
| 4.1 | A source that cannot be reached    | The card says it cannot reach the provider, in words; announced once | [ ]          |
| 4.2 | A provider not logged in           | The login instruction is readable and the command is copyable        | [ ]          |
| 4.3 | Stale data                         | The card says how old the data is, not just a colour                 | [ ]          |
| 4.4 | Reduced motion on                  | Nothing animates; speech is unchanged                                | [ ]          |
| 4.5 | Zoom 200 % (`Ctrl` `+` four times) | Reading order and names unchanged; no clipped text                   | [ ]          |

## 5. Keyboard-only walk-through (no Orca)

Complete everything in sections 1 to 3 with the keyboard alone. Tick when no step needed a
pointer and focus was never lost or trapped (a dialog traps only while open and returns
focus on close). [ ]
