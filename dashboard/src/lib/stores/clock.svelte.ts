let now = $state(Date.now());

/** Ticks `now` once a second while the page is visible; returns the function that stops it. */
export function startClock(): () => void {
  const timer = setInterval(() => {
    if (!document.hidden) now = Date.now();
  }, 1_000);
  const onVisible = (): void => {
    if (!document.hidden) now = Date.now();
  };
  document.addEventListener("visibilitychange", onVisible);
  return () => {
    clearInterval(timer);
    document.removeEventListener("visibilitychange", onVisible);
  };
}

if (typeof document !== "undefined") startClock();

export const clock = {
  get now(): number {
    return now;
  },
};
