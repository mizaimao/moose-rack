// Where does opening a big console stop?
//
// The page goes quiet after the click and never sends another word. That is
// consistent with two very different things — the main thread stuck inside
// synchronous work, or a command in Rust that never answers — and they want
// opposite fixes. So: a heartbeat that only a running thread can send, and a
// trace of every command that goes out and comes back.
(async () => {
  const core = window.__TAURI__.core;
  const realInvoke = core.invoke;
  const note = (m) => realInvoke("measure_note", { text: String(m) }).catch(() => {});
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));

  // A number that only goes up while the thread is free to run.
  let beats = 0;
  setInterval(() => { beats += 1; }, 100);

  // Every command out and back, so a Rust side that never returns is named
  // rather than guessed at.
  const open = new Map();
  let seq = 0;
  core.invoke = function (cmd, args, opts) {
    if (cmd === "measure_note") return realInvoke.call(this, cmd, args, opts);
    const id = ++seq;
    open.set(id, cmd);
    return realInvoke.call(this, cmd, args, opts).finally(() => open.delete(id));
  };
  const outstanding = () =>
    open.size ? [...new Set(open.values())].join(",") : "none";

  // The heartbeat, reported from a timer. If the thread is blocked this stops
  // arriving; if it keeps arriving while nothing else happens, the page is
  // idle and waiting on someone else.
  let last = beats;
  setInterval(() => {
    const moved = beats - last;
    last = beats;
    note(`beat +${moved} in flight: ${outstanding()}`);
  }, 2000);

  try {
    await wait(2500);
    for (const tab of document.querySelectorAll(".stab"))
      if (tab.dataset.id === "library") tab.click();
    await wait(3000);
    const card = document.querySelector('[data-slug="arcade"]');
    note(`about to click arcade: ${card ? "found" : "MISSING"}`);
    if (!card) return;
    const t0 = performance.now();
    card.click();
    note(`click returned after ${Math.round(performance.now() - t0)} ms`);
    for (let i = 0; i < 60; i++) {
      await wait(1000);
      const n = document.querySelectorAll("#list .gcard, #list .row").length;
      if (n) { note(`cards appeared: ${n} after ${Math.round((performance.now() - t0) / 100) / 10}s`); break; }
    }
  } catch (e) {
    note("threw: " + (e && e.stack ? e.stack : e));
  }
})();
