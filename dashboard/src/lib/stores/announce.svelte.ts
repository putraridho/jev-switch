// The single polite live region's queue (docs/dashboard/DESIGN.md 9.10): merges messages
// within 2 s and drops a repeat within 60 s. The shape is final; T07 writes the body.

/** What the live region says. */
export const announce: {
  say(message: string): void;
  readonly current: string;
} = {
  say: () => {
    return;
  },
  current: "",
};
