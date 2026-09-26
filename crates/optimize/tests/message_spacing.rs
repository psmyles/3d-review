//! No message this crate produces carries the indentation of the source it was
//! written in.
//!
//! A Rust string literal keeps every byte between its quotes, newlines and the
//! next line's indentation included. Written across two lines without a
//! trailing `\`, a sentence comes out with fourteen spaces in the middle of it —
//! and it reads as a bug in the layout rather than as a typo in a string,
//! because where it lands is a notification card that wraps text for a living.
//! (The card's own rule turns a single newline into a space, so the newline
//! disappears and only the indentation is left to see.)
//!
//! It is invisible in review: the source looks like a tidy paragraph, and the
//! fault only shows in a screenshot of the running program. So it is checked
//! here rather than looked for.

use std::path::Path;

/// A run of this many spaces between two words is indentation, not spacing.
///
/// Two is already wrong, but a deliberate double space after a full stop is a
/// house style some people hold to, and this test is not the place to argue
/// about it. Three never means anything.
const RUN: usize = 3;

/// Every `.rs` file under a directory, recursively.
fn sources(root: &Path, into: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            into.push(path);
        }
    }
}

/// Where `line` runs several spaces between two non-space characters inside
/// what looks like a string literal.
///
/// Deliberately crude — it does not parse Rust. It looks at the part of a line
/// between the first and last double quote, which over-reaches on a line with
/// two separate literals and under-reaches on none at all. Both are fine: the
/// thing being caught is a sentence, and a sentence is the only reason a line
/// has a long run of spaces in the middle of its quoted part.
fn offending_run(line: &str) -> Option<String> {
    let first = line.find('"')?;
    let last = line.rfind('"')?;
    if last <= first {
        return None;
    }
    let quoted = &line[first + 1..last];
    // Alignment in a format specifier is spaces on purpose, and so is the
    // padding a `{:<20}` produces. Neither is a run of literal spaces here.
    let mut run = 0usize;
    let mut previous = ' ';
    for (at, character) in quoted.char_indices() {
        if character == ' ' {
            run += 1;
            continue;
        }
        if run >= RUN && !previous.is_whitespace() && previous != '\\' {
            let from = at.saturating_sub(run + 24);
            let to = (at + 24).min(quoted.len());
            let window = quoted
                .get(from..to)
                .unwrap_or(quoted)
                .replace('\n', " ")
                .trim()
                .to_owned();
            return Some(window);
        }
        run = 0;
        previous = character;
    }
    None
}

#[test]
fn no_message_carries_the_indentation_it_was_written_with() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(
        !files.is_empty(),
        "found no sources to check under {root:?}"
    );

    let mut faults = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Comments and doc comments are prose about the code, and lining
            // something up inside one is nobody's problem.
            if trimmed.starts_with("//") {
                continue;
            }
            if let Some(window) = offending_run(line) {
                faults.push(format!(
                    "{}:{}: ...{window}...",
                    path.strip_prefix(&root).unwrap_or(path).display(),
                    number + 1
                ));
            }
        }
    }

    assert!(
        faults.is_empty(),
        "these strings carry their source indentation - end each line with a \\\\ so \
         the newline and the next line's indent are dropped:\n  {}",
        faults.join("\n  ")
    );
}
