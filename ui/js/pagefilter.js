// Search inside the page you are looking at.
//
// Not the header's search box, which spans the whole library and takes you to
// a different screen. This one narrows what is already in front of you —
// thirty-five consoles, twenty-seven collections, two and a half thousand
// arcade games — and leaves you where you are.
//
// It lived inside the collections list, drawn as part of that pane, which is
// why it only existed there and why it sat in the middle of a page of cards
// looking like something that had fallen out of a dialog. It is furniture of
// the tab row now: one box, always in the same place, filtering whatever the
// row is pointing at.

import { el, invoke } from "./state.js";
import { windowedList } from "./visible.js";

/// Hidden by the filter. A class rather than the `hidden` attribute so nothing
/// else that hides things — a view emptying itself, a card with no artwork —
/// gets confused with a search that did not match.
const OUT = "filtered-out";

/// What a row or card is called, for matching.
///
/// Every list here puts its name in one of these, and the fallback is the
/// node's own text — which catches the ones that do not, at the cost of also
/// matching a game's platform or its size. That is the right way round: a
/// filter that misses things is worse than one that is generous.
function nameOf(node) {
  const named = node.querySelector(".name, .nm, .hist-name");
  return (named?.textContent ?? node.textContent ?? "").toLowerCase();
}

function targets() {
  const region = el.consoles && !el.consoles.hidden ? [el.consoles, el.list] : [el.list];
  return region.flatMap((r) => [...r.querySelectorAll(".card, .gcard, .row, .prow, .hist-game")]);
}

let current = "";

/// Apply `text` to what is on screen. Empty text puts everything back.
///
/// What counts as a match, and when a group heading has been left with nothing
/// under it, are decided in `src/pagefilter.rs`. This finds the nodes, hands
/// over their names, and puts the class on whatever did not survive.
export async function applyPageFilter(text) {
  current = String(text ?? "").trim().toLowerCase();

  // The names go with the question, every time.
  //
  // They were sent once per list and held by the backend between keystrokes,
  // which saved sending 2,506 arcade names per letter. But the backend is one
  // for every browser pointed at moose-service, so two tabs or two people
  // filtering different pages each filtered with whichever names were sent
  // last. The page only knew to resend when its count changed, so it went on
  // being answered about the other page's list. Sending them each time costs
  // one request per pause in the typing, and the answer is about this page.

  // A windowed list is narrowed rather than hidden: only a band of it is
  // drawn, so putting a class on the nodes that are there would search a few
  // hundred games out of two and a half thousand — a filter that finds less
  // the further down you have scrolled. The window is told which rows survive
  // and redraws from the top, and the cursor then moves through those and no
  // others.
  const win = windowedList();
  if (win && el.list.contains(win.container)) {
    const { visible, shown } = await invoke("page_filter", {
      query: current,
      names: win.names(),
    });
    win.narrow(current ? visible : null);
    return shown;
  }

  const nodes = targets();
  const heads = [...el.list.querySelectorAll(".ghead")];
  const { visible, headings, shown } = await invoke("page_filter", {
    query: current,
    names: nodes.map(nameOf),
    groups: groupsOf(heads, nodes),
  });
  nodes.forEach((node, i) => node.classList.toggle(OUT, !visible[i]));
  heads.forEach((head, i) => head.classList.toggle(OUT, !!headings[i]));
  return shown;
}

/// Which entries sit under each heading, so a heading with nothing left under
/// it can go too: a search that leaves five headings and no games reads as a
/// broken page.
function groupsOf(heads, nodes) {
  const at = new Map(nodes.map((n, i) => [n, i]));
  return heads.map((head) =>
    [...(head.parentElement?.querySelectorAll(".card, .row, .gcard") ?? [])]
      .map((n) => at.get(n))
      .filter((i) => i !== undefined)
  );
}

/// Re-apply after a list is redrawn. A redraw builds fresh nodes, which have
/// never seen the filter — without this, changing the order or coming back to
/// a tab quietly undoes the search still sitting in the box.
export function refreshPageFilter() {
  if (current) return applyPageFilter(current);
}

export function pageFilterText() {
  return current;
}

/// Empty the box and put everything back. Called when the view changes: a
/// filter typed for one screen means nothing on the next, and leaving the text
/// there means arriving at a list that is missing things for no visible
/// reason.
export function clearPageFilter() {
  current = "";
  windowedList()?.narrow(null);
  if (el.pageFilter) el.pageFilter.value = "";
  for (const node of targets()) node.classList.remove(OUT);
  for (const head of el.list.querySelectorAll(".ghead")) head.classList.remove(OUT);
}

/// Say what this page holds, so the box can say what it will search.
export function setPageFilterLabel(what) {
  if (el.pageFilter) el.pageFilter.placeholder = what ? `Filter ${what}…` : "Filter this page…";
}

/// The slot beside the box, for the button that orders the list.
///
/// Collections have one; a console list does not. Whatever is put here is
/// replaced wholesale on the next view, so a view that says nothing gets an
/// empty slot rather than the last view's button.
export function setPageFilterExtra(node) {
  const slot = document.getElementById("page-filter-extra");
  if (!slot) return null;
  slot.replaceChildren();
  if (node) slot.appendChild(node);
  return slot;
}

export function installPageFilter() {
  if (!el.pageFilter) return;
  let timer;
  el.pageFilter.addEventListener("input", () => {
    clearTimeout(timer);
    // A beat behind the typing: on 2,506 games, filtering on every keystroke
    // is 2,506 class toggles per letter.
    timer = setTimeout(() => applyPageFilter(el.pageFilter.value), 120);
  });
  el.pageFilter.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") {
      clearPageFilter();
      el.pageFilter.blur();
    }
    // The list below owns the arrows; this box owns its own text.
    ev.stopPropagation();
  });
}
