// Keeping a save made in the browser.
//
// A browser is another device, and the service already knows how to sync one.
// `src-service/src/saves.rs` holds the rule and it is not restated here: a
// difference is only resolvable when exactly one side moved, and when both
// moved it is a conflict that is never resolved silently. So this sends what
// the Flip sends -- a device id and a hash per save -- and does what the plan
// says. There is no second sync.
//
// Two things were measured before any of this was written, in a real browser
// against the live server:
//
// * `FS.writeFile(getSaveFilePath(), bytes)` then `loadSaveFiles()` puts a save
//   into the running core, and it takes: `getSaveFile()` flushes the core's own
//   SRAM to disk before reading, so 8192/8192 bytes coming back is the core's
//   memory and not an echo of the file.
// * EmulatorJS's own IndexedDB copy did NOT survive a reload in that test --
//   32 of 8192 bytes. So the browser is not a place a save can be left, which
//   is what makes this worth doing rather than a convenience.
//
// The download happens on the emulator's `start` event, deliberately after its
// own restore rather than racing it. Strictly later cannot lose.

import { md5 } from "./md5.js";

const DEVICE_KEY = "moose_device_id";

/// This browser, as a device the server can keep bookkeeping against.
///
/// Per browser, not per account. The guest account is shared, so two people
/// reading the same library would otherwise share one device id -- and the
/// conflict rule works by asking what *this device* last agreed with the
/// server, which two devices sharing an id cannot answer.
export async function deviceId() {
  try {
    const saved = localStorage.getItem(DEVICE_KEY);
    if (saved) return saved;
  } catch {
    // Private window: register a fresh one each session rather than refuse.
  }
  const name = `browser-${Math.random().toString(36).slice(2, 8)}`;
  const r = await fetch("/api/devices", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name }),
  });
  if (!r.ok) throw new Error(`registering this browser: ${r.status}`);
  const { id } = await r.json();
  try {
    localStorage.setItem(DEVICE_KEY, id);
  } catch {
    // Not remembering it costs a device row, not a save.
  }
  return id;
}

/// What the server holds for one game.
export async function serverSaves(romId) {
  const r = await fetch(`/api/saves?rom_id=${encodeURIComponent(romId)}`);
  if (!r.ok) return [];
  return r.json();
}

/// Ask the service what should happen, using the same negotiate every other
/// device uses.
export async function negotiate(device, saves) {
  const r = await fetch("/api/sync/negotiate", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ device_id: device, saves }),
  });
  if (!r.ok) throw new Error(`negotiate: ${r.status}`);
  return r.json();
}

/// Send one save up. `overwrite` only after a conflict a person has answered.
export async function upload(device, romId, fileName, bytes, overwrite = false) {
  const form = new FormData();
  form.append("saveFile", new Blob([bytes]), fileName);
  const q = new URLSearchParams({ rom_id: String(romId), device_id: device });
  if (overwrite) q.set("overwrite", "true");
  const r = await fetch(`/api/saves?${q}`, { method: "POST", body: form });
  // 409 is the service saying both sides moved. It is an answer, not a fault.
  if (r.status === 409) return { conflict: true, detail: await r.json().catch(() => ({})) };
  if (!r.ok) throw new Error(`upload: ${r.status}`);
  return { conflict: false, save: await r.json().catch(() => null) };
}

export async function download(saveId) {
  const r = await fetch(`/api/saves/${encodeURIComponent(saveId)}/content`);
  if (!r.ok) throw new Error(`download: ${r.status}`);
  return new Uint8Array(await r.arrayBuffer());
}

/// The save this game is holding right now, and what it is called.
///
/// `getSaveFile()` flushes the core before reading, so this is what the game
/// has actually got rather than what was last written to disk.
export function readSave(gm) {
  try {
    const bytes = gm?.getSaveFile?.();
    if (!bytes || !bytes.length) return null;
    const path = gm.getSaveFilePath();
    return { bytes, fileName: path.split("/").pop() };
  } catch {
    return null;
  }
}

/// Put a save into the running core.
export function writeSave(gm, bytes) {
  gm.FS.writeFile(gm.getSaveFilePath(), bytes);
  gm.loadSaveFiles();
}

/// One game's worth of sync, in the direction the plan says.
///
/// Returns what happened so the caller can say it out loud: a save moving
/// between machines silently is how somebody loses an evening without ever
/// being told which copy won.
export async function syncOne(gm, romId, { onConflict } = {}) {
  const device = await deviceId();
  const here = readSave(gm);
  const mine = here
    ? [{
        rom_id: romId,
        file_name: here.fileName,
        content_hash: md5(here.bytes),
        updated_at: new Date().toISOString(),
        file_size_bytes: here.bytes.length,
      }]
    : [];

  const plan = await negotiate(device, mine);
  const op = (plan.operations ?? []).find((o) => o.rom_id === romId);
  if (!op) return { action: "no_op", why: "nothing to do" };

  if (op.action === "upload") {
    const res = await upload(device, romId, here.fileName, here.bytes);
    if (!res.conflict) return { action: "upload", bytes: here.bytes.length };
    // Both moved. Ask, never guess -- this is somebody's evening.
    const keep = await onConflict?.(op);
    if (keep !== "mine") return { action: "conflict", resolved: false };
    await upload(device, romId, here.fileName, here.bytes, true);
    return { action: "upload", bytes: here.bytes.length, forced: true };
  }

  if (op.action === "download") {
    const bytes = await download(op.save_id);
    writeSave(gm, bytes);
    return { action: "download", bytes: bytes.length };
  }

  if (op.action === "conflict") {
    const keep = await onConflict?.(op);
    if (keep === "mine" && here) {
      await upload(device, romId, here.fileName, here.bytes, true);
      return { action: "upload", bytes: here.bytes.length, forced: true };
    }
    if (keep === "theirs") {
      const bytes = await download(op.save_id);
      writeSave(gm, bytes);
      return { action: "download", bytes: bytes.length, forced: true };
    }
    return { action: "conflict", resolved: false };
  }

  return { action: op.action ?? "no_op" };
}
