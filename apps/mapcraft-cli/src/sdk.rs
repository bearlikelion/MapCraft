//! `mapcraft-cli sdk ...`: the SDK manager without the editor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use mapcraft_sdk_manager::{self as manager, Level};

pub const USAGE: &str = "  mapcraft-cli sdk list [--registry <url|path>] [--data <dir>]
  mapcraft-cli sdk check <dir|id> [--var name=value]... [--data <dir>]
  mapcraft-cli sdk install <id> [--registry <url|path>] [--data <dir>]
  mapcraft-cli sdk remove <id> [--data <dir>]
  mapcraft-cli sdk package <dir> [--out <dir>] [--url <base>]
  mapcraft-cli sdk new <dir> --id <id> --name <name>";

#[derive(Default)]
struct Args {
    positional: Vec<String>,
    registry: Option<String>,
    data: Option<PathBuf>,
    vars: BTreeMap<String, String>,
    out: Option<PathBuf>,
    url: Option<String>,
    id: Option<String>,
    name: Option<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--registry" => a.registry = Some(value()?),
            "--data" => a.data = Some(value()?.into()),
            "--out" => a.out = Some(value()?.into()),
            "--url" => a.url = Some(value()?),
            "--id" => a.id = Some(value()?),
            "--name" => a.name = Some(value()?),
            "--var" => {
                let kv = value()?;
                let (k, v) = kv.split_once('=').ok_or("--var takes name=value")?;
                a.vars.insert(k.to_string(), v.to_string());
            }
            _ => a.positional.push(arg.clone()),
        }
    }
    Ok(a)
}

fn first(a: &Args) -> Result<&str, String> {
    a.positional.first().map(String::as_str).ok_or_else(|| USAGE.to_string())
}

fn user_dir(data: Option<&Path>) -> Result<PathBuf, String> {
    data.map(manager::user_dir).ok_or_else(|| "no user data folder; pass --data <dir>".to_string())
}

/// Runs `sdk <command>`; false when a check failed.
pub fn run(args: &[String]) -> Result<bool, String> {
    let (cmd, rest) = args.split_first().ok_or_else(|| USAGE.to_string())?;
    let a = parse(rest)?;
    let data = a.data.clone().or_else(manager::data_dir);
    let data = data.as_deref();
    let err = |e: manager::Error| e.to_string();
    match cmd.as_str() {
        "list" => {
            let Some(registry) = &a.registry else {
                for i in manager::installed(&manager::search_dirs(data, &[])) {
                    match &i.sdk {
                        Ok(s) => println!("{:<16} {:<10} {:<12} {}", s.manifest.id, s.manifest.version, i.origin.label(), i.dir.display()),
                        Err(e) => println!("{:<16} {:<10} {:<12} {}: {e}", "(broken)", "", i.origin.label(), i.dir.display()),
                    }
                }
                return Ok(true);
            };
            let reg = manager::fetch_registry(registry).map_err(err)?;
            println!("{} ({} SDKs)", if reg.name.is_empty() { registry } else { &reg.name }, reg.sdks.len());
            for e in &reg.sdks {
                let note = e.unsupported().unwrap_or_default();
                let by = if e.author.is_empty() { String::new() } else { format!(" by {}", e.author) };
                println!("{:<16} {:<10} {:<10} {:>8} KiB  {}{by} {note}", e.id, e.version, e.license, e.size.div_ceil(1024), e.name);
            }
            Ok(true)
        }
        "check" => {
            let target = first(&a)?;
            let dir = if Path::new(target).join(mapcraft_sdk::MANIFEST).is_file() {
                PathBuf::from(target)
            } else {
                let found = manager::installed(&manager::search_dirs(data, &[]));
                let hit = found.into_iter().find(|i| i.sdk.as_ref().is_ok_and(|s| s.manifest.id == target));
                hit.map(|i| i.dir).ok_or_else(|| format!("{target}: not an SDK folder or the id of an installed SDK"))?
            };
            let host = mapcraft_plugin_host::Host::new().map_err(|e| eprintln!("plugins cannot be loaded: {e}")).ok();
            let items = manager::check(&dir, &a.vars, host.as_ref());
            for i in &items {
                println!("{:<5} {}: {}", i.level.label(), i.what, i.message);
            }
            let count = |l| items.iter().filter(|i| i.level == l).count();
            println!("{}: {} passed, {} warnings, {} failed", dir.display(), count(Level::Pass), count(Level::Warn), count(Level::Fail));
            Ok(manager::worst(&items) != Level::Fail)
        }
        "install" => {
            let id = first(&a)?;
            let registry = a.registry.clone().unwrap_or_else(|| manager::DEFAULT_REGISTRY.to_string());
            let reg = manager::fetch_registry(&registry).map_err(err)?;
            let entry = reg.sdks.iter().find(|e| e.id == id).ok_or_else(|| format!("{registry} has no SDK `{id}`"))?;
            let sdk = manager::install(entry, &registry, &user_dir(data)?, &mut |_, _| {}, &AtomicBool::new(false)).map_err(err)?;
            println!("installed {} {} into {}", sdk.manifest.name, sdk.manifest.version, sdk.dir.display());
            Ok(true)
        }
        "remove" => {
            let id = first(&a)?;
            manager::uninstall(&user_dir(data)?, id).map_err(err)?;
            println!("removed {id}");
            Ok(true)
        }
        "package" => {
            let out = a.out.clone().unwrap_or_else(|| PathBuf::from("."));
            let (zip, entry) = manager::package(Path::new(first(&a)?), &out, a.url.as_deref()).map_err(err)?;
            eprintln!("wrote {}", zip.display());
            println!("{}", serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?);
            Ok(true)
        }
        "new" => {
            let (id, name) = a.id.as_deref().zip(a.name.as_deref()).ok_or("sdk new needs --id and --name")?;
            let sdk = manager::create(Path::new(first(&a)?), id, name).map_err(err)?;
            println!("created {} in {}", sdk.manifest.name, sdk.dir.display());
            Ok(true)
        }
        _ => Err(USAGE.to_string()),
    }
}
