// The UI's clock (docs/dashboard/DESIGN.md 7.2): epoch milliseconds, ticking on the minute
// and only while the document is visible. The shape is final; T07 writes the body.

/** "Now" for countdowns and ages. */
export const clock: { readonly now: number } = { now: 0 };
