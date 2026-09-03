// The shader backdrop, with a stand-in for WebGL.
//
// jsdom has no WebGL at all, so `getContext("webgl2")` returns null and the
// whole of this module's real work was never once executed by a test. That is
// how a reference error in it — `apply` calling `resize` a dozen lines before
// `resize` exists — shipped, threw out of startup, and took the tab row, the
// settings button and the page background with it. A backdrop is decoration
// and it managed to be fatal.
//
// The stand-in answers every call this module makes and records nothing about
// how they are drawn. What is under test is that the code runs at all: builds,
// switches style, resizes, and animates without throwing.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, backdrop, calls;

/// Enough of WebGL2 to get through: every entry point this module touches,
/// returning something of the right shape.
function fakeGl() {
  const handle = (name) => ({ name });
  return {
    FRAGMENT_SHADER: 1,
    VERTEX_SHADER: 2,
    COMPILE_STATUS: 3,
    LINK_STATUS: 4,
    ARRAY_BUFFER: 5,
    STATIC_DRAW: 6,
    FLOAT: 7,
    TRIANGLES: 8,
    createShader: () => handle("shader"),
    shaderSource: () => {},
    compileShader: () => {},
    getShaderParameter: () => true,
    getShaderInfoLog: () => "",
    deleteShader: () => {},
    createProgram: () => handle("program"),
    attachShader: () => {},
    linkProgram: () => {},
    getProgramParameter: () => true,
    getProgramInfoLog: () => "",
    useProgram: (p) => calls.push(["useProgram", p?.name]),
    createBuffer: () => handle("buffer"),
    bindBuffer: () => {},
    bufferData: () => {},
    getAttribLocation: () => 0,
    enableVertexAttribArray: () => {},
    vertexAttribPointer: () => {},
    getUniformLocation: (_p, n) => handle(n),
    uniform1f: (loc, v) => calls.push(["uniform1f", loc?.name, v]),
    uniform2f: () => {},
    uniform3fv: () => {},
    viewport: () => {},
    clearColor: () => {},
    clear: () => {},
    drawArrays: () => calls.push(["draw"]),
  };
}

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.localStorage = dom.window.localStorage;
  global.HTMLElement = dom.window.HTMLElement;
  global.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });
  // The animation loop reaches for the bare global, as it does in a browser.
  // It hands back an id and never calls back: the loop reschedules itself
  // every frame, so a stub that actually fires would keep this process alive
  // for as long as node is willing to wait.
  global.requestAnimationFrame = () => 1;
  global.cancelAnimationFrame = () => {};
  dom.window.HTMLCanvasElement.prototype.getContext = function (kind) {
    return kind === "webgl2" ? fakeGl() : null;
  };
  dom.window.__TAURI__ = {
    core: { invoke: async () => [], convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };
  backdrop = await import("../js/backdrop.js");
});

beforeEach(() => {
  calls = [];
  backdrop.stopBackdrop();
  dom.window.localStorage.clear();
});

describe("starting it", () => {
  test("it builds without throwing", () => {
    assert.ok(backdrop.startBackdrop(), "the backdrop did not start");
    assert.ok(
      dom.window.document.getElementById("backdrop"),
      "no canvas was put on the page"
    );
  });

  /// The failure that took the window down. Whatever is wrong in here, the
  /// caller has to survive it: the tab row, the settings button and the page
  /// background are all built after this call.
  test("a broken shader does not escape", () => {
    const good = dom.window.HTMLCanvasElement.prototype.getContext;
    dom.window.HTMLCanvasElement.prototype.getContext = () => {
      throw new Error("no GPU today");
    };
    try {
      assert.doesNotThrow(() => backdrop.startBackdrop());
      assert.equal(backdrop.startBackdrop(), null);
    } finally {
      dom.window.HTMLCanvasElement.prototype.getContext = good;
    }
  });
});

