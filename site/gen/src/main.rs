//! Builds the MapCraft website: copies `site/`, then renders the guide in `docs/guide/`, the
//! technical docs in `docs/` and `godot/README.md`.
//!
//! `cargo run --manifest-path site/gen/Cargo.toml -- [--out DIR] [--sync]`
//! writes to `target/site` by default; `--sync` also rewrites the shortcut table in
//! `docs/guide/shortcuts.md` from the code.
//!
//! Each section's `README.md` is its index page; its `##` headings group the sidebar and its link
//! order sets the page order. Pages are split into blocks at `##` and `###` headings, and a
//! paragraph holding only images becomes the block's screenshots, shown beside its text
//! (`img/x.webp#wide` spans the block instead).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};

type Result<T> = std::result::Result<T, String>;

const DEFAULT_REPO: &str = "https://github.com/bearlikelion/MapCraft";
const SHORTCUTS_START: &str = "<!-- shortcuts:start -->";
const SHORTCUTS_END: &str = "<!-- shortcuts:end -->";
const SECTIONS: [&str; 2] = ["Guide", "Technical"];
/// Folders whose `img/` is copied beside their pages.
const IMAGES: [(&str, &str); 2] = [("docs/guide", "guide"), ("docs", "technical")];
const FONTS: &str = "https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@100..125,600..900&family=Source+Sans+3:ital,wght@0,400..700;1,400&family=JetBrains+Mono:wght@400..600&display=swap";

struct Page {
    /// Repository path of the Markdown source.
    src: String,
    /// Site path of the generated page.
    out: String,
    title: String,
    section: &'static str,
    /// The sidebar group, from the section index's `##` headings.
    group: String,
    text: String,
}

struct Heading {
    id: String,
    text: String,
}

struct Ctx<'a> {
    root: &'a Path,
    pages: &'a [Page],
    repo: &'a str,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("site: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().map_err(|e| format!("{}: {e}", root.display()))?;
    let mut out = root.join("target/site");
    let mut sync = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a folder")?),
            "--sync" => sync = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let index = read(&root.join("site/index.html"))?;
    let repo = attr(&index, "data-repo").unwrap_or(DEFAULT_REPO).trim_end_matches('/').to_string();

    let mut pages = collect(&root)?;
    let table = shortcut_table(&root)?;
    for p in pages.iter_mut().filter(|p| p.src == "docs/guide/shortcuts.md") {
        if let Some(text) = splice(&p.text, &table) {
            if sync && text != p.text {
                write(&root.join(&p.src), &text)?;
            }
            p.text = text;
        }
    }

    copy_dir(&root.join("site"), &out, &["gen"])?;
    for (from, to) in IMAGES {
        let dir = root.join(from).join("img");
        if dir.is_dir() {
            copy_dir(&dir, &out.join(to).join("img"), &[])?;
        }
    }
    let ctx = Ctx { root: &root, pages: &pages, repo: &repo };
    for p in &pages {
        write(&out.join(&p.out), &render(p, &ctx))?;
    }
    println!("site: {} pages in {}", pages.len(), out.display());
    Ok(())
}

/// Every page in sidebar order: the guide, then the technical docs.
fn collect(root: &Path) -> Result<Vec<Page>> {
    let guide = md_files(&root.join("docs/guide"))?.into_iter().map(|n| format!("docs/guide/{n}")).collect();
    let mut technical: Vec<String> = md_files(&root.join("docs"))?.into_iter().map(|n| format!("docs/{n}")).collect();
    technical.push("godot/README.md".into());
    let mut pages = section(root, "Guide", "docs/guide/README.md", "guide", guide)?;
    pages.extend(section(root, "Technical", "docs/README.md", "technical", technical)?);
    Ok(pages)
}

