# Icons

[Lucide](https://lucide.dev), ISC licensed — see `LICENSE`.

Vendored rather than loaded from a CDN. The app has to work with no network:
the whole point is a local library you can browse and launch offline, and a
button that renders as a broken image when the internet is down is worse than
a text glyph.

## Using one

The webview loads these as plain files relative to the UI root:

```html
<button class="icon-btn" title="Settings">
  <img src="icons/settings.svg" alt="" width="18" height="18" />
</button>
```

Lucide strokes use `currentColor`, so an icon inherits the surrounding text
colour and needs no per-theme variant. Set `width`/`height` explicitly — the
source files are 24×24 and will render at that size otherwise.

## Adding one

Fetch the file from the Lucide repository rather than hand-editing an existing
icon, so it stays consistent with the set and updates cleanly:

```sh
curl -sSfLO --output-dir ui/icons \
  https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/<name>.svg
```

## What is here

Every icon in this folder, and where it is drawn. Anything that stops being
used should leave with the code that used it — a folder of "might be handy"
icons is a folder nobody can prune later.

| icon | drawn |
|---|---|
| `settings` | the gear that opens Settings |
| `search` | the header's search box |
| `list`, `layout-grid` | grid / list, in the header |
| `chevron-left` | Back |
| `play` | Play, and the video tile in the artwork strip |
| `download` | Take offline |
| `book-open` | the Manual tag |
| `external-link` | the Trailer tag, which leaves for a browser |
| `hard-drive-download` | a game that is on this machine |
| `cloud` | a game that is still on the server |
| `panel-right-open`, `panel-right-close` | the preview toggle |
| `arrow-down-narrow-wide` | the sort button |
| `funnel` | the filter button |
| `dice-5` | Random |
| `gamepad-2` | Sofa, and the pad badge in Settings |
| `columns-2` | Desk |
| `square` | *(unused — the old Sofa icon)* |
| `x` | close buttons |
| `star`, `house`, `folder`, `info`, `monitor`, `refresh-cw`, `arrow-left`, `arrow-right`, `grid-2x2` | *(unused)* |

The unused ones are kept because they are 300 bytes each and the set is easier
to browse whole. They are marked so nobody has to grep to find out.
