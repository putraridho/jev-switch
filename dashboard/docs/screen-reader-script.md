# Screen-reader pass (Orca, installed app, real WebKitGTK view)

Run by the user with the lead on the **installed** app (`install.sh`, then launch from the
desktop entry). Start Orca (`orca`), then the app without `--hidden`. For each line press
the keys, tick the box if Orca says what is written, otherwise write what it said.

Fixture: use the isolated launch (stub `codex`/`claude`, temp `HOME`) so the numbers are
known. Fill in the expected numbers from that fixture where a line says `<n>`.

Every difference is a finding and goes to the lead. v1 is not done until none is open.

## 0. The D16 assumption: `role="meter"`

| #   | Keys                       | Orca must say                                                                                                              | Said instead |
| --- | -------------------------- | -------------------------------------------------------------------------------------------------------------------------- | ------------ |
| 0.1 | Tab to the first meter row | "Claude 5-hour", then a meter or progress bar role                                                                         | [ ]          |
| 0.2 | Stay on it                 | "<n> percent used, resets in 2 hours 14 minutes, today at …, status allowed, seen at …" (one decimal: "50.0 percent used") | [ ]          |
| 0.3 | If 0.1 or 0.2 are silent   | Fallback needed: same text in a hidden element via `aria-describedby`                                                      | [ ]          |

## 1. Overview

| #    | Keys                                               | Orca must say                                                                     | Said instead |
| ---- | -------------------------------------------------- | --------------------------------------------------------------------------------- | ------------ |
| 1.1  | Launch                                             | Window title "Flashboard: Overview"                                               | [ ]          |
| 1.2  | `Tab` (first stop)                                 | "Flashboard", a button, current page                                              | [ ]          |
| 1.3  | `Tab`                                              | "Settings", a button                                                              | [ ]          |
| 1.4  | `Tab` twice more                                   | "Refresh", then the card header: "Claude Max 5x" (the visible words, in order)    | [ ]          |
| 1.5  | `Tab` through the meters                           | Each meter: provider and window name, then the full detail (as 0.2)               | [ ]          |
| 1.6  | `Tab` order                                        | Matches the reading order top to bottom; no stop is skipped or repeated           | [ ]          |
| 1.7  | `R` (focus not in a field)                         | "Updated <time>" once                                                             | [ ]          |
| 1.8  | Wait for 20 number updates                         | Nothing is announced while no level changes                                       | [ ]          |
| 1.9  | Cause a level change                               | One sentence naming provider and new level, once                                  | [ ]          |
| 1.10 | Tab to a meter, wait                               | Nothing extra is read: the detail is the meter's value text and is not read twice | [ ]          |
| 1.11 | `Esc` on a focused meter                           | The detail closes; focus stays on the meter; the view does not change             | [ ]          |
| 1.12 | Settings, uncheck "Single-key shortcuts", then `R` | Nothing happens; `Esc` still goes back                                            | [ ]          |

### First run (a fresh state, `first_run_seen` false)

| #    | Keys                       | Orca must say                                                                     | Said instead |
| ---- | -------------------------- | --------------------------------------------------------------------------------- | ------------ |
| 1.13 | Launch                     | The heading "Usage, in one place." is read; the list says what was found in words | [ ]          |
| 1.14 | `Tab` to "Start at login"  | "Start at login, checked"; Space unchecks it and no other change is spoken        | [ ]          |
| 1.15 | `Tab`, `Enter` on Continue | Focus lands on the heading "Usage at a glance", which is read                     | [ ]          |

## 2. Provider detail

| #   | Keys                                                     | Orca must say                                                                             | Said instead |
| --- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ------------ |
| 2.1 | `Enter` on a card header                                 | Title "Flashboard: <provider>", focus on the view heading, which is read                  | [ ]          |
| 2.2 | `Tab` to the range, then to "Inspect history"            | The range group "Range" with the chosen range; then a slider named "Inspect history"      | [ ]          |
| 2.3 | Left and Right, Home, End on the slider                  | Each press says the time and the percent ("9:40 AM, 72 percent"); not read twice          | [ ]          |
| 2.4 | `Esc`                                                    | Back on Overview, focus on the card header you came from, which is read                   | [ ]          |
| 2.5 | `1` to `6`                                               | Opens the provider in the fixed order; a hidden one does nothing                          | [ ]          |
| 2.6 | Browse to the chart image                                | One description: "5-hour usage over the last day: peak 72% at 9:40 AM, now 50%, 4 resets" | [ ]          |
| 2.7 | Browse into the data table after the chart               | A caption (the same sentence) and a Time and a Value for each point                       | [ ]          |
| 2.8 | `Tab` to "Meter" (a provider with two meters), change it | The new meter's history replaces the chart; its description names the meter               | [ ]          |
| 2.9 | On a failing card, `Tab` to Retry, `Enter`               | "Retry", then "Checking Codex again" once                                                 | [ ]          |

## 3. Settings

| #    | Keys                                            | Orca must say                                                                                                                                   | Said instead |
| ---- | ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| 3.1  | `,`                                             | Title "Flashboard: Settings"; the heading "Settings" is read                                                                                    | [ ]          |
| 3.2  | `Tab` to the Warning field                      | Label "Warning (%)", its value, and its allowed range                                                                                           | [ ]          |
| 3.3  | Type a value above Critical                     | An error read with the field: "Warn must be below critical (95)"; invalid                                                                       | [ ]          |
| 3.4  | Correct it, `Tab` away                          | "Saved" once                                                                                                                                    | [ ]          |
| 3.5  | `Tab` through the toggles                       | Each says its label and on or off                                                                                                               | [ ]          |
| 3.6  | `?`                                             | Opens Settings; the heading "Behaviour" is read; Tab reads "Single-key shortcuts" and the list of keys                                          | [ ]          |
| 3.7  | `Tab` to a login command's Copy button, `Enter` | "Copy codex login", then "Copied" once                                                                                                          | [ ]          |
| 3.8  | `Esc`                                           | Back one view; with a dialog open, closes the dialog first                                                                                      | [ ]          |
| 3.9  | `Tab` to "Claude live check", press Space       | A dialog "Turn on the Claude live check?" is announced; focus is on Cancel; `Tab` stays inside; `Esc` closes it and focus returns to the switch | [ ]          |
| 3.10 | `Enter` on "About and diagnostics"              | It expands; the version and the history size are read, then each source with its last success and last problem                                  | [ ]          |

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