/// A section's index page, then its other pages in the order the index links them, each in the
/// group of the `##` heading it is linked under; unlinked pages come last under More.
fn section(root: &Path, name: &'static str, index: &str, dir: &str, srcs: Vec<String>) -> Result<Vec<Page>> {
    let text = read(&root.join(index))?;
    let base = index.rsplit_once('/').map_or("", |(d, _)| d);
    let mut order: Vec<(String, String)> = Vec::new();
    let mut group = String::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            group = h.trim().to_string();
            continue;
        }
        for l in links(line) {
            let path = l.split('#').next().unwrap_or_default();
            if let Some(src) = normalize(&format!("{base}/{path}")).filter(|s| !order.iter().any(|(o, _)| o == s)) {
                order.push((src, group.clone()));
            }
        }
    }
    let mut rest = Vec::new();
    for src in srcs.into_iter().filter(|s| s != index) {
        let at = order.iter().position(|(s, _)| *s == src);
        let group = at.and_then(|i| order.get(i)).map_or_else(|| "More".to_string(), |(_, g)| g.clone());
        let file = match stem(&src) {
            "README" => src.rsplit('/').nth(1).unwrap_or("readme"),
            s => s,
        };
        let text = read(&root.join(&src))?;
        rest.push((at.unwrap_or(usize::MAX), page(src.clone(), format!("{dir}/{file}.html"), name, group, text)));
    }
    rest.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase())));
    let mut pages = vec![page(index.to_string(), format!("{dir}/index.html"), name, String::new(), text)];
    pages.extend(rest.into_iter().map(|(_, p)| p));
    Ok(pages)
}

fn page(src: String, out: String, section: &'static str, group: String, text: String) -> Page {
    let title = text.lines().find_map(|l| l.strip_prefix("# ")).map_or_else(|| stem(&src).to_string(), |t| t.replace('`', ""));
    Page { src, out, title, section, group, text }
}

fn render(p: &Page, ctx: &Ctx) -> String {
    let (body, toc) = markdown(&p.text, p, ctx, true);
    layout(p, ctx, &body, &toc)
}

/// A block of a page: the text under one heading, and the screenshots that go beside it.
struct Block<'a> {
    level: usize,
    text: Vec<Event<'a>>,
    media: String,
    wide: String,
}

impl Block<'_> {
    fn html(self) -> String {
        let mut text = String::new();
        html::push_html(&mut text, self.text.into_iter());
        if self.level == 0 {
            return text + &self.wide;
        }
        let (class, media) = if self.media.is_empty() { ("", String::new()) } else { (" has-media", format!("<div class=\"block-media\">\n{}</div>\n", self.media)) };
        let wide = if self.wide.is_empty() { String::new() } else { format!("<div class=\"block-wide\">\n{}</div>\n", self.wide) };
        let kind = if self.level == 1 { "intro" } else { "block" };
        format!("<section class=\"{kind} l{}{class}\">\n<div class=\"block-text\">\n{text}</div>\n{media}{wide}</section>\n", self.level)
    }
}

/// Markdown to HTML with heading anchors, wrapped tables, keycaps for chords, links rewritten to
/// generated pages or to the repository on GitHub, and, with `blocks`, one section per heading.
fn markdown(text: &str, p: &Page, ctx: &Ctx, blocks: bool) -> (String, Vec<Heading>) {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM;
    let dir = ctx.root.join(p.src.rsplit_once('/').map_or("", |(d, _)| d));
    let mut done: Vec<Block> = Vec::new();
    let mut block = Block { level: if blocks { 1 } else { 0 }, text: Vec::new(), media: String::new(), wide: String::new() };
    let mut toc = Vec::new();
    let mut ids = BTreeSet::new();
    let mut heading: Option<(usize, Vec<Event>)> = None;
    let mut para: Option<Vec<Event>> = None;
    for ev in Parser::new_ext(text, opts) {
        let ev = match ev {
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Link { link_type, dest_url: link(&dest_url, &p.src, &p.out, ctx).into(), title, id })
            }
            Event::Code(code) => match keys(&code) {
                Some(h) => Event::InlineHtml(h.into()),
                None => Event::Code(code),
            },
            Event::Start(Tag::Table(align)) => {
                block.text.push(Event::Html("<div class=\"table-wrap\">".into()));
                Event::Start(Tag::Table(align))
            }
            Event::End(TagEnd::Table) => {
                block.text.push(Event::End(TagEnd::Table));
                Event::Html("</div>\n".into())
            }
            Event::Start(Tag::Paragraph) if heading.is_none() => {
                para = Some(Vec::new());
                continue;
            }
            Event::End(TagEnd::Paragraph) if heading.is_none() => {
                let buf = para.take().unwrap_or_default();
                match figures(&buf, &dir) {
                    Some(figs) => {
                        for (wide, fig) in figs {
                            if wide || !blocks { block.wide.push_str(&fig) } else { block.media.push_str(&fig) }
                        }
                    }
                    None => {
                        block.text.push(Event::Start(Tag::Paragraph));
                        block.text.extend(buf);
                        block.text.push(Event::End(TagEnd::Paragraph));
                    }
                }
                continue;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level = level as usize;
                if blocks && (2..=3).contains(&level) {
                    let next = Block { level, text: Vec::new(), media: String::new(), wide: String::new() };
                    done.push(std::mem::replace(&mut block, next));
                }
                heading = Some((level, Vec::new()));
                continue;
            }
            Event::End(TagEnd::Heading(_)) => {
                let Some((level, inner)) = heading.take() else { continue };
                let text: String = inner
                    .iter()
                    .filter_map(|e| match e {
                        Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                        _ => None,
                    })
                    .collect();
                let id = unique(slug(&text), &mut ids);
                let mut h = String::new();
                html::push_html(&mut h, inner.into_iter());
                let anchor = if level > 1 { format!("<a class=\"anchor\" href=\"#{id}\" aria-hidden=\"true\" tabindex=\"-1\">#</a>") } else { String::new() };
                if level == 2 {
                    toc.push(Heading { id: id.clone(), text: text.clone() });
                }
                Event::Html(format!("<h{level} id=\"{id}\">{h}{anchor}</h{level}>\n").into())
            }
            other => other,
        };
        match (&mut heading, &mut para) {
            (Some((_, inner)), _) => inner.push(ev),
            (None, Some(buf)) => buf.push(ev),
            (None, None) => block.text.push(ev),
        }
    }
    done.push(block);
    let out = done.into_iter().map(Block::html).collect();
    (out, toc)
}

