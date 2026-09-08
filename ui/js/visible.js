// Draw only the rows you can see.
//
// The arcade console is 2,506 games and every one of them used to be inserted
// into the document on each platform switch. This keeps a band of them around
// the viewport and stands two spacers in for the rest — one above, one below,
// each the height of the rows it replaces, so the scrollbar and every scroll
// position stay exactly what they would have been.
//
// It works because a grid of covers is *uniform*: `.gcards` is
// `repeat(auto-fill, var(--gcard))`, so every column is the same width, and
// one `--ar` on the container shapes every card in it. Given the column count
// and one row's height, where any row sits is arithmetic rather than a
// measurement — which is also what lets the cursor move to a row that is not
// drawn.
//
// Only the flat list is windowed. Grouped search results are capped at 200 by
// the backend and a grouped collection is sections of a few hundred; below the
// threshold the whole thing is drawn, exactly as before, because a window over
// a short list is machinery with nothing to do.

/// Below this many rows, draw the lot.
///
/// Well above a screenful at any zoom, so nothing anybody can see at once is
/// ever windowed. The point is the two-thousand-row case.
export const THRESHOLD = 400;

/// How far beyond the viewport to keep drawn, as a multiple of its height, in
/// each direction.
///
/// Not a tuning knob so much as a guarantee: a whole screen of overscan means
/// the cursor's next stop is always already drawn, whichever direction it goes
/// and however fast a held key repeats. Paging moves three rows, which is well
/// inside it.
export const OVERSCAN = 1.5;

/// Which rows to draw, and how much empty space to leave for the rest.
///
/// `top` is how far the container's first row has been scrolled past — negative
/// while the list is still below the top of the viewport. Everything is in
/// whole rows, so the band always starts on a row boundary and the spacers are
/// always an exact number of rows: half a row of error is a grid that jumps by
/// half a row as you scroll.
///
/// Pure arithmetic, and separate from anything that touches the page, because
/// this is the part that can be wrong in ways nobody sees — a band one row
/// short looks like a list with a hole in it, and only at one scroll position.
export function slice({ total, columns, rowHeight, top, viewport, overscan = OVERSCAN }) {
  const cols = Math.max(1, Math.floor(columns) || 1);
  const rows = Math.ceil(total / cols);
  if (!total || !(rowHeight > 0)) {
    return { first: 0, count: total, before: 0, after: 0, rows };
  }
  const margin = viewport * overscan;
  const firstRow = clamp(Math.floor((top - margin) / rowHeight), 0, Math.max(0, rows - 1));
  const lastRow = clamp(
    Math.ceil((top + viewport + margin) / rowHeight),
    firstRow + 1,
    rows
  );
  const first = firstRow * cols;
  return {
    first,
    count: Math.min(total - first, (lastRow - firstRow) * cols),
    before: firstRow * rowHeight,
    after: (rows - lastRow) * rowHeight,
    rows,
  };
}

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(v, hi));
}

/// Whether a list this long is worth windowing.
export function worthWindowing(total) {
  return total > THRESHOLD;
}

/// The row band currently drawn, so navigation can tell whether the row it
/// wants is on the page. Null when nothing is windowed.
let live = null;

export function windowedList() {
  return live;
}

export function stopWindowing() {
  live?.stop();
  live = null;
}

