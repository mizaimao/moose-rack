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
    // A core that has never loaded a save still has SRAM, and it is all zeros.
    // Reporting that as a save makes the very first sync of a game you played
    // on the handheld a *conflict*: both sides have something, they differ, and
    // this device has agreed to nothing yet. Which is safe -- nothing is
    // overwritten -- and wrong, because there is nothing here to weigh against
    // an actual save. An empty cartridge is not a save.
    if (bytes.every((b) => b === 0)) return null;
    // Emscripten's virtual filesystem, not the host's. It is POSIX on every
    // platform -- `/data/saves/Snes9x/ActRaiser (USA).srm` on Windows too --
    // because the core is a WebAssembly build with its own MEMFS. So this
    // separator is the right one and not a host assumption.
    const path = gm.getSaveFilePath(); // separator-literal-ok
    return { bytes, fileName: path.split("/").pop() }; // separator-literal-ok
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


// --- Save states -------------------------------------------------------------
//
// A different thing from a save, and synced differently on purpose. A save is
// the cartridge's battery memory and it changes continuously, so it is flushed
// on a timer. A state is a freeze-frame somebody made deliberately, at a moment
// they chose, and there is nothing to merge: `/api/states` takes what it is
// given and answers no conflict, because a freeze-frame from one emulator build
// cannot be reconciled with another.
//
// So states are not automatic. Making one and restoring one are both acts, and
// the play bar has a button for each.

/// Every state the server holds for this game.
export async function serverStates(romId) {
  const r = await fetch(`/api/states?rom_id=${encodeURIComponent(romId)}`);
  if (!r.ok) return [];
  return r.json();
}

/// Freeze the running game and send it up.
///
/// The emulator name travels with it: a state belongs to the core that made it,
/// and restoring a snes9x state into a different SNES core is a crash rather
/// than a wrong picture.
export async function pushState(gm, romId, { core, name } = {}) {
  const bytes = gm.getState();
  if (!bytes || !bytes.length) throw new Error("the core produced no state");
  const form = new FormData();
  const fileName = name ?? `${new Date().toISOString().replace(/[:.]/g, "-")}.state`;
  form.append("stateFile", new Blob([bytes]), fileName);
  const q = new URLSearchParams({ rom_id: String(romId) });
  if (core) q.set("emulator", core);
  const r = await fetch(`/api/states?${q}`, { method: "POST", body: form });
  if (!r.ok) throw new Error(`saving the state: ${r.status}`);
  return r.json();
}

/// Bring one back into the running game.
export async function pullState(gm, stateId) {
  const r = await fetch(`/api/states/${encodeURIComponent(stateId)}/content`);
  if (!r.ok) throw new Error(`loading the state: ${r.status}`);
  const bytes = new Uint8Array(await r.arrayBuffer());
  gm.loadState(bytes);
  return bytes.length;
}


// --- The desktop window ------------------------------------------------------
//
// A browser on another machine is a different device and negotiates. The
// desktop window is not: it is another emulator on the machine that holds the
// library, so its save has to *be* the file RetroArch reads and the file
// `sync_saves` sends. Anything else would give one game two saves on one disk,
// and the sync would then have to pick between them.
//
// So there is no negotiation here at all -- read the local save on the way in,
// write it back on the way out -- and the device's existing sync carries it to
// the server unchanged, the same as a game played in RetroArch.

/// Base64 for bytes, in chunks: `String.fromCharCode(...bytes)` on a megabyte
/// of save data is an argument list long enough to blow the stack.
export function toBase64(bytes) {
  let out = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    out += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(out);
}

export function fromBase64(text) {
  const bin = atob(text);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
  return out;
}

/// Put this machine's save into the running core, if it has one.
export async function localSaveIn(gm, romId, invoke) {
  const held = await invoke("local_save", { id: romId });
  if (!held?.data) return { action: "no_op", why: "no save on this machine" };
  const bytes = fromBase64(held.data);
  if (!bytes.length) return { action: "no_op", why: "empty save" };
  writeSave(gm, bytes);
  return { action: "download", bytes: bytes.length, fileName: held.file_name };
}

/// Write what the core is holding back to this machine's save file.
export async function localSaveOut(gm, romId, invoke) {
  const here = readSave(gm);
  if (!here) return { action: "no_op", why: "nothing in the cartridge yet" };
  await invoke("put_local_save", { id: romId, data: toBase64(here.bytes) });
  return { action: "upload", bytes: here.bytes.length };
}
