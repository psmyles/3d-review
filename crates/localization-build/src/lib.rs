//! Build-time half of the localization system (invariant 12): the English
//! catalog under `crates/localization/locales/en/` is parsed here and turned into typed
//! Rust keys, so a message a consumer names is checked by the compiler rather
//! than looked up and missed at run time.
//!
//! Two consumers call in:
//!
//! * every crate that *shows* text (`ui`, `app`, `shell-macos`) calls
//!   [`generate_keys`] from its `build.rs`, which writes a `keys.rs` of `const
//!   Key`s into that crate's `OUT_DIR`. The consts land in the consuming crate,
//!   which is the whole point — an unused `pub const` in a library warns about
//!   nothing, but an unused private one is `dead_code`, and `-D warnings` turns
//!   that into a failed build. A misspelled key is a plain name error.
//! * `review-localization`'s own `build.rs` calls [`generate_resources`], which embeds
//!   every locale's files and cross-checks each against `en`.
//!
//! This crate deliberately depends on `fluent-syntax` alone, not the whole
//! Fluent runtime: a build script should compile the parser and nothing else.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use fluent_syntax::ast;

/// One message, or one attribute of one message, as the English catalog defines
/// it. The unit both the generated keys and the test-facing catalog are built
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDef {
    /// File stem the message lives in (`ui-stats`), which is also its id prefix.
    pub file: String,
    /// Full Fluent message id (`ui-stats-draws`).
    pub id: String,
    /// Attribute name, for an attribute like `.description`.
    pub attr: Option<String>,
    /// Variable names the pattern references (`$modifier`), sorted and deduped.
    /// A message with any of these gets a typed formatter beside its key.
    pub vars: Vec<String>,
    /// The message's text with placeables collapsed, for the key's doc comment.
    pub preview: String,
}

impl MessageDef {
    /// `DRAWS`, `DRAWS_DESCRIPTION` — the generated const's name.
    fn const_name(&self, prefix: &str) -> String {
        let stem = self.id.strip_prefix(prefix).unwrap_or(&self.id);
        let mut name = screaming_snake(stem);
        if let Some(attr) = &self.attr {
            name.push('_');
            name.push_str(&screaming_snake(attr));
        }
        name
    }
}

/// A build-time failure. Every one of these fails the consumer's build, which is
/// the contract: a catalog that does not build cannot be loaded either.
#[derive(Debug)]
pub struct CatalogError {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file.display(), self.line, self.message)
    }
}

impl std::error::Error for CatalogError {}

type Result<T> = std::result::Result<T, CatalogError>;

/// Read and validate one locale's whole catalog, in file order then source order.
///
/// Enforces the rules the naming scheme rests on: a file's messages are all
/// prefixed with its stem (which is what keeps ids unique inside the one flat
/// Fluent bundle, and what lets the generated const drop the prefix), no id is
/// defined twice, and every message says something (a bare `id =` with no value
/// and no attributes is a typo, not a message).
pub fn load_catalog(locales_dir: &Path, locale: &str) -> Result<Vec<MessageDef>> {
    let dir = locales_dir.join(locale);
    let mut defs = Vec::new();
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();

    for path in ftl_files(&dir)? {
        let stem = file_stem(&path);
        let source = read(&path)?;
        let resource = parse(&path, &source)?;

        for entry in &resource.body {
            let ast::Entry::Message(message) = entry else {
                continue;
            };
            let id = message.id.name.to_owned();
            let line = line_of(&source, &id);

            if !id.starts_with(&format!("{stem}-")) {
                return Err(CatalogError {
                    file: path.clone(),
                    line,
                    message: format!(
                        "message `{id}` must start with its file's stem, `{stem}-` — that \
                         prefix is what keeps ids unique across the one flat bundle, and what \
                         the generated const name drops"
                    ),
                });
            }
            if let Some(first) = seen.insert(id.clone(), path.clone()) {
                return Err(CatalogError {
                    file: path.clone(),
                    line,
                    message: format!("message `{id}` is already defined in {}", first.display()),
                });
            }
            if message.value.is_none() && message.attributes.is_empty() {
                return Err(CatalogError {
                    file: path.clone(),
                    line,
                    message: format!("message `{id}` has neither a value nor attributes"),
                });
            }

            if let Some(pattern) = &message.value {
                defs.push(message_def(stem, &id, None, pattern, &path, line)?);
            }
            for attribute in &message.attributes {
                defs.push(message_def(
                    stem,
                    &id,
                    Some(attribute.id.name),
                    &attribute.value,
                    &path,
                    line,
                )?);
            }
        }
    }
    Ok(defs)
}

