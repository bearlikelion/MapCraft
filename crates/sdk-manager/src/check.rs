use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mapcraft_engine::{Severity, validate};
use mapcraft_fgd::GameData;
use mapcraft_plugin_host::Host;
use mapcraft_sdk::{Sdk, Step, Vfs, builtin_vars, expand};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Pass,
    Warn,
    Fail,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Pass => "pass",
            Level::Warn => "warn",
            Level::Fail => "fail",
        }
    }
}

/// One finding: what was checked and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub level: Level,
    pub what: String,
    pub message: String,
}

fn item(level: Level, what: impl Into<String>, message: impl Into<String>) -> Item {
    Item { level, what: what.into(), message: message.into() }
}

/// The most serious level among the items.
pub fn worst(items: &[Item]) -> Level {
    items.iter().map(|i| i.level).max().unwrap_or(Level::Pass)
}

/// The SDK's sample maps, `samples/*.mcmap`, sorted.
pub fn samples(dir: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(dir.join("samples")).into_iter().flatten().flatten();
    let mut out: Vec<PathBuf> = entries.map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x == mapcraft_format::EXTENSION)).collect();
    out.sort();
    out
}

fn on_path(program: &str) -> bool {
    let p = Path::new(program);
    if p.is_absolute() || p.components().count() > 1 {
        return p.is_file();
    }
    let exe = format!("{program}.exe");
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file() || (cfg!(windows) && d.join(&exe).is_file())))
}

