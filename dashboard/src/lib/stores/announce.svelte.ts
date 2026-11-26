let current = $state("");
let queued: string[] = [];
const recent: Record<string, number> = {};
let timer: ReturnType<typeof setTimeout> | null = null;

export const announce = {
  get current(): string {
    return current;
  },
  say(message: string): void {
    const now = Date.now();
    const last = recent[message];
    if (last !== undefined && now - last < 60_000) return;
    recent[message] = now;
    queued.push(message);
    timer ??= setTimeout(() => {
      current = queued.join(". ");
      queued = [];
      timer = null;
    }, 2_000);
  },
};
