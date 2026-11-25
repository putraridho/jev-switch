// `Esc` has one order of precedence everywhere (DESIGN 9.1): it closes an open dialog, else
// an open level-1 disclosure, else it goes back one view. One press does one of these.

/** A layer that `Esc` can close; `true` from the handler means it closed something. */
export type EscapeLayer = "dialog" | "disclosure";

type Handler = () => boolean;

const layers: Record<EscapeLayer, Handler[]> = { dialog: [], disclosure: [] };

/** Registers a closer for as long as its layer is open; returns the function that removes it. */
export function pushEscape(layer: EscapeLayer, handler: Handler): () => void {
  layers[layer].push(handler);
  return () => {
    const at = layers[layer].indexOf(handler);
    if (at >= 0) layers[layer].splice(at, 1);
  };
}

/** Closes the topmost open layer, dialogs first. `false` when nothing was open. */
export function closeTopmost(): boolean {
  for (const layer of ["dialog", "disclosure"] as const) {
    const handlers = layers[layer];
    for (let i = handlers.length - 1; i >= 0; i -= 1) {
      if (handlers[i]?.() === true) return true;
    }
  }
  return false;
}
