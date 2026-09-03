// The Library tab: the things that go and fetch something.
//
// Its own tab because these are the controls that take a minute and need to
// report while they work — at the bottom of General, under six unrelated
// headings, the BIOS control was simply not found.
import { invoke, listen } from "../state.js";
import { toast } from "../util.js";
import { wireConfigFields } from "./fields.js";

export const html = `      <h4>Library</h4>
      <div class="srow">
        <label>Folder</label>
        <div class="ctl"><input class="cf-text" data-field="library_root"
          type="text" spellcheck="false" placeholder="./library" />
          <button class="set-pick" data-pick="library_root"
                  title="Choose a folder">Browse…</button></div>
      </div>
      <p class="hint">Everything downloaded lives here — games, artwork, save
        backups. Deleting this folder reclaims all of it.</p>
      <div class="srow set-fileaccess-row" hidden>
        <label>File access</label>
        <div class="ctl">
          <button class="set-fileaccess">Allow access to files…</button>
          <span class="set-fileaccess-state"></span>
        </div>
      </div>
      <p class="hint set-fileaccess-row" hidden>Android will not let an app read
        the folders below until you turn this on, and it is a switch in system
        settings rather than something this app can ask for. The button opens
        that screen.</p>

      <div class="srow">
        <label>ES-DE folder</label>
        <div class="ctl"><input class="cf-text" data-field="esde_root"
          type="text" spellcheck="false" placeholder="(the library folder)" />
          <button class="set-pick" data-pick="esde_root"
                  title="Choose a folder">Browse…</button></div>
      </div>
      <p class="hint">Where <code>gamelists/</code> and <code>downloaded_media/</code>
        live — the metadata and artwork for a whole collection. Empty means the
        library folder above, which already has that shape. Point it at a real
        ES-DE install and its artwork is used instead of fetching it again; on
        Android that is usually <code>/storage/emulated/0/ES-DE</code>, which
        needs the file access above.</p>

      <div class="srow">
        <label>ES-DE ROMs folder</label>
        <div class="ctl"><input class="cf-text" data-field="esde_roms"
          type="text" spellcheck="false" placeholder="(roms, inside the library folder)" />
          <button class="set-pick" data-pick="esde_roms"
                  title="Choose a folder">Browse…</button></div>
      </div>
      <p class="hint">Where the games themselves are — usually somewhere else
        entirely, which is why ES-DE keeps the two apart. Empty means
        <code>roms</code> inside the library folder. On Android often
        <code>/storage/emulated/0/ROMs</code>.</p>

      <div class="srow">
        <label>Fetch game list</label>
        <div class="ctl">
          <button class="set-libsync">Sync library</button>
          <button class="set-libsync-full">Full resync</button>
        </div>
      </div>
      <p class="hint">The index the grid is built from. Nothing is downloaded —
        but a fresh install shows nothing until this has run once.</p>
      <p class="hint set-libsync-status"></p>

      <div class="srow">
        <label>BIOS files</label>
        <div class="ctl">
          <button class="set-bios">Check BIOS</button>
          <progress class="set-bios-bar" hidden max="1" value="0"></progress>
        </div>
      </div>
      <p class="hint">Neo Geo, PlayStation and the MAME family will not start
        without these. Optional and only when you ask — it is a few hundred MB.
        Needs <code>firmware.read</code> on the token.</p>
      <p class="hint set-bios-status"></p>

      <div class="srow">
        <label>Missing artwork</label>
        <div class="ctl"><button class="set-scrape">Find missing artwork</button></div>
      </div>
      <p class="hint">For games ES-DE never scraped. Your server identifies each one and
      fetches its box art, so no login is needed here. One game at a time, so
      its allowance is not spent in a burst.</p>
      <p class="hint set-scrape-status"></p>`;

