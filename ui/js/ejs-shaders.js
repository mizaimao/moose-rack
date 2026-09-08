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

/// Consoles that were never on a CRT.
///
/// A Game Boy is a reflective LCD held a foot from your face: it has no
/// scanlines, no aperture grille and no curved glass, and every one of the four
/// presets above is a picture of a television. Offering them on a handheld is
/// not a matter of taste but of drawing an artefact that never existed.
///
/// EmulatorJS ships no LCD shader, so the honest answer for these is none --
/// the pixels as they are, which is what the screen actually looked like.
const HANDHELD = new Set([
  "gb", "gbc", "gba", "gamegear", "ngp", "neo-geo-pocket",
  "wonderswan", "wonderswancolor", "nds", "lynx",
]);

export function isHandheld(platformSlug) {
  return HANDHELD.has(String(platformSlug ?? "").toLowerCase());
}

/// What the picker should offer for this console.
///
/// Handhelds get "None" and nothing else, with the reason on it rather than an
/// empty list that reads as a bug.
export function presetsFor(platformSlug) {
  if (!isHandheld(platformSlug)) return PRESETS;
  return [{ id: "", label: "None", note: "This console had an LCD — a CRT mask would be an artefact it never had" }];
}

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

export function chosenShader(configuredForPlatform, platformSlug) {
  // A handheld never gets one, whatever is remembered or configured. The
  // preference is stored per browser rather than per console, so without this
  // choosing a CRT for the SNES would put scanlines on the Game Boy too.
  if (isHandheld(platformSlug)) return "";
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
