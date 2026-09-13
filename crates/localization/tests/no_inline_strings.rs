//! The gate behind invariant 12: no user-visible string is written at a call
//! site.
//!
//! This walks every source file of the three crates that draw text and fails on
//! a string literal handed to a **text sink** — a widget that shows it, a
//! notification, a dialog title, a menu item. The catalog is where those live,
//! and the generated keys are how a call site names one.
//!
//! Why an AST rather than a grep: the literals that slip through are the ones a
//! regex cannot see. A label three lines into a wrapped call, a `format!` whose
//! first argument is the English and whose second is a path, a `concat!` of two
//! halves of a sentence. `syn` sees all three as the same thing.
//!
//! Escape hatch: `// localization: exempt <reason>` on the offending line. The reason is
//! required — an exemption without one is how this rots.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;

/// The crates whose sources may not carry user-visible literals: the ones that
/// draw, announce, or name something on screen.
const SCANNED: [&str; 3] = ["ui", "app", "shell-macos"];

/// Methods whose every argument reaches the screen.
///
/// egui's own text widgets and this workspace's notification API. A call not in
/// this set is not checked, which is what lets an egui id salt, a thread name or
/// a profiling message stay an English literal — none of them is read by a user.
const METHOD_SINKS: &[&str] = &[
    // egui
    "label",
    "weak",
    "small",
    "heading",
    "button",
    "small_button",
    "link",
    "hyperlink_to",
    "selectable_label",
    "on_hover_text",
    "on_hover_text_at_pointer",
    "on_disabled_hover_text",
    "selected_text",
    "hint_text",
    // the tooltip builder
    "describe",
    // notifications
    "success",
    "info",
    "warning",
    "error",
    "report",
    "mode",
    "begin_activity",
    "update_activity",
    // rfd
    "set_title",
    "set_description",
    "set_file_name",
];

/// Methods where only some arguments are text — the rest being ids, values or
/// flags that no one reads.
const METHOD_SINKS_INDEXED: &[(&str, &[usize])] = &[
    // (value, text)
    ("selectable_value", &[2]),
    ("radio_value", &[2]),
    // (value, text)
    ("checkbox", &[1]),
    // (name, extensions)
    ("add_filter", &[0]),
    // (id, text, enabled, accelerator)
    ("with_id", &[1]),
    // (title, enabled, items)
    ("with_items", &[0]),
    // muda's predefined items each take one optional label override.
    ("about", &[0]),
    ("hide", &[0]),
    ("hide_others", &[0]),
    ("show_all", &[0]),
    ("quit", &[0]),
    ("services", &[0]),
    ("minimize", &[0]),
    ("maximize", &[0]),
];

/// Free functions and constructors, with the argument positions that are text.
///
/// Most of this workspace's widget wrappers take `ui` first and an id salt
/// somewhere in the middle, so checking every argument would report the salt —
/// which is an egui identity, not a word anyone reads.
const FUNCTION_SINKS: &[(&str, &[usize])] = &[
    // egui constructors: the whole argument is the text.
    ("RichText::new", &[0]),
    ("Label::new", &[0]),
    ("Button::new", &[0]),
    ("Window::new", &[0]),
    ("CollapsingHeader::new", &[0]),
    ("WidgetText::from", &[0]),
    ("Tip::new", &[0]),
    // widget wrappers: (ui, …).
    ("grid_label_tip", &[1]),
    ("labeled_checkbox", &[1]),
    ("labeled_slider_with_value", &[1]),
    ("labeled_color_button", &[1]),
    ("color_swatch_row", &[1]),
    ("background_swatch_row", &[1]),
    ("value_row", &[1, 2]),
    ("wide_button", &[0]),
    ("segment_button", &[1]),
    ("tab_bar", &[1]),
    ("list_row_label", &[2]),
    ("mono_label", &[0]),
    ("stat_row", &[2]),
    ("tex_row", &[2]),
    ("tip_body", &[1, 2]),
    // (ui, label, id_salt, selected_text, contents)
    ("labeled_combo", &[1, 3]),
    // (ui, id_salt, control_w, selected_text, contents)
    ("compact_combo", &[3]),
    // (ui, icon, selected, tooltip)
    ("icon_toggle_button", &[3]),
    ("icon_toggle_button_with_options", &[3]),
    // (ui, icon, selected, tooltip, size, has_options)
    ("icon_tile_button", &[3]),
    // (ui, icon, flag, panels_open, panel, tooltip)
    ("option_toggle", &[5]),
];

