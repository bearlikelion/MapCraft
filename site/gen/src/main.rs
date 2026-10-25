//! Builds the MapCraft website: copies `site/`, then renders the guides in `docs/guide/`, the
//! reference docs in `docs/`, `godot/README.md` and a roadmap page from `ROADMAP.md`.
//!
//! `cargo run --manifest-path site/gen/Cargo.toml -- [--out DIR] [--sync]`
//! writes to `target/site` by default; `--sync` also rewrites the shortcut table in
//! `docs/guide/shortcuts.md` from the code.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};

type Result<T> = std::result::Result<T, String>;

const DEFAULT_REPO: &str = "https://github.com/bearlikelion/MapCraft";
const SHORTCUTS_START: &str = "<!-- shortcuts:start -->";
const SHORTCUTS_END: &str = "<!-- shortcuts:end -->";
const SECTIONS: [&str; 3] = ["Guide", "Reference", "Project"];

struct Page {
    /// Repository path of the Markdown source.
    src: String,
    /// Site path of the generated page.
    out: String,
    title: String,
    section: &'static str,
    text: String,
}

struct Heading {
    level: usize,
    id: String,
    text: String,
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
    for p in &pages {
        let page = if p.src == "ROADMAP.md" { roadmap(p, &pages, &repo) } else { render(p, &pages, &repo) };
        write(&out.join(&p.out), &page)?;
    }
    println!("site: {} pages in {}", pages.len(), out.display());
    Ok(())
}

/// Every page in sidebar order: the guide (its README's link order first), the reference docs by
/// title, then the roadmap.
fn collect(root: &Path) -> Result<Vec<Page>> {
    let mut pages = Vec::new();
    let readme = read(&root.join("docs/guide/README.md"))?;
    let order: Vec<String> = links(&readme).into_iter().map(|l| l.trim_start_matches("./").to_string()).collect();
    let mut guides = md_files(&root.join("docs/guide"))?;
    guides.retain(|n| n != "README.md");
    guides.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
    pages.push(page("docs/guide/README.md".into(), "guide/index.html".into(), "Guide", readme));
    for n in guides {
        let src = format!("docs/guide/{n}");
        let text = read(&root.join(&src))?;
        pages.push(page(src, format!("guide/{}.html", stem(&n)), "Guide", text));
    }
    let mut reference = Vec::new();
    for n in md_files(&root.join("docs"))? {
        let src = format!("docs/{n}");
        let text = read(&root.join(&src))?;
        reference.push(page(src, format!("reference/{}.html", stem(&n)), "Reference", text));
    }
    let godot = read(&root.join("godot/README.md"))?;
    reference.push(page("godot/README.md".into(), "reference/godot.html".into(), "Reference", godot));
    reference.sort_by_key(|p| p.title.to_lowercase());
    pages.extend(reference);
    pages.push(page("ROADMAP.md".into(), "roadmap.html".into(), "Project", read(&root.join("ROADMAP.md"))?));
    Ok(pages)
}

fn page(src: String, out: String, section: &'static str, text: String) -> Page {
    let title = text.lines().find_map(|l| l.strip_prefix("# ")).map_or_else(|| stem(&src).to_string(), |t| t.replace('`', ""));
    Page { src, out, title, section, text }
}

fn render(p: &Page, pages: &[Page], repo: &str) -> String {
    let (body, toc) = markdown(&p.text, &p.src, &p.out, pages, repo);
    layout(p, pages, repo, &body, &toc, "prose")
}

/// Markdown to HTML with heading anchors, wrapped tables, keycaps for chords and links rewritten
/// to generated pages or to the repository on GitHub.
fn markdown(text: &str, src: &str, from: &str, pages: &[Page], repo: &str) -> (String, Vec<Heading>) {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut events: Vec<Event> = Vec::new();
    let mut toc = Vec::new();
    let mut ids = BTreeSet::new();
    let mut heading: Option<(usize, Vec<Event>)> = None;
    for ev in Parser::new_ext(text, opts) {
        let ev = match ev {
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Link { link_type, dest_url: link(&dest_url, src, from, pages, repo).into(), title, id })
            }
            Event::Code(code) => match keys(&code) {
                Some(h) => Event::InlineHtml(h.into()),
                None => Event::Code(code),
            },
            Event::Start(Tag::Table(align)) => {
                events.push(Event::Html("<div class=\"table-wrap\">".into()));
                Event::Start(Tag::Table(align))
            }
            Event::End(TagEnd::Table) => {
                events.push(Event::End(TagEnd::Table));
                Event::Html("</div>\n".into())
            }
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level as usize, Vec::new()));
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
                if (2..=3).contains(&level) {
                    toc.push(Heading { level, id: id.clone(), text: text.clone() });
                }
                Event::Html(format!("<h{level} id=\"{id}\">{h}{anchor}</h{level}>\n").into())
            }
            other => other,
        };
        match &mut heading {
            Some((_, inner)) => inner.push(ev),
            None => events.push(ev),
        }
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    (out, toc)
}

