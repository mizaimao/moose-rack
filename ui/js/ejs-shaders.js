// Which shader the in-page emulator draws with.
//
// EmulatorJS carries four CRT presets and nothing else — `crt-aperture`,
// `crt-easymode`, `crt-geom`, `crt-mattias` — so this is a much smaller set
// than RetroArch's, where the desktop picks from hundreds under
// `shaders_slang/`. The job here is to honour what the library was already
// configured to want, and to be honest when it cannot.
//
// `[shaders.by_platform]` in config.toml names a RetroArch preset path, like
// `crt/crt-easymode` or `handheld/lcd-grid`. The name at the end is the only
// part that can carry across.

/// The four EmulatorJS ships, in the order the picker offers them.
export const PRESETS = [
  { id: "", label: "None" },
  { id: "crt-easymode.glslp", label: "CRT — easymode", note: "Sharp scanlines, the safe default" },
  { id: "crt-aperture.glslp", label: "CRT — aperture", note: "Aperture grille, brighter" },
  { id: "crt-geom.glslp", label: "CRT — geom", note: "Curved glass" },
  { id: "crt-mattias.glslp", label: "CRT — mattias", note: "Softer, heavier bloom" },
];

const KNOWN = new Set(PRESETS.map((p) => p.id).filter(Boolean));

/// Turn whatever the library configured into one of the four, or nothing.
///
/// Matched on the last path segment, because `crt/crt-geom`,
/// `shaders_slang/crt/crt-geom.slangp` and `crt-geom.glslp` are all the same
/// shader named three ways. Anything with no counterpart here returns null
/// rather than a guess: a handheld LCD grid is not a CRT mask, and quietly
/// substituting one would be worse than drawing none.
export function toBrowserShader(configured) {
  const raw = String(configured ?? "").trim();
  if (!raw || raw === "none") return null;
  const stem = raw.split("/").pop().replace(/\.(glslp|slangp|cgp)$/i, "");
  const id = `${stem}.glslp`;
  return KNOWN.has(id) ? id : null;
}

/// What to draw with: the viewer's own choice if they made one, else whatever
/// the library configured for this console.
///
/// Per browser, not per server: two people on two machines should not fight
/// over each other's scanlines, and there is nothing to sync.
const KEY = "ejsShader";

export function chosenShader(configuredForPlatform) {
  try {
    const saved = localStorage.getItem(KEY);
    // "" is a real choice — it means the viewer turned shaders off.
    if (saved !== null) return saved;
  } catch {
    // Private windows and blocked storage: fall through to the library's.
  }
  return toBrowserShader(configuredForPlatform) ?? "";
}

export function rememberShader(id) {
  try {
    localStorage.setItem(KEY, id ?? "");
  } catch {
    // Not being able to remember it is not a reason to refuse to apply it.
  }
}