/// Write `<out_dir>/keys.rs`: a module per catalog file whose stem starts with one
/// of `prefixes`, holding a `Key` const per message and attribute, plus a typed
/// formatter for every message that takes variables.
///
/// `prefixes` is how one catalog serves three crates without handing each of them
/// the others' keys: `ui` takes `["common-", "ui-"]`, `app` `["common-", "app-"]`,
/// `shell-macos` `["menu-"]`. A key two crates need goes in `common.ftl`.
///
/// Emits the `cargo:rerun-if-changed` lines for the catalog, so editing a `.ftl`
/// rebuilds the consumer.
pub fn generate_keys(locales_dir: &Path, prefixes: &[&str], out_dir: &Path) -> Result<()> {
    emit_rerun(locales_dir);
    let defs = load_catalog(locales_dir, "en")?;

    let mut by_file: BTreeMap<&str, Vec<&MessageDef>> = BTreeMap::new();
    for def in &defs {
        if prefixes.iter().any(|prefix| {
            def.id.starts_with(prefix) || format!("{}-", def.file) == *prefix || def.file == *prefix
        }) {
            by_file.entry(&def.file).or_default().push(def);
        }
    }

    let mut rust = String::new();
    rust.push_str(
        "// @generated by review-localization-build from crates/localization/locales/en — do not edit.\n\
         //\n\
         // One module per catalog file; one `Key` per message and attribute, plus a typed\n\
         // formatter for every message that takes variables. An unused key here is\n\
         // `dead_code`, which `-D warnings` turns into a failed build (invariant 12).\n\n",
    );

    for (file, mut file_defs) in by_file {
        file_defs.sort_by(|a, b| (&a.id, &a.attr).cmp(&(&b.id, &b.attr)));
        let module = snake(file);
        let prefix = format!("{file}-");

        let _ = writeln!(rust, "/// Messages from `{file}.ftl`.");
        let _ = writeln!(rust, "pub(crate) mod {module} {{");
        rust.push_str("    #[allow(unused_imports)]\n");
        rust.push_str("    use review_localization::{FluentArgs, FluentValue, Key, tr_args};\n\n");

        let mut used: BTreeSet<String> = BTreeSet::new();
        for def in file_defs {
            let name = def.const_name(&prefix);
            if !used.insert(name.clone()) {
                return Err(CatalogError {
                    file: locales_dir.join("en").join(format!("{file}.ftl")),
                    line: 0,
                    message: format!(
                        "`{}`{} generates the const name `{name}`, which is already taken — \
                         rename one of them",
                        def.id,
                        def.attr
                            .as_ref()
                            .map(|a| format!(".{a}"))
                            .unwrap_or_default(),
                    ),
                });
            }

            let target = match &def.attr {
                Some(attr) => format!("{}.{attr}", def.id),
                None => def.id.clone(),
            };
            let _ = writeln!(rust, "    /// `{target}`: {}", doc_escape(&def.preview));
            let attr = match &def.attr {
                Some(attr) => format!("Some(\"{attr}\")"),
                None => "None".to_owned(),
            };
            let _ = writeln!(
                rust,
                "    pub(crate) const {name}: Key = Key::new(\"{}\", {attr});",
                def.id
            );

            if !def.vars.is_empty() {
                let fn_name = name.to_lowercase();
                let params = def
                    .vars
                    .iter()
                    .map(|var| format!("{var}: impl Into<FluentValue<'a>>"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(
                    rust,
                    "    /// `{target}`, with its variables. A missing argument is a compile error."
                );
                rust.push_str("    #[allow(dead_code)]\n");
                let _ = writeln!(
                    rust,
                    "    pub(crate) fn {fn_name}<'a>({params}) -> String {{"
                );
                rust.push_str("        let mut args = FluentArgs::new();\n");
                for var in &def.vars {
                    let _ = writeln!(rust, "        args.set(\"{var}\", {var}.into());");
                }
                let _ = writeln!(rust, "        tr_args({name}, &args)");
                rust.push_str("    }\n");
            }
            rust.push('\n');
        }
        rust.push_str("}\n\n");
    }

    write(&out_dir.join("keys.rs"), &rust)
}

/// Write `<out_dir>/resources.rs` (every locale's files, embedded) and
/// `<out_dir>/catalog.rs` (the English catalog as data, for tests).
///
/// Cross-checks each non-English locale against `en`: an id `en` does not define
/// fails the build (a translation for a message that no longer exists is dead
/// weight the code can never reach), as does a message whose variables differ
/// from the English ones (its formatter would be called with the wrong
/// arguments). A message `en` has and the locale does not is only a warning —
/// that is a translation in progress, and the runtime falls back to English.
pub fn generate_resources(locales_dir: &Path, out_dir: &Path) -> Result<()> {
    emit_rerun(locales_dir);
    let english = load_catalog(locales_dir, "en")?;
    let english_index: BTreeMap<(String, Option<String>), &MessageDef> = english
        .iter()
        .map(|def| ((def.id.clone(), def.attr.clone()), def))
        .collect();

    let mut locales = vec!["en".to_owned()];
    for entry in read_dir(locales_dir)? {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            if name != "en" {
                locales.push(name);
            }
        }
    }
    locales[1..].sort();

    let mut rust = String::from(
        "// @generated by review-localization-build — do not edit.\n\n\
         /// Every embedded locale, English first (it is the fallback, and the only\n\
         /// catalog the generated keys are checked against).\n\
         pub(crate) const LOCALES: &[(&str, &[(&str, &str)])] = &[\n",
    );

    for locale in &locales {
        if locale != "en" {
            check_translation(locales_dir, locale, &english_index)?;
        }
        let _ = writeln!(rust, "    (\"{locale}\", &[");
        for path in ftl_files(&locales_dir.join(locale))? {
            let stem = file_stem(&path);
            let include = path.to_string_lossy().replace('\\', "/");
            let _ = writeln!(rust, "        (\"{stem}\", include_str!(\"{include}\")),");
        }
        rust.push_str("    ]),\n");
    }
    rust.push_str("];\n");
    write(&out_dir.join("resources.rs"), &rust)?;

    let mut catalog = String::from(
        "// @generated by review-localization-build — do not edit.\n\n\
         /// The English catalog as data, so a test can format every message in every\n\
         /// locale without naming them one at a time.\n\
         pub const MESSAGES: &[MessageInfo] = &[\n",
    );
    for def in &english {
        let attr = match &def.attr {
            Some(attr) => format!("Some(\"{attr}\")"),
            None => "None".to_owned(),
        };
        let vars = def
            .vars
            .iter()
            .map(|var| format!("\"{var}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            catalog,
            "    MessageInfo {{ file: \"{}\", id: \"{}\", attr: {attr}, vars: &[{vars}] }},",
            def.file, def.id
        );
    }
    catalog.push_str("];\n");
    write(&out_dir.join("catalog.rs"), &catalog)
}

/// Check one translated locale against the English catalog.
fn check_translation(
    locales_dir: &Path,
    locale: &str,
    english: &BTreeMap<(String, Option<String>), &MessageDef>,
) -> Result<()> {
    let defs = load_catalog(locales_dir, locale)?;
    let mut present = BTreeSet::new();

    for def in &defs {
        let key = (def.id.clone(), def.attr.clone());
        let Some(source) = english.get(&key) else {
            return Err(CatalogError {
                file: locales_dir.join(locale).join(format!("{}.ftl", def.file)),
                line: 0,
                message: format!(
                    "`{}`{} is not in the English catalog — no code can reach it",
                    def.id,
                    def.attr
                        .as_ref()
                        .map(|a| format!(".{a}"))
                        .unwrap_or_default(),
                ),
            });
        };
        if def.vars != source.vars {
            return Err(CatalogError {
                file: locales_dir.join(locale).join(format!("{}.ftl", def.file)),
                line: 0,
                message: format!(
                    "`{}`{} takes {:?} but the English message takes {:?} — the generated \
                     formatter passes the English set",
                    def.id,
                    def.attr
                        .as_ref()
                        .map(|a| format!(".{a}"))
                        .unwrap_or_default(),
                    def.vars,
                    source.vars,
                ),
            });
        }
        present.insert(key);
    }

    let missing = english.keys().filter(|key| !present.contains(*key)).count();
    if missing > 0 {
        println!(
            "cargo:warning={locale}: {missing} message(s) not translated yet — those fall back to English"
        );
    }
    Ok(())
}

/// Collect one message's variables and a preview of its text.
fn message_def(
    file: &str,
    id: &str,
    attr: Option<&str>,
    pattern: &ast::Pattern<&str>,
    path: &Path,
    line: usize,
) -> Result<MessageDef> {
    let mut vars = BTreeSet::new();
    let mut preview = String::new();
    walk_pattern(pattern, &mut vars, &mut preview);

    for var in &vars {
        if !is_ident(var) || is_keyword(var) {
            return Err(CatalogError {
                file: path.to_owned(),
                line,
                message: format!(
                    "`{id}` uses the variable `${var}`, which is not usable as a Rust parameter \
                     name — rename it (`$modifier`, not `$mod`)"
                ),
            });
        }
    }

    Ok(MessageDef {
        file: file.to_owned(),
        id: id.to_owned(),
        attr: attr.map(str::to_owned),
        vars: vars.into_iter().collect(),
        preview: preview.split_whitespace().collect::<Vec<_>>().join(" "),
    })
}

fn walk_pattern(pattern: &ast::Pattern<&str>, vars: &mut BTreeSet<String>, preview: &mut String) {
    for element in &pattern.elements {
        match element {
            ast::PatternElement::TextElement { value } => preview.push_str(value),
            ast::PatternElement::Placeable { expression } => {
                walk_expression(expression, vars, preview);
            }
        }
    }
}

fn walk_expression(
    expression: &ast::Expression<&str>,
    vars: &mut BTreeSet<String>,
    preview: &mut String,
) {
    match expression {
        ast::Expression::Inline(inline) => walk_inline(inline, vars, preview),
        ast::Expression::Select { selector, variants } => {
            walk_inline(selector, vars, &mut String::new());
            // The preview shows the default variant: it is the one an untranslated
            // reading of the message means, and a doc comment listing every branch
            // would be longer than the message.
            if let Some(default) = variants.iter().find(|variant| variant.default) {
                walk_pattern(&default.value, vars, preview);
            }
            for variant in variants {
                if !variant.default {
                    walk_pattern(&variant.value, vars, &mut String::new());
                }
            }
        }
    }
}

fn walk_inline(
    inline: &ast::InlineExpression<&str>,
    vars: &mut BTreeSet<String>,
    preview: &mut String,
) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            vars.insert(id.name.to_owned());
            preview.push('{');
            preview.push_str(id.name);
            preview.push('}');
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            for positional in &arguments.positional {
                walk_inline(positional, vars, &mut String::new());
            }
            for named in &arguments.named {
                walk_inline(&named.value, vars, &mut String::new());
            }
        }
        ast::InlineExpression::Placeable { expression } => {
            walk_expression(expression, vars, preview);
        }
        ast::InlineExpression::StringLiteral { value } => preview.push_str(value),
        ast::InlineExpression::NumberLiteral { value } => preview.push_str(value),
        ast::InlineExpression::MessageReference { .. }
        | ast::InlineExpression::TermReference { .. } => {}
    }
}

