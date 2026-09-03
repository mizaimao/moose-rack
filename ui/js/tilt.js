// Cards that turn to face the cursor.
//
// The angle is written to two custom properties and the stylesheet does the
// rest, so this file never touches `style` beyond those two numbers and the
// look stays where every other look is.
//
// Bound on the container, not on each card: the Continue playing strip is
// rebuilt whenever the platform screen is drawn, and a listener per card would
// have to be re-attached every time — the same delegated-listener reason the
// game grid uses one listener for two thousand rows.

/// How far it turns at the very edge of a card, in degrees.
///
/// Small on purpose. A page held in one hand moves a few degrees; past about
/// fifteen the card stops looking like an object and starts looking like a bug.
const MAX = 13.5;

export function installTilt(root, { selector = ".gcard", max = MAX } = {}) {
  if (!root) return;

  const set = (card, rx, ry) => {
    card.style.setProperty("--rx", `${rx.toFixed(2)}deg`);
    card.style.setProperty("--ry", `${ry.toFixed(2)}deg`);
  };

  root.addEventListener("pointermove", (ev) => {
    // Only a real mouse. A finger is already on the thing it is pointing at,
    // and a pad has no pointer at all — tilting under either is motion nobody
    // asked for.
    if (ev.pointerType !== "mouse") return;
    const card = ev.target.closest?.(selector);
    if (!card || !root.contains(card)) return;
    const r = card.getBoundingClientRect();
    if (!r.width || !r.height) return;
    // -1..1 from the center, so the sign falls out and the edges are the
    // extremes without a second scaling step.
    const px = (ev.clientX - r.left) / r.width * 2 - 1;
    const py = (ev.clientY - r.top) / r.height * 2 - 1;
    // Y movement tips the top away from you, which is rotateX *negative* —
    // getting this backwards makes the card lean into the cursor and feels
    // immediately wrong without being easy to name.
    card.classList.add("tilt");
    set(card, -py * max, px * max);
  });

  const clear = (ev) => {
    const card = ev.target.closest?.(selector);
    if (!card) return;
    card.classList.remove("tilt");
    // Left at the last angle the CSS transition can ease from; removing the
    // class restores the plain hover transform, which animates back.
    set(card, 0, 0);
  };
  root.addEventListener("pointerleave", clear, true);
  root.addEventListener("pointerdown", clear);
}
