//! Renders the whole editor offscreen to a PNG after a list of steps, no window needed:
//! `cargo run -p mapcraft-ui --example snapshot -- out.png [map.mcmap] [steps]`.
//!
//! Steps run in order: `--command <id>`, `--click <label>`, `--click-at x,y`, `--alt-click-at x,y`,
//! `--drag x,y x,y`, `--hold x,y x,y` (`alt-`, `shift-` and `ctrl-` before `click-at`, `drag` or `hold` hold those keys, as in `--alt-shift-click-at`), `--hover x,y`, `--scroll <lines>`, `--key <name>` (or a chord such as `Ctrl+Shift+D`), `--type <text>`, `--select-all`, `--wait <ms>`,
//! `--look x,y,z x,y,z` (points the 3D camera from the first point at the second); `--size WxH` sets the window size
//! (1600x1000 by default) and `--scale <n>` its pixels per point; `--var name=value` sets an SDK path
//! and `--registry <url|path>` replaces the SDK Manager's registries; `--settings <dir>` keeps the
//! settings and data folder there instead of a throwaway one; `--fresh` finds SDKs only in that data
//! folder, as a fresh install would, so the editor opens with the Add an SDK window.

use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_ui::{MapCraftApp, Options};

fn pos(s: &str) -> Pos2 {
    let (x, y) = s.split_once(',').unwrap_or(("0", "0"));
    Pos2::new(x.trim().parse().unwrap_or(0.0), y.trim().parse().unwrap_or(0.0))
}

fn modifiers(step: &str) -> Modifiers {
    [("alt-", Modifiers::ALT), ("shift-", Modifiers::SHIFT), ("ctrl-", Modifiers::COMMAND)]
        .iter()
        .filter(|(k, _)| step.contains(k))
        .fold(Modifiers::NONE, |m, (_, x)| m | *x)
}

fn press(h: &Harness<MapCraftApp>, p: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args.first() else {
        eprintln!("usage: snapshot <out.png> [map.mcmap] [steps]");
        std::process::exit(2);
    };
    let settings = std::env::temp_dir().join(format!("mapcraft-snapshot-{}", std::process::id()));
    let mut options = Options { settings_dir: Some(settings.clone()), ..Options::default() };
    let mut keep = false;
    let mut steps: Vec<(String, Vec<String>)> = Vec::new();
    let mut registries = Vec::new();
    let mut size = egui::Vec2::new(1600.0, 1000.0);
    let mut scale = 1.0;
    let mut it = args.iter().skip(1).peekable();
    while let Some(a) = it.next() {
        let take =
            |n: usize, it: &mut std::iter::Peekable<std::iter::Skip<std::slice::Iter<String>>>| (0..n).filter_map(|_| it.next().cloned()).collect::<Vec<_>>();
        match a.as_str() {
            "--var" => {
                if let Some((k, v)) = it.next().and_then(|kv| kv.split_once('=')) {
                    options.sdk_vars.push((k.to_string(), v.to_string()));
                }
            }
            "--registry" => registries.extend(it.next().cloned()),
            "--fresh" => options.fresh_install = true,
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x')) {
                    size = egui::Vec2::new(w.parse().unwrap_or(size.x), h.parse().unwrap_or(size.y));
                }
            }
            "--settings" => keep = it.next().map(PathBuf::from).inspect(|d| options.settings_dir = Some(d.clone())).is_some(),
            "--select-all" => steps.push(("--command".into(), vec!["edit.select_all".into()])),
            "--command" | "--click" | "--key" | "--type" | "--wait" | "--hover" | "--scroll" | "--drop" => steps.push((a.clone(), take(1, &mut it))),
            s if s.ends_with("click-at") => steps.push((a.clone(), take(1, &mut it))),
            s if s == "--look" || s.ends_with("drag") || s.ends_with("hold") => steps.push((a.clone(), take(2, &mut it))),
            path => options.open = Some(PathBuf::from(path)),
        }
    }
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(scale).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    if !registries.is_empty() {
        h.state_mut().set_registries_for_test(registries);
    }
    h.run_steps(4);
    for (step, args) in &steps {
        let arg = args.first().map(String::as_str).unwrap_or_default();
        match step.as_str() {
            "--click" => h.get_by_label(arg).click(),
            "--wait" => {
                for _ in 0..10 {
                    std::thread::sleep(std::time::Duration::from_millis(arg.parse::<u64>().unwrap_or(500) / 10));
                    h.run_steps(1);
                }
            }
            "--command" => h.state_mut().run_command_for_test(arg),
            "--drop" => h.state_mut().drop_file_for_test(PathBuf::from(arg)),
            "--look" => {
                let point = |s: &str| {
                    let v: Vec<f64> = s.split(',').map(|n| n.trim().parse().unwrap_or(0.0)).collect();
                    mapcraft_geom::DVec3::new(v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0))
                };
                h.state_mut().look_for_test(0, point(arg), point(args.get(1).map(String::as_str).unwrap_or_default()));
            }
            "--hover" => h.hover_at(pos(arg)),
            "--scroll" => h.event(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, arg.parse().unwrap_or(1.0)),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }),
            "--key" => {
                let (mods, name) = arg.rsplit_once('+').unwrap_or(("", arg));
                let m = mods.split('+').fold(Modifiers::NONE, |m, part| match part {
                    "Ctrl" => m | Modifiers::COMMAND,
                    "Shift" => m | Modifiers::SHIFT,
                    "Alt" => m | Modifiers::ALT,
                    _ => m,
                });
                if let Some(k) = egui::Key::from_name(name) {
                    h.key_press_modifiers(m, k);
                }
            }
            "--type" => h.event(Event::Text(arg.into())),
            s if s.ends_with("click-at") && s != "--right-click-at" => {
                let m = modifiers(s);
                let p = pos(arg);
                h.event(Event::ModifiersChanged(m));
                h.hover_at(p);
                h.run_steps(1);
                press(&h, p, true, m);
                h.run_steps(1);
                press(&h, p, false, m);
                h.run_steps(1);
                h.event(Event::ModifiersChanged(Modifiers::NONE));
            }
            "--right-click-at" => {
                let p = pos(arg);
                h.hover_at(p);
                h.run_steps(1);
                for pressed in [true, false] {
                    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
                    h.run_steps(1);
                }
            }
            s if s.ends_with("drag") || s.ends_with("hold") => {
                let (a, b) = (pos(arg), pos(args.get(1).map(String::as_str).unwrap_or_default()));
                let m = modifiers(s);
                h.event(Event::ModifiersChanged(m));
                h.hover_at(a);
                h.run_steps(1);
                press(&h, a, true, m);
                h.run_steps(1);
                for i in 1..=8 {
                    h.hover_at(a + (b - a) * (i as f32 / 8.0));
                    h.run_steps(1);
                }
                if step.ends_with("drag") {
                    press(&h, b, false, m);
                    h.run_steps(1);
                    h.event(Event::ModifiersChanged(Modifiers::NONE));
                }
            }
            _ => {}
        }
        h.run_steps(2);
    }
    h.run_steps(3);
    match h.render() {
        Ok(img) => {
            img.save(out).unwrap();
            println!("wrote {out}");
        }
        Err(e) => eprintln!("render failed: {e}"),
    }
    if !keep {
        let _ = std::fs::remove_dir_all(settings);
    }
}
