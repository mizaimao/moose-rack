//! The Icon sets tab: preview an ES-DE theme's console artwork before fetching it.
//!
//! Every theme in the official ES-DE list that carries per-system artwork —
//! 54 of the 65. The other eleven draw a console's frontend rather than its
//! systems, so there is nothing here for them to offer.
//!
//! The list comes from `moose_rack::iconart`, keyed by the same `reponame`
//! the published themes list uses, so the two join on a key rather than by
//! matching names.
//!
//! ## What a preview shows
//!
//! The set's own console pictures, fetched one file at a time straight from
//! the theme's repository — see `moose_rack::iconart`. Not the author's
//! screenshots, which is what this tab did first and which turned out to be
//! misleading: a screenshot is a picture of the theme's *interface*, and the
//! interface is the one part of a theme this app never installs.
//!
//! ## Sets against the shared pool
//!
//! `fetch_icons` fills `_platforms/<style>/` from four themes at once, taking
//! the best available picture of each kind for each console. A set instead
//! keeps one designer's work together under `_platforms/sets/<set>/<style>/`.
//! Both exist; `icons.set` chooses between them, and a set with no picture for
//! some console falls through to the pool rather than leaving a hole.

use serde::Serialize;

/// A readable name for a set the published themes list no longer carries.
///
/// `retromega-revisited-es-de` -> `Retromega Revisited`. Only ever a fallback:
/// while the list has the theme, its own spelling is used.
pub fn pretty(dir: &str) -> String {
    dir.trim_end_matches("-es-de")
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Serialize)]
pub struct IconSetView {
    /// What the tab prints, from the themes list rather than [`WANTED`], so it
    /// reads as its author named it.
    pub name: String,
    /// Stable identifier, and the folder the art is kept under.
    pub dir: String,
    pub author: String,
    pub variants: usize,
    /// Raw URLs of this theme's own console pictures, for a few of the
    /// library's systems. The webview loads them directly.
    ///
    /// Not the author's screenshots, which is what this held first: a
    /// screenshot shows the theme's interface, and the interface is the one
    /// part of a theme this app never takes.
    pub icons: Vec<String>,
    /// Which kinds of picture the set carries — hardware, controller, logo.
    pub kinds: Vec<String>,
    /// The set draws system *names* and nothing else. Three of the nine do.
    pub wordmarks_only: bool,
    /// Pictures already downloaded for this set, across every style.
    pub installed: usize,
    /// The set the grid is currently drawing from.
    pub active: bool,
    /// Set when the themes list no longer carries this one.
    pub missing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_with_no_published_entry_still_gets_a_readable_name() {
        assert_eq!(pretty("retromega-revisited-es-de"), "Retromega Revisited");
        assert_eq!(pretty("codywheel-es-de"), "Codywheel");
        assert_eq!(pretty("x-grid-es-de"), "X Grid");
    }
}