/// The tab that fetches things: the game index, BIOS, and artwork the
/// scrapers missed. Each reports before it works and while it works, because a
/// button that goes quiet for a minute reads as one that does nothing.
export function wire(box) {
  // Missing artwork.
  const scrapeBtn = box.querySelector(".set-scrape");
  const scrapeStatus = box.querySelector(".set-scrape-status");
  scrapeBtn?.addEventListener("click", async () => {
    scrapeBtn.disabled = true;
    scrapeStatus.textContent = "Counting…";
    const stop = await listen("scrape-progress", ({ payload }) => {
      scrapeStatus.textContent = String(payload);
    });
    try {
      scrapeStatus.textContent = await invoke("scrape_missing", { platform: null });
      // The grid is showing blanks for exactly these games.
      window.__TAURI__?.event?.emit?.("art-changed");
    } catch (e) {
      scrapeStatus.textContent = `Failed — ${e}`;
    } finally {
      stop?.();
      scrapeBtn.disabled = false;
    }
  });

  // BIOS. Progress by name rather than a spinner: it is 67 files here, and a
  // spinner says nothing about whether it is nearly done or barely started.
  const biosBtn = box.querySelector(".set-bios");
  const biosStatus = box.querySelector(".set-bios-status");
  const biosBar = box.querySelector(".set-bios-bar");

  // Two presses rather than one. The old button started a sync and said nothing
  // until the listing came back — indistinguishable from a control that does
  // nothing — and where the files were already present it did all that work to
  // report "already complete". Now the first press asks, and downloading is
  // only offered when there is something to download.
  let biosPlan = null;

  async function startBios() {
    biosBtn.disabled = true;
    biosBar.hidden = false;
    biosBar.max = biosPlan.total;
    biosBar.value = 0;
    // A count alone does not show progress at a glance; the bar does, and the
    // filename says which one a stall is sitting on.
    const stop = await listen("bios-progress", ({ payload }) => {
      const [done, total, name] = payload;
      biosBar.max = total;
      biosBar.value = done;
      biosStatus.textContent = `${done}/${total}  ${name}`;
    });
    try {
      biosStatus.textContent = await invoke("sync_bios");
    } catch (e) {
      biosStatus.textContent = `Failed — ${e}`;
    } finally {
      stop?.();
      biosBar.hidden = true;
      biosBtn.disabled = false;
      biosBtn.textContent = "Check BIOS";
      biosPlan = null;
    }
  }

  biosBtn?.addEventListener("click", async () => {
    if (biosPlan) return startBios();
    biosBtn.disabled = true;
    biosStatus.textContent = "Asking the server…";
    try {
      const [total, have, bytes] = await invoke("bios_status");
      if (have >= total) {
        biosStatus.textContent = `All ${total} BIOS files are already here.`;
        biosPlan = null;
      } else {
        biosPlan = { total, have, bytes };
        biosStatus.textContent =
          `${total - have} of ${total} missing, about ${(bytes / 1e6).toFixed(0)} MB.`;
        biosBtn.textContent = `Download ${total - have} files`;
      }
    } catch (e) {
      biosStatus.textContent = `Failed — ${e}`;
    } finally {
      biosBtn.disabled = false;
    }
  });

  // Library. This is the one the Windows build had no way to run: the release
  // ships only the GUI, so a fresh install had an empty cache, an empty grid,
  // and nothing anywhere to fill it.
  const libStatus = box.querySelector(".set-libsync-status");
  const runLibSync = async (btn, full) => {
    const buttons = [
      box.querySelector(".set-libsync"),
      box.querySelector(".set-libsync-full"),
    ];
    buttons.forEach((b) => b && (b.disabled = true));
    libStatus.textContent = full ? "Re-fetching everything…" : "Syncing…";
    // A full pull of ~9,000 games takes several seconds, so the backend says
    // which stage it is on rather than leaving the panel looking hung.
    const stop = await listen("sync-progress", ({ payload }) => {
      libStatus.textContent = String(payload);
    });
    try {
      libStatus.textContent = await invoke("sync_library", { full });
      // The grid is built from the cache, so it has to be drawn again to show
      // what just arrived — but this is the settings window and it cannot
      // reach that document. It used to `import("./library.js")`, which from
      // here is *this* file: `showPlatforms` came back undefined, calling it
      // threw, and the catch below reported a sync that had in fact worked as
      // "Sync failed".
      await window.__TAURI__.event.emit("library-synced");
    } catch (e) {
      libStatus.textContent = `Sync failed — ${e}`;
    } finally {
      stop?.();
      buttons.forEach((b) => b && (b.disabled = false));
    }
  };
  box.querySelector(".set-libsync")?.addEventListener("click", (e) =>
    runLibSync(e.currentTarget, false)
  );
  box.querySelector(".set-libsync-full")?.addEventListener("click", (e) =>
    runLibSync(e.currentTarget, true)
  );

  // All files access, on Android only.
  //
  // The row is hidden unless the bridge is there, so desktop never sees a
  // control for a permission it does not have. `MooseAndroid` is the
  // JavaScript interface MainActivity attaches — see Bridge there for why this
  // is not a Tauri command.
  //
  // A button rather than only the prompt at launch: that prompt fires once per
  // start and dismissing it left no way back to the switch, which made the
  // permission unreachable for anyone who did not want it at that exact moment.
  const bridge = window.MooseAndroid;
  if (bridge) {
    const rows = box.querySelectorAll(".set-fileaccess-row");
    rows.forEach((r) => (r.hidden = false));
    const state = box.querySelector(".set-fileaccess-state");
    const btn = box.querySelector(".set-fileaccess");
    const paint = () => {
      let ok = false;
      try {
        ok = bridge.hasAllFilesAccess();
      } catch {
        ok = false;
      }
      state.textContent = ok ? "Allowed" : "Not allowed yet";
      btn.textContent = ok ? "Review in settings…" : "Allow access to files…";
    };
    paint();
    btn?.addEventListener("click", () => {
      try {
        bridge.openAllFilesAccess();
      } catch (e) {
        toast(`Could not open that screen — ${e}`, 6000);
      }
    });
    // Coming back from system settings is a resume, not a reload, so the
    // answer is asked for again rather than remembered.
    window.addEventListener("focus", paint);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) paint();
    });
  }

  // Browse, for every path on this tab.
  //
  // Typing an absolute path is not a reasonable ask on a handheld, and it is
  // not much of one on a desktop either — a library is wherever the user put
  // it, which is exactly the thing they should not have to spell.
  //
  // Two mechanisms, because the platforms differ in kind. Desktop has a dialog
  // that can be awaited. Android has no such thing: it starts an activity and
  // the answer arrives later, so the bridge takes the name of the field that
  // asked and calls back with it. `__folderPicked` is that callback.
  const saveField = async (field, value) => {
    const input = box.querySelector(`[data-field="${field}"]`);
    if (input) input.value = value;
    try {
      toast(await invoke("set_config_field", { field, value }));
    } catch (e) {
      toast(`Could not save — ${e}`, 8000);
    }
  };

  window.__folderPicked = (field, path) => {
    if (!path) return toast("That folder could not be read", 6000);
    saveField(field, path).then(() => {
      // Neither takes effect on its own. The artwork root is read once when
      // the app starts, and the ES-DE system name that decides which artwork
      // folder each game reads is written only by the local scan — which is
      // the first half of Sync library. Without both, every game falls back to
      // this app's own downloads and the ES-DE library looks like it was never
      // found.
      if (field === "esde_root" || field === "esde_roms") {
        toast("Saved. Restart the app, then Sync library, to read it.", 9000);
      }
    });
  };

  for (const btn of box.querySelectorAll(".set-pick")) {
    const field = btn.dataset.pick;
    btn.addEventListener("click", async () => {
      // Android: fires and forgets; the answer comes back to __folderPicked.
      if (window.MooseAndroid?.pickFolder) {
        try {
          window.MooseAndroid.pickFolder(field);
        } catch (e) {
          toast(`Could not open the picker — ${e}`, 6000);
        }
        return;
      }
      // Desktop: invoked directly rather than imported from
      // @tauri-apps/plugin-dialog, because frontendDist is ui/ and
      // node_modules is not in the bundle — the import would take the whole
      // module graph, and the page, down with it.
      try {
        const dir = await invoke("plugin:dialog|open", {
          options: { directory: true, multiple: false, title: "Choose a folder" },
        });
        if (dir) saveField(field, dir);
      } catch (e) {
        toast(String(e), 6000);
      }
    });
  }

  // The text fields: the library folder and the two ES-DE paths.
  //
  // This call was missing. `wireConfigFields` was imported at the top of this
  // file and never invoked, so every `data-field` control on this tab rendered
  // its placeholder, held no value, and saved nothing when edited — on every
  // platform, not just Android. The other three panes that carry config fields
  // (general, control, emulators) all call it; this one was overlooked, and a
  // field that shows its placeholder looks exactly like a field that is simply
  // empty, which is why it went unnoticed.
  wireConfigFields(box);
}
