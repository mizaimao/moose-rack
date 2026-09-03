// What the page is actually holding.
//
// The page process weighs 208 MB with a big console open, no artwork loaded
// and the backdrop off. That is far more than a document of eighty-five cards
// and a list of two and a half thousand names has any business costing, and
// guessing at it from the outside has been wrong four times. So: count things
// from the inside, at each step, and put the counts beside the weight.
(async () => {
  const { invoke } = window.__TAURI__.core;
  const note = (m) => invoke("measure_note", { text: String(m) }).catch(() => {});
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const kb = (n) => `${Math.round(n / 1024)}k`;

  const state = (await import("./js/state.js")).state;

  const census = (where) => {
    const nodes = document.querySelectorAll("*").length;
    const pics = document.querySelectorAll("img,canvas").length;
    const sheets = [...document.styleSheets].reduce((n, s) => {
      try { return n + s.cssRules.length; } catch { return n; }
    }, 0);
    // Every array on `state` big enough to matter, and what it would weigh as
    // text. Not its real size in the engine, but the right order of magnitude
    // and the only number a page can get at.
    const held = [];
    for (const [key, value] of Object.entries(state)) {
      if (Array.isArray(value) && value.length > 50) {
        let bytes = 0;
        try { bytes = JSON.stringify(value).length; } catch { bytes = -1; }
        held.push(`${key}=${value.length}(${kb(bytes)})`);
      }
    }
    note(`${where} | nodes ${nodes} | pics ${pics} | cssRules ${sheets} | ${held.join(" ") || "no big arrays"}`);
  };

  try {
    await wait(3000);
    census("STARTUP");
    for (const tab of document.querySelectorAll(".stab"))
      if (tab.dataset.id === "library") tab.click();
    await wait(3000);
    census("CONSOLES");
    const card = document.querySelector('[data-slug="arcade"]');
    if (card) {
      card.click();
      await wait(6000);
      census("ARCADE");
      const list = document.getElementById("list");
      for (let i = 1; i <= 10; i++) {
        list.scrollTop = (list.scrollHeight - list.clientHeight) * (i / 10);
        list.dispatchEvent(new Event("scroll"));
        await wait(700);
      }
      await wait(4000);
      census("SCROLLED");
      await wait(20000);
      census("SETTLED");
    }
    await wait(20000);
    census("HELD");

    // Now take everything of ours away and see what is left. If the page
    // process stays where it is, the memory is not our document, our data or
    // our stylesheet — it is WebKit's own floor, or something that has leaked
    // out of reach. If it falls, it is ours and we can go and find it.
    document.body.replaceChildren();
    for (const [key, value] of Object.entries(state))
      if (Array.isArray(value)) state[key] = [];
    for (const sheet of [...document.styleSheets]) sheet.disabled = true;
    if (globalThis.gc) globalThis.gc();
    await wait(25000);
    census("STRIPPED");
    await wait(20000);
    census("STRIPPED-STILL");
  } catch (e) {
    note("failed: " + (e && e.stack ? e.stack : e));
  }
})();
