// A Windows 9x game's .w9x, as tools/win9x_pack.py writes it and the page
// reads it. The two are in different languages, so the format is pinned here
// with a file of the packer's exact shape.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const { parseW9x, win9xUrl, heavyQuestion, JSDOS, keptDisk, KEPT_BYTES } = await import("../js/win9x-player.js");

const W9X = `# Windows 9x game for the browser, written by tools/win9x_pack.py from
# eXo's !win9x/1996/AMBER - Journeys Beyond (1996)/Play.conf. The page fetches the files named
# below from eXoWin9x/ and boots this config in js-dos.
# zip = eXo/eXoWin9x/1996/AMBER - Journeys Beyond (1996).zip
# base = moose/moose98-3.vhd
# bytes = 564000000
# unpacked = 732000000
# file = AMBER - Journeys Beyond (1996)/AMBER_JB.moose.cue
# | FILE "amber_jb.bin" BINARY
# |   TRACK 01 MODE1/2352
# |
# |     INDEX 01 00:00:00
[dosbox]
memsize=64
[autoexec]
imgmount c c.vhd
imgmount e "AMBER - Journeys Beyond (1996)/AMBER_JB.moose.cue" -t cdrom -ide 2m
BOOT -l c
`;

describe("reading a .w9x", () => {
  test("the files to fetch and their sizes", () => {
    const g = parseW9x(W9X);
    assert.equal(g.zip, "eXo/eXoWin9x/1996/AMBER - Journeys Beyond (1996).zip");
    assert.equal(g.base, "moose/moose98-3.vhd");
    assert.equal(g.bytes, 564e6);
    assert.equal(g.unpacked, 732e6);
  });

  test("a corrected CD sheet comes back as the file it was", () => {
    const g = parseW9x(W9X);
    assert.deepEqual(Object.keys(g.files), ["AMBER - Journeys Beyond (1996)/AMBER_JB.moose.cue"]);
    assert.equal(
      g.files["AMBER - Journeys Beyond (1996)/AMBER_JB.moose.cue"],
      'FILE "amber_jb.bin" BINARY\r\n  TRACK 01 MODE1/2352\r\n\r\n    INDEX 01 00:00:00\r\n'
    );
  });

  test("the config is the whole text, for DOSBox-X to read past the comments", () => {
    assert.equal(parseW9x(W9X).conf, W9X);
  });

  test("a .w9x missing its names says so rather than fetching undefined", () => {
    const g = parseW9x("[autoexec]\nBOOT -l c\n");
    assert.equal(g.zip, undefined);
    assert.equal(g.base, undefined);
    assert.deepEqual(g.files, {});
  });
});

describe("the page and the service agree", () => {
  test("a path with spaces and brackets survives the query string", () => {
    const path = "eXo/eXoWin9x/1996/AMBER - Journeys Beyond (1996).zip";
    const u = new URL(win9xUrl(path), "https://dev.lan");
    assert.equal(u.pathname, "/win9x");
    assert.equal(u.searchParams.get("path"), path);
  });

  /// The page asks for js-dos by version, and the fetch script unpacks it
  /// under the version its manifest pins. One of them moving alone is a 404 on
  /// every Windows game.
  test("the js-dos version is the one the manifest pins", () => {
    const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const manifest = readFileSync(join(root, "assets/jsdos/MANIFEST.tsv"), "utf8");
    const row = manifest.split("\n").find((l) => l && !l.startsWith("#"));
    assert.equal(row.split("\t")[2], JSDOS);
  });

  /// The Android app and the desktop say it from Rust, the page from here.
  test("the devices and the page say the same thing about where these play", async () => {
    const { OWN_PLAYER } = await import("../js/ejs-systems.js");
    const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const rust = readFileSync(join(root, "src/commands.rs"), "utf8");
    const m = rust.match(/pub const PLAYS_IN_A_BROWSER: &str = "([^"]+)";/);
    assert.ok(m, "PLAYS_IN_A_BROWSER is gone from src/commands.rs");
    assert.equal(m[1], OWN_PLAYER.windows9x);
  });

  test("the packer writes the keys the page reads", () => {
    const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const packer = readFileSync(join(root, "tools/win9x_pack.py"), "utf8");
    for (const key of ["zip", "base", "bytes", "unpacked"]) {
      assert.match(packer, new RegExp(`f"# ${key} = `), `win9x_pack.py no longer writes "# ${key} = "`);
    }
    assert.match(packer, /f"# file = \{name\}"/);
    assert.match(packer, /f"# \| \{x\}"/);
  });
});

