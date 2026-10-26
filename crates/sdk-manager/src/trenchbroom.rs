//! Builds an SDK from a TrenchBroom game config as func_godot exports it, plus the func_godot map
//! settings of the Godot project behind it.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use mapcraft_fgd::GameData;
use mapcraft_sdk::{Sdk, Vfs, valid_id};
use serde_json::Value;

use crate::{Error, io_err};

const CONFIG: &str = "GameConfig.cfg";
const PROJECT: &str = "project.godot";
const SETTINGS_CLASS: &str = "script_class=\"FuncGodotMapSettings\"";
const ADDON_SETTINGS: &str = "addons/func_godot/func_godot_default_map_settings.tres";
const MAX_CONFIG: u64 = 1 << 20;
const MAX_FGD: u64 = 16 << 20;
const MAX_MAP: u64 = 16 << 20;
/// Bytes of `.map` files read in all while counting textures.
const MAX_MAPS_READ: u64 = 64 << 20;
const MAX_TEXTURE_NAMES: usize = 10_000;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 16;
/// func_godot's default inverse scale factor: map units per Godot meter.
const FUNC_GODOT_SCALE: f64 = 32.0;
/// func_godot's default texture file extensions.
const FUNC_GODOT_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "bmp", "tga", "webp", "wal"];
/// Image types MapCraft can show.
const IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "tga", "bmp"];
const PLUGINS: [&str; 2] = ["godot", "quake"];

/// A TrenchBroom game read for import, ready to be written as an SDK.
#[derive(Clone, Debug)]
pub struct TrenchBroomImport {
    pub name: String,
    /// The folder holding `GameConfig.cfg`.
    pub game: PathBuf,
    pub project: Option<PathBuf>,
    /// Entity definitions, `/`-separated and relative to `game`.
    fgds: Vec<String>,
    units_per_meter: f64,
    /// Texture folders as manifest paths, such as `${project}/textures`.
    textures: Vec<String>,
    extensions: Vec<String>,
    scale: f64,
    default_material: Option<String>,
    /// TrenchBroom's game path as a manifest path; FGD model paths are relative to it.
    models: String,
    /// Where the project keeps its `.map` files, relative to the project.
    maps: String,
    /// What the import assumed or could not carry over.
    pub notes: Vec<String>,
}

/// What a `GameConfig.cfg` holds that an SDK needs.
#[derive(Debug, Default, PartialEq)]
struct Config {
    name: String,
    root: Option<String>,
    extensions: Vec<String>,
    excludes: Vec<String>,
    definitions: Vec<String>,
    scale: Option<f64>,
    /// Material patterns of the brush face tags, such as `clip`.
    tags: Vec<String>,
}

impl Config {
    fn parse(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(&strip_comments(text)).map_err(|e| format!("not a TrenchBroom game config: {e}"))?;
        let name = v.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if name.is_empty() || name.len() > 128 || name.contains(char::is_control) {
            return Err("the game config has no usable \"name\"".into());
        }
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
        };
        // Version 9 says "materials"; older versions say "textures", and before 8 nest the root and extensions.
        let tex = v.get("materials").or_else(|| v.get("textures"));
        let root = tex.and_then(|t| t.get("root").or_else(|| t.pointer("/package/root"))).and_then(Value::as_str).map(String::from);
        let extensions = strings(tex.and_then(|t| t.get("extensions").or_else(|| t.pointer("/format/extensions"))));
        let tags =
            v.pointer("/tags/brushface").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.get("pattern")?.as_str()).map(String::from).collect());
        Ok(Self {
            name: name.to_string(),
            root,
            extensions,
            excludes: strings(tex.and_then(|t| t.get("excludes"))),
            definitions: strings(v.pointer("/entities/definitions")),
            scale: v.pointer("/faceattribs/defaults/scale/0").and_then(Value::as_f64),
            tags: tags.unwrap_or_default(),
        })
    }
}

/// `text` without `//` and `/* */` comments outside strings.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => {
                in_string = c == '"';
                out.push(c);
            }
        }
    }
    out
}

/// Reads at most `max` bytes, refusing larger files.
fn read_capped(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max {
        return Err(std::io::Error::other(format!("it is larger than {} KiB", max >> 10)));
    }
    let mut out = Vec::new();
    file.take(max).read_to_end(&mut out)?;
    Ok(out)
}

fn first_line(path: &Path) -> Option<String> {
    let mut line = String::new();
    BufReader::new(std::fs::File::open(path).ok()?.take(512)).read_line(&mut line).ok()?;
    Some(line)
}

