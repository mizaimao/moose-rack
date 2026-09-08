// Which shader the in-page emulator draws with.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { toBrowserShader, chosenShader, rememberShader, PRESETS, presetsFor, isHandheld } from "../js/ejs-shaders.js";

/// A localStorage that behaves, and one that throws the way a private window's
/// does.
function storage(impl) {
  Object.defineProperty(globalThis, "localStorage", { value: impl, configurable: true });
}
beforeEach(() => {
  const m = new Map();
  storage({
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => m.set(k, String(v)),
  });
});

describe("mapping the library's shader to the browser's", () => {
  test("the same shader named three ways is the same shader", () => {
    for (const s of ["crt-geom", "crt/crt-geom", "shaders_slang/crt/crt-geom.slangp",
                     "crt-geom.glslp", "crt/crt-geom.cgp"]) {
      assert.equal(toBrowserShader(s), "crt-geom.glslp", s);
    }
  });

  test("off is off", () => {
    for (const s of ["", "  ", "none", null, undefined]) {
      assert.equal(toBrowserShader(s), null, JSON.stringify(s));
    }
  });

  /// A handheld LCD grid is not a CRT mask. Substituting one for the other
  /// would be worse than drawing none, and much harder to notice.
  test("a shader with no counterpart is not swapped for a lookalike", () => {
    for (const s of ["handheld/lcd-grid", "zfast_lcd", "scanlines", "crt-royale"]) {
      assert.equal(toBrowserShader(s), null, s);
    }
  });

  test("every preset offered is one EmulatorJS actually has", () => {
    // Read from the pinned release rather than trusted: if the pin moves and a
    // preset goes, the picker must not offer it.
    const ids = PRESETS.map((p) => p.id).filter(Boolean);
    assert.deepEqual(
      ids.slice().sort(),
      ["crt-aperture.glslp", "crt-easymode.glslp", "crt-geom.glslp", "crt-mattias.glslp"],
    );
  });
});

describe("what actually gets drawn", () => {
  test("the library's choice applies when the viewer has not made one", () => {
    assert.equal(chosenShader("crt/crt-easymode", "snes"), "crt-easymode.glslp");
    assert.equal(chosenShader("handheld/lcd-grid", "snes"), "", "an unmappable one draws none");
    assert.equal(chosenShader(null, "snes"), "");
  });

  test("the viewer's own choice wins, including turning it off", () => {
    rememberShader("crt-geom.glslp");
    assert.equal(chosenShader("crt/crt-easymode", "snes"), "crt-geom.glslp");
    rememberShader("");
    assert.equal(chosenShader("crt/crt-easymode", "snes"), "", "off did not stick");
  });

  /// Private windows throw on both. Neither is a reason to refuse to draw.
  test("storage that throws is survivable", () => {
    storage({ getItem: () => { throw new Error("no"); }, setItem: () => { throw new Error("no"); } });
    assert.doesNotThrow(() => rememberShader("crt-geom.glslp"));
    assert.equal(chosenShader("crt/crt-geom", "snes"), "crt-geom.glslp");
  });
});

describe("applying it to a running game", () => {
  /// `setShader` does not exist. The first version called it, the picker moved,
  /// nothing changed, and nothing said why -- an optional-call `?.()` on a
  /// missing method is a silent no-op.
  test("the player calls the method EmulatorJS actually has", async () => {
    const { readFileSync } = await import("node:fs");
    const src = readFileSync(new URL("../js/player.js", import.meta.url), "utf8");
    const code = src.split("\n").filter((l) => !l.trim().startsWith("//")).join("\n");
    assert.match(code, /changeSettingOption\("shader"/, "the live shader change is gone");
    assert.ok(!/setShader/.test(code), "setShader is not a method EmulatorJS has");
  });

  /// And the build we pin really does have it, so this cannot rot silently
  /// when the pin moves.
  test("the pinned EmulatorJS has changeSettingOption and the four presets", async () => {
    const { readFileSync, existsSync } = await import("node:fs");
    const url = new URL("../../assets/emulatorjs/data/emulator.min.js", import.meta.url);
    if (!existsSync(url)) return; // not fetched in this checkout
    const js = readFileSync(url, "utf8");
    assert.ok(js.includes("changeSettingOption"), "the pinned build has no changeSettingOption");
    for (const p of PRESETS.map((p) => p.id).filter(Boolean)) {
      assert.ok(js.includes(p), `the pinned build does not ship ${p}`);
    }
  });
});

describe("consoles that were never on a CRT", () => {
  /// A Game Boy is a reflective LCD held a foot from your face. Scanlines, an
  /// aperture grille and curved glass are all pictures of a television, and
  /// drawing one on a handheld is an artefact it never had rather than a taste.
  test("handhelds are offered no CRT mask at all", () => {
    for (const p of ["gb", "gbc", "gba", "gamegear", "ngp", "wonderswan", "wonderswancolor", "nds"]) {
      assert.ok(isHandheld(p), p);
      const offered = presetsFor(p);
      assert.equal(offered.length, 1, `${p} is offered ${offered.length} shaders`);
      assert.equal(offered[0].id, "");
      assert.match(offered[0].note ?? "", /LCD/, "the reason should be on it");
    }
  });

  test("televisions still get all four", () => {
    for (const p of ["snes", "sfc", "nes", "famicom", "megadrive", "psx", "n64", "arcade"]) {
      assert.equal(isHandheld(p), false, p);
      assert.equal(presetsFor(p).length, 5, p);
    }
  });

  /// The preference is per browser, not per console: without this, choosing a
  /// CRT for the SNES would put scanlines on the Game Boy too.
  test("a remembered CRT does not follow you onto a handheld", () => {
    rememberShader("crt-geom.glslp");
    assert.equal(chosenShader("crt/crt-geom", "snes"), "crt-geom.glslp");
    assert.equal(chosenShader("crt/crt-geom", "gb"), "", "the Game Boy got scanlines");
    assert.equal(chosenShader("crt/crt-geom", "gba"), "");
  });
});
