// Playing in the window on the desktop.
//
// The same player runs in two places that answer "where is EmulatorJS" and
// "where is the game" differently: over HTTP from the service, or off disk
// through the app's own URI scheme. Getting that wrong is silent -- the loader
// concatenates rather than resolving, so a base with no trailing slash fetches
// every core from one directory up and the emulator sits on "Downloading core"
// with nothing in the console.
//
// The save path differs too, and for a reason worth stating: a browser on
// another machine is a different device and negotiates, while the desktop
// window is another emulator on the machine that holds the library. Its save
// has to be the file RetroArch reads, or one game ends up with two saves on one
// disk and the sync has to pick between them.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { urlsFor, ejsUrls, forgetEjsUrls } from "../js/ejs-base.js";
import { toBase64, fromBase64, localSaveIn, localSaveOut } from "../js/browser-saves.js";

describe("where the player gets its files", () => {
  beforeEach(() => {
    forgetEjsUrls();
    delete globalThis.__MOOSE_WEB;
  });

  test("the web build uses the service's own routes", () => {
    const u = urlsFor({ web: true });
    assert.equal(u.available, true);
    assert.equal(u.desktop, false);
    assert.equal(u.data, "/emulatorjs/data/");
    assert.equal(u.rom(-10793), "/rom?id=-10793");
  });

  test("the desktop builds on the scheme the app reported", () => {
    const u = urlsFor({ web: false, base: "moose://localhost/" });
    assert.equal(u.available, true);
    assert.equal(u.desktop, true);
    assert.equal(u.data, "moose://localhost/data/");
    assert.equal(u.rom(-10793), "moose://localhost/rom/-10793");
  });

  test("a base with no trailing slash gets one", () => {
    // Without it `EJS_pathtodata` is `moose://localhost/data` and every core is
    // fetched from the parent, which 404s with no error anywhere.
    assert.equal(urlsFor({ web: false, base: "moose://localhost" }).data, "moose://localhost/data/");
  });

  test("Windows' mapped scheme works the same way", () => {
    const u = urlsFor({ web: false, base: "http://moose.localhost/" });
    assert.equal(u.data, "http://moose.localhost/data/");
    assert.equal(u.rom(7), "http://moose.localhost/rom/7");
  });

  test("no EmulatorJS on this install is not available, not broken", () => {
    assert.deepEqual(urlsFor({ web: false, base: null }), { available: false, desktop: true });
  });

  test("an answer of the wrong shape is not treated as a URL", () => {
    // An older build with no such command, or a stub that answers everything
    // with an object. `[object Object]data/` is not a diagnosable failure.
    for (const base of [{}, 42, [], undefined])
      assert.equal(urlsFor({ web: false, base }).available, false);
  });

  test("the web build never asks the backend", async () => {
    globalThis.__MOOSE_WEB = true;
    let asked = 0;
    const u = await ejsUrls(async () => {
      asked += 1;
      return null;
    });
    assert.equal(asked, 0);
    assert.equal(u.data, "/emulatorjs/data/");
  });

  test("the answer is asked for once and remembered", async () => {
    let asked = 0;
    const invoke = async () => {
      asked += 1;
      return "moose://localhost/";
    };
    await ejsUrls(invoke);
    await ejsUrls(invoke);
    assert.equal(asked, 1);
  });

  test("a backend that throws leaves it unavailable rather than failing", async () => {
    const u = await ejsUrls(async () => {
      throw new Error("unknown command browser_play_base");
    });
    assert.equal(u.available, false);
  });
});

describe("the desktop's save is this machine's save file", () => {
  /// The bits of EmulatorJS's `gameManager` these two touch.
  function fakeCore(sram) {
    const written = [];
    return {
      written,
      gm: {
        getSaveFile: () => sram,
        getSaveFilePath: () => "/data/saves/Snes9x/ActRaiser (USA).srm",
        FS: { writeFile: (path, bytes) => written.push({ path, bytes }) },
        loadSaveFiles: () => written.push({ loaded: true }),
      },
    };
  }

  test("a save on disk is put into the running core", async () => {
    const { gm, written } = fakeCore(new Uint8Array(8));
    const bytes = new Uint8Array([1, 2, 3, 4]);
    const out = await localSaveIn(gm, -10793, async (cmd, args) => {
      assert.equal(cmd, "local_save");
      assert.equal(args.id, -10793);
      return { file_name: "ActRaiser (USA).srm", data: toBase64(bytes) };
    });
    assert.equal(out.action, "download");
    assert.equal(out.bytes, 4);
    assert.deepEqual([...written[0].bytes], [1, 2, 3, 4]);
    // Written *and* loaded: `FS.writeFile` alone puts it on the emulator's
    // virtual disk and leaves the core running on its old memory.
    assert.equal(written[1].loaded, true);
  });

  test("no save on this machine is a no-op, not an empty write", async () => {
    const { gm, written } = fakeCore(new Uint8Array(8));
    const out = await localSaveIn(gm, 1, async () => null);
    assert.equal(out.action, "no_op");
    assert.equal(written.length, 0);
  });

  test("what the core is holding goes back to disk", async () => {
    const sram = new Uint8Array([9, 8, 7]);
    const { gm } = fakeCore(sram);
    let sent = null;
    const out = await localSaveOut(gm, -10793, async (cmd, args) => {
      assert.equal(cmd, "put_local_save");
      sent = args;
      return "saved";
    });
    assert.equal(out.action, "upload");
    assert.deepEqual([...fromBase64(sent.data)], [9, 8, 7]);
    assert.equal(sent.id, -10793);
  });

  test("an untouched cartridge is not written out", async () => {
    // All-zero SRAM is what a core that has never loaded a save holds. Writing
    // it would overwrite a real save on disk with nothing, which is the one
    // outcome that loses an evening.
    const { gm } = fakeCore(new Uint8Array(8192));
    let called = false;
    const out = await localSaveOut(gm, 1, async () => {
      called = true;
    });
    assert.equal(out.action, "no_op");
    assert.equal(called, false);
  });

  test("base64 survives a save too big to spread into an argument list", () => {
    // `String.fromCharCode(...bytes)` on this blows the stack, which is why the
    // encoder goes in chunks. A DS save is 512 KB.
    const big = new Uint8Array(512 * 1024);
    for (let i = 0; i < big.length; i += 1) big[i] = i % 251;
    const back = fromBase64(toBase64(big));
    assert.equal(back.length, big.length);
    assert.deepEqual([...back.subarray(0, 300)], [...big.subarray(0, 300)]);
    assert.deepEqual([...back.subarray(big.length - 300)], [...big.subarray(big.length - 300)]);
  });
});
