//! Which picture the app itself wears — in the Dock, the taskbar, the switcher.
//!
//! A registry rather than a pair of hardcoded paths, because the point of the
//! setting is that the list grows. Adding an icon is two steps and no logic:
//! drop a 1024x1024 PNG into `assets/appicons/`, add a line to [`ICONS`]. The
//! build script turns every PNG in that folder into the shapes each OS wants
//! without being told which ones exist, and the Settings picker draws whatever
//! this list holds.
//!
//! The id is the PNG's filename without its extension, and it is what goes into
//! `config.toml`. So an id is permanent once shipped: renaming one silently
//! resets everybody who chose it back to the default.

use anyhow::{bail, Result};

/// One choice in the picker.
pub struct AppIcon {
    /// Filename stem in `assets/appicons/`, and the value stored in config.
    pub id: &'static str,
    /// What the picker calls it. Not drawn — the picker shows the picture,
    /// since that is the whole question — but it is the button's accessible
    /// name and its tooltip.
    pub label: &'static str,
}

/// Every icon that ships. Order is the order the picker draws them.
pub const ICONS: &[AppIcon] = &[
    AppIcon {
        id: "arcade",
        label: "Arcade cabinet",
    },
    AppIcon {
        id: "shelf",
        label: "Shelf of cases",
    },
];

/// What a fresh install wears, and what an unrecognized id falls back to.
pub const DEFAULT: &str = "arcade";

/// The icon with this id, if it is one that ships.
pub fn find(id: &str) -> Option<&'static AppIcon> {
    ICONS.iter().find(|i| i.id == id)
}

/// The chosen icon, or the default when the config names one that no longer
/// exists — an id that was dropped between releases must not leave the app
/// with no icon at all.
pub fn chosen(configured: Option<&str>) -> &'static AppIcon {
    configured
        .and_then(find)
        .or_else(|| find(DEFAULT))
        .unwrap_or(&ICONS[0])
}

/// Store the choice. Refuses an unknown id rather than writing it: the file is
/// read on every launch, and a typo there would be a silent reset.
pub fn set(id: &str) -> Result<&'static AppIcon> {
    let Some(icon) = find(id) else {
        bail!("{id} is not an icon this build ships");
    };
    crate::config::set_table_entry("config.toml", "appearance", "app_icon", id)?;
    Ok(icon)
}

/// The macOS icon file inside an icon's built folder.
pub fn icns_name(id: &str) -> String {
    format!("{id}.icns")
}

/// The picture the Settings picker draws — rounded, so what it shows is what
/// the Dock will show.
pub const PREVIEW_NAME: &str = "preview.png";

/// The square PNG the window icon is set from on Windows and Linux, where the
/// window and taskbar draw an icon and macOS does not.
pub const WINDOW_NAME: &str = "256x256.png";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_a_real_icon() {
        assert!(find(DEFAULT).is_some(), "DEFAULT names an icon not in ICONS");
    }

    #[test]
    fn ids_are_unique() {
        for (i, icon) in ICONS.iter().enumerate() {
            assert!(
                !ICONS[..i].iter().any(|o| o.id == icon.id),
                "duplicate icon id {}",
                icon.id
            );
        }
    }

    #[test]
    fn an_unknown_choice_falls_back_rather_than_blanking() {
        assert_eq!(chosen(Some("no-such-icon")).id, DEFAULT);
        assert_eq!(chosen(None).id, DEFAULT);
        assert_eq!(chosen(Some("shelf")).id, "shelf");
    }
}