describe("the styles", () => {
  test("every one names itself and contributes a body", () => {
    assert.ok(backdrop.BACKDROPS.length >= 5, "one shape suits one room");
    for (const b of backdrop.BACKDROPS) {
      assert.ok(b.id && b.label && b.hint, `${b.id} is missing its name or note`);
      assert.match(b.body, /base\s*=/, `${b.id} never sets a color`);
    }
    const ids = backdrop.BACKDROPS.map((b) => b.id);
    assert.equal(new Set(ids).size, ids.length, "two styles share an id");
  });

  test("an unknown style falls back rather than failing", () => {
    assert.equal(backdrop.backdropStyle("no-such-thing").id, backdrop.BACKDROPS[0].id);
  });

  /// A style is a long object literal ending in a shader body, and the bodies
  /// are long enough that the brace closing one is a screen away from the
  /// brace opening the next. Drop that pair while editing and the two entries
  /// merge into one: still valid JavaScript, still parses, but the later
  /// `id:` quietly wins and the earlier style is simply gone from the list —
  /// no error, no failing test, just a shape that never appears in Settings.
  ///
  /// Counting the ids in the source against the ids in the array catches it,
  /// because the merged object still has both `id:` lines in the file.
  test("no style has been swallowed by the one after it", () => {
    const src = readFileSync(join(uiDir, "js/backdrop.js"), "utf8");
    const arr = /export const BACKDROPS = \[([\s\S]*?)\n\];/.exec(src)?.[1] ?? "";
    assert.ok(arr.length > 500, "the styles array was not found; update this test");
    const written = [...arr.matchAll(/^\s{4}id: "([^"]+)"/gm)].map((m) => m[1]);
    assert.deepEqual(
      written,
      backdrop.BACKDROPS.map((b) => b.id),
      "a style is written in the file but missing from the array"
    );
  });

  /// Switching style rebuilds the program and looks its uniforms up again. A
  /// location belongs to the program it came from, and reusing one across
  /// programs is how the second style you try comes out black.
  test("switching style keeps running", () => {
    backdrop.startBackdrop();
    for (const b of backdrop.BACKDROPS) {
      assert.doesNotThrow(
        () => backdrop.applyBackdropSettings({ style: b.id }),
        `switching to ${b.label} threw`
      );
    }
    // Back to the first one, which is already compiled.
    assert.doesNotThrow(() => backdrop.applyBackdropSettings({ style: backdrop.BACKDROPS[0].id }));
  });
});

