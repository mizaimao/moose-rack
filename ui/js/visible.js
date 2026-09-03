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
    // Everything between the two spacers is the old band.
    while (before.nextSibling && before.nextSibling !== after) before.nextSibling.remove();
    const markup = [];
    for (let i = band.first; i < band.first + band.count; i++) {
      markup.push(html(rows[view[i]], i));
    }
    after.insertAdjacentHTML("beforebegin", markup.join(""));
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
