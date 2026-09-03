// What the library has actually been used for.
//
// The app knew when a game was last played and nothing else — and it did not
// even know that first-hand, since `last_played` came from the server and the
// server only heard about a session if something told it. So a shelf of two
// thousand games had no answer to the one question anybody asks of a
// collection: which of these did I really play.
//
// Three lists, because time played on its own is a leaderboard and a
// leaderboard is not interesting after the first look. Hours per console says
// what the collection is *for*; the top games say what you did with it; and
// the ones you kept opening and kept putting down are the only list here that
// tells you something you did not already know.

import { state, invoke } from "./state.js";
import { region, enter } from "./shell.js";
import { escapeHtml, toast } from "./util.js";

/// Longest bar in the group is full width; everything else is relative to it.
/// Absolute scaling would leave every console after the first as a stub.
function bars(rows, max) {
  return rows
    .map(
      (p) => `
      <div class="hist-row">
        <div class="hist-name">${escapeHtml(p.name)}</div>
        <div class="hist-bar"><span style="width:${max ? (p.seconds / max) * 100 : 0}%"></span></div>
        <div class="hist-time">${escapeHtml(p.spelled)}</div>
      </div>`
    )
    .join("");
}

function games(rows, note) {
  return rows
    .map(
      (g) => `
      <div class="hist-row hist-game" data-id="${g.id}">
        <div class="hist-name">${escapeHtml(g.name)}<em>${escapeHtml(g.platform)}</em></div>
        <div class="hist-time">${escapeHtml(note(g))}</div>
      </div>`
    )
    .join("");
}

export async function showHistory() {
  // It set no buttons at all, so it kept whatever the tab before it left on
  // screen — Back, Take offline, the zoom slider — all of them acting on a
  // console that is no longer showing. A view that declares nothing gets
  // nothing, which is what this page wants: it is a page, not a list.
  // Its own view name. Without one it kept whatever the last tab set, so the
  // section machinery parked History under "platforms" and restored the
  // console grid every time you came back to it — the tab simply never showed.
  state.view = "history";
  // No column: this is a page, not a list with something to pick from beside
  // it. Emptying the column left a 240px strip of nothing down the left with a
  // drag handle on it, which reads as a list that failed to load.
  enter({ title: "History", picker: false, filter: false });

  let h;
  try {
    h = await invoke("play_history");
  } catch (e) {
    return toast(`Could not read the history — ${e}`);
  }

  // Nothing recorded yet is the normal state on a fresh install, and an empty
  // page with three empty headings reads as broken rather than as new.
  if (!h.sessions) {
    region("games").innerHTML = `
      <div class="hist-empty">
        <h2>Nothing recorded yet</h2>
        <p>Time gets counted from the next game you start here. Sessions shorter
           than a minute are ignored — starting the wrong game and quitting
           straight back out is not playing it.</p>
      </div>`;
    return;
  }

  const maxPlatform = Math.max(...h.platforms.map((p) => p.seconds), 0);
  const hours = Math.round(h.total_seconds / 360) / 10;

  region("games").innerHTML = `
    <div class="hist">
      <div class="hist-top">
        <div><strong>${hours}</strong><span>hours</span></div>
        <div><strong>${h.games}</strong><span>games</span></div>
        <div><strong>${h.sessions}</strong><span>sessions</span></div>
      </div>
      <p class="hist-note">Counted from sessions this app started. Anything
        played on the handheld, through ES-DE, or before this was built is not
        here and cannot be.</p>

      <h3>By console</h3>
      <div class="hist-list">${bars(h.platforms, maxPlatform)}</div>

      <h3>Most played</h3>
      <div class="hist-list">${games(h.top, (g) => g.spelled)}</div>

      ${
        h.abandoned.length
          ? `<h3>Picked up and put down</h3>
             <p class="hist-note">Opened more than once, under half an hour all
               told. Either they deserve another go or they are the ones to
               stop reinstalling.</p>
             <div class="hist-list">${games(
               h.abandoned,
               (g) => `${g.sessions} goes · ${g.spelled}`
             )}</div>`
          : ""
      }
    </div>`;
}
