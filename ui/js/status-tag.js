// The tag in the top-right corner: what it says and what colour its dot is.
//
// Its own file because it is the one piece of `main.js` worth testing on its
// own -- importing `main.js` starts the whole app -- and because the decision
// it makes is not obvious: the backend reports the same thing whether you are
// sitting at the machine or looking at it from another room, so the difference
// is drawn from the address in the browser's own location bar.

export function bareUrl(url) {
  return String(url ?? "")
    .replace(/^https?:\/\//, "")
    .replace(/\/+$/, "");
}

/// Where the person looking at this is, relative to the backend answering it.
///
/// The backend cannot tell. It sees the same `status` either way: a library on
/// this machine and no upstream server. Only the browser knows which address
/// was typed to reach the page, so the difference between "I am sitting at the
/// machine" and "I am looking at it from somewhere else" is decided here.
///
/// The desktop app is always local: it serves its own window over a Tauri
/// protocol rather than over http, so anything that is not http counts as being
/// at the machine.
export function viewerIsRemote(loc = globalThis.location) {
  if (!loc || !/^https?:$/.test(loc.protocol || "")) return false;
  const h = (loc.hostname || "").toLowerCase();
  return !(
    h === "localhost" ||
    h === "127.0.0.1" ||
    h === "::1" ||
    h === "[::1]" ||
    h === "0.0.0.0" ||
    h === "" ||
    h.endsWith(".localhost")
  );
}

/// Whether the link to the library is encrypted, for the hover card and the gold
/// border on the tag. `null` when there is no link to describe.
///
/// In a browser the page itself is that link, so the address bar decides: the
/// same service answers on http and https, and only the browser knows which was
/// typed. The desktop app's window is local, so there it is the link the backend
/// makes to the server named in config.toml.
export function secureLink(s, loc = globalThis.location) {
  if (loc && /^https?:$/.test(loc.protocol || "")) return loc.protocol === "https:";
  if (!s.connected) return null;
  return /^https:\/\//i.test(String(s.server ?? ""));
}

/// The words for the hover card's Connection line.
export function linkLabel(secure) {
  if (secure === null) return "no server";
  return secure ? "Encrypted (HTTPS)" : "Not encrypted (HTTP)";
}

/// The three things the tag can mean, as one decision.
///
/// * a server name, when this backend syncs from one -- the usual case, and the
///   only one where the word names a machine that is not this one;
/// * "server mode", when the backend has no upstream but you are looking at it
///   from another machine, which is what this service is for;
/// * "offline", when there is no upstream and you are sitting at the machine.
///
/// Returned rather than assigned so it can be tested without a document.
export function statusTag(s, remote = viewerIsRemote()) {
  if (!s.configured && !remote) return { text: "no config.toml", state: "unset" };
  if (s.connected) return { text: bareUrl(s.server), state: "on" };
  if (remote) return { text: "server mode", state: "serving" };
  return { text: "offline", state: "off" };
}
