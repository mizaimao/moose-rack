// Editing credentials, in a box of their own.
//
// A password sitting in a settings form is wrong twice over: it is on screen
// whenever the pane is, and it saves on blur, so a half-typed token gets written
// the moment focus moves. Behind a button and inside a dialog, nothing is
// written until Save, and nothing is shown that was not just typed.
//
// Existing values are never read back out of config.toml. The dialog says
// whether something is set and lets you replace it — a settings pane has no
// reason to hand a stored credential to a webview to display.

import { invoke } from "./state.js";

function esc(s) {
  const d = document.createElement("div");
  d.textContent = s == null ? "" : String(s);
  return d.innerHTML;
}

/// One editable line in the dialog.
///
/// `secret` fields render as a password box and show "already set" as their
/// placeholder rather than their value, so leaving one untouched keeps what is
/// stored.
export function credentialDialog({ title, note, fields, verify }) {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    overlay.innerHTML = `<div class="conflict-box cred-box">
        <header><span class="icon icon-settings"></span><h2>${esc(title)}</h2></header>
        ${note ? `<p class="lead">${esc(note)}</p>` : ""}
        ${fields
          .map(
            (f) => `
          <div class="srow">
            <label>${esc(f.label)}</label>
            <div class="ctl">
              <input class="cred" data-field="${esc(f.field)}"
                     type="${f.secret ? "password" : "text"}"
                     spellcheck="false"
                     placeholder="${esc(f.secret && f.isSet ? "already set — leave blank to keep" : f.placeholder || "")}"
                     value="${esc(f.secret ? "" : f.value || "")}" />
            </div>
          </div>`
          )
          .join("")}
        <p class="hint cred-status"></p>
        <div class="cred-actions">
          ${verify ? `<button class="cred-verify">Verify</button>` : ""}
          <button class="cred-save">Save</button>
          <button class="cred-cancel">Cancel</button>
        </div>
      </div>`;

    const status = overlay.querySelector(".cred-status");
    const inputs = [...overlay.querySelectorAll(".cred")];
    const values = () =>
      Object.fromEntries(inputs.map((i) => [i.dataset.field, i.value]));

    let settled = false;
    const finish = (result) => {
      if (settled) return;
      settled = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(result);
    };

    // Verification is advice, not a gate. A server that is merely off right now
    // is not a reason to refuse to save what someone typed, and refusing would
    // make the app unconfigurable exactly when it is most needed.
    let verified = null;
    const runVerify = async () => {
      if (!verify) return true;
      status.textContent = "Checking…";
      try {
        status.textContent = await verify(values());
        status.classList.remove("bad");
        verified = true;
      } catch (e) {
        status.textContent = String(e);
        status.classList.add("bad");
        verified = false;
      }
      return verified;
    };

    overlay.querySelector(".cred-verify")?.addEventListener("click", runVerify);
    overlay.querySelector(".cred-cancel").addEventListener("click", () => finish(null));

    overlay.querySelector(".cred-save").addEventListener("click", async () => {
      // Not yet checked: check now, so Save alone is enough and Verify is only
      // for someone who wants to know before committing.
      if (verify && verified === null) await runVerify();

      if (verify && verified === false) {
        const ok = window.confirm(
          `${status.textContent}\n\nSave these settings anyway?`
        );
        // Either answer is respected: yes writes them, no leaves the dialog
        // open so the values are still there to correct.
        if (!ok) return;
      }
      finish(values());
    });

    const onKey = (ev) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        finish(null);
      }
      ev.stopPropagation();
    };
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    inputs[0]?.focus();
  });
}

/// The server box: URL, token, and a real connection check.
export async function editServer(current) {
  const out = await credentialDialog({
    title: "RomM server",
    note: "A client token is preferred over a password. Create one in RomM with roms.read, platforms.read, collections.read, assets.read/write and devices.read/write.",
    fields: [
      { field: "server_url", label: "Address", value: current.server_url, placeholder: "http://dev.lan" },
      { field: "server_token", label: "Token", secret: true, isSet: current.server_token_set },
      { field: "server_username", label: "Username", value: current.server_username, placeholder: "only needed without a token" },
    ],
    verify: (v) =>
      invoke("verify_server", {
        url: v.server_url,
        token: v.server_token || null,
        username: v.server_username || null,
        password: null,
      }),
  });
  return out;
}

export async function editAchievements(current) {
  return credentialDialog({
    title: "RetroAchievements",
    note: "Both a username and a token are needed — either alone authenticates nothing. The token is what RetroArch stores after a successful login.",
    fields: [
      { field: "achievements_username", label: "Username", value: current.achievements_username },
      { field: "achievements_token", label: "Token", secret: true, isSet: current.achievements_token_set },
    ],
  });
}

export async function editScraper(current) {
  return credentialDialog({
    title: "ScreenScraper",
    // Your login only. The developer credentials ScreenScraper issues to a
    // registered application belong to the application, not to the person
    // using it — asking every user for a pair they would have to register for
    // is asking them to do our paperwork.
    note: "Your ScreenScraper login. Not used by the app yet.",
    fields: [
      { field: "scraper_ssid", label: "Login", value: current.scraper_ssid || "" },
      { field: "scraper_sspassword", label: "Password", secret: true, isSet: current.scraper_sspassword_set },
    ],
  });
}