/// Every double-quoted string in a Godot resource value, unescaped.
fn godot_strings(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = raw.chars();
    while chars.by_ref().any(|c| c == '"') {
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => s.extend(chars.next()),
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

/// The `key = value` lines of a Godot resource's `[resource]` section.
fn resource_values(text: &str) -> BTreeMap<&str, &str> {
    let mut out = BTreeMap::new();
    let mut in_resource = false;
    for line in text.lines() {
        if line.starts_with('[') {
            in_resource = line.starts_with("[resource]");
        } else if in_resource && let Some((key, value)) = line.split_once(" = ") {
            out.insert(key.trim(), value.trim());
        }
    }
    out
}

/// A relative path that stays inside its folder.
fn is_inside(rel: &Path) -> bool {
    !rel.as_os_str().is_empty() && rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn slashed(rel: &Path) -> String {
    rel.components().filter(|c| matches!(c, Component::Normal(_))).map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// A folder from a config as a manifest path under `base`, or as given when absolute.
fn under(base: &str, folder: &str) -> Option<String> {
    let folder = folder.trim().replace('\\', "/");
    if folder.contains("${") || folder.contains(char::is_control) {
        return None;
    }
    if Path::new(&folder).is_absolute() {
        return Some(folder);
    }
    let rel = folder.trim_start_matches("./").trim_end_matches('/');
    Some(if rel.is_empty() || rel == "." { base.to_string() } else { format!("{base}/{rel}") })
}

/// A `res://` folder as a manifest path.
fn res_folder(res: &str) -> Option<String> {
    under("${project}", res.strip_prefix("res://")?)
}

/// A manifest path as a folder on disk, given the project.
fn on_disk(root: &str, project: &Path) -> PathBuf {
    match root.strip_prefix("${project}") {
        Some(rest) => project.join(rest.trim_start_matches('/')),
        None => PathBuf::from(root),
    }
}

/// `*` wildcard matching, ignoring case.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.to_lowercase().chars().collect(), text.to_lowercase().chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if p.get(pi) == Some(&'*') {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if p.get(pi).is_some_and(|c| Some(c) == t.get(ti)) {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p.get(pi..).is_some_and(|rest| rest.iter().all(|c| *c == '*'))
}

/// Godot's per-project data folders (`app_userdata`) on Linux, macOS and Windows.
fn godot_user_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = env("HOME");
    let linux = env("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share"))).map(|d| d.join("godot"));
    let macos = home.map(|h| h.join("Library/Application Support/Godot"));
    [linux, macos, env("APPDATA").map(|d| d.join("Godot"))].into_iter().flatten().map(|d| d.join("app_userdata")).collect()
}

/// The project's name from `project.godot`.
fn project_name(project: &Path) -> Option<String> {
    let text = read_capped(&project.join(PROJECT), MAX_CONFIG).ok()?;
    let text = String::from_utf8_lossy(&text);
    let line = text.lines().find_map(|l| l.trim().strip_prefix("config/name="))?;
    godot_strings(line).into_iter().next().filter(|n| !n.trim().is_empty())
}

/// The TrenchBroom game folder func_godot last exported this project's game config to, from the
/// settings func_godot keeps in the project's user data folder.
fn exported_config(project: &Path, user_dirs: &[PathBuf]) -> Option<PathBuf> {
    let name = project_name(project)?;
    // Godot replaces characters a folder name cannot hold.
    let folder: String = name.chars().map(|c| if "\\/:*?\"<>|%".contains(c) || c.is_control() { '_' } else { c }).collect();
    if folder == "." || folder == ".." {
        return None;
    }
    let files = ["func_godot_config.json".to_string(), format!("{}_FuncGodotConfig.json", name.replace(' ', "_"))];
    let settings = user_dirs.iter().map(|d| d.join(&folder)).flat_map(|d| files.iter().map(move |f| d.join(f)));
    settings
        .filter_map(|p| serde_json::from_slice::<Value>(&read_capped(&p, MAX_CONFIG).ok()?).ok())
        .filter_map(|v| v.get("TRENCHBROOM_GAME_CONFIG_FOLDER")?.as_str().filter(|s| !s.is_empty()).map(PathBuf::from))
        .find(|d| d.join(CONFIG).is_file())
}

/// The game path TrenchBroom's preferences, two folders above the game folder, hold for `name`.
fn trenchbroom_game_path(game: &Path, name: &str) -> Option<PathBuf> {
    let prefs = game.parent()?.parent()?.join("Preferences.json");
    let v: Value = serde_json::from_slice(&read_capped(&prefs, MAX_CONFIG).ok()?).ok()?;
    let path = PathBuf::from(v.get(format!("Games/{name}/Path"))?.as_str()?);
    path.is_dir().then_some(path)
}

/// What a walk over the Godot project found.
#[derive(Default)]
struct Scan {
    /// FuncGodotMapSettings resources outside `addons`, sorted.
    settings: Vec<PathBuf>,
    /// Folders holding `.map` files, relative to the project, and how many each holds.
    map_dirs: BTreeMap<String, usize>,
    /// How many brush faces of those maps use each texture.
    textures: BTreeMap<String, usize>,
}

