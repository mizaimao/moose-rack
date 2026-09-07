// Which shader the in-page emulator draws with.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { toBrowserShader, chosenShader, rememberShader, PRESETS } from "../js/ejs-shaders.js";

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
    assert.equal(chosenShader("crt/crt-easymode"), "crt-easymode.glslp");
    assert.equal(chosenShader("handheld/lcd-grid"), "", "an unmappable one draws none");
    assert.equal(chosenShader(null), "");
  });

  test("the viewer's own choice wins, including turning it off", () => {
    rememberShader("crt-geom.glslp");
    assert.equal(chosenShader("crt/crt-easymode"), "crt-geom.glslp");
    rememberShader("");
    assert.equal(chosenShader("crt/crt-easymode"), "", "off did not stick");
  });

  /// Private windows throw on both. Neither is a reason to refuse to draw.
  test("storage that throws is survivable", () => {
    storage({ getItem: () => { throw new Error("no"); }, setItem: () => { throw new Error("no"); } });
    assert.doesNotThrow(() => rememberShader("crt-geom.glslp"));
    assert.equal(chosenShader("crt/crt-geom"), "crt-geom.glslp");
  });
});
