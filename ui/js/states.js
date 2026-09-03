// Deleting a save state.
//
// One implementation, used from the shelf in the info pane and from the
// right-click menu on a game. They asked the same question and did the same
// thing, in two places, which is how one of them ends up with the confirmation
// setting and the other without it.

import { invoke } from "./state.js";
import { toast } from "./util.js";

/// Delete one state, asking first if the setting says to.
///
/// `slot` is an entry from `game_states`. Returns true if something was
/// actually deleted, so the caller can redraw.
export async function deleteState(id, slot) {
  const label = slot.label ?? `slot ${slot.slot}`;

  // Off by default. Clearing out old states is housekeeping done several at a
  // time, and a dialog for each turns a tidy-up into a chore. The file is
  // copied to the backups folder either way, so the undo exists whether or not
  // the question was asked — the question is only about whether the press was
  // meant.
  let ask = false;
  try {
    ask = await invoke("confirm_delete_state");
  } catch {
    // A setting that cannot be read is not a reason to skip the question.
    ask = true;
  }
  if (ask && !window.confirm(`Delete ${label}? A copy goes to the backups folder.`)) {
    return false;
  }

  try {
    toast(await invoke("delete_state", { id, slot: slot.slot }));
    return true;
  } catch (e) {
    toast(`Could not delete — ${e}`, 8000);
    return false;
  }
}