fn scan(project: &Path) -> Scan {
    let mut out = Scan::default();
    let (mut seen, mut read) = (0usize, 0u64);
    let mut stack = vec![(project.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return out;
            }
            let Ok(ty) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let path = entry.path();
            if ty.is_dir() {
                // TrenchBroom keeps autosaves beside the maps.
                if depth < MAX_DEPTH && !name.starts_with('.') && name != "addons" && name != "autosave" {
                    stack.push((path, depth + 1));
                }
            } else if !ty.is_file() {
                continue;
            } else if name.ends_with(".tres") && first_line(&path).is_some_and(|l| l.contains(SETTINGS_CLASS)) {
                out.settings.push(path);
            } else if name.ends_with(".map") {
                let rel = dir.strip_prefix(project).map(slashed).unwrap_or_default();
                *out.map_dirs.entry(rel).or_default() += 1;
                if read < MAX_MAPS_READ
                    && let Ok(bytes) = read_capped(&path, MAX_MAP)
                {
                    read += bytes.len() as u64;
                    count_textures(&String::from_utf8_lossy(&bytes), &mut out.textures);
                }
            }
        }
    }
    out.settings.sort();
    out
}

/// Counts the texture of every brush face line, the name after a face's three points.
fn count_textures(map: &str, counts: &mut BTreeMap<String, usize>) {
    for line in map.lines().map(str::trim_start).filter(|l| l.starts_with('(')) {
        let Some(name) = line.splitn(4, ')').nth(3).and_then(|rest| rest.split_whitespace().next()) else { continue };
        if let Some(n) = counts.get_mut(name) {
            *n += 1;
        } else if counts.len() < MAX_TEXTURE_NAMES {
            counts.insert(name.to_string(), 1);
        }
    }
}

/// TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl TrenchBroomImport {
    /// Reads a TrenchBroom game folder (one holding `GameConfig.cfg`) or a Godot project whose
    /// func_godot game config was exported, and finds the other through TrenchBroom's preferences
    /// or func_godot's settings.
    pub fn read(folder: &Path) -> Result<Self, Error> {
        Self::read_with(folder, &godot_user_dirs())
    }

    fn read_with(folder: &Path, godot_dirs: &[PathBuf]) -> Result<Self, Error> {
        let (game, project) = if folder.join(CONFIG).is_file() {
            (folder.to_path_buf(), None)
        } else if folder.join(PROJECT).is_file() {
            let game = exported_config(folder, godot_dirs).ok_or_else(|| {
                Error::Invalid(format!(
                    "MapCraft cannot find the TrenchBroom game config func_godot exported for {}; export it from the project's TrenchBroomGameConfig resource, or pick its folder in TrenchBroom's games folder",
                    folder.display()
                ))
            })?;
            (game, Some(folder.to_path_buf()))
        } else {
            return Err(Error::Invalid(format!("{} holds neither a TrenchBroom {CONFIG} nor a Godot {PROJECT}", folder.display())));
        };
        let path = game.join(CONFIG);
        let text = read_capped(&path, MAX_CONFIG).map_err(io_err(&path))?;
        let cfg = Config::parse(&String::from_utf8_lossy(&text)).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        let mut notes = Vec::new();

        let game_path = trenchbroom_game_path(&game, &cfg.name);
        let project = project.or_else(|| game_path.as_deref()?.ancestors().take(8).find(|a| a.join(PROJECT).is_file()).map(Path::to_path_buf));
        // func_godot's own setup makes TrenchBroom's game path the project folder.
        let base = match (game_path.as_deref(), project.as_deref()) {
            (Some(g), Some(p)) => g.strip_prefix(p).ok().map(slashed).filter(|r| !r.is_empty() && !r.contains("${")).map(|r| format!("${{project}}/{r}")),
            _ => None,
        }
        .unwrap_or_else(|| "${project}".to_string());
        if project.is_none() {
            notes.push(format!(
                "The Godot project of {} was not found, so its func_godot map settings were not read; set its folder in SDK Settings, and check units_per_meter and [textures] in sdk.toml.",
                cfg.name
            ));
        }

        let scan = project.as_deref().map(scan).unwrap_or_default();
        let settings = scan.settings.first().cloned().or_else(|| project.as_deref().map(|p| p.join(ADDON_SETTINGS)).filter(|p| p.is_file()));
        if let [first, others @ ..] = scan.settings.as_slice()
            && !others.is_empty()
        {
            let others: Vec<String> = others.iter().map(|p| p.display().to_string()).collect();
            notes.push(format!(
                "Used the func_godot map settings in {}; if your maps use {}, edit units_per_meter and [textures] in sdk.toml to match.",
                first.display(),
                others.join(", ")
            ));
        }
        let text = settings.as_deref().and_then(|p| read_capped(p, MAX_CONFIG).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let values = resource_values(&text);
        if project.is_some() && settings.is_none() {
            notes.push("The project has no FuncGodotMapSettings resource, so func_godot's defaults apply.".into());
        }

        let units_per_meter = match values.get("inverse_scale_factor") {
            None => FUNC_GODOT_SCALE,
            Some(raw) => match raw.parse::<f64>() {
                Ok(v) if v.is_finite() && v > 0.0 => v,
                _ => {
                    notes.push(format!("The inverse scale factor {raw} is not a positive number, so func_godot's {FUNC_GODOT_SCALE} units per meter apply."));
                    FUNC_GODOT_SCALE
                }
            },
        };

        let trenchbroom_root = cfg.root.as_deref().and_then(|r| under(&base, r));
        let func_godot_root = match values.get("base_texture_dir").and_then(|v| godot_strings(v).into_iter().next()) {
            None if settings.is_some() => Some("${project}/textures".to_string()),
            None => None,
            Some(dir) => {
                let root = res_folder(&dir);
                if root.is_none() {
                    notes.push(format!("The base texture folder {dir} is not a res:// folder, so TrenchBroom's material folder is used."));
                }
                root
            }
        };
        if let (Some(f), Some(t)) = (&func_godot_root, &trenchbroom_root)
            && f != t
        {
            notes.push(format!(
                "TrenchBroom reads materials from {t} but func_godot from {f}; MapCraft uses func_godot's folder, so texture names match what your FuncGodotMap builds."
            ));
        }
        let mut textures: Vec<String> = func_godot_root.or(trenchbroom_root).into_iter().collect();
        if textures.is_empty() {
            textures.push(format!("{base}/textures"));
            notes.push("The game config names no material folder, so MapCraft looks in textures.".into());
        }
        let materials = values.get("base_material_dir").and_then(|v| godot_strings(v).into_iter().next()).and_then(|d| res_folder(&d));
        if let (Some(m), Some(p)) = (materials, project.as_deref())
            && !textures.contains(&m)
            && on_disk(&m, p).is_dir()
        {
            textures.push(m);
        }

        let wanted = match values.get("texture_file_extensions") {
            Some(v) => godot_strings(v),
            None if settings.is_some() => FUNC_GODOT_EXTENSIONS.map(String::from).to_vec(),
            None => cfg.extensions.clone(),
        };
        let mut extensions: Vec<String> = Vec::new();
        for e in wanted.iter().map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()) {
            if IMAGES.contains(&e.as_str()) && !extensions.contains(&e) {
                extensions.push(e);
            }
        }
        if extensions.is_empty() {
            extensions = IMAGES.map(String::from).to_vec();
        }

        let scale = cfg.scale.filter(|s| s.is_finite() && *s != 0.0).unwrap_or(1.0);

        // A tool texture like clip makes a poor default, and TrenchBroom writes __TB_empty for faces without one.
        let usable = |name: &str| {
            let last = name.rsplit('/').next().unwrap_or(name);
            !name.starts_with("__") && !cfg.tags.iter().chain(&cfg.excludes).any(|p| glob(p, last) || glob(p, name))
        };
        let most_used = scan.textures.iter().filter(|(n, _)| usable(n)).max_by_key(|(n, count)| (**count, std::cmp::Reverse(*n))).map(|(n, _)| n.clone());
        let first_image = || {
            let roots = textures.iter().filter_map(|r| Some(on_disk(r, project.as_deref()?))).collect();
            Vfs::new(roots).list(&extensions).into_iter().map(|(n, _)| n).find(|n| usable(n))
        };
        let default_material = most_used.or_else(first_image);

        let maps = scan.map_dirs.iter().filter(|(d, _)| !d.contains("${")).max_by_key(|(d, n)| (**n, std::cmp::Reverse(*d))).map(|(d, _)| d.clone());
        if project.is_some() && maps.is_none() {
            notes.push("The project has no .map files yet, so maps export to its maps folder.".into());
        }

        let mut fgds = Vec::new();
        for def in &cfg.definitions {
            let rel = def.trim().replace('\\', "/");
            if !rel.to_ascii_lowercase().ends_with(".fgd") {
                notes.push(format!("Skipped the entity definitions {def}: MapCraft reads FGD files only."));
            } else if !is_inside(Path::new(&rel)) || rel.contains("${") {
                notes.push(format!("Skipped the entity definitions {def}: they are outside the game folder."));
            } else if !fgds.contains(&rel) {
                fgds.push(rel);
            }
        }

        Ok(Self {
            name: cfg.name,
            game,
            project,
            fgds,
            units_per_meter,
            textures,
            extensions,
            scale,
            default_material,
            models: base,
            maps: maps.unwrap_or_else(|| "maps".into()),
            notes,
        })
    }

    /// The SDK id the game's name gives, such as `igc-collabjam-26` for `IGC CollabJam 26`.
    pub fn id(&self) -> String {
        let mut id = String::new();
        for c in self.name.chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() || c == '_' {
                id.push(c);
            } else if !id.is_empty() && !id.ends_with('-') {
                id.push('-');
            }
        }
        let id: String = id.chars().take(64).collect();
        let id = id.trim_end_matches('-');
        if valid_id(id) { id.to_string() } else { "trenchbroom-game".into() }
    }

    /// Writes the SDK into `dir`, which must not exist yet: the manifest, the FGDs and the files
    /// they include, a README, and the `godot` and `quake` plugins from the first of the `sdks`
    /// folders that holds them. Returns it with every note.
    pub fn write(&self, dir: &Path, sdks: &[PathBuf]) -> Result<(Sdk, Vec<String>), Error> {
        if dir.exists() {
            return Err(Error::Invalid(format!("{} already exists; remove that SDK first to import it again", dir.display())));
        }
        let mut notes = self.notes.clone();
        match self.write_files(dir, sdks, &mut notes).and_then(|()| Ok(Sdk::load(dir)?)) {
            Ok(sdk) => Ok((sdk, notes)),
            Err(e) => {
                let _ = std::fs::remove_dir_all(dir);
                Err(e)
            }
        }
    }

    fn write_files(&self, dir: &Path, sdks: &[PathBuf], notes: &mut Vec<String>) -> Result<(), Error> {
        let mut files: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
        let mut fgd = Vec::new();
        for rel in &self.fgds {
            let gd = GameData::load(&self.game.join(rel), &mut |p| {
                let inside = p.strip_prefix(&self.game).ok().filter(|r| is_inside(r)).ok_or_else(|| std::io::Error::other("it is outside the game folder"))?;
                let bytes = read_capped(p, MAX_FGD)?;
                files.insert(inside.to_path_buf(), bytes.clone());
                Ok(bytes)
            });
            if !files.contains_key(Path::new(rel)) {
                let why = gd.diagnostics.first().map_or(String::new(), |(_, d)| d.message.clone());
                notes.push(format!("Skipped the entity definitions {rel}: {why}"));
                continue;
            }
            if let Some((file, d)) = gd.diagnostics.first() {
                let n = gd.diagnostics.len();
                notes.push(format!("{rel} has {n} {}, the first in {}:{d}", if n == 1 { "problem" } else { "problems" }, file.display()));
            }
            fgd.push(format!("fgd/{rel}"));
        }
        if fgd.is_empty() {
            notes.push("The game config lists no FGD MapCraft could read, so the SDK has no entities.".into());
        }
        for (rel, bytes) in &files {
            let path = dir.join("fgd").join(rel);
            path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, bytes)).map_err(io_err(&path))?;
        }

        let plugins = dir.join("plugins");
        std::fs::create_dir_all(&plugins).map_err(io_err(&plugins))?;
        for id in PLUGINS {
            let file = format!("{id}.wasm");
            match sdks.iter().map(|d| d.join("plugins").join(&file)).find(|p| p.is_file()) {
                Some(from) => {
                    std::fs::copy(&from, plugins.join(&file)).map_err(io_err(&from))?;
                }
                None => notes.push(format!("No installed SDK has the {id} plugin; copy {file} into {} before using the SDK.", plugins.display())),
            }
        }

        let manifest = dir.join(mapcraft_sdk::MANIFEST);
        std::fs::write(&manifest, self.manifest(&fgd)).map_err(io_err(&manifest))?;
        let readme = dir.join("README.md");
        std::fs::write(&readme, self.readme(&fgd)).map_err(io_err(&readme))
    }

    fn manifest(&self, fgd: &[String]) -> String {
        let list = |items: &[String]| items.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ");
        let default = self.default_material.as_deref().map(|d| format!("default = {}\n", quote(d))).unwrap_or_default();
        let project = self.project.as_deref().and_then(Path::to_str).map(|p| format!("default = {}\n", quote(p))).unwrap_or_default();
        let to = if self.maps.is_empty() { "${project}/${map_name}.map".to_string() } else { format!("${{project}}/{}/${{map_name}}.map", self.maps) };
        format!(
            r#"# Imported by MapCraft from a TrenchBroom game config that func_godot exported; see README.md.
id = "{id}"
name = {name}
version = "1"
description = {description}
plugins = [{plugins}]
# func_godot's inverse scale factor: map units per Godot meter.
units_per_meter = {units:?}
fgd = [{fgd}]

[textures]
roots = [{roots}]
extensions = [{extensions}]
{default}scale = {scale:?}

# FGD model paths are relative to TrenchBroom's game path.
[models]
roots = [{models}]

[paths.project]
prompt = "Godot project folder (the one holding project.godot)"
{project}
# A FuncGodotMap node in the project builds the exported .map.
[[compile]]
name = "Export .map to Godot project"
steps = [
    {{ export = "map", to = {to} }},
]
"#,
            id = self.id(),
            name = quote(&self.name),
            description = quote(&format!("{} maps for func_godot, imported from its TrenchBroom game config.", self.name)),
            plugins = list(&PLUGINS.map(String::from)),
            units = self.units_per_meter,
            fgd = list(fgd),
            roots = list(&self.textures),
            extensions = list(&self.extensions),
            scale = self.scale,
            models = quote(&self.models),
            to = quote(&to),
        )
    }

    fn readme(&self, fgd: &[String]) -> String {
        let fgd = if fgd.is_empty() { "none".to_string() } else { fgd.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ") };
        let maps = if self.maps.is_empty() { "the project folder".to_string() } else { format!("`{}` in the project", self.maps) };
        format!(
            "# {name} SDK for MapCraft\n\n\
             MapCraft made this SDK from the TrenchBroom game config that func_godot exported for {name}, in `{game}`.\n\n\
             - Entities come from {fgd}, copied from that folder.\n\
             - Materials come from `{roots}`, named by their path in that folder as func_godot names them.\n\
             - One Godot meter is {units} map units, func_godot's inverse scale factor.\n\
             - **Run > Export .map to Godot project** writes the map as a Valve220 `.map` into {maps} for your FuncGodotMap node to build, keeping a different `.map` already there as `.map.bak`.\n\
             - **File > Import > Quake map** brings in maps made in TrenchBroom.\n\n\
             After changing entities in Godot, export the game config again, remove this SDK in the SDK Manager and import it again.\n",
            name = self.name,
            game = self.game.display(),
            roots = self.textures.join("`, `"),
            units = self.units_per_meter,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A game config as func_godot's TrenchBroomGameConfig writes it, with comments added.
    const CONFIG_V9: &str = r#"{
	"version": 9,
	"name": "Test Game",
	"icon": "icon.png",
	"fileformats": [ { "format": "Valve", "initialmap": "initial_valve.map" } ],
	"filesystem": { "searchpath": ".", "packageformat": { "extension": ".zip", "format": "zip" } },
	// TrenchBroom configs may hold comments
	"materials": {
		"root": "Assets/Textures",
		"extensions": [".bmp", ".jpg", ".png", ".webp", ".D", ".C"],
		"excludes": [ "*_normal", "*_orm" ],
		"palette": "", /* even here */
		"attribute": "wad"
	},
	"entities": { "definitions": [ "game.fgd", "extra.def", "../escape.fgd", "/etc/abs.fgd" ], "defaultcolor": "0.6 0.6 0.6 1.0", "scale": 32 },
	"tags": { "brush": [], "brushface": [ { "name": "Clip", "attribs": [ "transparent" ], "match": "material", "pattern": "clip" } ] },
	"faceattribs": { "defaults": { "scale": [0.5, 0.5] }, "contentflags": [], "surfaceflags": [] }
}"#;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn parses_game_configs() {
        let cfg = Config::parse(CONFIG_V9).unwrap();
        assert_eq!(cfg.name, "Test Game");
        assert_eq!(cfg.root.as_deref(), Some("Assets/Textures"));
        assert_eq!(cfg.extensions.len(), 6);
        assert_eq!(cfg.excludes, ["*_normal", "*_orm"]);
        assert_eq!(cfg.definitions[0], "game.fgd");
        assert_eq!(cfg.scale, Some(0.5));
        assert_eq!(cfg.tags, ["clip"]);

        let v4 =
            r#"{ "version": 4, "name": "Old", "textures": { "package": { "type": "directory", "root": "textures" }, "format": { "extensions": ["png"] } } }"#;
        let old = Config::parse(v4).unwrap();
        assert_eq!((old.root.as_deref(), old.extensions.as_slice()), (Some("textures"), &["png".to_string()][..]));
        assert_eq!(strip_comments(r#"{ "a": "x // y /* z */" } // gone"#), r#"{ "a": "x // y /* z */" } "#);

        let deep = "[".repeat(100_000);
        let damaged = ["", "{", "[]", "{ \"name\": 5 }", "{ \"name\": \"  \" }", "{ \"name\": \"a\nb\" }", "/* open", "\"name\"", deep.as_str()];
        for text in damaged {
            assert!(Config::parse(text).is_err(), "{text:.40}");
        }
        let odd = Config::parse(
            r#"{ "name": "Odd", "materials": 3, "entities": { "definitions": [1, null, "a.fgd"] }, "faceattribs": { "defaults": { "scale": "x" } } }"#,
        )
        .unwrap();
        assert_eq!((odd.root, odd.definitions, odd.scale), (None, vec!["a.fgd".to_string()], None));
    }

    #[test]
    fn small_parsers() {
        assert_eq!(godot_strings(r#"Array[String](["png", "j\"pg"])"#), ["png", "j\"pg"]);
        assert_eq!(godot_strings("\"unclosed"), ["unclosed"]);
        let values = resource_values("[gd_resource]\n\n[ext_resource path=\"x\"]\n[resource]\nscript = ExtResource(\"1\")\ninverse_scale_factor = 16.0\n");
        assert_eq!(values.get("inverse_scale_factor"), Some(&"16.0"));
        assert!(glob("*_normal", "Wall_NORMAL") && glob("clip", "clip") && glob("*", "") && !glob("*_orm", "storm") && glob("a*b*c", "aXbYbc"));
        assert_eq!(res_folder("res://Assets/Textures/").as_deref(), Some("${project}/Assets/Textures"));
        assert_eq!(res_folder("res://").as_deref(), Some("${project}"));
        assert_eq!(res_folder("res://${evil}"), None);
        assert_eq!(res_folder("uid://abc"), None);
        assert_eq!(quote("a\"b\\c\n"), r#""a\"b\\c\u000a""#);
        let mut counts = BTreeMap::new();
        count_textures("( 0 0 0 ) ( 1 0 0 ) ( 0 1 0 ) base/wall [ 1 0 0 0 ] [ 0 1 0 0 ] 0 1 1\n( 0 0 0 ) (\n// ( a ) b\n", &mut counts);
        assert_eq!(counts, BTreeMap::from([("base/wall".to_string(), 1)]));
    }

    #[test]
    fn ids_from_names() {
        let named = |name: &str| TrenchBroomImport {
            name: name.into(),
            game: PathBuf::new(),
            project: None,
            fgds: Vec::new(),
            units_per_meter: 32.0,
            textures: Vec::new(),
            extensions: Vec::new(),
            scale: 1.0,
            default_material: None,
            models: String::new(),
            maps: String::new(),
            notes: Vec::new(),
        };
        assert_eq!(named("IGC CollabJam 26").id(), "igc-collabjam-26");
        assert_eq!(named("  mFPS!! ").id(), "mfps");
        assert_eq!(named("ÜÜÜ").id(), "trenchbroom-game");
        assert_eq!(named(&"a".repeat(100)).id().len(), 64);
    }

    /// A TrenchBroom folder with a game exported by func_godot, its preferences pointing at a
    /// Godot project with map settings, textures and a TrenchBroom map.
    fn fixture(root: &Path) -> (PathBuf, PathBuf) {
        let game = root.join("TrenchBroom/games/Test Game");
        let project = root.join("Godot/test_game");
        write(&game.join(CONFIG), CONFIG_V9);
        write(&game.join("game.fgd"), "@include \"base.fgd\"\n@PointClass base(Origin) = info_player_start : \"Spawn\" []\n@SolidClass = func_detail []");
        write(&game.join("base.fgd"), "@BaseClass = Origin [ origin(origin) : \"Origin\" ]");
        let prefs = serde_json::json!({ "Games/Test Game/Path": project.to_string_lossy() });
        write(&root.join("TrenchBroom/Preferences.json"), &prefs.to_string());
        write(&project.join(PROJECT), "config_version=5\n[application]\nconfig/name=\"Test \\\"Game\\\"\"\n");
        write(
            &project.join("fgd/settings.tres"),
            "[gd_resource type=\"Resource\" script_class=\"FuncGodotMapSettings\" format=3]\n\n[resource]\ninverse_scale_factor = 16.0\nbase_texture_dir = \"res://Assets/Textures\"\ntexture_file_extensions = Array[String]([\"png\", \"wal\"])\n",
        );
        write(&project.join("Assets/Textures/base/wall.png"), "png");
        write(&project.join("Assets/Textures/base/wall_normal.png"), "png");
        let face = |tex: &str| format!("( 0 0 0 ) ( 1 0 0 ) ( 0 1 0 ) {tex} [ 1 0 0 0 ] [ 0 -1 0 0 ] 0 1 1\n");
        write(&project.join("Levels/One/a.map"), &format!("{{\n{{\n{}{}{}}}\n}}\n", face("clip"), face("clip"), face("base/floor")));
        write(&project.join("Levels/One/autosave/a.1.map"), &face("base/old").repeat(9));
        (game, project)
    }

    #[test]
    fn imports_a_trenchbroom_game_and_its_project() {
        let root = tempfile::tempdir().unwrap();
        let (game, project) = fixture(root.path());
        let tb = TrenchBroomImport::read_with(&game, &[]).unwrap();
        assert_eq!(tb.project.as_deref(), Some(project.as_path()), "found through TrenchBroom's preferences");
        assert_eq!(tb.fgds, ["game.fgd"]);
        assert!(
            tb.notes.iter().any(|n| n.contains("extra.def")) && tb.notes.iter().filter(|n| n.contains("outside the game folder")).count() == 2,
            "{:#?}",
            tb.notes
        );

        let plugins = root.path().join("bundled/godot-sdk/plugins");
        write(&plugins.join("godot.wasm"), "not really wasm");
        let dir = root.path().join("sdks").join(tb.id());
        let (sdk, notes) = tb.write(&dir, &[root.path().join("bundled/missing"), root.path().join("bundled/godot-sdk")]).unwrap();
        assert!(notes.iter().any(|n| n.contains("quake plugin")), "{notes:#?}");
        assert!(dir.join("plugins/godot.wasm").is_file() && dir.join("README.md").is_file());
        let copied: Vec<_> = std::fs::read_dir(dir.join("fgd")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(copied.len(), 2, "the FGD and the file it includes");

        let m = &sdk.manifest;
        assert_eq!((m.id.as_str(), m.name.as_str()), ("test-game", "Test Game"));
        assert_eq!(m.units_per_meter, 16.0);
        assert_eq!(m.plugins, ["godot", "quake"]);
        assert_eq!(m.fgd, ["fgd/game.fgd"]);
        assert_eq!(m.textures.roots, ["${project}/Assets/Textures"]);
        assert_eq!(m.textures.extensions, ["png"]);
        assert_eq!(m.textures.scale, 0.5);
        assert_eq!(m.textures.default.as_deref(), Some("base/floor"), "the most used texture that is not a tool texture");
        assert_eq!(m.models.roots, ["${project}"]);
        assert_eq!(m.paths["project"].default.as_deref(), project.to_str());
        let step = &m.compile[0].steps[0];
        assert_eq!(step, &mapcraft_sdk::Step::Export { export: "map".into(), to: "${project}/Levels/One/${map_name}.map".into() });

        let vars = BTreeMap::from([("project".to_string(), project.to_string_lossy().into_owned())]);
        let gd = sdk.game_data(&vars).unwrap();
        assert!(gd.class("info_player_start").is_some() && gd.class("func_detail").is_some() && gd.diagnostics.is_empty());
        assert_eq!(sdk.vfs(&vars).unwrap().find_material("base/wall", &m.textures.extensions), Some(project.join("Assets/Textures/base/wall.png")));
        let items = crate::check(&dir, &vars, None);
        assert!(items.iter().any(|i| i.what == "plugin quake" && i.level == crate::Level::Fail), "the missing plugin shows in the check");
        assert!(items.iter().filter(|i| i.what != "plugin quake").all(|i| i.level != crate::Level::Fail), "{items:#?}");
        assert!(items.iter().any(|i| i.what == "textures ${project}/Assets/Textures" && i.level == crate::Level::Pass), "{items:#?}");

        assert!(tb.write(&dir, &[]).is_err(), "an existing SDK is never overwritten");
    }

    #[test]
    fn imports_from_the_godot_project() {
        let root = tempfile::tempdir().unwrap();
        let (game, project) = fixture(root.path());
        std::fs::remove_dir_all(project.join("fgd")).unwrap();
        std::fs::remove_dir_all(project.join("Levels")).unwrap();
        assert!(TrenchBroomImport::read_with(&project, &[]).is_err(), "no func_godot settings to find the game config through");

        let user = root.path().join("app_userdata");
        write(&user.join("Test _Game_/func_godot_config.json"), &serde_json::json!({ "TRENCHBROOM_GAME_CONFIG_FOLDER": game.to_string_lossy() }).to_string());
        let tb = TrenchBroomImport::read_with(&project, std::slice::from_ref(&user)).unwrap();
        assert_eq!((tb.game.as_path(), tb.project.as_deref()), (game.as_path(), Some(project.as_path())));
        assert_eq!(tb.units_per_meter, FUNC_GODOT_SCALE);
        assert_eq!(tb.textures, ["${project}/Assets/Textures"], "TrenchBroom's material folder without map settings");
        assert_eq!(tb.default_material.as_deref(), Some("base/wall"), "the first texture, skipping excluded maps");
        assert_eq!(tb.maps, "maps");
        assert!(tb.notes.iter().any(|n| n.contains("no FuncGodotMapSettings")), "{:#?}", tb.notes);
    }

    #[test]
    fn refuses_damaged_input() {
        let root = tempfile::tempdir().unwrap();
        assert!(TrenchBroomImport::read_with(root.path(), &[]).is_err(), "neither kind of folder");
        let game = root.path().join("game");
        for bad in ["not json", "{ \"version\": 9 }", &"x".repeat((MAX_CONFIG + 1) as usize)] {
            write(&game.join(CONFIG), bad);
            assert!(TrenchBroomImport::read_with(&game, &[]).is_err());
        }

        write(
            &game.join(CONFIG),
            r#"{ "name": "Broken", "materials": { "root": "${evil}" }, "entities": { "definitions": ["bad.fgd", "gone.fgd", "inc.fgd"] } }"#,
        );
        write(&game.join("bad.fgd"), "@PointClass = [[[ \"");
        write(&game.join("inc.fgd"), "@include \"../outside.fgd\"\n@PointClass = inside []");
        write(&root.path().join("outside.fgd"), "@PointClass = outside []");
        let tb = TrenchBroomImport::read_with(&game, &[]).unwrap();
        assert_eq!(tb.textures, ["${project}/textures"]);
        assert!(tb.notes.iter().any(|n| n.contains("project")), "{:#?}", tb.notes);
        let dir = root.path().join("out");
        let (sdk, notes) = tb.write(&dir, &[]).unwrap();
        assert_eq!(sdk.manifest.fgd, ["fgd/bad.fgd", "fgd/inc.fgd"], "a damaged FGD is kept with its problems noted; an unreadable one is left out");
        assert!(notes.iter().any(|n| n.starts_with("bad.fgd has")) && notes.iter().any(|n| n.contains("gone.fgd")), "{notes:#?}");
        assert!(notes.iter().any(|n| n.starts_with("inc.fgd has 1 problem,")) && !dir.join("outside.fgd").exists(), "includes never leave the game folder");

        let project = root.path().join("project");
        write(&project.join(PROJECT), "config/name=\"P\"\n");
        write(&project.join("s.tres"), "[gd_resource script_class=\"FuncGodotMapSettings\"]\n[resource]\ninverse_scale_factor = -0.0\n");
        write(&project.join("t.tres"), "[gd_resource script_class=\"FuncGodotMapSettings\"]\n[resource]\ninverse_scale_factor = NaN\n");
        let prefs = serde_json::json!({ "Games/Broken/Path": project.to_string_lossy() });
        write(&root.path().join("Preferences.json"), &prefs.to_string());
        let tb = TrenchBroomImport::read_with(&game, &[]).unwrap();
        assert!(tb.project.is_none(), "preferences are looked up two folders above the game folder");
        let moved = root.path().join("tb/games/game");
        write(&moved.join(CONFIG), r#"{ "name": "Broken" }"#);
        write(&root.path().join("tb/Preferences.json"), &prefs.to_string());
        let tb = TrenchBroomImport::read_with(&moved, &[]).unwrap();
        assert_eq!(tb.units_per_meter, FUNC_GODOT_SCALE, "{:#?}", tb.notes);
        assert!(tb.notes.iter().any(|n| n.contains("not a positive number")) && tb.notes.iter().any(|n| n.contains("t.tres")));
    }
}
