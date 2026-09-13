//! Turns the mdBook under `docs/book/src/<lang>/` into a compile-time page table
//! (invariant 12): the manual is written once and read both on the published site
//! and inside the Help window.
//!
//! What lands in `OUT_DIR/docs_pages.rs`:
//!
//! * a `Page` enum with one variant per English page, named from its path, so a
//!   panel that names a page which no longer exists is a compile error;
//! * each page's markdown, `include_str!`d — the text never depends on the
//!   install being intact, only images do;
//! * where each page's images and intra-book links *sit* in that markdown, as
//!   byte ranges, so the runtime splices in resolved image paths without parsing
//!   markdown per frame;
//! * `TOC`, the flattened `SUMMARY.md`, which is the Help window's contents list
//!   as well as the site's sidebar.
//!
//! Every check here has a matching failure on the site: a `SUMMARY.md` entry or a
//! relative link naming a file that does not exist is a 404 there and a build
//! error here, which is the earlier of the two places to find out.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A page as read off disk, before it is written out as Rust.
struct PageSource {
    /// Path id relative to the locale root, without the extension
    /// (`panels/ambient-occlusion`). The page's only name — no front matter,
    /// which mdBook does not have, and no id comment, which would be a second
    /// name to keep in step with the file that already has one.
    id: String,
    /// Rust enum variant built from the id (`PanelsAmbientOcclusion`).
    variant: String,
    /// Path to include the markdown from.
    path: PathBuf,
    /// The page's first `# ` heading.
    title: String,
    /// `![alt](dest)` occurrences: the byte range of the *destination* and its
    /// alt text.
    images: Vec<ImageRef>,
    /// Relative `.md` links, as written, paired with the page they resolve to.
    links: Vec<(String, String)>,
}

struct ImageRef {
    start: usize,
    end: usize,
    /// Destination normalised against the locale root, so `../images/x.png` from
    /// a page in `panels/` becomes `images/x.png`.
    path: String,
    alt: String,
}

pub fn generate(book_src: &Path, out_dir: &Path) -> Result<(), String> {
    let english = book_src.join("en");
    if !english.is_dir() {
        return Err(format!(
            "{} does not exist — the manual is the source both the site and the \
             Help window read (invariant 12)",
            english.display()
        ));
    }
    rerun_if_changed(book_src);

    let mut pages = Vec::new();
    collect(&english, &english, &mut pages)?;
    pages.sort_by(|a, b| a.id.cmp(&b.id));

    let ids: BTreeMap<&str, &str> = pages
        .iter()
        .map(|page| (page.id.as_str(), page.variant.as_str()))
        .collect();

    // Resolve each page's links now that every id is known: a link to a page that
    // does not exist would be a 404 on the site, so it fails the build instead.
    for page in &pages {
        for (href, target) in &page.links {
            if !ids.contains_key(target.as_str()) {
                return Err(format!(
                    "{}.md links to `{href}`, which is not a page — the site would 404 on it",
                    page.id
                ));
            }
        }
    }

    let toc = read_summary(&english, &ids)?;

    // Other locales are optional and partial: a page one has not translated falls
    // back to the English text at run time.
    let mut locales = vec!["en".to_owned()];
    for entry in std::fs::read_dir(book_src)
        .map_err(|err| err.to_string())?
        .flatten()
    {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        if path.is_dir() && name != "en" {
            locales.push(name);
        }
    }
    locales[1..].sort();

    let mut rust = String::new();
    write_header(&mut rust);
    write_enum(&mut rust, &pages, &toc);
    write_texts(&mut rust, book_src, &locales, &pages)?;
    write_toc(&mut rust, &toc);

    std::fs::write(out_dir.join("docs_pages.rs"), rust).map_err(|err| err.to_string())
}

fn collect(root: &Path, dir: &Path, pages: &mut Vec<PageSource>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir)
        .map_err(|err| err.to_string())?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, pages)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            // `SUMMARY.md` is the table of contents, not a page.
            if path.file_name().is_some_and(|name| name == "SUMMARY.md") {
                continue;
            }
            pages.push(read_page(root, &path)?);
        }
    }
    Ok(())
}

