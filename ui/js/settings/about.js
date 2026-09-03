// What this is, who wrote it, and where it lives.
//
// The version numbers were already at the foot of the rail, which answers
// "are these two machines running the same thing" and nothing else. Everything
// else about the app — that it is somebody's project rather than a product,
// that the source is readable, that there is somewhere to report the things
// that are wrong with it — was written down nowhere the app itself could show
// you.

import { invoke } from "../state.js";
import { toast } from "../util.js";

const REPO = "https://github.com/mizaimao/moose-rack";

export const html = `      <h4>Moose Rack</h4>
      <div class="srow">
        <label>Version</label>
        <div class="ctl"><span class="about-version">…</span></div>
      </div>
      <p class="hint">This build, and the server it has spoken to. Both,
        because "which version am I on" is usually asked when two machines
        behave differently.</p>
      <div class="srow">
        <label>Updates</label>
        <div class="ctl">
          <button class="about-check">Check for a newer version</button>
          <span class="about-update dim"></span>
        </div>
      </div>
      <p class="hint">Asks GitHub what the newest release is and tells you.
        Nothing is downloaded or replaced — a self-updating app needs signing
        and a way back, and this one would rather point you at the release.</p>

      <div class="srow">
        <label>By</label>
        <div class="ctl">
          <a class="link" data-href="https://github.com/mizaimao">mizaimao</a>
        </div>
      </div>

      <div class="srow">
        <label>Source</label>
        <div class="ctl">
          <a class="link" data-href="${REPO}">github.com/mizaimao/moose-rack</a>
        </div>
      </div>
      <p class="hint">Bugs and requests go in
        <a class="link" data-href="${REPO}/issues">the issues</a>. Icons are
        <a class="link" data-href="https://lucide.dev">Lucide</a>, ISC, bundled
        rather than fetched — this window works with no network at all.</p>`;

/// Send one link to the system browser rather than opening it in here.
///
/// The settings window has no address bar and no back button, so following a
/// link inside it turns the app into a browser with no way out.
function openInBrowser(a) {
  a.setAttribute("role", "link");
  a.setAttribute("tabindex", "0");
  a.addEventListener("click", async (ev) => {
    ev.preventDefault();
    try {
      await invoke("open_link", { url: a.dataset.href });
    } catch (e) {
      toast(`Could not open the link — ${e}`);
    }
  });
}

export async function wire(box) {
  // Every link goes out to the browser. A webview that follows one in place
  // turns the settings window into a web browser with no way back: no address
  // bar, no back button, and the app gone from underneath it.
  for (const a of box.querySelectorAll(".link")) {
    openInBrowser(a);
  }

  // Updates. Only when asked: a check on every open is a request a minute for
  // an answer that changes a few times a month.
  const upBtn = box.querySelector(".about-check");
  const upNote = box.querySelector(".about-update");
  upBtn?.addEventListener("click", async () => {
    upBtn.disabled = true;
    upNote.textContent = "Asking GitHub…";
    try {
      const u = await invoke("check_update");
      if (!u) {
        upNote.textContent = "This is the newest release.";
      } else {
        upNote.innerHTML = "";
        const a = document.createElement("a");
        a.className = "link";
        a.dataset.href = u.url;
        a.textContent = `${u.version} is out — you have ${u.running}`;
        // Wired here rather than left to the sweep below: that runs once when
        // the pane is built, and this link does not exist yet.
        openInBrowser(a);
        upNote.appendChild(a);
      }
    } catch (e) {
      // Offline is the ordinary case, and not worth a toast for a check
      // somebody asked for and can see the answer to in place.
      upNote.textContent = `Could not ask — ${e}`;
    } finally {
      upBtn.disabled = false;
    }
  });

  const where = box.querySelector(".about-version");
  if (!where) return;
  try {
    const [client, server] = await invoke("versions");
    // Guarded rather than trusted: an older backend that does not know this
    // command answers with something that is not a pair, and destructuring it
    // silently produced "undefined" on the one line whose entire job is to be
    // exact.
    where.textContent = client
      ? server
        ? `${client} · server ${server}`
        : String(client)
      : "unknown";
  } catch (e) {
    // A wrong version number is worse than none, since the only reason to read
    // it is to compare two machines.
    where.textContent = "unknown";
    where.title = String(e);
  }
}