describe("asking before a big download", () => {
  test("a small game with the disk already kept is not asked about", () => {
    assert.equal(heavyQuestion("ZuluWar!", 4e6, 0, 11e6), null);
  });

  test("the first game says the disk is part of it, once", () => {
    const q = heavyQuestion("ZuluWar!", 4e6, 264e6, 11e6);
    assert.match(q, /264 MB Windows 98 disk/);
    assert.match(q, /after the first game/);
  });

  test("a game too big for a tab says so", () => {
    assert.match(heavyQuestion("Big", 900e6, 0, 4.8e9), /4800 MB/);
    assert.doesNotMatch(heavyQuestion("Big", 900e6, 0, 1e9), /more than a tab/);
  });

  /// Over plain HTTP there is no Cache Storage, so "keeps" would be a promise
  /// the next game breaks.
  test("a browser that cannot keep the disk is not told it will", () => {
    const q = heavyQuestion("ZuluWar!", 4e6, 264e6, 11e6, false);
    assert.doesNotMatch(q, /keeps/);
    assert.match(q, /every time/);
  });
});

describe("the stage stays up while a game runs", () => {
  /// Chrome closes a dialog on a second Escape even when `cancel` is
  /// prevented, and the game then ran on with no way to stop it.
  test("closed by the browser, it opens again", async () => {
    const { JSDOM } = await import("jsdom");
    const { keepOpen } = await import("../js/win9x-player.js");
    const doc = new JSDOM("<dialog open></dialog>").window.document;
    const stage = doc.querySelector("dialog");
    keepOpen(stage);
    stage.removeAttribute("open");
    stage.dispatchEvent(new doc.defaultView.Event("close"));
    assert.ok(stage.hasAttribute("open"));
  });

  /// `showModal` focuses Stop, and the game's next Enter pressed it.
  test("reopened, focus goes back to the game, not to Stop", async () => {
    const { JSDOM } = await import("jsdom");
    const { keepOpen } = await import("../js/win9x-player.js");
    const doc = new JSDOM(`<dialog open>
      <div class="ejs-bar"><button class="ejs-close">Stop</button></div>
      <div class="ejs-frame" tabindex="-1" autofocus><canvas tabindex="0"></canvas></div>
    </dialog>`).window.document;
    const stage = doc.querySelector("dialog");
    // jsdom has no showModal. This one does what Chrome's does first: focus
    // the [autofocus] element, which fires focusin before showModal returns.
    // Without it the test passed while Chrome left focus on the frame.
    stage.showModal = () => {
      stage.setAttribute("open", "");
      stage.querySelector("[autofocus]").focus();
    };
    keepOpen(stage);
    const canvas = doc.querySelector("canvas");
    canvas.focus();
    doc.querySelector(".ejs-close").focus();
    stage.removeAttribute("open");
    stage.dispatchEvent(new doc.defaultView.Event("close"));
    assert.equal(doc.activeElement, canvas);
  });

  test("taken out of the page, it stays closed", async () => {
    const { JSDOM } = await import("jsdom");
    const { keepOpen } = await import("../js/win9x-player.js");
    const doc = new JSDOM("<dialog open></dialog>").window.document;
    const stage = doc.querySelector("dialog");
    keepOpen(stage);
    stage.remove();
    stage.removeAttribute("open");
    stage.dispatchEvent(new doc.defaultView.Event("close"));
    assert.ok(!stage.hasAttribute("open"));
  });
});

describe("the Windows disk this browser kept", () => {
  /// A stand-in for one Cache Storage bucket holding one response.
  const bucketWith = (res) => {
    const gone = [];
    return {
      gone,
      match: async () => res,
      delete: async (url) => gone.push(url),
    };
  };
  const stored = (n, len = n) =>
    new Response(new Uint8Array(n), { headers: { [KEPT_BYTES]: String(len) } });

  test("a whole copy is used", async () => {
    const c = bucketWith(stored(16));
    assert.equal((await keptDisk(c, "/d")).length, 16);
    assert.deepEqual(c.gone, []);
  });

  /// Handed to DOSBox-X, a short disk stops every game at a Z:\ prompt.
  test("a short copy is thrown away, to be fetched again", async () => {
    const c = bucketWith(stored(10, 16));
    assert.equal(await keptDisk(c, "/d"), null);
    assert.deepEqual(c.gone, ["/d"]);
  });

  test("a copy the browser cannot read back is thrown away", async () => {
    const broken = { headers: new Headers({ [KEPT_BYTES]: "16" }), arrayBuffer: async () => { throw new Error("blob gone"); } };
    const c = bucketWith(broken);
    assert.equal(await keptDisk(c, "/d"), null);
    assert.deepEqual(c.gone, ["/d"]);
  });

  test("a copy kept before lengths were stored is fetched again once", async () => {
    const c = bucketWith(new Response(new Uint8Array(16)));
    assert.equal(await keptDisk(c, "/d"), null);
    assert.deepEqual(c.gone, ["/d"]);
  });

  test("no bucket at all is simply nothing kept", async () => {
    assert.equal(await keptDisk(null, "/d"), null);
  });
});
