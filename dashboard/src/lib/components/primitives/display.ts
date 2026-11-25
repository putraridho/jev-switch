// Types and pure helpers of the data-display primitives. They live outside the .svelte
// files so that tsc and the type-checked lint rules can see them.

/** S15: one meter, four sizes. `mini` is solid dots only, no outline, no marker. */
export type WaffleSize = "large" | "medium" | "small" | "mini";

/** S41: a word with an icon, flat. Alert only past the mark; amber only for stale. */
export type StatusKind = "near_limit" | "stale" | "healthy" | "no_source";

export interface DetailRow {
  label: string;
  value: string;
  /** The value is past a threshold. */
  alert?: boolean;
}

/** S26: the reading's age in words; stale from 5 minutes 1 second. */
export function freshness(
  atMs: number,
  nowMs: number,
): { text: string; stale: boolean } {
  const seconds = Math.max(0, Math.floor((nowMs - atMs) / 1000));
  const amount =
    seconds < 60
      ? `${String(seconds)}\u2009s`
      : seconds < 3600
        ? `${String(Math.floor(seconds / 60))} min`
        : `${String(Math.floor(seconds / 3600))} h`;
  return { text: `Updated ${amount} ago`, stale: seconds > 300 };
}

/** The glyphs an `IconButton` can draw (S11). */
export type IconName = "refresh" | "close" | "back" | "more";

/** One entry of a `Menu` (S38). */
export interface MenuItem {
  label: string;
  onselect: () => void;
  /** A key that does the same, drawn at the right. */
  shortcut?: string;
  destructive?: boolean;
  /** A separator above this item. */
  separated?: boolean;
}

/** A table column (S43). */
export interface TableColumn {
  label: string;
  /** Right aligned, in the mono face. */
  numeric?: boolean;
  /** How the column's cells read: the row's name is strong, secondary facts are muted. */
  tone?: "strong" | "muted";
}

/** One table cell; `alert` marks a figure past the warning mark. */
export interface TableCell {
  text: string;
  tone?: "alert" | "muted";
}

export type SortDirection = "ascending" | "descending";
