let now = $state(Date.now());
if (typeof document !== "undefined") {
  setInterval(() => {
    if (!document.hidden) now = Date.now();
  }, 1_000);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) now = Date.now();
  });
}

export const clock = {
  get now(): number {
    return now;
  },
};
