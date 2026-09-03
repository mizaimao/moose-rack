// What a collection card shows for a picture.
//
// The card has always drawn one member's cover, which across a grid of forty
// collections reads as a wall of unrelated box art — the covers are doing the
// work a collection's own identity should. These are the alternatives, kept in
// one place so the setting, the renderer and the stylesheet agree on the names.

/// The choices, in the order Appearance offers them.
export const COLLECTION_ART = [
  ["single", "One cover", "The first member's box art. What it has always done."],
  ["fan", "Fanned covers", "Three covers overlapping, the first one in front."],
  ["tiles", "Four tiles", "A 2×2 of members — a real mosaic rather than one picture."],
  ["none", "Just the name", "No artwork at all. The quietest, and the fastest."],
];

const KEY = "collectionArt";

export function collectionArt() {
  const v = localStorage.getItem(KEY);
  return COLLECTION_ART.some(([id]) => id === v) ? v : "single";
}

export function setCollectionArt(id) {
  const ok = COLLECTION_ART.some(([x]) => x === id) ? id : "single";
  localStorage.setItem(KEY, ok);
  // Both windows: the setting lives in Settings and the cards live in the
  // library, exactly as the glass tint does.
  window.__TAURI__?.event?.emit?.("collection-art", ok);
  return ok;
}