/// Window `rows` into `container`, which must already be in the document.
///
/// `html(row, index)` draws one card or one row. `onDraw` is called after every
/// band change, so whatever depends on which nodes exist — the cover
/// observers, the cursor's map of the page — can be brought back into step.
export function windowRows({ container, scroller, rows, html, onDraw }) {
  stopWindowing();

  /// Which of `rows` are in play, as indices into it.
  ///
  /// All of them until the filter box narrows the list. Kept as a view rather
  /// than a second array so the window never holds a copy of anything, and so
  /// `narrow` is a pass over booleans rather than a rebuild.
  let view = rows.map((_, i) => i);

  const before = document.createElement("div");
  const after = document.createElement("div");
  before.className = "vspace";
  after.className = "vspace";
  container.replaceChildren(before, after);

  // Enough to measure with. One band is drawn from a guess, then the real
  // shape is read off it — a card's height cannot be known before a card has
  // been drawn, and estimating it from the zoom would be a second copy of the
  // stylesheet's arithmetic.
  let shape = { columns: 1, rowHeight: 0 };
  let drawn = null;

  /// Where the container's first row sits inside the scrollable content.
  ///
  /// Not `container.offsetTop`, which is measured from whichever ancestor
  /// happens to be positioned — the page, here, so it carries the height of
  /// the header and the tab bar with it and every band would sit that far
  /// wrong. Worked out from the two rectangles instead, and only when the page
  /// changes shape: reading a rectangle forces a layout, and this cannot be on
  /// the scroll path.
  let origin = 0;
  const findOrigin = () => {
    origin =
      scroller.scrollTop +
      (container.getBoundingClientRect().top - scroller.getBoundingClientRect().top);
  };

  const measure = () => {
    const cols = readColumns(container);
    const card = container.querySelector(".gcard, .row");
    if (!card) return false;
    const gap = Number.parseFloat(getComputedStyle(container).rowGap) || 0;
    const height = card.offsetHeight + gap;
    if (!(height > 0)) return false;
    const changed = cols !== shape.columns || Math.abs(height - shape.rowHeight) > 0.5;
    shape = { columns: cols, rowHeight: height };
    return changed;
  };

  const paint = (band) => {
    before.style.height = `${band.before}px`;
    after.style.height = `${band.after}px`;
    // Keep the cards that are still in the band.
    //
    // A band is a contiguous run of indices with one card per index, in order,
    // so scrolling by two rows is two rows off one end and two onto the other.
    // Rebuilding all of it -- which is what this did, on every row boundary --
    // threw away every card's artwork along with it: measured on SNES on
    // 2026-09-08, 18% of the cards on screen were holding a picture during a
    // scroll, and every one of them had to be asked for and decoded again.
    // Nothing about the window's arithmetic changes; only which nodes survive.
    const nf = band.first;
    const ne = band.first + band.count;
    const of = drawn ? drawn.first : 0;
    const oe = drawn ? drawn.first + drawn.count : 0;
    // Elements only. `html` returns markup that starts on a new line, so the
    // band is cards with whitespace text nodes between them -- walking every
    // sibling counted twice as many and the length check below never held, so
    // this rebuilt the lot every time and looked exactly like it had not been
    // written.
    const held = [];
    if (drawn) {
      for (let n = before.nextElementSibling; n && n !== after; n = n.nextElementSibling) {
        held.push(n);
      }
    }
    const keepFrom = Math.max(nf, of);
    const keepTo = Math.min(ne, oe);
    const markup = (from, to) => {
      const out = [];
      for (let i = from; i < to; i++) out.push(html(rows[view[i]], i));
      return out.join("");
    };
    // A jump has nothing in common with what is drawn -- the first band, a
    // filter, the cursor revealing a row far away -- and so is cheaper whole.
    // `held.length` is the check that this file's one assumption still holds:
    // one element per index. A caller whose markup grew a second root would
    // otherwise shift every card by one and quietly draw the wrong games.
    if (!drawn || keepFrom >= keepTo || held.length !== oe - of) {
      while (before.nextSibling && before.nextSibling !== after) before.nextSibling.remove();
      after.insertAdjacentHTML("beforebegin", markup(nf, ne));
      drawn = band;
      return;
    }
    for (let i = 0; i < keepFrom - of; i++) held[i].remove();
    for (let i = 0; i < oe - keepTo; i++) held[held.length - 1 - i].remove();
    if (nf < keepFrom) before.insertAdjacentHTML("afterend", markup(nf, keepFrom));
    if (ne > keepTo) after.insertAdjacentHTML("beforebegin", markup(keepTo, ne));
    drawn = band;
  };

  const update = (force) => {
    if (!container.isConnected) return;
    const band = slice({
      total: view.length,
      columns: shape.columns,
      rowHeight: shape.rowHeight,
      top: scroller.scrollTop - origin,
      viewport: scroller.clientHeight,
    });
    // Nothing crossed a row boundary, so the same rows are still the right
    // ones. Scrolling fires far more often than the band actually changes.
    if (!force && drawn && band.first === drawn.first && band.count === drawn.count) return;
    paint(band);
    onDraw?.(band);
  };

  // The first band, from the guess, purely to have something to measure.
  paint({ first: 0, count: Math.min(view.length, 60), before: 0, after: 0 });
  findOrigin();
  if (measure()) drawn = null;
  update(true);
  // A second pass: the first real band may be a different shape from the
  // sixty cards that were drawn to measure with — a taller card, or a column
  // count that only settles once the grid is full.
  if (measure()) update(true);

  const onScroll = () => update(false);
  scroller.addEventListener("scroll", onScroll, { passive: true });
  const onResize = () => {
    findOrigin();
    if (measure()) update(true);
    else update(false);
  };
  window.addEventListener("resize", onResize);

  // The container can change width without the window doing anything, and
  // measuring only on `resize` missed every one of those. Opening the detail
  // pane is the ordinary case and it is not a small error: measured on
  // 2026-09-08 with the pane open, this held four columns of 217px rows while
  // still believing eight columns of 274px. Everything downstream is arithmetic
  // on those two numbers -- which rows the band is, how tall the spacers are,
  // where the cursor thinks a row is -- so the band sat below the viewport with
  // no overscan above it at all, and scrolling up showed placeholders because
  // the rows above had never been drawn.
  //
  // Width only. A `ResizeObserver` on this container also fires for the height
  // the band itself changes, and reacting to that is a loop.
  let lastWidth = container.clientWidth;
  const ro = typeof ResizeObserver === "function"
    ? new ResizeObserver(() => {
        const width = container.clientWidth;
        if (Math.abs(width - lastWidth) < 1) return;
        lastWidth = width;
        onResize();
      })
    : null;
  ro?.observe(container);

  live = {
    /// How many rows the cursor can visit: what the filter box left, not what
    /// the list holds.
    get total() { return view.length; },
    get columns() { return shape.columns; },
    get rowHeight() { return shape.rowHeight; },
    get band() { return drawn; },
    /// The row at a place in the list, for a caller that has an index and
    /// wants the game.
    at(index) { return rows[view[index]] ?? null; },
    /// Every name in the list, in order — what the filter box searches. All of
    /// them, not the drawn ones: a filter that only matched what happened to
    /// be on screen would be a filter that finds less the further down you
    /// have scrolled.
    names() { return rows.map((r) => r.name ?? ""); },
    /// Keep only the rows `visible` marks true, and draw from the top.
    ///
    /// The cursor moves through what is left, because `view` is what every
    /// index here means — so a filtered list navigates like a short list
    /// rather than skipping over holes.
    narrow(visible) {
      view = visible ? rows.map((_, i) => i).filter((i) => visible[i]) : rows.map((_, i) => i);
      drawn = null;
      scroller.scrollTop = Math.min(scroller.scrollTop, container.offsetTop);
      update(true);
      return view.length;
    },
    /// Measure the cards again and redraw.
    ///
    /// For the zoom slider, which changes the width of a card and therefore
    /// the column count and the row height, without the window ever changing
    /// size — so the resize listener never hears about it.
    remeasure() {
      findOrigin();
      measure();
      update(true);
    },
    rows,
    container,
    scroller,
    /// Bring row `index` onto the page, drawing the band around it if it is
    /// not there already, and hand back its node.
    reveal(index) {
      if (index < 0 || index >= view.length) return null;
      if (!drawn || index < drawn.first || index >= drawn.first + drawn.count) {
        const row = Math.floor(index / Math.max(1, shape.columns));
        scroller.scrollTop = origin + row * shape.rowHeight - scroller.clientHeight / 2;
        update(true);
      }
      return container.querySelector(`[data-at="${index}"]`);
    },
    stop() {
      scroller.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onResize);
      ro?.disconnect();
    },
  };
  return live;
}

/// How many columns the grid resolved to.
///
/// Read off the browser rather than worked out from the zoom: the stylesheet
/// decides this — `repeat(auto-fill, var(--gcard))` against the container's
/// width — and re-deriving it here would be a second copy of that sum, which
/// would disagree the first time either changed.
function readColumns(container) {
  const template = getComputedStyle(container).gridTemplateColumns;
  if (!template || template === "none") return 1;
  return Math.max(1, template.split(/\s+/).filter(Boolean).length);
}