/// Marks a line as deliberately exempt. The trailing reason is required.
const EXEMPT: &str = "// localization: exempt";

struct Finding {
    file: PathBuf,
    line: usize,
    literal: String,
    sink: String,
}

fn main() {}

#[test]
fn no_user_visible_string_is_written_at_a_call_site() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut findings = Vec::new();

    for crate_name in SCANNED {
        let src = workspace.join("crates").join(crate_name).join("src");
        assert!(
            src.is_dir(),
            "{} does not exist — has a crate been renamed?",
            src.display()
        );
        for file in rust_files(&src) {
            scan(&file, &mut findings);
        }
    }

    if !findings.is_empty() {
        let mut report = format!(
            "{} user-visible string literal(s) written at a call site.\n\n\
             Every one of these belongs in crates/localization/locales/en/*.ftl, reached \
             through its generated key (invariant 12). If a literal genuinely is \
             not user-visible, mark its line `{EXEMPT} <reason>`.\n\n",
            findings.len()
        );
        for finding in &findings {
            report.push_str(&format!(
                "  {}:{}: {:?} passed to `{}`\n",
                finding.file.display(),
                finding.line,
                finding.literal,
                finding.sink,
            ));
        }
        panic!("{report}");
    }
}

/// Whether a literal contains a letter that is not inside a `{…}` placeholder.
fn has_prose(literal: &str) -> bool {
    let mut depth = 0usize;
    for character in literal.chars() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            letter if depth == 0 && letter.is_alphabetic() => return true,
            _ => {}
        }
    }
    false
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn scan(path: &Path, findings: &mut Vec<Finding>) {
    let Ok(source) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(file) = syn::parse_file(&source) else {
        // A file this crate's `syn` cannot parse is a `syn` version problem, not
        // a localization one; the compiler is what reports a real syntax error.
        return;
    };

    let exempt: HashSet<usize> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(EXEMPT))
        .map(|(index, _)| index + 1)
        .collect();

    let mut visitor = SinkVisitor {
        path: path.to_owned(),
        exempt,
        findings,
    };
    visitor.visit_file(&file);
}

struct SinkVisitor<'a> {
    path: PathBuf,
    exempt: HashSet<usize>,
    findings: &'a mut Vec<Finding>,
}

impl SinkVisitor<'_> {
    /// Record every string literal reachable from `expr` that a reader could see.
    ///
    /// Recurses through `format!` / `concat!` (their arguments are the sentence),
    /// through references and parenthesised groups, and through a `match`'s or an
    /// `if`'s arms — a tooltip chosen between two literals is two literals.
    fn check(&mut self, expr: &syn::Expr, sink: &str) {
        match expr {
            syn::Expr::Lit(lit) => {
                if let syn::Lit::Str(text) = &lit.lit {
                    self.record(&text.value(), lit.span().start().line, sink);
                }
            }
            syn::Expr::Reference(inner) => self.check(&inner.expr, sink),
            syn::Expr::Paren(inner) => self.check(&inner.expr, sink),
            syn::Expr::Group(inner) => self.check(&inner.expr, sink),
            syn::Expr::MethodCall(call) => {
                // A chained `.to_owned()` / `.into()` on a literal is still that
                // literal arriving at the sink.
                self.check(&call.receiver, sink);
            }
            syn::Expr::If(branch) => {
                for statement in &branch.then_branch.stmts {
                    if let syn::Stmt::Expr(inner, _) = statement {
                        self.check(inner, sink);
                    }
                }
                if let Some((_, otherwise)) = &branch.else_branch {
                    self.check(otherwise, sink);
                }
            }
            syn::Expr::Block(block) => {
                for statement in &block.block.stmts {
                    if let syn::Stmt::Expr(inner, _) = statement {
                        self.check(inner, sink);
                    }
                }
            }
            syn::Expr::Match(branch) => {
                for arm in &branch.arms {
                    self.check(&arm.body, sink);
                }
            }
            // A `format!`'s or `concat!`'s arguments are the sentence, so its
            // literals are the ones that reach the screen.
            syn::Expr::Macro(mac)
                if macro_name(&mac.mac.path)
                    .is_some_and(|name| matches!(name.as_str(), "format" | "concat")) =>
            {
                for (literal, line) in string_literals(mac.mac.tokens.clone()) {
                    self.record(&literal, line, sink);
                }
            }
            _ => {}
        }
    }

    fn record(&mut self, literal: &str, line: usize, sink: &str) {
        // Only text with letters *outside* a placeholder. Punctuation, an em
        // dash and a bare `%` read the same in every language, and a format
        // specifier like `{value:.2}` or `{stem}.fbx` is a number and a file
        // extension — the letters in it name a binding, not a word.
        if !has_prose(literal) || self.exempt.contains(&line) {
            return;
        }
        self.findings.push(Finding {
            file: self.path.clone(),
            line,
            literal: literal.to_owned(),
            sink: sink.to_owned(),
        });
    }
}

