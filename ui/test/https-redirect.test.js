// The web page moves itself from http to https.
//
// Its look and layout live in the browser's storage, which is kept per address,
// so http://dev.lan and https://dev.lan were two differently set-up apps. The
// inline script at the top of each page is run here against a stand-in
// location, since jsdom cannot navigate.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

for (const page of ["index.html", "settings.html"]) {
  const html = readFileSync(new URL(`../${page}`, import.meta.url), "utf8");
  const code = html.match(/<script>\s*(if \(location\.protocol === "http:"[\s\S]*?)<\/script>/)?.[1];

  const run = (href) => {
    const u = new URL(href);
    let went = null;
    const location = {
      protocol: u.protocol, hostname: u.hostname, pathname: u.pathname,
      search: u.search, hash: u.hash, replace: (to) => (went = to),
    };
    new Function("location", code)(location);
    return went;
  };

  test(`${page}: a browser on http is sent to the same page on https`, () => {
    assert.ok(code, "the redirect script is missing");
    assert.equal(run("http://dev.lan/?x=1#y"), "https://dev.lan/?x=1#y");
    assert.equal(run("http://dev.lan:80/settings.html"), "https://dev.lan/settings.html");
  });

  test(`${page}: https, and the machine itself, stay where they are`, () => {
    assert.equal(run("https://dev.lan/"), null);
    for (const h of ["localhost:8080", "127.0.0.1", "[::1]", "app.localhost"]) {
      assert.equal(run(`http://${h}/`), null, h);
    }
  });
}
