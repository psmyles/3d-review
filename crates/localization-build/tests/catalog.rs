//! The catalog rules the build enforces, driven through the public entry points
//! against small catalogs written to a temporary directory.
//!
//! Every consumer's build runs these checks on the real catalog, so a rule that
//! stopped firing would not fail anything — the catalog is valid — and would only
//! show up the day someone broke it. These pin each rule on a catalog that does.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use review_localization_build::{generate_keys, generate_resources, load_catalog};

/// A fresh locales directory, removed when dropped.
struct Locales {
    root: PathBuf,
}

impl Locales {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "review-localization-build-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create the temporary locales dir");
        Self { root }
    }

    /// Write `<locale>/<file>.ftl`.
    fn file(&self, locale: &str, file: &str, source: &str) -> &Self {
        let dir = self.root.join(locale);
        std::fs::create_dir_all(&dir).expect("create the locale dir");
        std::fs::write(dir.join(format!("{file}.ftl")), source).expect("write the catalog file");
        self
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn out(&self) -> PathBuf {
        let out = self.root.join("out");
        std::fs::create_dir_all(&out).expect("create the output dir");
        out
    }
}

impl Drop for Locales {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn error_of<T: std::fmt::Debug>(
    result: Result<T, review_localization_build::CatalogError>,
) -> String {
    result
        .expect_err("the catalog should be rejected")
        .to_string()
}

#[test]
fn a_message_and_its_attributes_become_separate_definitions() {
    let locales = Locales::new();
    locales.file(
        "en",
        "ui-demo",
        "ui-demo-title = Hello, { $name }, you have { $count } { $name }s\n    .tooltip = Plain\n",
    );
    let defs = load_catalog(locales.path(), "en").expect("a valid catalog");

    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0].id, "ui-demo-title");
    assert_eq!(defs[0].attr, None);
    assert_eq!(defs[0].vars, ["count", "name"], "sorted and deduplicated");
    assert_eq!(defs[1].attr.as_deref(), Some("tooltip"));
    assert!(defs[1].vars.is_empty());
}

#[test]
fn a_message_outside_its_files_prefix_is_rejected() {
    let locales = Locales::new();
    locales.file("en", "ui-demo", "ui-other-title = Hello\n");
    let message = error_of(load_catalog(locales.path(), "en"));
    assert!(
        message.contains("must start with its file's stem"),
        "{message}"
    );
}

#[test]
fn an_id_defined_in_two_files_is_rejected() {
    let locales = Locales::new();
    locales
        .file("en", "ui-a", "ui-a-title = One\n")
        // `ui-a-title` also starts with `ui-`, but not with this file's stem;
        // a genuine duplicate needs two files sharing a stem prefix.
        .file("en", "ui", "ui-a-title = Two\n");
    let message = error_of(load_catalog(locales.path(), "en"));
    assert!(message.contains("already defined"), "{message}");
}

#[test]
fn a_message_with_nothing_to_say_is_rejected() {
    let locales = Locales::new();
    // A bare `id =` does not parse as a message at all, so the empty case the
    // rule guards is reachable only through the parser's error path; either way
    // the catalog must not load.
    locales.file("en", "ui-demo", "ui-demo-title =\n");
    assert!(load_catalog(locales.path(), "en").is_err());
}

#[test]
fn a_variable_that_is_not_a_rust_identifier_is_rejected() {
    let locales = Locales::new();
    locales.file("en", "ui-demo", "ui-demo-title = { $mod } key\n");
    let message = error_of(load_catalog(locales.path(), "en"));
    assert!(
        message.contains("not usable as a Rust parameter"),
        "{message}"
    );
}

#[test]
fn a_message_with_variables_is_reached_only_through_its_formatter() {
    let locales = Locales::new();
    locales.file(
        "en",
        "ui-demo",
        "ui-demo-plain = Plain\nui-demo-greeting = Hello, { $name }\n",
    );
    let out = locales.out();
    generate_keys(locales.path(), &["ui-"], &out).expect("keys generate");
    let keys = std::fs::read_to_string(out.join("keys.rs")).expect("keys.rs written");

    assert!(keys.contains("pub(crate) const PLAIN: Key"), "{keys}");
    // Private, so an unused formatter is what `dead_code` reports.
    assert!(keys.contains("    const GREETING: Key"), "{keys}");
    assert!(!keys.contains("pub(crate) const GREETING"), "{keys}");
    assert!(
        keys.contains("pub(crate) fn greeting<'a>(name: impl Into<FluentValue<'a>>)"),
        "{keys}"
    );
    assert!(
        !keys.contains("allow(dead_code)"),
        "nothing may silence the unused-key check: {keys}"
    );
}

#[test]
fn a_crate_gets_only_the_files_its_prefixes_name() {
    let locales = Locales::new();
    locales
        .file("en", "ui-demo", "ui-demo-title = Title\n")
        .file("en", "app-demo", "app-demo-title = Title\n");
    let out = locales.out();
    generate_keys(locales.path(), &["app-"], &out).expect("keys generate");
    let keys = std::fs::read_to_string(out.join("keys.rs")).expect("keys.rs written");

    assert!(keys.contains("mod app_demo"), "{keys}");
    assert!(!keys.contains("mod ui_demo"), "{keys}");
}

#[test]
fn two_messages_that_generate_one_const_name_are_rejected() {
    let locales = Locales::new();
    // `ui-demo-a-b` and the `b` attribute of `ui-demo-a` both become `A_B`.
    locales.file(
        "en",
        "ui-demo",
        "ui-demo-a-b = One\nui-demo-a = Two\n    .b = Three\n",
    );
    let message = error_of(generate_keys(locales.path(), &["ui-"], &locales.out()));
    assert!(message.contains("already taken"), "{message}");
}

#[test]
fn a_translation_may_lag_behind_but_not_invent_or_reshape() {
    let english = "ui-demo-title = Title\nui-demo-greeting = Hello, { $name }\n";

    // Missing a message: a translation in progress, which falls back to English.
    let lagging = Locales::new();
    lagging
        .file("en", "ui-demo", english)
        .file("de", "ui-demo", "ui-demo-title = Titel\n");
    generate_resources(lagging.path(), &lagging.out()).expect("a partial translation builds");

    // A message English does not have: nothing could ever show it.
    let inventing = Locales::new();
    inventing
        .file("en", "ui-demo", english)
        .file("de", "ui-demo", "ui-demo-extra = Extra\n");
    let message = error_of(generate_resources(inventing.path(), &inventing.out()));
    assert!(message.contains("not in the English catalog"), "{message}");

    // Different variables: the formatter passes the English set.
    let reshaping = Locales::new();
    reshaping.file("en", "ui-demo", english).file(
        "de",
        "ui-demo",
        "ui-demo-greeting = Hallo, { $who }\n",
    );
    let message = error_of(generate_resources(reshaping.path(), &reshaping.out()));
    assert!(
        message.contains("but the English message takes"),
        "{message}"
    );
}