fn read_page(root: &Path, path: &Path) -> Result<PageSource, String> {
    let id = path
        .strip_prefix(root)
        .map_err(|err| err.to_string())?
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;

    let mut title = None;
    let mut images = Vec::new();
    let mut links = Vec::new();
    let mut heading_text: Option<String> = None;
    let mut alt = String::new();
    let mut in_image = false;

    let parser = Parser::new_ext(
        &source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    );
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => {
                heading_text = Some(String::new());
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                if title.is_none() {
                    title = heading_text.take();
                }
                heading_text = None;
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                in_image = true;
                alt.clear();
                // The destination's own span inside the `![…](…)` — the runtime
                // replaces exactly this, so a resolved path can be spliced in
                // without re-parsing.
                if let Some((start, end)) = dest_span(&source, range.clone(), &dest_url) {
                    images.push(ImageRef {
                        start,
                        end,
                        path: normalise(&id, &dest_url),
                        alt: String::new(),
                    });
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some(last) = images.last_mut() {
                    last.alt = alt.trim().to_owned();
                }
                in_image = false;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                // Only relative links into the book; `http(s)` and `#fragment`
                // links are the browser's and egui's to handle.
                let (target, _) = dest_url.split_once('#').unwrap_or((&dest_url, ""));
                if target.ends_with(".md") && !dest_url.contains("://") {
                    let resolved = normalise(&id, target);
                    links.push((
                        dest_url.to_string(),
                        resolved.trim_end_matches(".md").to_owned(),
                    ));
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if in_image {
                    alt.push_str(&text);
                } else if let Some(heading) = heading_text.as_mut() {
                    heading.push_str(&text);
                }
            }
            _ => {}
        }
    }

    let title = title.ok_or_else(|| {
        format!(
            "{}.md has no `# ` heading — the heading is the page's title in the \
             Help window's contents list",
            id
        )
    })?;

    Ok(PageSource {
        variant: variant_name(&id),
        path: path.to_owned(),
        title,
        images,
        links,
        id,
    })
}

/// The byte range of a link/image destination inside its `](…)` — found by
/// searching the element's own span, so a destination that also appears in the
/// surrounding text cannot be matched by mistake.
fn dest_span(source: &str, range: std::ops::Range<usize>, dest: &str) -> Option<(usize, usize)> {
    let slice = source.get(range.clone())?;
    let at = slice.rfind(&format!("]({dest}"))? + 2;
    Some((range.start + at, range.start + at + dest.len()))
}

