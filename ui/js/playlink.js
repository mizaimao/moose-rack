// `moose-rack://play/<platform>/<file>`: how the web page hands a game it
// can't run to the desktop app on the same computer. The app's side is
// `src/playlink.rs`, which reads exactly what `playLink` writes. See
// `docs/server-and-client.md`.

/// The link for one game.
export function playLink(platform, file) {
  return `moose-rack://play/${encodeURIComponent(platform)}/${encodeURIComponent(file)}`;
}

/// How a link is followed. An object so tests can catch the address instead
/// of the page trying to navigate to it.
///
/// A clicked link rather than `location.href`: browsers treat both the same
/// for an app's scheme, and a link is what Steam's and Zoom's pages use.
export const opener = {
  open(url, doc = document) {
    const a = doc.createElement("a");
    a.href = url;
    a.rel = "noopener";
    a.style.display = "none";
    doc.body.append(a);
    a.click();
    a.remove();
  },
};

/// Open the link for a game, and return what to tell the user now.
///
/// A page can't ask whether an app took a link. When one does, the browser
/// asks "Open Moose Rack?" or the app comes to the front, and either takes
/// focus from the page. If the page still has it `wait` ms later, nothing took
/// the link, and most likely the app isn't installed on this computer.
export function openInApp(platform, file, { toast, win = window, doc = document, wait = 3000 } = {}) {
  let left = false;
  const away = () => {
    left = true;
  };
  win.addEventListener("blur", away);
  doc.addEventListener("visibilitychange", away);
  opener.open(playLink(platform, file), doc);
  win.setTimeout(() => {
    win.removeEventListener("blur", away);
    doc.removeEventListener("visibilitychange", away);
    if (!left && doc.hasFocus()) {
      toast?.("Moose Rack didn't open. This game needs the desktop app on this computer.", 10_000);
    }
  }, wait);
  return "Opening in Moose Rack…";
}