/// The glass, which is one number and used to be two.
///
/// The preview pane had its own tint and its own slider, so the surface you
/// read from was the one surface that matched nothing around it — and the
/// slider that was supposed to fix that was a second control over an idea the
/// stylesheet only has one of. `--tint` is the opacity every sheet of glass
/// mixes its color in at; the pane is a large card and takes the same one.
describe("the glass", () => {
  const css = () =>
    readFileSync(join(uiDir, "style.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const ruleFor = (sel) =>
    new RegExp(`${sel}\\s*\\{[^}]*\\}`).exec(css())?.[0] ?? "";
  const backgroundOf = (sel) =>
    /background:\s*([^;]+);/.exec(ruleFor(sel))?.[1].replace(/\s+/g, " ").trim();

  /// The pane and the cards no longer look identical, and that is deliberate:
  /// a grid of forty translucent rectangles competes with the artwork inside
  /// them, while the pane is a surface you read from and needs something to
  /// read against. What must not drift is where the opacity comes from — the
  /// bug this file exists for was the pane having a tint and a slider of its
  /// own, which is how the two ended up disagreeing.
  test("the preview pane takes its opacity from the one variable", () => {
    assert.match(
      backgroundOf("#detail") ?? "",
      /var\(--tint\)/,
      "the preview pane has a private opacity again"
    );
  });

  /// Flattening the console grid was tried and reverted: with no pane and no
  /// outline there is nothing to say where one console ends and the next
  /// begins, and the grid reads as scattered logos rather than a list of
  /// things you can press. The Continue playing strip is the opposite case —
  /// five wide screenshots in a row, no column to align with — so it is the
  /// one place the frame comes off.
  test("cards keep their surface; only the recent strip is flat", () => {
    assert.match(
      backgroundOf("\\.card") ?? "",
      /var\(--tint\)/,
      "the console grid lost the edge that separates one card from the next"
    );
    const flat = ruleFor("\\.recent \\.gcard \\.art");
    assert.match(flat, /backdrop-filter:\s*none/, "the recent strip is not flattened");
    assert.match(flat, /border-color:\s*transparent/, "the recent strip kept its outline");
  });

  /// There is one slider for this, and Appearance is where a second one would
  /// reappear.
  test("Appearance offers one control for it, not two", () => {
    const pane = readFileSync(join(uiDir, "js/settings/appearance.js"), "utf8");
    const sliders = pane.match(/class="(glass-strength|pane-clarity)"/g) ?? [];
    assert.deepEqual(sliders, ['class="glass-strength"']);
  });

  /// The bug that made the pane solid, and it is worth stating on its own
  /// because it is invisible at the call site: `Number(null)` is `0`, `0` is
  /// finite and not negative, so a guard written as `n >= 0` accepts the
  /// absence of a setting as the setting `0`. Restore
  /// `Number(localStorage.getItem())` without the presence check and this fails.
  test("nothing stored means the default, not zero", () => {
    dom.window.localStorage.clear();
    assert.equal(backdrop.glassStrength(), 18, "an unset glass strength read as none");
  });

  /// The other half of the same guard: a zero somebody chose is not the same
  /// as a zero that fell out of an empty key, and it has to survive a restart.
  test("a zero that was chosen is kept", () => {
    dom.window.localStorage.setItem("glassStrength", "0");
    assert.equal(backdrop.glassStrength(), 0, "the glass slider snapped back off zero");
  });

  test("it is applied as a percentage the stylesheet can use", () => {
    backdrop.setGlassStrength(24);
    assert.equal(
      dom.window.document.documentElement.style.getPropertyValue("--tint"),
      "24%"
    );
  });
});

/// The three complaints about the shapes themselves, each with the arithmetic
/// that caused it.
///
/// jsdom has no GPU, so none of this can be checked by rendering it — these
/// read the shader source and assert the property that was wrong. That is
/// weaker than a picture and it is what is available; the failures they guard
/// are all "somebody edited the body back".
describe("the shapes", () => {
  test("every style says how fast it should run", () => {
    for (const b of backdrop.BACKDROPS) {
      assert.equal(typeof b.pace, "number", `${b.id} has no pace`);
      assert.ok(b.pace > 0, `${b.id} would never move`);
    }
  });

  /// Blobs drifts at 0.015 of `t` and Plasma sweeps at 0.31, so one number on
  /// the Motion slider meant twenty times the movement depending on which
  /// shape was drawing. Whatever the paces are tuned to, these two cannot end
  /// up being handed the same speed.
  test("the same Motion setting is not the same speed for every shape", () => {
    const speedFor = (style) => {
      backdrop.applyBackdropSettings({ style, speed: 5 });
      const seen = calls.filter((c) => c[0] === "uniform1f" && c[1] === "u_speed");
      return seen[seen.length - 1]?.[2];
    };
    backdrop.startBackdrop();
    const blobs = speedFor("blobs");
    const plasma = speedFor("plasma");
    assert.ok(blobs > 0 && plasma > 0, "no speed reached the shader");
    assert.ok(plasma < blobs, "Plasma is still being run as fast as Blobs");
  });

  /// Drift: a star's glow has a radius of 0.42 of a cell, so any star nearer
  /// than that to a cell edge had the rest of it drawn by a neighbouring cell
  /// that was never asked. What showed was points sliced flat along the same
  /// straight lines every frame. It has to look at the eight neighbours.
  test("Drift asks the cells around it, not only its own", () => {
    const body = backdrop.backdropStyle("stars").body;
    assert.match(body, /for\s*\(int/, "Drift is back to sampling one cell");
    assert.match(body, /cell \+ o/, "the neighbouring cell is not being hashed");
  });

  /// Ribbon is bands, not a mesh, and that distinction cost a whole style.
  ///
  /// The first build of it read the shader's name, built what the word
  /// suggested, and drew a wireframe grid in perspective running to a horizon.
  /// RetroArch's has no lines in it anywhere, no mesh and no depth — it is soft
  /// sheets of light over a gradient. A lattice term reappearing here means
  /// somebody has made that same reading again.
  test("Ribbon draws bands rather than a mesh", () => {
    const code = backdrop.backdropStyle("ribbon").body.replace(/\/\/[^\n]*/g, "");
    assert.doesNotMatch(code, /fract\(/, "there is a lattice in Ribbon again");
    assert.doesNotMatch(code, /fwidth\(/, "Ribbon is measuring pixels, which only a grid needs");
  });

  /// Ribbon: the bands are added, so where two cross the overlap is brighter
  /// than either. That is the whole of the effect — blended or maxed instead,
  /// four ribbons draw as four ribbons rather than as woven light.
  test("Ribbon's bands add where they cross", () => {
    const code = backdrop.backdropStyle("ribbon").body.replace(/\/\/[^\n]*/g, "");
    assert.match(code, /lit \+= band \* band/, "the bands no longer accumulate");
  });

  /// Cubes and Towers are the two halves of the same boot sequence and two
  /// separate styles. Towers was the one that was meant to be it and landed as
  /// something else; it is kept as what it became, and this is the scene it was
  /// aiming at.
  test("Cubes and Towers are both there and are not each other", () => {
    const ids = backdrop.BACKDROPS.map((b) => b.id);
    assert.ok(ids.includes("cubes"), "Cubes is gone");
    assert.ok(ids.includes("towers"), "Towers was replaced rather than kept");
    assert.notEqual(
      backdrop.backdropStyle("cubes").body,
      backdrop.backdropStyle("towers").body
    );
  });

  /// Cubes: two sparks the same colour is the failure that keeps coming back.
  /// Hashing the hue outright was five draws from one hat — blue, blue, cyan,
  /// green, green. Jittering an even spacing kept the same failure in
  /// miniature: two of them could close to twenty-nine degrees apart, which is
  /// two greens. An even fifth of the wheel is seventy-two degrees, always, and
  /// the guarantee is that there is nothing added to it.
  test("no two of Cubes' sparks can be the same colour", () => {
    const code = backdrop.backdropStyle("cubes").body.replace(/\/\/[^\n]*/g, "");
    const tone = /float tone = ([^;]+);/.exec(code)?.[1]?.trim();
    assert.equal(tone, "f / 5.0", "the spark hue is not an even fifth of the wheel any more");
  });

  /// Cubes: noise sampled on `atan` steps across the cut at the negative x
  /// axis, and what drew was a hard horizontal seam running left out of the
  /// middle of the glow — through the brightest part of the picture. The
  /// direction vector carries the same information with no cut in it.
  test("Cubes lays its haze out along the ray, not along the angle", () => {
    const body = backdrop.backdropStyle("cubes").body;
    const code = body.replace(/\/\/[^\n]*/g, "");
    assert.doesNotMatch(code, /atan/, "the angle is being sampled again");
    assert.match(code, /eye \/ rad/, "the direction vector is gone");
  });

  /// Cubes: a derivative asked for inside the hit branch is a derivative under
  /// non-uniform control flow, which is undefined rather than slow — the pixel
  /// next door may not have taken that branch at all. The pixel width is taken
  /// once, before the loop.
  test("Cubes takes its derivative outside the loop", () => {
    const code = backdrop.backdropStyle("cubes").body.replace(/\/\/[^\n]*/g, "");
    const declared = code.indexOf("fwidth(");
    const loop = code.indexOf("for (int i");
    assert.ok(declared > 0, "the pixel width is gone; the silhouettes are hard again");
    assert.ok(declared < loop, "fwidth moved inside the loop, where it is undefined");
  });

  /// Cubes: the blocks are grey glass with a cast of the scheme over them, not
  /// the scheme's own colour. Shaded through the ramp they came out navy on
  /// Midnight, and would come out green on Moss; glass does not do that. The
  /// haze is the thing that carries the scheme.
  test("Cubes shades its blocks in grey, not in the scheme", () => {
    const code = backdrop.backdropStyle("cubes").body.replace(/\/\/[^\n]*/g, "");
    // The assignment, not the declaration above the loop: `vec3 lit = vec3(0.0)`
    // is where the colour starts, and matching it tests nothing.
    const block = /^\s+lit = ([^;]+);/m.exec(code)?.[1] ?? "";
    assert.ok(block, "the block colour is gone");
    assert.doesNotMatch(block, /ramp\(/, "the blocks take the ramp's colour again");
    assert.match(block, /u_high/, "the scheme no longer reaches the blocks at all");
  });

  /// Cubes: abs() lit the face turned away from the light exactly as brightly
  /// as the one turned towards it, so all three faces of a block came out the
  /// same value and it read as a flat hexagon. One bright face, one middling
  /// and one nearly black is what stops it looking moulded.
  test("Cubes lights its faces from one side", () => {
    const code = backdrop.backdropStyle("cubes").body.replace(/\/\/[^\n]*/g, "");
    const sheen = /float sheen = ([^;]+);/.exec(code)?.[1] ?? "";
    assert.ok(sheen, "the diffuse term is gone");
    assert.doesNotMatch(sheen, /abs\(/, "the faces are being lit from both sides again");
  });

  /// Cubes: mostly transparent, and scaled down rather than clamped down. A
  /// clamp flattens the edges and the rim into the faces and leaves a uniform
  /// tile — it changes the shape, where scaling only turns it down.
  ///
  /// The factor itself is read rather than named. What it should be is a
  /// judgement that has been asked for twice and changed twice; what must not
  /// change is that there is one, that it is small, and that the clamp above it
  /// still opens all the way to one.
  test("Cubes' blocks are scaled down to mostly transparent, not clipped", () => {
    const code = backdrop.backdropStyle("cubes").body.replace(/\/\/[^\n]*/g, "");
    const cover = /^\s+cover = ([\s\S]*?);/m.exec(code)?.[1] ?? "";
    assert.ok(cover, "the block opacity is gone");
    assert.match(cover, /clamp\([^)]*, 0\.0, 1\.0\)/, "the opacity is being clipped, not scaled");
    const scale = Number(/\*\s*(0\.\d+)\s*\*\s*\(0\.32/.exec(cover)?.[1]);
    assert.ok(
      scale > 0 && scale < 0.5,
      `the blocks are ${scale} opaque at most, which is not "mostly transparent"`
    );
  });

  /// Cubes: the far edges seen through the near face are the whole difference
  /// between glass and a moulded plastic box. Shading only the entry face drew
  /// six flat panels with a line round them; a cube head-on has to show a
  /// smaller square inside it, and one at an angle the back corner crossing the
  /// front ones. Both intersections are already paid for — the exit distance is
  /// what the slab test answers — so dropping this saves nothing.
  test("Cubes shows the far edges through the near face", () => {
    const code = backdrop.backdropStyle("cubes").body.replace(/\/\/[^\n]*/g, "");
    assert.match(code, /vec3 pout = ro \+ rl \* tf;/, "the exit point is gone");
    assert.match(code, /float dout =/, "the far face's edges are no longer measured");
    assert.match(code, /float farEdge =/, "the far edges are not being drawn");
  });

  /// Grid: `max(p.y, 0.04)` froze the perspective divide across the bottom of
  /// the screen, so the lines stopped converging at a fixed height and ran
  /// straight down from it — and `fwidth` of a frozen coordinate is zero, so
  /// the same seam was also a divide by zero.
  test("Grid guards its divide instead of clamping what goes into it", () => {
    const body = backdrop.backdropStyle("grid").body;
    assert.doesNotMatch(body, /max\(p\.y,/, "the perspective input is clamped again");
    assert.match(body, /max\(fwidth\([^)]*\), *1e-4\)/, "fwidth can still be zero here");
  });
});

/// A backtick inside the shader source ends the JS template literal that holds
/// it, and the module then fails to *parse* — so every backdrop test goes red
/// at once and the message points at prose rather than at code. It has now
/// happened three times: in comments quoting a mix() call, a plus sign, and a
/// variable name.
///
/// Read as text, never through an import. The failure this guards against
/// stops the module loading at all, so a test that imports it cannot run to
/// report anything — it just dies with the rest.
/// `tools/backdrop-preview.html` compiles what the app compiles by calling
/// `fragmentFor`, so that it is not a second copy of the shared frame. An
/// export nothing in the app itself calls is exactly the kind that gets tidied
/// away, and the only sign would be a preview window that stopped working.
describe("the preview tool's way in", () => {
  test("fragmentFor assembles the frame around a style's body", () => {
    assert.equal(typeof backdrop.fragmentFor, "function", "the preview has no way in");
    for (const b of backdrop.BACKDROPS) {
      const src = backdrop.fragmentFor(b.id);
      assert.ok(src.startsWith("#version 300 es"), `${b.id} has no version line`);
      assert.ok(src.includes(b.body), `${b.id}'s body is not in its shader`);
      assert.ok(src.includes("void main()"), `${b.id} has no main`);
      assert.ok(src.trimEnd().endsWith("}"), `${b.id} does not close`);
    }
  });
});

describe("the shader source", () => {
  test("has no backtick inside a template literal", () => {
    const src = readFileSync(join(uiDir, "js/backdrop.js"), "utf8");
    // Every template literal in the file, and whether it holds a stray
    // backtick that would have closed it early. An unbalanced count is the
    // symptom: the parser sees the string end where the comment meant a quote.
    const bodies = [...src.matchAll(/body:\s*`([\s\S]*?)`,\n/g)].map((m) => m[1]);
    assert.ok(bodies.length >= 8, `only found ${bodies.length} shader bodies`);
    for (const b of bodies) {
      assert.ok(!b.includes("`"), "a shader body contains a backtick");
    }
    const head = /const SHADER_HEAD = `([\s\S]*?)`;/.exec(src)?.[1] ?? "";
    const tail = /const SHADER_TAIL = `([\s\S]*?)`;/.exec(src)?.[1] ?? "";
    assert.ok(head.length > 100 && tail.length > 50, "the shared frame was not found");
  });

  /// The shared tail declares its own names for the vignette, so a body that
  /// declares one too fails to compile and the style silently never switches.
  /// That is exactly how Sweep and Starfield both shipped broken — both used
  /// `d`, and nothing said a word.
  ///
  /// Comments are stripped first. GLSL is not the only thing in these strings:
  /// the comment explaining this very rule contains the words "float d", and
  /// scanning the raw text flagged the explanation as the offence.
  test("no body redeclares a name the shared frame already uses", () => {
    const src = readFileSync(join(uiDir, "js/backdrop.js"), "utf8");
    const strip = (glsl) => glsl.replace(/\/\/[^\n]*/g, "");
    const tail = strip(/const SHADER_TAIL = `([\s\S]*?)`;/.exec(src)?.[1] ?? "");
    const decls = (glsl) =>
      [...glsl.matchAll(/\b(?:float|vec2|vec3|int)\s+(\w+)/g)].map((m) => m[1]);
    const taken = new Set(decls(tail));
    assert.ok(taken.size, "the shared tail declares nothing, which cannot be right");
    assert.ok(taken.has("d"), "the tail's vignette variable is gone; update this test");
    for (const b of backdrop.BACKDROPS) {
      for (const name of decls(strip(b.body))) {
        assert.ok(
          !taken.has(name),
          `${b.id} redeclares "${name}", which the shared tail already declares`
        );
      }
    }
  });
});
