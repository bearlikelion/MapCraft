//! Renders the whole editor offscreen to a PNG after a list of steps, no window needed:
//! `cargo run -p mapcraft-ui --example snapshot -- out.png [map.mcmap] [steps]`.
//!
//! Steps run in order: `--command <id>`, `--click <label>`, `--click-at x,y`, `--alt-click-at x,y`,
//! `--drag x,y x,y`, `--key <name>`, `--select-all`, `--wait <ms>`; `--var name=value` sets an SDK path.

use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_ui::{MapCraftApp, Options};

fn pos(s: &str) -> Pos2 {
    let (x, y) = s.split_once(',').unwrap_or(("0", "0"));
    Pos2::new(x.trim().parse().unwrap_or(0.0), y.trim().parse().unwrap_or(0.0))
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
    let mut steps: Vec<(String, Vec<String>)> = Vec::new();
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
            "--select-all" => steps.push(("--command".into(), vec!["edit.select_all".into()])),
            "--command" | "--click" | "--click-at" | "--alt-click-at" | "--key" | "--wait" => steps.push((a.clone(), take(1, &mut it))),
            "--drag" => steps.push((a.clone(), take(2, &mut it))),
            path => options.open = Some(PathBuf::from(path)),
        }
    }
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
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
            "--key" => {
                if let Some(k) = egui::Key::from_name(arg) {
                    h.key_press(k);
                }
            }
            "--click-at" | "--alt-click-at" => {
                let m = if step == "--alt-click-at" { Modifiers::ALT } else { Modifiers::NONE };
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
            "--drag" => {
                let (a, b) = (pos(arg), pos(args.get(1).map(String::as_str).unwrap_or_default()));
                h.hover_at(a);
                h.run_steps(1);
                press(&h, a, true, Modifiers::NONE);
                h.run_steps(1);
                for i in 1..=8 {
                    h.hover_at(a + (b - a) * (i as f32 / 8.0));
                    h.run_steps(1);
                }
                press(&h, b, false, Modifiers::NONE);
                h.run_steps(1);
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
    let _ = std::fs::remove_dir_all(settings);
}