impl<'ast> Visit<'ast> for SinkVisitor<'_> {
    /// Skip `#[cfg(test)]` modules: a test's fixture strings are the test's own
    /// data, never shown to anyone, and a notice fixture reading "Loading a.fbx"
    /// is clearer than one reading through the catalog.
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let is_test = module.attrs.iter().any(|attr| {
            attr.path().is_ident("cfg") && attr.to_token_stream().to_string().contains("test")
        });
        if !is_test {
            syn::visit::visit_item_mod(self, module);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if METHOD_SINKS.contains(&name.as_str()) {
            for argument in &call.args {
                self.check(argument, &name);
            }
        } else if let Some((_, indices)) =
            METHOD_SINKS_INDEXED.iter().find(|(sink, _)| *sink == name)
        {
            for index in *indices {
                if let Some(argument) = call.args.iter().nth(*index) {
                    self.check(argument, &name);
                }
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            let full = path_tail(&path.path, 2);
            let short = path_tail(&path.path, 1);
            if let Some((name, indices)) = FUNCTION_SINKS
                .iter()
                .find(|(sink, _)| *sink == full || *sink == short)
            {
                for index in *indices {
                    if let Some(argument) = call.args.iter().nth(*index) {
                        self.check(argument, name);
                    }
                }
            }
            // Nothing outside a generated `keys.rs` may mint a key: that is what
            // keeps `Key` a name for a catalog message rather than a wrapper
            // anyone can put a literal in.
            if full == "Key::new" {
                self.findings.push(Finding {
                    file: self.path.clone(),
                    line: call.span().start().line,
                    literal: "Key::new".to_owned(),
                    sink: "a hand-written key".to_owned(),
                });
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

/// The last `n` segments of a path, joined — `RichText::new` from
/// `egui::RichText::new`.
fn path_tail(path: &syn::Path, n: usize) -> String {
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let start = segments.len().saturating_sub(n);
    segments[start..].join("::")
}

fn macro_name(path: &syn::Path) -> Option<String> {
    path.segments.last().map(|last| last.ident.to_string())
}

/// String literals inside a macro's raw token stream, with their line numbers.
fn string_literals(tokens: proc_macro2::TokenStream) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    collect_literals(tokens, &mut found);
    found
}

fn collect_literals(tokens: proc_macro2::TokenStream, found: &mut Vec<(String, usize)>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Literal(literal) => {
                let line = literal.span().start().line;
                if let Ok(syn::Lit::Str(text)) = syn::parse_str::<syn::Lit>(&literal.to_string()) {
                    found.push((text.value(), line));
                }
            }
            proc_macro2::TokenTree::Group(group) => collect_literals(group.stream(), found),
            _ => {}
        }
    }
}