/// A destination as written on `page`, expressed relative to the locale root.
fn normalise(page_id: &str, dest: &str) -> String {
    let mut parts: Vec<&str> = page_id.split('/').collect();
    parts.pop(); // the page's own file name
    for segment in dest.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// `panels/ambient-occlusion` → `PanelsAmbientOcclusion`.
fn variant_name(id: &str) -> String {
    id.split(['/', '-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// `SUMMARY.md` flattened to (depth, page id), in order. Both the site's sidebar
/// and the Help window's contents list read this one file.
fn read_summary(root: &Path, ids: &BTreeMap<&str, &str>) -> Result<Vec<(u8, String)>, String> {
    let path = root.join("SUMMARY.md");
    let source =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut toc = Vec::new();

    for (number, line) in source.lines().enumerate() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        // A list entry is a chapter; a bare `[Title](page.md)` with no bullet is a
        // prefix chapter, which mdBook puts before the numbered ones.
        let entry = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .unwrap_or(trimmed);
        let Some(rest) = entry.strip_prefix('[') else {
            continue;
        };
        let Some((_, rest)) = rest.split_once("](") else {
            continue;
        };
        let Some((dest, _)) = rest.split_once(')') else {
            continue;
        };
        if dest.is_empty() || dest.contains("://") {
            continue;
        }

        let id = dest.trim_end_matches(".md").replace('\\', "/");
        if !ids.contains_key(id.as_str()) {
            return Err(format!("SUMMARY.md:{}: `{dest}` is not a page", number + 1));
        }
        // mdBook nests by two spaces per level.
        toc.push(((indent / 2) as u8, id));
    }

    if toc.is_empty() {
        return Err("SUMMARY.md lists no pages".to_owned());
    }
    Ok(toc)
}

fn write_header(rust: &mut String) {
    rust.push_str(
        "// @generated by crates/ui/build/pages.rs from docs/book/src — do not edit.\n\n\
         /// Where a manual page's image sits in its markdown, so the runtime can splice\n\
         /// in a resolved path (or the alt text, when the image is not installed)\n\
         /// without parsing markdown per frame.\n\
         #[derive(Debug)]\n\
         pub struct ImageRef {\n\
         \x20   /// Byte range of the destination inside the page's markdown.\n\
         \x20   pub start: usize,\n\
         \x20   pub end: usize,\n\
         \x20   /// Destination relative to the locale root (`images/x.png`).\n\
         \x20   pub path: &'static str,\n\
         \x20   /// Alt text, shown in the image's place when it is not on disk.\n\
         \x20   pub alt: &'static str,\n\
         }\n\n\
         /// One page's text and the two things the Help window has to resolve in it.\n\
         #[derive(Debug)]\n\
         pub struct PageText {\n\
         \x20   pub markdown: &'static str,\n\
         \x20   pub images: &'static [ImageRef],\n\
         \x20   /// Intra-book links: the href as written, and the page it opens.\n\
         \x20   pub links: &'static [(&'static str, Page)],\n\
         }\n\n",
    );
}

fn write_enum(rust: &mut String, pages: &[PageSource], toc: &[(u8, String)]) {
    rust.push_str(
        "/// A page of the manual, named after its path under `docs/book/src/<lang>/`.\n\
         ///\n\
         /// Generated, so adding a page needs no Rust edit — but naming one from code\n\
         /// (a panel's `help_page`, say) is checked by the compiler.\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\n\
         pub enum Page {\n",
    );
    for page in pages {
        let _ = writeln!(rust, "    /// `{}.md`", page.id);
        let _ = writeln!(rust, "    {},", page.variant);
    }
    rust.push_str("}\n\n");

    rust.push_str("impl Page {\n    /// Every page, in path order.\n");
    let _ = writeln!(rust, "    pub const ALL: [Page; {}] = [", pages.len());
    for page in pages {
        let _ = writeln!(rust, "        Page::{},", page.variant);
    }
    rust.push_str("    ];\n\n");

    rust.push_str("    /// The page's path id (`panels/ambient-occlusion`).\n");
    rust.push_str("    pub fn id(self) -> &'static str {\n        match self {\n");
    for page in pages {
        let _ = writeln!(
            rust,
            "            Page::{} => \"{}\",",
            page.variant, page.id
        );
    }
    rust.push_str("        }\n    }\n\n");

    rust.push_str(
        "    /// The page's `# ` heading, for the contents list.\n\
         \x20   ///\n\
         \x20   /// English for now: a translated page keeps its own heading in its own\n\
         \x20   /// markdown, and the contents list will read it from there when a second\n\
         \x20   /// locale exists.\n",
    );
    rust.push_str("    pub fn title(self) -> &'static str {\n        match self {\n");
    for page in pages {
        let _ = writeln!(
            rust,
            "            Page::{} => \"{}\",",
            page.variant,
            page.title.replace('"', "'")
        );
    }
    rust.push_str("        }\n    }\n\n");

    rust.push_str(
        "    /// The page a path id names, for resolving a link at run time.\n\
         \x20   pub fn from_id(id: &str) -> Option<Page> {\n\
         \x20       Page::ALL.into_iter().find(|page| page.id() == id)\n\
         \x20   }\n}\n\n",
    );

    // The manual's front page, which is whatever `SUMMARY.md` lists first. Taken
    // from the contents rather than hard-coded, so a book that renames or drops
    // `index.md` still has a first page to open at.
    let first = toc.first().map_or("Index", |(_, id)| id.as_str());
    let _ = writeln!(
        rust,
        "impl Default for Page {{\n\
         \x20   /// The first page in `SUMMARY.md` — where Help opens with no page asked for.\n\
         \x20   fn default() -> Self {{\n\
         \x20       Page::{}\n\
         \x20   }}\n}}\n",
        variant_name(first)
    );
}

fn write_texts(
    rust: &mut String,
    book_src: &Path,
    locales: &[String],
    pages: &[PageSource],
) -> Result<(), String> {
    rust.push_str(
        "impl Page {\n\
         \x20   /// The page's markdown and the spans to resolve in it.\n\
         \x20   ///\n\
         \x20   /// A locale that has not translated this page falls back to English, per\n\
         \x20   /// page rather than per catalog — a half-translated manual is still worth\n\
         \x20   /// showing.\n\
         \x20   pub fn text(self, locale: &str) -> &'static PageText {\n",
    );

    for locale in locales {
        if locale == "en" {
            continue;
        }
        let _ = writeln!(rust, "        if locale == \"{locale}\" {{");
        let _ = writeln!(
            rust,
            "            if let Some(text) = LOCALE_{}.get(self as usize).copied().flatten() {{",
            locale.to_uppercase().replace('-', "_")
        );
        rust.push_str("                return text;\n            }\n        }\n");
    }
    rust.push_str("        let _ = locale;\n        EN[self as usize]\n    }\n}\n\n");

    for locale in locales {
        let dir = book_src.join(locale);
        let ident = if locale == "en" {
            "EN".to_owned()
        } else {
            format!("LOCALE_{}", locale.to_uppercase().replace('-', "_"))
        };

        if locale == "en" {
            let _ = writeln!(
                rust,
                "/// Every English page, indexed by `Page as usize`.\nstatic EN: [&PageText; {}] = [",
                pages.len()
            );
            for page in pages {
                write_page_text(rust, page, &page.path, 4)?;
            }
            rust.push_str("];\n\n");
        } else {
            let _ = writeln!(
                rust,
                "/// `{locale}`'s pages, `None` where it has not translated one.\n\
                 static {ident}: [Option<&PageText>; {}] = [",
                pages.len()
            );
            for page in pages {
                let translated = dir.join(format!("{}.md", page.id));
                if translated.is_file() {
                    rust.push_str("    Some(");
                    write_page_text(rust, page, &translated, 4)?;
                    rust.push_str("    ),\n");
                } else {
                    rust.push_str("    None,\n");
                }
            }
            rust.push_str("];\n\n");
        }
    }
    Ok(())
}

fn write_page_text(
    rust: &mut String,
    page: &PageSource,
    path: &Path,
    indent: usize,
) -> Result<(), String> {
    let pad = " ".repeat(indent);
    let include = path.to_string_lossy().replace('\\', "/");
    let _ = writeln!(rust, "{pad}&PageText {{");
    let _ = writeln!(rust, "{pad}    markdown: include_str!(\"{include}\"),");

    if page.images.is_empty() {
        let _ = writeln!(rust, "{pad}    images: &[],");
    } else {
        let _ = writeln!(rust, "{pad}    images: &[");
        for image in &page.images {
            let _ = writeln!(
                rust,
                "{pad}        ImageRef {{ start: {}, end: {}, path: \"{}\", alt: \"{}\" }},",
                image.start,
                image.end,
                image.path,
                image.alt.replace('\\', "\\\\").replace('"', "\\\"")
            );
        }
        let _ = writeln!(rust, "{pad}    ],");
    }

    if page.links.is_empty() {
        let _ = writeln!(rust, "{pad}    links: &[],");
    } else {
        let _ = writeln!(rust, "{pad}    links: &[");
        for (href, target) in &page.links {
            let _ = writeln!(
                rust,
                "{pad}        (\"{href}\", Page::{}),",
                variant_name(target)
            );
        }
        let _ = writeln!(rust, "{pad}    ],");
    }
    let _ = writeln!(rust, "{pad}}},");
    Ok(())
}

fn write_toc(rust: &mut String, toc: &[(u8, String)]) {
    rust.push_str(
        "/// `SUMMARY.md` flattened: nesting depth and page, in order. The Help\n\
         /// window's contents list and the site's sidebar are the same list.\n",
    );
    let _ = writeln!(rust, "pub const TOC: [(u8, Page); {}] = [", toc.len());
    for (depth, id) in toc {
        let _ = writeln!(rust, "    ({depth}, Page::{}),", variant_name(id));
    }
    rust.push_str("];\n");
}

fn rerun_if_changed(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rerun_if_changed(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
