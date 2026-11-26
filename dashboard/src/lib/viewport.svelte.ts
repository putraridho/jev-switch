// The one layout breakpoint script needs to know about (S15, S51): from 900 px up a meter
// uses the large waffle, below it the medium one with the number beside it.

const query = "(min-width: 56.25rem)";

const media =
  typeof window.matchMedia === "function" ? window.matchMedia(query) : null;

let wide = $state(media?.matches ?? false);
media?.addEventListener("change", (event) => {
  wide = event.matches;
});

export const viewport = {
  /** The window is at least 900 CSS px wide. */
  get wide(): boolean {
    return wide;
  },
};