/// A link to another rendered Markdown file becomes a link to its page; any other repository path
/// goes to GitHub.
fn link(url: &str, src: &str, from: &str, pages: &[Page], repo: &str) -> String {
    if url.starts_with('#') || url.contains("://") || url.starts_with("mailto:") {
        return url.to_string();
    }
    let (path, frag) = url.split_once('#').map_or((url, String::new()), |(p, f)| (p, format!("#{f}")));
    let dir = src.rsplit_once('/').map_or("", |(d, _)| d);
    let Some(target) = normalize(&format!("{dir}/{path}")) else { return url.to_string() };
    match pages.iter().find(|p| p.src == target) {
        Some(p) => format!("{}{}{frag}", up(from), p.out),
        None => format!("{repo}/blob/main/{target}{frag}"),
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

/// The roadmap as milestones with progress, open items first, later items apart and done items folded.
fn roadmap(p: &Page, pages: &[Page], repo: &str) -> String {
    struct Milestone {
        name: String,
        done: Vec<String>,
        next: Vec<String>,
        later: Vec<String>,
    }
    let mut milestones: Vec<Milestone> = Vec::new();
    for line in p.text.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            milestones.push(Milestone { name: name.trim().to_string(), done: Vec::new(), next: Vec::new(), later: Vec::new() });
            continue;
        }
        let Some(m) = milestones.last_mut() else { continue };
        if let Some(item) = line.strip_prefix("- [x] ").or_else(|| line.strip_prefix("- [X] ")) {
            m.done.push(item.to_string());
        } else if let Some(item) = line.strip_prefix("- [ ] ") {
            match item.strip_prefix("Later: ") {
                Some(later) => m.later.push(capitalize(later)),
                None => m.next.push(item.to_string()),
            }
        }
    }
    let inline = |text: &str| -> String {
        let (h, _) = markdown(text, &p.src, &p.out, pages, repo);
        h.trim().trim_start_matches("<p>").trim_end_matches("</p>").to_string()
    };
    let count = |m: &Milestone| (m.done.len(), m.done.len() + m.next.len() + m.later.len());
    let (done, total) = milestones.iter().map(count).fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
    let mut body = String::new();
    let _ = writeln!(body, "<h1 id=\"roadmap\">{}</h1>", esc(&p.title));
    let _ = writeln!(
        body,
        "<p class=\"rm-lead\">{done} of {total} items done across {} milestones. Open items are what comes next; later items are planned but not scheduled.</p>",
        milestones.len()
    );
    body.push_str("<ol class=\"rm-overview\">\n");
    let mut ids = BTreeSet::new();
    let ids: Vec<String> = milestones.iter().map(|m| unique(slug(&m.name), &mut ids)).collect();
    for (m, id) in milestones.iter().zip(&ids) {
        let (d, t) = count(m);
        let _ = writeln!(
            body,
            "<li><a href=\"#{id}\"><span class=\"rm-name\">{}</span><span class=\"rm-count\">{d}/{t}</span>{}</a></li>",
            esc(&m.name),
            bar(d, t)
        );
    }
    body.push_str("</ol>\n");
    for (m, id) in milestones.iter().zip(&ids) {
        let (d, t) = count(m);
        let (class, status) = match (d, t) {
            (d, t) if t > 0 && d == t => ("done", "Done"),
            (0, _) => ("planned", "Planned"),
            _ => ("active", "In progress"),
        };
        let _ = writeln!(
            body,
            "<section class=\"rm-milestone\" id=\"{id}\">\n<div class=\"rm-head\"><h2>{}</h2><span class=\"pill {class}\">{status}</span><span class=\"rm-count\">{d} of {t} done</span></div>\n{}",
            esc(&m.name),
            bar(d, t)
        );
        for (items, label, class) in [(&m.next, "Next", "next"), (&m.later, "Later", "later")] {
            if !items.is_empty() {
                let _ = writeln!(body, "<h3 class=\"rm-label\">{label}</h3>\n<ul class=\"rm-list {class}\">");
                for i in items.iter() {
                    let _ = writeln!(body, "<li>{}</li>", inline(i));
                }
                body.push_str("</ul>\n");
            }
        }
        if !m.done.is_empty() {
            let _ = writeln!(body, "<details class=\"rm-done\"><summary>Done ({})</summary>\n<ul class=\"rm-list done\">", m.done.len());
            for i in &m.done {
                let _ = writeln!(body, "<li>{}</li>", inline(i));
            }
            body.push_str("</ul>\n</details>\n");
        }
        body.push_str("</section>\n");
    }
    layout(p, pages, repo, &body, &[], "prose wide")
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

fn bar(done: usize, total: usize) -> String {
    let pct = (done * 100).checked_div(total).unwrap_or(0);
    format!("<span class=\"bar\" role=\"img\" aria-label=\"{done} of {total} done\"><span style=\"--p: {pct}%\"></span></span>")
}

fn layout(p: &Page, pages: &[Page], repo: &str, body: &str, toc: &[Heading], class: &str) -> String {
    let base = up(&p.out);
    let mut nav = String::new();
    for section in SECTIONS {
        let _ = writeln!(nav, "<p class=\"doc-nav-title\">{section}</p>\n<ul>");
        for q in pages.iter().filter(|q| q.section == section) {
            let current = if q.out == p.out { " aria-current=\"page\"" } else { "" };
            let _ = writeln!(nav, "<li><a href=\"{base}{}\"{current}>{}</a></li>", q.out, esc(&q.title));
        }
        nav.push_str("</ul>\n");
    }
    let mut aside = String::new();
    if toc.len() > 1 {
        aside.push_str("<aside class=\"doc-toc\" aria-label=\"On this page\">\n<p>On this page</p>\n<ul>\n");
        for h in toc {
            let _ = writeln!(aside, "<li class=\"toc-h{}\"><a href=\"#{}\">{}</a></li>", h.level, h.id, esc(&h.text));
        }
        aside.push_str("</ul>\n</aside>\n");
    }
    let siblings: Vec<&Page> = pages.iter().filter(|q| q.section == p.section).collect();
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
    let on = " aria-current=\"true\"";
    let (docs, project) = if p.section == "Project" { ("", on) } else { (on, "") };
    format!(
        r##"<!doctype html>
<html lang="en" data-repo="{repo}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} | MapCraft</title>
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="#121316" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#f6f5f2" media="(prefers-color-scheme: light)">
<link rel="icon" href="{base}assets/mapcraft.svg" type="image/svg+xml">
<link rel="stylesheet" href="{base}style.css">
</head>
<body>

<header class="top">
	<div class="wrap top-inner">
		<a class="brand" href="{base}index.html"><img src="{base}assets/mapcraft.svg" width="28" height="28" alt="">MapCraft</a>
		<nav aria-label="Site">
			<a class="opt" href="{base}index.html#features">Features</a>
			<a href="{base}guide/index.html"{docs}>Docs</a>
			<a class="opt" href="{base}roadmap.html"{project}>Roadmap</a>
			<a href="{repo}">GitHub</a>
		</nav>
	</div>
</header>

<div class="wrap doc-layout">
<details class="doc-nav" id="doc-nav">
<summary>Docs menu</summary>
{nav}</details>
<main class="doc-main">
<article class="{class}">
{body}</article>
{pager}<p class="doc-source"><a href="{repo}/blob/main/{src}">View the source of this page on GitHub</a></p>
</main>
{aside}</div>

<footer class="foot">
	<div class="wrap foot-inner">
		<p>MapCraft is free software under the <a href="{repo}/blob/main/LICENSE">MIT license</a>.</p>
		<p>Docs are built from the Markdown in the repository.</p>
	</div>
</footer>

<script>
	if (matchMedia("(min-width: 901px)").matches) document.getElementById("doc-nav").open = true;
</script>
</body>
</html>
"##,
        title = esc(&p.title),
        src = p.src,
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

    #[test]
    fn helpers() {
        assert_eq!(normalize("docs/../wit/plugin.wit").as_deref(), Some("wit/plugin.wit"));
        assert_eq!(normalize("/ROADMAP.md").as_deref(), Some("ROADMAP.md"));
        assert_eq!(normalize("docs/../../x"), None);
        assert_eq!(up("guide/views.html"), "../");
        assert_eq!(up("roadmap.html"), "");
        assert_eq!(slug("The `.mcmap` format"), "the-mcmap-format");
        assert!(keys("Ctrl+Shift+M").is_some());
        assert!(keys("Alt+Right Click, Alt+Shift+Right Click").is_some());
        assert!(keys("F9").is_some());
        assert!(keys("+map").is_none());
        assert!(keys("sdk.toml").is_none());
        assert!(keys("C++").is_none());
        let t = splice("a\n<!-- shortcuts:start -->\nold\n<!-- shortcuts:end -->\nb", "new\n").unwrap_or_default();
        assert!(t.contains("new") && !t.contains("old") && t.ends_with("b"));
    }

    #[test]
    fn links_resolve() {
        let pages = vec![page("docs/plugin-api.md".into(), "reference/plugin-api.html".into(), "Reference", "# Plugin API".into())];
        let repo = "https://github.com/o/r";
        assert_eq!(link("../plugin-api.md#imports", "docs/guide/x.md", "guide/x.html", &pages, repo), "../reference/plugin-api.html#imports");
        assert_eq!(
            link("../wit/plugin.wit", "docs/plugin-api.md", "reference/plugin-api.html", &pages, repo),
            "https://github.com/o/r/blob/main/wit/plugin.wit"
        );
        assert_eq!(link("https://x.y/z", "ROADMAP.md", "roadmap.html", &pages, repo), "https://x.y/z");
    }
}