/// The figures of a paragraph that holds only images, as (spans the block, HTML).
fn figures(events: &[Event], dir: &Path) -> Option<Vec<(bool, String)>> {
    let mut out = Vec::new();
    let mut image: Option<(String, String, String)> = None;
    for e in events {
        match (e, &mut image) {
            (Event::Start(Tag::Image { dest_url, title, .. }), None) => image = Some((dest_url.to_string(), title.to_string(), String::new())),
            (Event::End(TagEnd::Image), Some(_)) => {
                let (url, title, alt) = image.take()?;
                out.push(figure(&url, &title, &alt, dir));
            }
            (Event::Text(t), Some((_, _, alt))) => alt.push_str(t),
            (Event::SoftBreak | Event::HardBreak, None) => {}
            (Event::Text(t), None) if t.trim().is_empty() => {}
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

fn figure(url: &str, title: &str, alt: &str, dir: &Path) -> (bool, String) {
    let (path, wide) = url.strip_suffix("#wide").map_or((url, false), |p| (p, true));
    let size = webp_size(&dir.join(path)).map_or_else(String::new, |(w, h)| format!(" width=\"{w}\" height=\"{h}\""));
    let caption = if title.is_empty() { String::new() } else { format!("<figcaption>{}</figcaption>", esc(title)) };
    let path = esc(path);
    let html = format!("<figure class=\"shot\"><a href=\"{path}\"><img src=\"{path}\" alt=\"{}\"{size} loading=\"lazy\" decoding=\"async\"></a>{caption}</figure>\n", esc(alt));
    (wide, html)
}

/// Width and height of a WebP image, from its header.
fn webp_size(path: &Path) -> Option<(u32, u32)> {
    let b = fs::read(path).ok()?;
    let le = |at: usize, n: usize| b.get(at..at + n).map(|s| s.iter().rev().fold(0u32, |a, &x| (a << 8) | u32::from(x)));
    match b.get(12..16)? {
        b"VP8X" => Some((le(24, 3)? + 1, le(27, 3)? + 1)),
        b"VP8L" => le(21, 4).map(|v| ((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1)),
        b"VP8 " => Some((le(26, 2)? & 0x3fff, le(28, 2)? & 0x3fff)),
        _ => None,
    }
}

/// A link to another rendered Markdown file becomes a link to its page; any other repository path
/// goes to GitHub.
fn link(url: &str, src: &str, from: &str, ctx: &Ctx) -> String {
    if url.starts_with('#') || url.contains("://") || url.starts_with("mailto:") {
        return url.to_string();
    }
    let (path, frag) = url.split_once('#').map_or((url, String::new()), |(p, f)| (p, format!("#{f}")));
    let dir = src.rsplit_once('/').map_or("", |(d, _)| d);
    let Some(target) = normalize(&format!("{dir}/{path}")) else { return url.to_string() };
    match ctx.pages.iter().find(|p| p.src == target) {
        Some(p) => format!("{}{}{frag}", up(from), p.out),
        None => format!("{}/blob/main/{target}{frag}", ctx.repo),
    }
}

/// Resolves `.` and `..` in a repository path; `None` when it climbs out of the repository.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The way back to the site root from a page.
fn up(page: &str) -> String {
    "../".repeat(page.matches('/').count())
}

/// Keycaps for a code span that is only key chords, such as `Ctrl+Shift+M` or `Alt+Right Click, F1`.
fn keys(code: &str) -> Option<String> {
    let chords: Vec<Vec<&str>> = code.split(", ").map(|c| c.split('+').map(str::trim).collect()).collect();
    if !chords.iter().flatten().all(|k| is_key(k)) {
        return None;
    }
    let html: Vec<String> = chords.iter().map(|c| c.iter().map(|k| format!("<kbd>{}</kbd>", esc(k))).collect::<Vec<_>>().join("+")).collect();
    Some(format!("<span class=\"keys\">{}</span>", html.join(", ")))
}

fn is_key(k: &str) -> bool {
    const NAMED: &[&str] = &[
        "Ctrl",
        "Shift",
        "Alt",
        "Cmd",
        "Space",
        "Delete",
        "Home",
        "End",
        "Enter",
        "Esc",
        "Escape",
        "Tab",
        "Backspace",
        "Minus",
        "Plus",
        "Comma",
        "Period",
        "PageUp",
        "PageDown",
        "Insert",
        "Up",
        "Down",
        "Left",
        "Right",
        "[",
        "]",
        "Click",
        "Drag",
        "Right Click",
        "Right Drag",
        "Middle Drag",
        "Scroll",
        "WASD",
    ];
    let function = k.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n));
    NAMED.contains(&k) || function || (k.len() == 1 && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
}

fn layout(p: &Page, ctx: &Ctx, body: &str, toc: &[Heading]) -> String {
    let base = up(&p.out);
    let repo = ctx.repo;
    let mut nav = String::new();
    // Each section has its own sidebar; the header switches between them.
    for section in SECTIONS.into_iter().filter(|s| *s == p.section) {
        let class = section.to_lowercase();
        let note = if section == "Technical" { "<span class=\"side-note\">optional</span>" } else { "" };
        let _ = writeln!(nav, "<div class=\"side-section {class}\">\n<p class=\"side-title\">{section}{note}</p>");
        let mut group: Option<&str> = None;
        for q in ctx.pages.iter().filter(|q| q.section == section) {
            if group != Some(q.group.as_str()) {
                if group.is_some() {
                    nav.push_str("</ul>\n");
                }
                if !q.group.is_empty() {
                    let _ = writeln!(nav, "<p class=\"side-group\">{}</p>", esc(&q.group));
                }
                nav.push_str("<ul>\n");
                group = Some(q.group.as_str());
            }
            let title = if q.out.ends_with("/index.html") { "Overview" } else { q.title.as_str() };
            if q.out != p.out {
                let _ = writeln!(nav, "<li><a href=\"{base}{}\">{}</a></li>", q.out, esc(title));
                continue;
            }
            let _ = write!(nav, "<li><a href=\"{base}{}\" aria-current=\"page\">{}</a>", q.out, esc(title));
            if toc.len() > 1 {
                nav.push_str("\n<ul class=\"side-toc\">\n");
                for h in toc {
                    let _ = writeln!(nav, "<li><a href=\"#{}\">{}</a></li>", h.id, esc(&h.text));
                }
                nav.push_str("</ul>\n");
            }
            nav.push_str("</li>\n");
        }
        if group.is_some() {
            nav.push_str("</ul>\n");
        }
        nav.push_str("</div>\n");
    }
    let siblings: Vec<&Page> = ctx.pages.iter().filter(|q| q.section == p.section).collect();
    let side = if siblings.len() > 1 {
        format!(
            "<nav class=\"side\" aria-label=\"{section}\">\n<details class=\"side-menu\" id=\"side-menu\">\n<summary>{section} pages</summary>\n{nav}</details>\n</nav>\n",
            section = p.section
        )
    } else {
        String::new()
    };
    let shell = if side.is_empty() { "doc-shell single" } else { "doc-shell" };
    let at = siblings.iter().position(|q| q.out == p.out).unwrap_or(0);
    let mut pager = String::new();
    let prev = at.checked_sub(1).and_then(|i| siblings.get(i));
    let next = siblings.get(at + 1);
    if prev.is_some() || next.is_some() {
        pager.push_str("<nav class=\"pager\" aria-label=\"Previous and next page\">\n");
        if let Some(q) = prev {
            let _ = writeln!(pager, "<a class=\"prev\" href=\"{base}{}\"><span>Previous</span>{}</a>", q.out, esc(&q.title));
        }
        if let Some(q) = next {
            let _ = writeln!(pager, "<a class=\"next\" href=\"{base}{}\"><span>Next</span>{}</a>", q.out, esc(&q.title));
        }
        pager.push_str("</nav>\n");
    }
    let crumb = match (p.section, p.group.as_str()) {
        (s, "") => format!("<p class=\"crumb {}\"><span>{s}</span></p>\n", s.to_lowercase()),
        (s, g) => format!("<p class=\"crumb {}\"><span>{s}</span>{}</p>\n", s.to_lowercase(), esc(g)),
    };
    let current = |s: &str| if p.section == s { " aria-current=\"true\"" } else { "" };
    format!(
        r##"<!doctype html>
<html lang="en" data-repo="{repo}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} | MapCraft</title>
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="#15171b" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#eef0f3" media="(prefers-color-scheme: light)">
<link rel="icon" href="{base}assets/mapcraft.svg" type="image/svg+xml">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="{FONTS}">
<link rel="stylesheet" href="{base}style.css">
</head>
<body class="docs-page">

<a class="skip" href="#main">Skip to content</a>
<header class="top">
	<div class="top-inner">
		<a class="brand" href="{base}index.html"><img src="{base}assets/mapcraft.svg" width="28" height="28" alt="">MapCraft</a>
		<nav aria-label="Site">
			<a class="opt" href="{base}index.html#download">Download</a>
			<a href="{base}guide/index.html"{guide}>Guide</a>
			<a href="{base}technical/index.html"{technical}>Technical</a>
			<a href="{repo}">GitHub</a>
		</nav>
	</div>
</header>

<div class="{shell}">
{side}<main class="doc" id="main">
{crumb}{body}{pager}<p class="doc-source"><a href="{repo}/blob/main/{src}">Edit this page on GitHub</a></p>
</main>
</div>

<footer class="foot">
	<div class="foot-inner">
		<p>MapCraft is made by <a href="https://github.com/bearlikelion">Mark</a> and is free under the <a href="{repo}/blob/main/LICENSE">MIT License</a>.</p>
		<p>Docs are built from the Markdown in the repository.</p>
	</div>
</footer>

<script>
	const menu = document.getElementById("side-menu");
	if (menu && matchMedia("(min-width: 1000px)").matches) menu.open = true;
</script>
</body>
</html>
"##,
        title = esc(&p.title),
        src = p.src,
        guide = current("Guide"),
        technical = current("Technical"),
    )
}

/// The shortcut tables, from the shortcut lists in the editor's code.
fn shortcut_table(root: &Path) -> Result<String> {
    let ui = read(&root.join("crates/ui/src/shortcuts.rs"))?;
    let engine = read(&root.join("crates/engine/src/commands.rs"))?;
    let dialogs = read(&root.join("crates/ui/src/dialogs.rs"))?;
    let mut actions: Vec<[String; 3]> = Vec::new();
    for t in tuples(block(&ui, "UI_ACTIONS", "];")?) {
        if let [id, label, chord] = t.as_slice() {
            actions.push([id.clone(), label.clone(), chord.clone()]);
        }
    }
    for spec in block(&engine, "const COMMANDS", "\n];")?.split("CommandSpec {").skip(1) {
        let field = |name: &str| spec.split_once(name).and_then(|(_, rest)| rest.split('"').nth(1)).map(String::from);
        let (Some(id), Some(label)) = (field("id: "), field("label: ")) else { continue };
        let chord = if spec.contains("shortcut: Some(") { field("shortcut: Some(").unwrap_or_default() } else { String::new() };
        actions.push([id, label, chord]);
    }
    let fixed = tuples(block(&dialogs, "const FIXED", "];")?);

    const GROUPS: &[(&str, &str)] = &[
        ("file.", "File"),
        ("edit.", "Edit"),
        ("tool.", "Tools"),
        ("shape.", "Tool options"),
        ("clip.", "Tool options"),
        ("inspector.", "Tool options"),
        ("tools.", "Brushes and organizing"),
        ("layer.", "Brushes and organizing"),
        ("prefab.", "Prefabs"),
        ("view.", "View"),
        ("panel.", "Panels"),
        ("run.", "Run"),
        ("help.", "Help"),
        ("uv.", "UV editor"),
        ("vertex.", "Vertex tool"),
    ];
    let group = |id: &str| GROUPS.iter().find(|(p, _)| id.starts_with(p)).map_or("Other", |(_, g)| *g);
    let cell = |chord: &str| if chord.is_empty() { "*none*".to_string() } else { format!("`{chord}`") };
    let mut out = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for g in GROUPS.iter().map(|(_, g)| *g).chain(["Other"]) {
        if seen.contains(&g) {
            continue;
        }
        seen.push(g);
        let rows: Vec<&[String; 3]> = actions.iter().filter(|a| group(&a[0]) == g).collect();
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(out, "### {g}\n\n| Action | Shortcut |\n|---|---|");
        for [_, label, chord] in rows {
            let _ = writeln!(out, "| {label} | {} |", cell(chord));
        }
        out.push('\n');
    }
    out.push_str("### Mouse and fixed keys\n\nThese cannot be rebound.\n\n| Action | Keys |\n|---|---|\n");
    for t in &fixed {
        if let [label, chord] = t.as_slice() {
            let _ = writeln!(out, "| {label} | {} |", cell(chord));
        }
    }
    let overrides = |name: &str| -> Result<Vec<Vec<String>>> { Ok(tuples(block(&ui, &format!("Preset::{name} => &["), "],")?)) };
    let (hammer, trench) = (overrides("Hammer")?, overrides("TrenchBroom")?);
    let mut unique_ids: Vec<&String> = Vec::new();
    for id in hammer.iter().chain(&trench).filter_map(|t| t.first()) {
        if !unique_ids.contains(&id) {
            unique_ids.push(id);
        }
    }
    out.push_str("\n### Hammer and TrenchBroom presets\n\nThese actions change when you pick a preset; everything else keeps the keys above.\n\n| Action | Hammer | TrenchBroom |\n|---|---|---|\n");
    for id in unique_ids {
        let Some(action) = actions.iter().find(|a| &a[0] == id) else { continue };
        let pick = |list: &[Vec<String>]| list.iter().find(|t| t.first() == Some(id)).and_then(|t| t.get(1).cloned()).unwrap_or_else(|| action[2].clone());
        let (h, t) = (pick(&hammer), pick(&trench));
        if h != action[2] || t != action[2] {
            let _ = writeln!(out, "| {} | {} | {} |", action[1], cell(&h), cell(&t));
        }
    }
    Ok(out)
}

/// The text of `src` from `start` to the first `end` after it.
fn block<'a>(src: &'a str, start: &str, end: &str) -> Result<&'a str> {
    let from = src.find(start).ok_or_else(|| format!("`{start}` not found"))?;
    let rest = &src[from..];
    Ok(rest.find(end).map_or(rest, |i| &rest[..i]))
}

/// The string literals of each parenthesized tuple in `block`.
fn tuples(block: &str) -> Vec<Vec<String>> {
    block
        .split('(')
        .skip(1)
        .filter_map(|s| {
            let inner = s.split(')').next()?;
            let parts: Vec<String> = inner.split('"').skip(1).step_by(2).map(String::from).collect();
            (!parts.is_empty()).then_some(parts)
        })
        .collect()
}

/// `text` with the generated table between the shortcut markers, or `None` without markers.
fn splice(text: &str, table: &str) -> Option<String> {
    let (head, rest) = text.split_once(SHORTCUTS_START)?;
    let (_, tail) = rest.split_once(SHORTCUTS_END)?;
    Some(format!("{head}{SHORTCUTS_START}\n\n{table}\n{SHORTCUTS_END}{tail}"))
}

fn links(text: &str) -> Vec<String> {
    Parser::new(text)
        .filter_map(|e| match e {
            Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.to_string()),
            _ => None,
        })
        .collect()
}