fn parse<'a>(path: &Path, source: &'a str) -> Result<ast::Resource<&'a str>> {
    fluent_syntax::parser::parse(source).map_err(|(_, errors)| {
        let first = &errors[0];
        CatalogError {
            file: path.to_owned(),
            line: offset_line(source, first.pos.start),
            message: format!("{:?}", first.kind),
        }
    })
}

fn ftl_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = read_dir(dir)?
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ftl"))
        .collect();
    files.sort();
    Ok(files)
}

fn read_dir(dir: &Path) -> Result<impl Iterator<Item = std::fs::DirEntry>> {
    let entries = std::fs::read_dir(dir).map_err(|err| CatalogError {
        file: dir.to_owned(),
        line: 0,
        message: format!("cannot read: {err}"),
    })?;
    Ok(entries.flatten())
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|err| CatalogError {
        file: path.to_owned(),
        line: 0,
        message: format!("cannot read: {err}"),
    })
}

fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).map_err(|err| CatalogError {
        file: path.to_owned(),
        line: 0,
        message: format!("cannot write: {err}"),
    })
}

fn emit_rerun(locales_dir: &Path) {
    println!("cargo:rerun-if-changed={}", locales_dir.display());
    if let Ok(entries) = std::fs::read_dir(locales_dir) {
        for entry in entries.flatten() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
            if let Ok(files) = std::fs::read_dir(entry.path()) {
                for file in files.flatten() {
                    println!("cargo:rerun-if-changed={}", file.path().display());
                }
            }
        }
    }
}

