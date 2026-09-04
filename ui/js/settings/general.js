// The General tab: where RetroArch is, who the server is, and the accounts.
import { invoke } from "../state.js";
import { toast } from "../util.js";
import { wireConfigFields } from "./fields.js";
import { syncSaves } from "../savesync.js";

export const html = `      <h4>RetroArch</h4>
      <div class="srow set-ra-path">
        <label>Location</label>
        <div class="ctl">
          <input class="set-ra" type="text" spellcheck="false"
                 placeholder="search the usual places" />
          <button class="set-ra-pick" title="Choose a folder">Browse…</button>
          <button class="set-ra-save">Save</button>
        </div>
      </div>
      <p class="hint set-ra-path">Empty searches the usual locations. Set it when
        the install lives elsewhere, such as <code>E:\\Emulators\\RetroArch</code>.</p>

      <div class="srow set-ra-pkg" hidden>
        <label>RetroArch</label>
        <div class="ctl"><span class="set-ra-pkg-state"></span></div>
      </div>
      <p class="hint set-ra-pkg" hidden>Android installs RetroArch as an app, so
        there is no folder to point at — it is either on the device or it is not.
        Install it from the Play Store or retroarch.com and it will be found.</p>

      <p class="hint set-ra-status"></p>

      <h4>Saves</h4>
      <div class="srow">
        <label>Folder</label>
        <div class="ctl"><input class="cf-text" data-field="saves_root"
          type="text" spellcheck="false" placeholder="./Saves" />
          <button class="set-pick" data-pick="saves_root"
                  title="Choose a folder">Browse…</button></div>
      </div>
      <p class="hint">Where RetroArch puts battery saves and save states —
        <code>saves/</code> and <code>states/</code> are made inside it. This is
        written into the config handed to RetroArch at launch, so it is the
        folder the emulator actually uses rather than one this app merely reads.
        Empty means <code>./Saves</code> beside the app.</p>

      <div class="srow">
        <label>Sync now</label>
        <div class="ctl"><button class="set-savesync">See what would sync</button></div>
      </div>
      <p class="hint">Asks the server what it would do with your saves and shows
        you the list — which way each one would move, and which changed in both
        places. Nothing is transferred until you accept it.</p>
      <p class="hint set-savesync-status"></p>

      <h4>Server</h4>
      <div class="srow">
        <label>Library server</label>
        <div class="ctl">
          <button class="cred-open" data-cred="server">Edit…</button>
          <span class="cred-summary" data-cred-summary="server"></span>
        </div>
      </div>
      <p class="hint">Address and credentials, with a connection check before
        anything is written.</p>

      <h4>Achievements</h4>
      <div class="srow">
        <label>RetroAchievements</label>
        <div class="ctl"><button data-field="achievements_enabled">…</button></div>
      </div>
      <div class="srow">
        <label>Account</label>
        <div class="ctl">
          <button class="cred-open" data-cred="achievements">Edit…</button>
          <span class="cred-summary" data-cred-summary="achievements"></span>
        </div>
      </div>
      <div class="srow">
        <label>Login</label>
        <div class="ctl">
          <button class="ra-check">Check login</button>
          <span class="ra-state dim">not checked</span>
        </div>
      </div>
      <p class="hint">Checks the username and token above against
        RetroAchievements, through the same login RetroArch uses — so a tick
        means the login the emulator will attempt, not merely that the account
        exists.</p>
      <div class="srow">
        <label>Hardcore mode</label>
        <div class="ctl"><button data-field="achievements_hardcore">…</button></div>
      </div>
      <p class="hint">Hardcore disables save states, fast-forward and rewind —
        four of the hotkeys this app binds.</p>

      <h4>Save states</h4>
      <div class="srow">
        <label>Ask before deleting</label>
        <div class="ctl"><button data-field="confirm_delete_state">…</button></div>
      </div>
      <p class="hint">Off by default — clearing old states is done several at a time. A copy
      always goes to the backups folder, so deleting one is undoable by hand.</p>

      <h4>ScreenScraper</h4>
      <div class="srow">
        <label>Account</label>
        <div class="ctl">
          <button class="cred-open" data-cred="scraper">Edit…</button>
          <span class="cred-summary" data-cred-summary="scraper"></span>
        </div>
      </div>
      <p class="hint">Stored but not used yet — kept with the rest of the
        configuration rather than in someone's notes.</p>

`;

