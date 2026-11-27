// DESIGN 9.10 "Contrast": computed from the token file, in both themes, on every surface a
// colour is used on (paper, chip). Text 4.5:1 (primary text 7:1); the waffle's empty cells,
// the filled critical cells and the control boundaries 3:1.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(
  resolve(process.cwd(), "src/lib/design/tokens.css"),
  "utf8",
);

type Palette = Record<string, string>;

/** The `--name: #rrggbb` declarations of the block that follows `selector`. */
function palette(selector: string): Palette {
  const start = css.indexOf(selector);
  if (start < 0) throw new Error(`no block for ${selector}`);
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  const found: Palette = {};
  for (const match of css
    .slice(open, close)
    .matchAll(/--([a-z-]+):\s*(#[0-9a-fA-F]{6})/g)) {
    const name = match[1];
    const value = match[2];
    if (name !== undefined && value !== undefined) found[name] = value;
  }
  return found;
}

function channel(hex: string, at: number): number {
  const v = parseInt(hex.slice(at, at + 2), 16) / 255;
  return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

function luminance(hex: string): number {
  return (
    0.2126 * channel(hex, 1) +
    0.7152 * channel(hex, 3) +
    0.0722 * channel(hex, 5)
  );
}

function ratio(a: string, b: string): number {
  const first = luminance(a);
  const second = luminance(b);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

const light = palette(":root {");
const dark = palette(':root[data-theme="dark"]');
const themes: readonly { name: string; colors: Palette }[] = [
  { name: "light", colors: light },
  { name: "dark", colors: dark },
];
const surfaces = ["color-paper", "color-chip"] as const;

function colour(colors: Palette, name: string): string {
  const value = colors[name];
  if (value === undefined) throw new Error(`missing token ${name}`);
  return value;
}

describe("token contrast", () => {
  for (const theme of themes) {
    const name = theme.name;
    const colors = theme.colors;
    for (const surface of surfaces) {
      const bg = colour(colors, surface);
      it(`${name}: primary text is 7:1 on ${surface}`, () => {
        expect(ratio(colour(colors, "color-ink"), bg)).toBeGreaterThanOrEqual(
          7,
        );
      });
      for (const text of ["color-muted", "color-alert", "color-stale"]) {
        it(`${name}: ${text} is 4.5:1 on ${surface}`, () => {
          expect(ratio(colour(colors, text), bg)).toBeGreaterThanOrEqual(4.5);
        });
      }
      for (const graphic of ["color-faint", "color-critical-fill"]) {
        it(`${name}: ${graphic} is 3:1 on ${surface}`, () => {
          expect(ratio(colour(colors, graphic), bg)).toBeGreaterThanOrEqual(3);
        });
      }
    }
  }

  // D9.4 / D9.10: the chart's line, threshold lines and hairlines against the plot
  // background are 3:1 or better in both themes, on every surface the chart can sit on.
  for (const theme of themes) {
    for (const surface of surfaces) {
      const marks: readonly { mark: string; token: string }[] = [
        { mark: "history line", token: "color-ink" },
        { mark: "warn threshold", token: "color-stale" },
        { mark: "critical threshold", token: "color-alert" },
        { mark: "reset hairline", token: "color-faint" },
        { mark: "axis words", token: "color-muted" },
      ];

      for (const entry of marks) {
        it(`${theme.name}: chart ${entry.mark} is 3:1 on ${surface}`, () => {
          expect(
            ratio(
              colour(theme.colors, entry.token),
              colour(theme.colors, surface),
            ),
          ).toBeGreaterThanOrEqual(3);
        });
      }
    }
  }

  // The ink surface (tooltips, the floating meter detail): paper text 7:1, the muted labels
  // and the alert value 4.5:1, in both themes.
  for (const theme of themes) {
    const ink = colour(theme.colors, "color-ink");
    it(`${theme.name}: paper text is 7:1 on the ink surface`, () => {
      expect(
        ratio(colour(theme.colors, "color-paper"), ink),
      ).toBeGreaterThanOrEqual(7);
    });
    for (const text of ["color-on-ink-muted", "color-on-ink-alert"]) {
      it(`${theme.name}: ${text} is 4.5:1 on the ink surface`, () => {
        expect(ratio(colour(theme.colors, text), ink)).toBeGreaterThanOrEqual(
          4.5,
        );
      });
    }
  }

  it("the system-dark block repeats the explicit dark theme", () => {
    const media = palette(':root:not([data-theme="light"])');
    expect(media).toEqual(dark);
  });
});
