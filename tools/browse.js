// A scripted browse, for measuring what the app weighs while it fills with
// artwork. `MOOSE_MEASURE=tools/browse.js moose-gui`, and weigh the process from
// outside; the notes say where it has got to and — the important part — how
// many pictures are on the page when each weight was taken.
//
// Without that count a memory number means nothing: a figure that does not
// move could be a cache that will not let go, or a change that never took
// effect, and those want opposite fixes.
(async () => {
  const { invoke } = window.__TAURI__.core;
  const note = (m) => invoke("measure_note", { text: String(m) }).catch(() => {});
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const list = () => document.getElementById("list");
  const shown = () => {
    const l = list();
    return `${l.querySelectorAll("img,canvas").length} pics / ${
      l.querySelectorAll(".gcard,.row").length
    } cards`;
  };
  try {
    // The measuring switches. Applied here rather than in `state.js`, which is
    // imported long before `MOOSE_MEASURE_FLAGS` is put on the window — so the
    // flag read there was always empty and one whole comparison measured
    // nothing at all.
    if (globalThis.__MOOSE_FLAGS?.includes("no-glass"))
      document.body.classList.add("plain-cards");
    if (globalThis.__MOOSE_FLAGS?.includes("uncontained"))
      document.body.classList.add("uncontained");
    note(`script running — ${shown()} flags=${globalThis.__MOOSE_FLAGS ?? "none"}`);
    // Home first. The app restores whatever screen it was last on, so the
    // console card is not reliably there — which is what made two earlier runs
    // stop dead with nothing to say.
    for (const tab of document.querySelectorAll(".stab")) {
      if (tab.dataset.id === "library") tab.click();
    }
    await wait(2500);
    note(`at home — ${shown()}`);

    let card = document.querySelector('[data-slug="arcade"]');
    if (!card) {
      // Already inside a console, because the app restores the screen it was
      // last on. Back out and try again — otherwise whether a run browses at
      // all depends on how the previous run happened to end, and two passes
      // of an A/B stop being comparable.
      note("not at the consoles; backing out");
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await wait(2500);
      for (const tab of document.querySelectorAll(".stab"))
        if (tab.dataset.id === "library") tab.click();
      await wait(2500);
      card = document.querySelector('[data-slug="arcade"]');
    }
    if (!card) {
      note(`no arcade card; screen has ${document.querySelectorAll("[data-slug]").length} consoles`);
      return;
    }
    card.click();
    for (let i = 0; i < 30 && !list().querySelector(".gcard,.row"); i++) await wait(500);
    await wait(4000);
    note(`ARCADE OPEN — ${shown()}`);

    // How far down to go. A tenth against the whole thing, so "why does a full
    // scroll cost so much" can be answered by whether the cost follows the
    // distance travelled or is a fixed price for scrolling at all.
    const reach = globalThis.__MOOSE_FLAGS?.includes("short-scroll") ? 0.1 : 1;
    const steps = 30;
    for (let i = 1; i <= steps; i++) {
      const l = list();
      l.scrollTop = (l.scrollHeight - l.clientHeight) * reach * (i / steps);
      l.dispatchEvent(new Event("scroll"));
      await wait(900);
      if (i % 6 === 0)
        note(`scrolled ${Math.round((i / steps) * reach * 100)}% — ${shown()}`);
    }
    note(`SCROLLED — ${shown()}`);
    await wait(12000);
    note(`SETTLED — ${shown()}`);
    await wait(20000);
    note(`HELD — ${shown()}`);
  } catch (e) {
    note("failed: " + (e && e.stack ? e.stack : e));
  }
})();
