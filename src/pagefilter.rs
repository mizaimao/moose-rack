// Search inside the page you are looking at.
//
// Not the library-wide search, which spans everything and takes you to a
// different screen. This one narrows what is already in front of you —
// thirty-five consoles, twenty-seven collections, two and a half thousand
// arcade games — and leaves you where you are.
//
// Two rules, and both of them are about not producing a page that looks
// broken.

/// Does this entry survive the text typed into the box?
///
/// Generous on purpose. Case and surrounding space are ignored, and the match
/// is anywhere in the name rather than at the start, because a filter that
/// misses things is worse than one that is too generous: a miss looks like the
/// game is not in the library.
///
/// Empty text keeps everything, which is how clearing the box puts the page
/// back.
pub fn matches(name: &str, query: &str) -> bool {
    let query = normalize(query);
    query.is_empty() || name.to_lowercase().contains(&query)
}

/// The typed text, as it is compared.
///
/// Exposed because callers hold the query across many rows and normalising it
/// once per row on 2,506 games is 2,506 allocations per keystroke.
pub fn normalize(query: &str) -> String {
    query.trim().to_lowercase()
}

/// Which of `names` survive.
pub fn visible(names: &[String], query: &str) -> Vec<bool> {
    let query = normalize(query);
    if query.is_empty() {
        return vec![true; names.len()];
    }
    names.iter().map(|n| n.to_lowercase().contains(&query)).collect()
}

/// Whether each group heading should go too.
///
/// A search that leaves five console headings and no games under any of them
/// reads as a broken page rather than as a search with no results. `groups`
/// holds the indices into `visible` that sit under each heading.
///
/// A heading with no members at all stays: an empty group is the page's own
/// business, and hiding it would make an unfiltered page differ from itself.
pub fn empty_groups(groups: &[Vec<usize>], visible: &[bool], query: &str) -> Vec<bool> {
    if normalize(query).is_empty() {
        return vec![false; groups.len()];
    }
    groups
        .iter()
        .map(|g| !g.is_empty() && !g.iter().any(|&i| visible.get(i).copied().unwrap_or(false)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["Super Mario Bros", "Metroid", "SUPER METROID", "Zelda"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    fn kept(query: &str) -> Vec<String> {
        let names = names();
        visible(&names, query)
            .into_iter()
            .zip(&names)
            .filter(|(v, _)| *v)
            .map(|(_, n)| n.clone())
            .collect()
    }

    /// Ported from `ui/test/pagefilter.test.js`.
    #[test]
    fn it_keeps_what_matches_and_hides_the_rest() {
        assert_eq!(kept("metroid"), ["Metroid", "SUPER METROID"]);
    }

    #[test]
    fn case_does_not_matter_and_neither_does_stray_space() {
        assert_eq!(kept("  MeTrOiD  "), ["Metroid", "SUPER METROID"]);
    }

    #[test]
    fn emptying_it_puts_everything_back() {
        assert_eq!(kept("").len(), 4);
        assert_eq!(kept("   ").len(), 4, "a box holding only spaces hid the page");
    }

    /// A filter that misses things is worse than one that is too generous: a
    /// miss reads as the game not being in the library at all.
    #[test]
    fn it_matches_anywhere_in_the_name_not_only_the_start() {
        assert_eq!(kept("mario"), ["Super Mario Bros"]);
    }

    /// A search that leaves five headings and no games reads as a broken page.
    #[test]
    fn a_heading_with_nothing_left_under_it_goes_too() {
        let names = names();
        let groups = vec![vec![0], vec![1, 2], vec![3]];
        let seen = visible(&names, "metroid");
        assert_eq!(empty_groups(&groups, &seen, "metroid"), [true, false, true]);
    }

    /// An empty group is the page's own business. Hiding it would make an
    /// unfiltered page differ from itself.
    #[test]
    fn an_unfiltered_page_hides_no_headings() {
        let names = names();
        let groups = vec![vec![0], vec![], vec![1, 2, 3]];
        let seen = visible(&names, "");
        assert_eq!(empty_groups(&groups, &seen, ""), [false, false, false]);
    }
}
