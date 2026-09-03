// The right-click menu.
//
// One implementation, because there were two places that wanted one and the
// first was written inline for save states — with no stylesheet, which is why
// it appeared past the end of the page and looked like a menu that did not
// work at all.
//
// A menu of our own rather than the browser's: this is an application, and
// WebKit's offers "Open Image in New Window" and "Copy Image" for a console
// icon, which are not things this app does.

let open = null;
/// Unhooks the listeners that watch for the menu being dismissed. Held here
/// rather than fired at the menu as an event: `closeMenu` runs *before* the
/// chosen item's action, so anything that can throw in it takes the action
/// with it.
let unwatch = null;

/// Show a menu of `items` at `(x, y)`.
///
/// Each item is `{ label, danger, run }`; a null item draws a separator. The
/// menu closes on the first click, key, or scroll after it opens, whichever
/// comes first — a menu that survives the next interaction is a menu people
/// end up clicking through.
export function showMenu(items, x, y) {
  closeMenu();
  const usable = items.filter(Boolean);
  if (!usable.length) return null;

  const menu = document.createElement("div");
  menu.className = "ctx-menu";
  for (const item of items) {
    if (!item) {
      menu.appendChild(document.createElement("hr"));
      continue;
    }
    const b = document.createElement("button");
    b.textContent = item.label;
    if (item.danger) b.className = "danger";
    // A line that says why there is nothing to do. Leaving it out instead
    // makes an empty answer look like a broken menu — which is exactly how
    // "no save states" read.
    if (item.disabled) {
      b.disabled = true;
      b.className = "dim";
    } else {
      b.addEventListener("click", () => {
        // Sticky items leave the menu up. A filter is built out of two or
        // three choices, and a menu that shuts on each one turns that into
        // four trips to the same button.
        if (!item.sticky) closeMenu();
        item.run();
      });
    }
    menu.appendChild(b);
  }

  // Positioned before measuring, then pulled back inside the window: a menu
  // opened near an edge would otherwise hang off it with no way to reach the
  // items that fell outside.
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  document.body.appendChild(menu);
  const box = menu.getBoundingClientRect();
  if (box.bottom > window.innerHeight) menu.style.top = `${Math.max(0, y - box.height)}px`;
  if (box.right > window.innerWidth) menu.style.left = `${Math.max(0, x - box.width)}px`;

  open = menu;
  // Anywhere but the menu itself.
  //
  // This closed on the next pointerdown wherever it landed — and pressing a
  // menu item *is* a pointerdown. The menu came off the page between the press
  // and the release, so the click never reached the button that was pressed
  // and every item in every menu in the app did nothing at all: the orders
  // here, and the delete on a save state. It looked like a menu that opens
  // fine and ignores you.
  const away = (ev) => {
    if (menu.contains(ev.target)) return;
    closeMenu();
  };
  unwatch = () => {
    window.removeEventListener("pointerdown", away);
    window.removeEventListener("keydown", closeMenu);
    window.removeEventListener("wheel", closeMenu);
    unwatch = null;
  };
  // Next frame, or the same click that opened this closes it again.
  setTimeout(() => {
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", closeMenu, { once: true });
    window.addEventListener("wheel", closeMenu, { once: true, passive: true });
  }, 0);
  return menu;
}

export function closeMenu() {
  // Unhooked here as well as on dismissal: a menu chosen from rather than
  // clicked away would otherwise leave a listener on the window for every menu
  // ever opened.
  unwatch?.();
  open?.remove();
  open = null;
}