export function wire(box) {
  wireAchievementCheck(box);
  // RetroArch location. The backend verifies the path before writing it to
  // config.toml, so an invalid one is reported here rather than failing later
  // at launch time.
  const raInput = box.querySelector(".set-ra");
  const raStatus = box.querySelector(".set-ra-status");
  invoke("status")
    .then((s) => {
      // On Android the backend cannot answer this: it looks for a folder and
      // RetroArch is a package, so `s.retroarch` is always false there. The row
      // above already says installed or not, from the package manager — this
      // line would sit under it contradicting it and telling the user to set a
      // path that does not exist. Left blank instead.
      if (window.MooseAndroid?.retroArchPackage) {
        raStatus.textContent = "";
        return;
      }
      if (s?.retroarch) raInput.placeholder = s.retroarch;
      raStatus.textContent = s?.retroarch
        ? `Currently using ${s.retroarch} (${s.cores_installed} cores)`
        : "Not found. Set a path, or install RetroArch.";
    })
    .catch(() => {});
  box.querySelector(".set-ra-pick").addEventListener("click", async () => {
    try {
      // Invoked directly rather than imported from @tauri-apps/plugin-dialog:
      // frontendDist is ui/, so node_modules is not in the bundle and the
      // import fails there — taking the whole module graph, and the page, with
      // it.
      const dir = await invoke("plugin:dialog|open", {
        options: { directory: true, multiple: false,
                   title: "Select the RetroArch folder" },
      });
      if (dir) raInput.value = dir;
    } catch (e) {
      raStatus.textContent = String(e);
    }
  });
  box.querySelector(".set-ra-save").addEventListener("click", async () => {
    raStatus.textContent = "Checking…";
    try {
      raStatus.textContent = await invoke("set_retroarch_root", { path: raInput.value });
      toast("RetroArch path saved");
    } catch (e) {
      raStatus.textContent = String(e);
    }
  });

  // Saves. The button disables itself while running: the scan plus a round
  // trip per file takes a few seconds, and a second click would start a
  // concurrent sync over the same files.
  //
  // The flow itself lives in savesync.js because the header has the same button
  // on it now. Reporting is the only part that differs — here there is a line
  // under the button to write into, which says more than a toast that has gone
  // by the time the sync finishes.
  const syncBtn = box.querySelector(".set-savesync");
  const syncStatus = box.querySelector(".set-savesync-status");
  syncBtn?.addEventListener("click", async () => {
    syncBtn.disabled = true;
    try {
      await syncSaves({ say: (m) => (syncStatus.textContent = m) });
    } finally {
      syncBtn.disabled = false;
    }
  });

  // config.toml fields. Loaded once and written back on change, through a
  // targeted TOML edit so the hand-written comments in that file survive.
  // Browse, for the saves folder. Same two mechanisms as the Library tab: a
  // dialog that can be awaited on desktop, and on Android an activity whose
  // answer arrives later at __folderPicked.
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
    saveField(field, path);
  };
  for (const btn of box.querySelectorAll(".set-pick")) {
    const field = btn.dataset.pick;
    btn.addEventListener("click", async () => {
      if (window.MooseAndroid?.pickFolder) {
        try {
          window.MooseAndroid.pickFolder(field);
        } catch (e) {
          toast(`Could not open the picker — ${e}`, 6000);
        }
        return;
      }
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

  // Android has no RetroArch *path*. It is a package: installed or not.
  //
  // The folder row asks a question that cannot be answered there — there is
  // nothing to browse to, and a path saved into config.toml would be read by a
  // launcher that will be using an Intent rather than a command line. So on
  // Android the row is replaced by a straight statement of whether the app is
  // present, read from the package manager through the bridge.
  const bridge = window.MooseAndroid;
  if (bridge?.retroArchPackage) {
    box.querySelectorAll(".set-ra-path").forEach((n) => (n.hidden = true));
    box.querySelectorAll(".set-ra-pkg").forEach((n) => (n.hidden = false));
    let pkg = "";
    try {
      pkg = bridge.retroArchPackage();
    } catch {
      pkg = "";
    }
    const state = box.querySelector(".set-ra-pkg-state");
    if (state) {
      state.textContent = pkg ? `Installed — ${pkg}` : "Not installed";
    }
  }

  wireConfigFields(box);
}


/// Ask RetroAchievements whether the login works, and say so plainly.
///
/// Three states rather than two. "Not checked" is honest — the app has not
/// asked — and matters because the other two both make a claim: a tick says
/// the emulator's own login will succeed, and a cross names the reason the
/// server gave. Blurring "unknown" into "bad" is how an indicator becomes
/// something people learn to ignore.
export function wireAchievementCheck(box) {
  const btn = box.querySelector(".ra-check");
  const out = box.querySelector(".ra-state");
  if (!btn || !out) return;

  const paint = (cls, text) => {
    out.className = `ra-state ${cls}`;
    out.textContent = text;
  };

  btn.addEventListener("click", async () => {
    btn.disabled = true;
    paint("dim", "checking…");
    try {
      const v = await invoke("verify_achievements");
      if (v.ok) {
        paint("ok", `token accepted for ${v.user}`);
        toast("RetroAchievements login confirmed");
      } else {
        paint("bad", v.error || "rejected");
        toast(`RetroAchievements: ${v.error || "login rejected"}`, 7000);
      }
    } catch (e) {
      paint("bad", String(e));
      toast(`Could not check: ${e}`, 7000);
    } finally {
      btn.disabled = false;
    }
  });
}