fn file_stem(path: &Path) -> &str {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
}

/// The 1-based line a message id is declared on, for an error a reader can jump
/// to. Found by scanning rather than carried through the AST, because
/// `fluent-syntax`'s identifiers hold no span.
fn line_of(source: &str, id: &str) -> usize {
    source
        .lines()
        .position(|line| {
            line.strip_prefix(id)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
        .map_or(0, |index| index + 1)
}

fn offset_line(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())].lines().count().max(1)
}

fn screaming_snake(text: &str) -> String {
    text.replace('-', "_").to_uppercase()
}

fn snake(text: &str) -> String {
    text.replace('-', "_")
}

fn is_ident(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with(|c: char| c.is_ascii_digit())
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_keyword(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false",
        "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
        "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "abstract", "become", "box", "do", "final",
        "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield", "gen",
    ];
    KEYWORDS.contains(&text)
}

/// Keep a preview on one doc-comment line and stop a stray `]` or newline from
/// breaking the generated source.
fn doc_escape(text: &str) -> String {
    let flat = text.replace(['\r', '\n'], " ");
    let trimmed: String = flat.chars().take(100).collect();
    let suffix = if flat.chars().count() > 100 {
        "…"
    } else {
        ""
    };
    format!(
        "\"{}{suffix}\"",
        trimmed
            .replace('"', "'")
            .replace('[', "\\[")
            .replace(']', "\\]")
    )
}
