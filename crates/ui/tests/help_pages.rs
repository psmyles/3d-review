//! The manual must be reachable and coherent from inside the app.
//!
//! The build script already fails on a page that does not exist, a `SUMMARY.md`
//! entry that names nothing, and a link to a missing page. These check what it
//! cannot: that every way *in* lands somewhere real, that every image the pages
//! reference is actually in the repository, and that a fragment link points at a
//! heading rather than at nothing.

use std::collections::HashSet;
use std::path::PathBuf;

use review_ui::docs::{Page, TOC};

fn book_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/book/src/en")
}

/// A reader who opens the contents list must be able to reach every page. A page
/// missing from `SUMMARY.md` is compiled into the binary and shown by no route.
#[test]
fn every_page_is_in_the_contents() {
    let listed: HashSet<Page> = TOC.iter().map(|(_, page)| *page).collect();
    let orphans: Vec<&str> = Page::ALL
        .into_iter()
        .filter(|page| !listed.contains(page))
        .map(Page::id)
        .collect();
    assert!(
        orphans.is_empty(),
        "these pages are in the book but not in SUMMARY.md, so nothing links to \
         them: {orphans:?}"
    );
}

/// And the contents must not list a page twice, which would put two rows on the
/// same target and make the selected-row highlight ambiguous.
#[test]
fn the_contents_lists_each_page_once() {
    let mut seen = HashSet::new();
    for (_, page) in TOC {
        assert!(
            seen.insert(page),
            "`{}` is listed twice in SUMMARY.md",
            page.id()
        );
    }
}

/// Every page's title is its `# ` heading, and an empty one would leave a blank
/// row in the contents list.
#[test]
fn every_page_has_a_title() {
    for page in Page::ALL {
        assert!(
            !page.title().trim().is_empty(),
            "`{}` has an empty title",
            page.id()
        );
    }
}

/// Each `Page` must round-trip through its own path id: that is what resolves a
/// link at run time.
#[test]
fn every_page_resolves_from_its_id() {
    for page in Page::ALL {
        assert_eq!(Page::from_id(page.id()), Some(page));
    }
    assert_eq!(Page::from_id("not/a/page"), None);
}

/// Every image a page references must be in the repository. The runtime degrades
/// a missing one to its alt text, which is right for a broken *install* but wrong
/// for a page that names a file nobody ever added.
#[test]
fn every_referenced_image_exists() {
    let root = book_root();
    let mut missing = Vec::new();
    for page in Page::ALL {
        for image in page.text("en").images {
            if !root.join(image.path).is_file() {
                missing.push(format!("{}: {}", page.id(), image.path));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "these pages reference images that are not in docs/book/src/en: {missing:#?}"
    );
}

/// A `#fragment` link must name a heading in the page it points at, or it is a
/// dead anchor on the published site and a jump to nowhere in the Help window.
#[test]
fn every_fragment_link_names_a_heading() {
    let mut broken = Vec::new();
    for page in Page::ALL {
        for (href, target) in page.text("en").links {
            let Some((_, fragment)) = href.split_once('#') else {
                continue;
            };
            if fragment.is_empty() {
                continue;
            }
            let headings = heading_anchors(target.text("en").markdown);
            if !headings.contains(fragment) {
                broken.push(format!("{} -> {href}", page.id()));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "these links point at a heading that does not exist: {broken:#?}"
    );
}

/// The manual, like the catalog, is written in characters a keyboard can type.
///
/// Same reasoning as `localization/tests/typeable_characters.rs`, and the same trap: a
/// page edited by hand comes back with a hyphen next to the em dash above it, and
/// the in-app reader depends on the bundled fonts having whatever mark was used.
/// This checks the text as it is *compiled in*, which is what Help shows.
#[test]
fn every_page_is_typeable_ascii() {
    let mut offences = Vec::new();
    for page in Page::ALL {
        for (number, line) in page.text("en").markdown.lines().enumerate() {
            let bad: Vec<char> = line
                .chars()
                .filter(|c| !matches!(c, ' '..='~' | '\t'))
                .collect();
            if !bad.is_empty() {
                offences.push(format!("{}.md:{}: {bad:?}", page.id(), number + 1));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the manual must be typeable ASCII - use a hyphen for a dash, three \
         dots for an ellipsis:\n{}",
        offences.join("\n")
    );
}

/// The manual is written without markdown tables, and that is a constraint the
/// in-app reader imposes rather than a matter of taste.
///
/// `egui_commonmark` lays a table out as an `egui::Grid` whose every cell is a
/// plain horizontal row, which in egui means `TextWrapMode::Extend`: a cell is
/// one unbroken line however long its sentence is. So a table is the only thing
/// in the manual that cannot be made to fit the reading pane. The widest one
/// here came out four times the width of the window and dragged a horizontal
/// scrollbar under the page.
///
/// Forcing the wrap mode on instead is not the way out. A `Grid` sizes its
/// columns from what they measured last frame, so wrapping feeds back on itself
/// and collapses the first column until its words break mid-word.
///
/// Every table this manual had was a two-column "name - what it is" list, which
/// a bullet says just as well and which wraps. Write one of those instead.
#[test]
fn no_page_uses_a_markdown_table() {
    let mut offenders = Vec::new();
    for page in Page::ALL {
        for (number, line) in page.text("en").markdown.lines().enumerate() {
            if line.trim_start().starts_with('|') {
                offenders.push(format!("{}.md:{}", page.id(), number + 1));
                break;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the in-app reader cannot wrap a table cell, so these pages would be \
         drawn with a horizontal scrollbar - use a bullet list instead: \
         {offenders:#?}"
    );
}

/// mdBook's anchor for a heading: lowercased, spaces to hyphens, punctuation
/// dropped.
fn heading_anchors(markdown: &str) -> HashSet<String> {
    markdown
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix('#'))
        .map(|rest| {
            rest.trim_start_matches('#')
                .trim()
                .to_lowercase()
                .chars()
                .filter_map(|c| match c {
                    ' ' => Some('-'),
                    c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                    _ => None,
                })
                .collect()
        })
        .collect()
}