/// Checks the SDK in `dir` with the user's path settings: manifest, FGDs, plugins (loaded with
/// `host` when given), paths, texture folders, compile programs and sample maps. Unset paths
/// warn; anything set but broken fails.
pub fn check(dir: &Path, vars: &BTreeMap<String, String>, host: Option<&Host>) -> Vec<Item> {
    use Level::{Fail, Pass, Warn};
    let mut out = Vec::new();
    let sdk = match Sdk::load(dir) {
        Ok(sdk) => sdk,
        Err(e) => return vec![item(Fail, "manifest", e.to_string())],
    };
    let m = &sdk.manifest;
    out.push(item(Pass, "manifest", format!("{} {}", m.name, m.version).trim_end().to_string()));
    let mut vars: BTreeMap<String, String> = vars.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, setting) in &m.paths {
        if let (None, Some(d)) = (vars.get(k), &setting.default) {
            vars.insert(k.clone(), d.clone());
        }
    }

    if m.fgd.is_empty() {
        out.push(item(Warn, "fgd", "no entity definitions are listed"));
    }
    let mut game = GameData::default();
    for raw in &m.fgd {
        let what = format!("fgd {raw}");
        let path = match sdk.resolve(raw, &vars) {
            Ok(p) => p,
            Err(e) => {
                out.push(item(Warn, what, format!("{e}; set it in SDK Settings")));
                continue;
            }
        };
        if !path.is_file() {
            out.push(item(Fail, what, format!("{} does not exist", path.display())));
            continue;
        }
        let gd = GameData::load(&path, &mut |p| std::fs::read(p));
        if gd.diagnostics.is_empty() {
            out.push(item(Pass, what.clone(), format!("{} classes", gd.class_names().count())));
        }
        for (file, d) in gd.diagnostics.iter().take(20) {
            out.push(item(Fail, what.clone(), format!("{}:{d}", file.display())));
        }
        if gd.diagnostics.len() > 20 {
            out.push(item(Fail, what, format!("and {} more problems", gd.diagnostics.len() - 20)));
        }
        game.extend(gd);
    }

    for id in &m.plugins {
        let what = format!("plugin {id}");
        let path = sdk.dir.join("plugins").join(format!("{id}.wasm"));
        if !path.is_file() {
            out.push(item(Fail, what, format!("plugins/{id}.wasm is missing")));
            continue;
        }
        let Some(host) = host else {
            out.push(item(Warn, what, "found, but the plugin host could not start to load it"));
            continue;
        };
        out.push(match host.load_file(&path, Vfs::default()) {
            Ok(p) if p.manifest.id != *id => item(Fail, what, format!("the file holds plugin `{}`", p.manifest.id)),
            Ok(p) => {
                let pm = &p.manifest;
                let lists = if pm.materials { ", lists materials" } else { "" };
                item(Pass, what, format!("{} {}: {} commands, {} importers{lists}", pm.name, pm.version, pm.commands.len(), pm.importers.len()))
            }
            Err(e) => item(Fail, what, format!("does not load: {e}")),
        });
    }

    for (key, setting) in &m.paths {
        let what = format!("path {key}");
        out.push(match vars.get(key) {
            None => item(Warn, what, format!("not set: {}", setting.prompt)),
            Some(v) if if setting.kind == "file" { Path::new(v).is_file() } else { Path::new(v).is_dir() } => item(Pass, what, v.clone()),
            Some(v) => item(Fail, what, format!("{v} does not exist")),
        });
    }

    if m.textures.roots.is_empty() {
        out.push(item(Warn, "textures", "no texture folders are listed"));
    }
    for raw in &m.textures.roots {
        let what = format!("textures {raw}");
        out.push(match sdk.resolve(raw, &vars) {
            Err(e) => item(Warn, what, format!("{e}; set it in SDK Settings")),
            Ok(p) if p.is_dir() => {
                let count = Vfs::new(vec![p.clone()]).list(&m.textures.extensions).len();
                item(if count == 0 { Warn } else { Pass }, what, format!("{count} materials in {}", p.display()))
            }
            Ok(p) => item(Fail, what, format!("{} does not exist", p.display())),
        });
    }

    let mut cvars = vars.clone();
    cvars.extend(builtin_vars(&sdk.dir.join("samples/check.mcmap"), &sdk.dir));
    let mut seen = BTreeSet::new();
    for profile in &m.compile {
        for step in &profile.steps {
            let Step::Run { run, wine, .. } = step else { continue };
            if !seen.insert((run.clone(), *wine)) {
                continue;
            }
            let what = format!("compile {}", profile.name);
            out.push(match expand(run, &cvars) {
                Err(e) => item(Warn, what, format!("{e}; set it in SDK Settings")),
                Ok(_) if *wine && !cfg!(windows) && !on_path("wine") => item(Fail, what, "wine is not installed"),
                Ok(program) if on_path(&program) => item(Pass, what, program),
                Ok(program) => item(Fail, what, format!("{program} not found")),
            });
        }
    }

    let maps = samples(&sdk.dir);
    if maps.is_empty() {
        out.push(item(Warn, "samples", "no sample maps in samples/; add one so Test can open it"));
    }
    for path in maps {
        let what = format!("sample {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let file = match mapcraft_format::load(&path) {
            Ok(f) => f,
            Err(e) => {
                out.push(item(Fail, what, e.to_string()));
                continue;
            }
        };
        if file.sdk.as_deref() != Some(m.id.as_str()) {
            out.push(item(Warn, what.clone(), format!("made with SDK `{}`", file.sdk.as_deref().unwrap_or("none"))));
        }
        let issues = validate(&file.map, (!m.fgd.is_empty()).then_some(&game));
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        out.push(match (errors.first(), issues.first()) {
            (Some(e), _) => item(Fail, what, format!("{} errors, first: {}", errors.len(), e.message)),
            (None, Some(w)) => item(Warn, what, format!("{} warnings, first: {}", issues.len(), w.message)),
            (None, None) => item(Pass, what, format!("{} nodes, no issues", file.map.len())),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels<'a>(items: &'a [Item], prefix: &str) -> Vec<(Level, &'a str)> {
        items.iter().filter(|i| i.what.starts_with(prefix)).map(|i| (i.level, i.message.as_str())).collect()
    }

    #[test]
    fn surfsup_passes() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks/surfsup");
        let host = Host::new().ok();
        let items = check(&dir, &BTreeMap::new(), host.as_ref());
        assert!(matches!(levels(&items, "manifest").as_slice(), [(Level::Pass, m)] if m.starts_with("SurfsUp ")), "{items:#?}");
        assert!(levels(&items, "fgd").iter().all(|(l, _)| *l == Level::Pass), "{items:#?}");
        assert!(levels(&items, "path project").iter().all(|(l, _)| *l == Level::Warn), "unset paths only warn");
        let sample = levels(&items, "sample surf_mapcraft");
        assert!(sample.iter().all(|(l, _)| *l != Level::Fail), "{sample:?}");
        assert_eq!(levels(&items, "plugin ").len(), 3, "plugins pass or fail depending on `cargo xtask plugins`, but are always checked");
        assert_ne!(worst(&items), Level::Pass, "SurfsUp asks for paths the test does not set");
    }

    #[test]
    fn broken_sdk_fails() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("fgd")).unwrap();
        std::fs::create_dir_all(p.join("plugins")).unwrap();
        std::fs::create_dir_all(p.join("samples")).unwrap();
        std::fs::write(
            p.join("sdk.toml"),
            r#"
id = "broken"
name = "Broken"
plugins = ["ghost", "junk"]
fgd = ["fgd/bad.fgd", "fgd/missing.fgd"]
[textures]
roots = ["textures"]
[paths.game]
prompt = "Game"
[[compile]]
name = "Build"
steps = [{ run = "/no/such/compiler" }, { run = "${game}/tool" }]
"#,
        )
        .unwrap();
        std::fs::write(p.join("fgd/bad.fgd"), "@PointClass = [ oops").unwrap();
        std::fs::write(p.join("plugins/junk.wasm"), b"not wasm").unwrap();
        std::fs::write(p.join("samples/bad.mcmap"), "{ not a map").unwrap();
        let host = Host::new().ok();
        let vars = BTreeMap::from([("game".to_string(), "/no/such/game".to_string())]);
        let items = check(p, &vars, host.as_ref());
        let fails: Vec<&str> = items.iter().filter(|i| i.level == Level::Fail).map(|i| i.what.as_str()).collect();
        for what in
            ["fgd fgd/bad.fgd", "fgd fgd/missing.fgd", "plugin ghost", "plugin junk", "path game", "textures textures", "compile Build", "sample bad.mcmap"]
        {
            assert!(fails.contains(&what), "{what} should fail: {items:#?}");
        }
        assert_eq!(worst(&items), Level::Fail);

        std::fs::write(p.join("sdk.toml"), "id = \"Bad Id\"\nname = \"x\"").unwrap();
        let items = check(p, &BTreeMap::new(), None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].level, Level::Fail);
    }

    #[test]
    fn template_has_no_failures() {
        let dir = tempfile::tempdir().unwrap();
        let sdk = dir.path().join("new");
        crate::create(&sdk, "new-game", "New Game").unwrap();
        let items = check(&sdk, &BTreeMap::new(), None);
        assert!(items.iter().all(|i| i.level != Level::Fail), "{items:#?}");
    }
}
