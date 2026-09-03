// Reading and writing config.toml from a settings pane.
//
// Shared by three tabs. Every control that carries a `data-field` goes through
// here, so a new setting is an attribute in the markup and a line in the
// backend's allowlist, with nothing to wire.
import { invoke } from "../state.js";
import { toast } from "../util.js";
import { editServer, editAchievements, editScraper } from "../credentials.js";

/// Bind every `data-field` control in a pane to config.toml.
///
/// Text fields save on blur rather than on every keystroke: writing the file on
/// each character would rewrite it thirty times while someone types a path, and
/// a half-typed path saved and reloaded is worse than no path.
export async function wireConfigFields(box) {
  const fields = box.querySelectorAll("[data-field]");
  if (!fields.length) return;

  let current;
  try {
    current = await invoke("config_fields");
  } catch (e) {
    box.querySelectorAll(".cf-text").forEach((i) => {
      i.disabled = true;
      i.placeholder = `unavailable — ${e}`;
    });
    return;
  }

  if (!current.config_exists) {
    // Writing settings into a file that does not exist creates one with only
    // those settings in it, which is a worse starting point than the template.
    const warn = document.createElement("p");
    warn.className = "hint";
    warn.textContent = `No config.toml at ${current.config_path} — copy config.example.toml there first.`;
    box.prepend(warn);
  }

  const save = async (field, value) => {
    try {
      toast(await invoke("set_config_field", { field, value: String(value) }));
    } catch (e) {
      toast(`Could not save — ${e}`, 8000);
    }
  };

  // Credentials live behind a button and inside a dialog: nothing is written
  // until Save, and a stored secret is never handed back to be displayed.
  const summarise = () => {
    const set = (name, text) => {
      const el = box.querySelector(`[data-cred-summary="${name}"]`);
      if (el) el.textContent = text;
    };
    set("server", current.server_url ? `${current.server_url}${current.server_token_set ? " · token set" : ""}` : "not configured");
    set("achievements", current.achievements_username
      ? `${current.achievements_username}${current.achievements_token_set ? " · token set" : ""}`
      : "not configured");
    set("scraper", current.scraper_ssid ? current.scraper_ssid : "not configured");
  };
  summarise();

  for (const btn of box.querySelectorAll(".cred-open")) {
    btn.addEventListener("click", async () => {
      const which = btn.dataset.cred;
      const editor =
        which === "server" ? editServer : which === "achievements" ? editAchievements : editScraper;
      const out = await editor(current);
      if (!out) return;
      for (const [field, value] of Object.entries(out)) {
        // A blank secret means "keep what is stored", not "clear it" — the
        // dialog never shows the existing value, so blank is the normal state
        // for a field nobody touched.
        const secret = field.includes("token") || field.includes("password");
        if (secret && !value) continue;
        await save(field, value);
        current[field] = value;
        if (secret) current[`${field}_set`] = true;
      }
      summarise();
    });
  }

  for (const node of fields) {
    const field = node.dataset.field;
    const value = current[field];

    if (node.tagName === "INPUT") {
      node.value = value ?? "";
      // Blur and Enter, not input: see above.
      node.addEventListener("change", () => save(field, node.value));
      continue;
    }

    // A dropdown: a setting with more than two states. Its options are in the
    // markup rather than fetched, because they are a fixed set the backend
    // parses by name.
    if (node.classList.contains("cf-select")) {
      node.value = String(value ?? node.options[0]?.value ?? "");
      node.addEventListener("change", () => save(field, node.value));
      continue;
    }

    // Everything else is a toggle rendered as a button, so it works under a
    // controller the same way every other control here does.
    let on = !!value;
    const paint = () => {
      node.textContent = on ? "On" : "Off";
      node.classList.toggle("active", on);
    };
    paint();
    node.addEventListener("click", () => {
      on = !on;
      paint();
      save(field, on);
    });
  }
}
