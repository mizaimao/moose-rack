// Which console the list has scrolled into.
//
// There were two attempts at this and both were wrong in the same way.
// Sticky and frosted, the heading floated over a game's cover; sticky and
// opaque, it still covered the row above it, because a sticky heading is over
// the content by construction and paint does not change that. Not sticky at
// all, and it scrolls away — a console with more than a screenful of games
// loses its name entirely.
//
// The third way is to give it its own space. A strip above the list, outside
// the part that scrolls, saying which section you are in. Nothing is ever
// covered because there is nothing underneath it: the list starts below. The
// headings stay in the list too, in the flow, marking where each section
// begins as you pass it.

import { el } from "./state.js";

let watching = null;

/// Where the strip's text comes from: the last heading that has scrolled up
/// past the top of the list.
function currentHeading() {
  const heads = [...el.list.querySelectorAll(".ghead")];
  if (!heads.length) return null;
  const top = el.list.getBoundingClientRect().top;
  let seen = null;
  for (const h of heads) {
    // A heading level with the top of the list counts as passed: it is the one
    // whose games fill the screen.
    if (h.getBoundingClientRect().top - top <= 1) seen = h;
    else break;
  }
  return seen;
}

function paint() {
  const strip = el.sectionStrip;
  if (!strip) return;
  const head = currentHeading();
  // Empty until the first heading has gone by. Before that the heading itself
  // is on screen, and two copies of the same name is one too many.
  strip.textContent = head ? head.textContent.replace(/\s+/g, " ").trim() : "";
  strip.classList.toggle("showing", !!head);
}

/// Start following a grouped list, or stop if this one has no groups.
///
/// Called after every redraw: the headings are new nodes each time, and the
/// strip is only meaningful where there is more than one section to be in.
export function followSections() {
  const strip = el.sectionStrip;
  if (!strip) return;
  const grouped = el.list.querySelectorAll(".ghead").length > 1;
  strip.hidden = !grouped;
  strip.textContent = "";
  strip.classList.remove("showing");
  if (watching) {
    el.list.removeEventListener("scroll", watching);
    watching = null;
  }
  if (!grouped) return;
  // Read on scroll rather than observed: it is a handful of getBoundingClientRect
  // calls against however many sections a search turned up, which is far cheaper
  // than an observer per heading and cannot drift out of step with the layout.
  watching = () => paint();
  el.list.addEventListener("scroll", watching, { passive: true });
  paint();
}