fn slug(text: &str) -> String {
    let mut s = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() {
            s.push(c);
        } else if (c == ' ' || c == '-' || c == '_') && !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-');
    if s.is_empty() { "section".into() } else { s.into() }
}

fn unique(id: String, seen: &mut BTreeSet<String>) -> String {
    let mut candidate = id.clone();
    let mut n = 2;
    while !seen.insert(candidate.clone()) {
        candidate = format!("{id}-{n}");
        n += 1;
    }
    candidate
}

fn attr<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    html.split_once(&format!("{name}=\""))?.1.split('"').next()
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn stem(name: &str) -> &str {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.strip_suffix(".md").unwrap_or(file)
}

fn md_files(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();
    names.sort();
    Ok(names)
}

fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let name = entry.file_name();
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() {
            copy_dir(&src, &dst, &[])?;
        } else {
            fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_page(src: &str, out: &str, text: &str) -> Page {
        page(src.into(), out.into(), "Guide", String::new(), text.into())
    }

    #[test]
    fn helpers() {
        assert_eq!(normalize("docs/../wit/plugin.wit").as_deref(), Some("wit/plugin.wit"));
        assert_eq!(normalize("/ROADMAP.md").as_deref(), Some("ROADMAP.md"));
        assert_eq!(normalize("docs/../../x"), None);
        assert_eq!(up("guide/views.html"), "../");
        assert_eq!(up("index.html"), "");
        assert_eq!(slug("The `.mcmap` format"), "the-mcmap-format");
        assert!(keys("Ctrl+Shift+M").is_some());
        assert!(keys("Alt+Right Click, Alt+Shift+Right Click").is_some());
        assert!(keys("F9").is_some());
        assert!(keys("+map").is_none());
        assert!(keys("sdk.toml").is_none());
        assert!(keys("C++").is_none());
        let t = splice("a\n<!-- shortcuts:start -->\nold\n<!-- shortcuts:end -->\nb", "new\n").unwrap_or_default();
        assert!(t.contains("new") && !t.contains("old") && t.ends_with("b"));
        let card = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/card-views.webp");
        assert_eq!(webp_size(&card), Some((800, 500)));
    }

    #[test]
    fn links_resolve() {
        let pages = vec![page("docs/plugin-api.md".into(), "technical/plugin-api.html".into(), "Technical", String::new(), "# Plugin API".into())];
        let ctx = Ctx { root: Path::new("."), pages: &pages, repo: "https://github.com/o/r" };
        assert_eq!(link("../plugin-api.md#imports", "docs/guide/x.md", "guide/x.html", &ctx), "../technical/plugin-api.html#imports");
        assert_eq!(link("../wit/plugin.wit", "docs/plugin-api.md", "technical/plugin-api.html", &ctx), "https://github.com/o/r/blob/main/wit/plugin.wit");
        assert_eq!(link("https://x.y/z", "docs/README.md", "technical/index.html", &ctx), "https://x.y/z");
    }

    #[test]
    fn blocks_put_images_beside_text() {
        let text = "# T\n\nLede.\n\n## Step\n\nDo it.\n\n![Alt](img/a.webp)\n\n![Wide](img/b.webp#wide \"Cap\")\n\n### Sub\n\n> [!TIP]\n> Hint.\n";
        let p = ctx_page("docs/guide/t.md", "guide/t.html", text);
        let pages = vec![];
        let ctx = Ctx { root: Path::new("."), pages: &pages, repo: "r" };
        let (html, toc) = markdown(text, &p, &ctx, true);
        assert_eq!(toc.len(), 1);
        assert!(html.starts_with("<section class=\"intro l1\">"));
        assert!(html.contains("<section class=\"block l2 has-media\">"));
        assert!(html.contains("<div class=\"block-media\">\n<figure class=\"shot\"><a href=\"img/a.webp\">"));
        assert!(html.contains("<div class=\"block-wide\">\n<figure class=\"shot\"><a href=\"img/b.webp\">"));
        assert!(html.contains("<figcaption>Cap</figcaption>"));
        assert!(html.contains("<section class=\"block l3\">"));
        assert!(html.contains("markdown-alert-tip"));
        assert!(!html.contains("<p><img"));
    }
}
