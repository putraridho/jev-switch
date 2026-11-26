// The first import of the entry module (DESIGN 9.7): set the theme on <html> from the URL
// fragment before anything renders, so a dark override never flashes the light theme.
if (typeof document !== "undefined" && typeof window !== "undefined") {
  const theme = new URLSearchParams(window.location.hash.slice(1)).get("theme");
  if (theme === "light" || theme === "dark") {
    document.documentElement.dataset["theme"] = theme;
  }
}
export {};
