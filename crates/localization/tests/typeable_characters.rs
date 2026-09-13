//! Every character in the catalog must be one a keyboard can type.
//!
//! Typographic punctuation - an em dash, a curly quote, a horizontal ellipsis, a
//! middle dot - costs more here than it pays. It cannot be typed by whoever edits
//! a message next, so a line touched by hand comes back with a hyphen beside the
//! em dash three lines up; it is invisible in a diff; it depends on the bundled
//! fonts having the glyph, which is exactly the failure that made Fluent's own
//! isolation marks render as tofu; and a translator working in a plain editor has
//! to reproduce marks their layout has no key for.
//!
//! So the catalog is ASCII: a hyphen for every dash, three dots for an ellipsis,
//! `x` for a multiplication sign. This test is what keeps it that way, since
//! nothing else would notice one em dash arriving in a message.
//!
//! Icon glyphs are a separate matter and are not catalog text: the reorder
//! arrows, the remove mark and the Help window's Back arrow are marks drawn by
//! the font in place of an icon, they are not read as words, and they have no
//! ASCII equivalent that does not look broken.

use std::path::{Path, PathBuf};

fn locales_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales")
}

/// Printable ASCII, plus the two whitespace characters a `.ftl` file is laid out
/// with. A tab is legal in a value and a newline is how a long value is wrapped.
fn is_typeable(character: char) -> bool {
    matches!(character, ' '..='~' | '\n' | '\t')
}

fn check_file(path: &Path, offences: &mut Vec<String>) {
    let text = std::fs::read_to_string(path).expect("catalog file is readable");
    for (number, line) in text.lines().enumerate() {
        let bad: Vec<char> = line.chars().filter(|c| !is_typeable(*c)).collect();
        if !bad.is_empty() {
            offences.push(format!(
                "{}:{}: {bad:?} in `{}`",
                path.display(),
                number + 1,
                line.trim()
            ));
        }
    }
}

#[test]
fn every_catalog_is_typeable_ascii() {
    let mut offences = Vec::new();
    let mut locales: Vec<PathBuf> = std::fs::read_dir(locales_dir())
        .expect("the locales directory exists")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    locales.sort();

    let mut files_seen = 0;
    for locale in locales {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&locale)
            .expect("a locale directory is readable")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "ftl"))
            .collect();
        files.sort();
        for file in files {
            files_seen += 1;
            check_file(&file, &mut offences);
        }
    }

    assert!(files_seen > 0, "no catalog files were checked");
    assert!(
        offences.is_empty(),
        "the catalog must be typeable ASCII - use a hyphen for a dash, three dots \
         for an ellipsis, `x` for a multiplication sign:\n{}",
        offences.join("\n")
    );
}
