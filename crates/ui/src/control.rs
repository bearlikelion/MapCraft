//! The control channel: `mapcraft-cli mcp` forwards agents' tool calls here so they edit the open map.

use std::fmt::Display;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use mapcraft_engine::control::{self, CONTROL_FILE, bad, check_discard, ids_arg, info, issues_json, num_or, str_arg, text_result, vec_arg};
use mapcraft_fgd::{ClassKind, Io};
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_map::{Generator, NodeId};
use mapcraft_plugin_host::{ParamKind, ParamSpec};
use mapcraft_render::{Camera, View};
use serde_json::{Value, json};

use crate::app::{MapCraftApp, frame_camera};

/// How long a call waits for the editor; the UI drops calls older than this unanswered.
const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_LINE: u64 = 16 << 20;

pub(crate) struct Request {
    call: Value,
    sent: Instant,
    reply: mpsc::Sender<Value>,
}

pub(crate) struct Channel {
    rx: mpsc::Receiver<Request>,
    file: PathBuf,
}

impl Channel {
    /// Removes the control file unless an editor started later has replaced it.
    pub(crate) fn stop(&self) {
        let pid = std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.get("pid")?.as_u64());
        if pid == Some(u64::from(std::process::id())) {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

/// Listens on a loopback port; each connection must first send the token written next to the port in [`CONTROL_FILE`].
pub(crate) fn start(data: &Path, ctx: &egui::Context) -> Option<Channel> {
    let (listener, token, file) = listen(data).map_err(|e| log::warn!("control channel: {e}")).ok()?;
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx, token) = (tx.clone(), ctx.clone(), token.clone());
            std::thread::spawn(move || serve(stream, &token, &tx, &ctx));
        }
    });
    Some(Channel { rx, file })
}

fn listen(data: &Path) -> std::io::Result<(TcpListener, String, PathBuf)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = token();
    let file = data.join(CONTROL_FILE);
    let _ = std::fs::remove_file(&file);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    open.open(&file)?.write_all(json!({"port": port, "token": token, "pid": std::process::id()}).to_string().as_bytes())?;
    Ok((listener, token, file))
}

/// 128 bits from the standard library's hasher keys, which the OS seeds randomly.
fn token() -> String {
    (0..2u64)
        .map(|i| {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(i);
            format!("{:016x}", h.finish())
        })
        .collect()
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One line without its newline, or `None` at the end of the stream, on a timeout or past `max` bytes.
fn read_line(r: &mut impl BufRead, max: u64) -> Option<String> {
    let mut line = String::new();
    r.by_ref().take(max).read_line(&mut line).ok()?;
    line.ends_with('\n').then(|| line.trim_end().to_string())
}

fn serve(stream: TcpStream, token: &str, tx: &mpsc::Sender<Request>, ctx: &egui::Context) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(mut out) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let hello = read_line(&mut reader, 256).and_then(|l| serde_json::from_str::<Value>(&l).ok());
    if !hello.as_ref().and_then(|h| h.get("token")?.as_str()).is_some_and(|t| same(t, token)) {
        return;
    }
    let _ = reader.get_ref().set_read_timeout(None);
    while let Some(line) = read_line(&mut reader, MAX_LINE) {
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(call) => ask(call, tx, ctx),
            Err(e) => text_result(Err(e)),
        };
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            return;
        }
    }
}

fn ask(call: Value, tx: &mpsc::Sender<Request>, ctx: &egui::Context) -> Value {
    let (reply, rx) = mpsc::channel();
    if tx.send(Request { call, sent: Instant::now(), reply }).is_err() {
        return text_result(Err("the editor is closing"));
    }
    ctx.request_repaint();
    // Waits past TIMEOUT so a call the UI has started still gets its answer.
    rx.recv_timeout(TIMEOUT + Duration::from_secs(30)).unwrap_or_else(|_| text_result(Err("the editor did not answer; is a dialog open in it?")))
}

fn msg(e: impl Display) -> String {
    e.to_string()
}

fn param_json(p: &ParamSpec) -> Value {
    let kind = match &p.kind {
        ParamKind::Number { min, max, .. } => json!({"type": "number", "min": min, "max": max}),
        ParamKind::Integer { min, max } => json!({"type": "integer", "min": min, "max": max}),
        ParamKind::Toggle => json!({"type": "boolean"}),
        ParamKind::Choice(c) => json!({"type": "choice", "choices": c}),
        ParamKind::Text => json!({"type": "text"}),
        ParamKind::Material => json!({"type": "material"}),
    };
    json!({"id": p.id, "label": p.label, "help": p.help, "default": p.initial, "kind": kind})
}

fn class_kind(k: &ClassKind) -> &'static str {
    if *k == ClassKind::Solid { "solid" } else { "point" }
}

impl MapCraftApp {
    /// Answers the calls agents sent since the last frame.
    pub(crate) fn poll_control(&mut self) {
        let calls: Vec<Request> = self.control.as_ref().map(|c| c.rx.try_iter().collect()).unwrap_or_default();
        for r in calls.into_iter().filter(|r| r.sent.elapsed() < TIMEOUT) {
            let name = r.call.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = r.call.get("arguments").unwrap_or(&Value::Null);
            let result =
                if name == "screenshot" { self.render_views(args).unwrap_or_else(|e| text_result(Err(e))) } else { text_result(self.tool(name, args)) };
            let _ = r.reply.send(result);
        }
    }

    /// Runs a tool against the open map: the editor's own version when it has one, else the engine's.
    fn tool(&mut self, name: &str, a: &Value) -> Result<Value, String> {
        match name {
            "new" => {
                let path = PathBuf::from(str_arg(a, "path").map_err(msg)?);
                if path.exists() {
                    return Err("that file already exists; use open".into());
                }
                check_discard(&self.session, a).map_err(msg)?;
                self.new_now(a.get("sdk").and_then(Value::as_str).map(String::from).or_else(|| self.session.sdk.clone()));
                self.session.path = Some(path);
                Ok(self.info())
            }
            "open" => {
                check_discard(&self.session, a).map_err(msg)?;
                self.open_file(Path::new(str_arg(a, "path").map_err(msg)?)).map_err(msg)?;
                Ok(self.info())
            }
            "save" => {
                let path = match a.get("path").and_then(Value::as_str) {
                    Some(p) => PathBuf::from(p),
                    None => self.session.path.clone().ok_or("the map has no path yet; pass one")?,
                };
                self.save_to(&path).map_err(msg)?;
                Ok(self.info())
            }
            "info" => Ok(self.info()),
            "validate" => {
                self.check_problems();
                Ok(issues_json(&self.issues))
            }
            "frame" => {
                let b = match a.get("ids") {
                    Some(_) => self.bounds_of(&ids_arg(&self.session, a, "ids").map_err(msg)?),
                    None if !self.session.selection.is_empty() => self.selection_bounds(),
                    None => self.map_bounds(),
                };
                self.frame_bounds(b.ok_or("nothing there has a size to frame")?);
                Ok(self.info())
            }
            "entity_classes" => {
                self.require_sdk()?;
                let game = &self.project.game;
                let filter = a.get("filter").and_then(Value::as_str).unwrap_or_default().to_lowercase();
                Ok(game
                    .class_names()
                    .filter_map(|n| game.class(n))
                    .filter(|c| c.name.to_lowercase().contains(&filter) || c.description.to_lowercase().contains(&filter))
                    .map(|c| json!({"name": c.name, "kind": class_kind(&c.kind), "description": c.description.chars().take(160).collect::<String>()}))
                    .collect())
            }
            "entity_class" => {
                self.require_sdk()?;
                let name = str_arg(a, "name").map_err(msg)?;
                let c = self.project.game.class(name).ok_or_else(|| format!("the SDK has no entity class `{name}`; see entity_classes"))?;
                let io = |list: &[Io]| list.iter().map(|i| json!({"name": i.name, "type": i.ty, "description": i.description})).collect::<Vec<_>>();
                let keys: Vec<Value> = c
                    .properties
                    .iter()
                    .map(|p| {
                        let choices: Vec<Value> = p.choices.iter().map(|ch| json!({"value": ch.value, "label": ch.label})).collect();
                        json!({"name": p.name, "type": p.ty, "display": p.display, "default": p.default, "description": p.description, "choices": choices})
                    })
                    .collect();
                Ok(json!({"name": c.name, "kind": class_kind(&c.kind), "description": c.description, "keys": keys, "inputs": io(&c.inputs), "outputs": io(&c.outputs)}))
            }
            "materials" => {
                self.require_sdk()?;
                let filter = a.get("filter").and_then(Value::as_str).unwrap_or_default().to_lowercase();
                let limit = a.get("limit").and_then(Value::as_u64).map_or(200, |n| usize::try_from(n).unwrap_or(usize::MAX));
                let found: Vec<&str> = self.project.materials.iter().map(|m| m.name.as_str()).filter(|n| n.to_lowercase().contains(&filter)).collect();
                let names: Vec<&str> = found.iter().take(limit).copied().collect();
                Ok(json!({"total": found.len(), "names": names, "still_loading": self.material_list.is_some()}))
            }
            "plugin_commands" => Ok(self
                .project
                .plugins
                .iter()
                .map(|p| {
                    let commands: Vec<Value> = p
                        .manifest
                        .commands
                        .iter()
                        .map(|c| json!({"command": c.id, "label": c.label, "generator": c.generator, "params": c.params.iter().map(param_json).collect::<Vec<_>>()}))
                        .collect();
                    json!({"plugin": p.manifest.id, "name": p.manifest.name, "commands": commands})
                })
                .collect()),
            "run_plugin" => self.run_plugin_tool(a),
            "compile_profiles" => {
                let sdk = self.project.sdk.as_ref().ok_or("the open map has no SDK loaded")?;
                Ok(sdk.manifest.compile.iter().map(|p| json!(p.name)).collect())
            }
            "compile" => self.compile_tool(a),
            "output" => {
                let n = a.get("lines").and_then(Value::as_u64).map_or(50, |n| usize::try_from(n).unwrap_or(usize::MAX));
                let lines: Vec<&String> = self.output.iter().skip(self.output.len().saturating_sub(n)).collect();
                Ok(json!({"running": self.compile.is_some(), "lines": lines}))
            }
            "batch" => control::batch(a, |n, a| self.tool(n, a)).map_err(msg),
            "screenshot" => Err("screenshot cannot run inside a batch".into()),
            _ => {
                let result = control::call(&mut self.session, name, a).map_err(msg);
                self.mark_changed();
                result
            }
        }
    }

    fn info(&self) -> Value {
        let mut v = info(&self.session);
        if let Some(o) = v.as_object_mut() {
            o.insert("editor".into(), json!(true));
            o.insert("sdk_loaded".into(), json!(self.project.sdk.is_some()));
        }
        v
    }

    fn require_sdk(&self) -> Result<(), String> {
        match self.project.sdk {
            Some(_) => Ok(()),
            None => Err("the open map has no SDK loaded, so it has no entity classes, materials or plugins".into()),
        }
    }

    fn bounds_of(&self, ids: &[NodeId]) -> Option<Aabb> {
        ids.iter().filter_map(|id| self.node_bounds(*id)).reduce(|a, b| a.union(&b))
    }

    fn run_plugin_tool(&mut self, a: &Value) -> Result<Value, String> {
        let (plugin, command) = (str_arg(a, "plugin").map_err(msg)?, str_arg(a, "command").map_err(msg)?);
        let spec = self
            .project
            .plugins
            .iter()
            .filter(|p| p.manifest.id == plugin)
            .flat_map(|p| &p.manifest.commands)
            .find(|c| c.id == command)
            .cloned()
            .ok_or_else(|| format!("plugin `{plugin}` has no command `{command}`; see plugin_commands"))?;
        let mut params = spec.defaults();
        if let (Some(p), Some(given)) = (params.as_object_mut(), a.get("params").and_then(Value::as_object)) {
            for (k, v) in given {
                if !p.contains_key(k) {
                    return Err(format!("`{command}` has no parameter `{k}`; see plugin_commands"));
                }
                p.insert(k.clone(), v.clone());
            }
        }
        let origin = if a.get("origin").is_some() { vec_arg(a, "origin").map_err(msg)? } else { DVec3::ZERO };
        let yaw = num_or(a, "yaw", 0.0).map_err(msg)?;
        let frags = self.run_plugin(plugin, command, &params)?;
        let made = if spec.generator {
            let g = Generator { plugin: plugin.into(), command: command.into(), params, origin, angles: DVec3::new(0.0, yaw, 0.0) };
            self.session.create_generator(&spec.label, g, frags).map(|_| ())
        } else {
            self.session.insert_output(&spec.label, frags, origin).map(|_| ())
        };
        self.mark_changed();
        made.map_err(msg)?;
        Ok(self.info())
    }

    fn compile_tool(&mut self, a: &Value) -> Result<Value, String> {
        if self.compile.is_some() {
            return Err("a compile is already running; follow it with output".into());
        }
        if self.session.path.is_none() {
            return Err("save the map to a path first".into());
        }
        let name = str_arg(a, "profile").map_err(msg)?;
        let index = self
            .project
            .sdk
            .as_ref()
            .and_then(|s| s.manifest.compile.iter().position(|p| p.name.eq_ignore_ascii_case(name)))
            .ok_or_else(|| format!("no compile profile `{name}`; see compile_profiles"))?;
        self.start_compile(index);
        if self.confirm_tools.is_some() {
            Ok(json!({"status": "waiting for the user to allow this SDK's tools in the editor; it starts once they do"}))
        } else if self.compile.is_some() {
            Ok(json!({"status": "started; follow it with output"}))
        } else {
            Err(self.status.clone())
        }
    }

    /// Renders the map offscreen into one PNG, as an MCP image result.
    fn render_views(&mut self, a: &Value) -> Result<Value, String> {
        let views: &[View] = match a.get("view").and_then(Value::as_str).unwrap_or("all") {
            "all" => &View::ALL,
            "3d" => &[View::Perspective],
            "top" => &[View::Top],
            "front" => &[View::Front],
            "side" => &[View::Side],
            _ => return Err(msg(bad("view", "all, 3d, top, front or side"))),
        };
        let size = |key: &str, default: u32| a.get(key).and_then(Value::as_u64).map_or(default, |n| u32::try_from(n.clamp(64, 2048)).unwrap_or(default));
        let (w, h) = (size("width", 1024), size("height", 768));
        let focus = match a.get("focus") {
            Some(_) => self.bounds_of(&ids_arg(&self.session, a, "focus").map_err(msg)?),
            None => self.map_bounds(),
        };
        let b = focus.unwrap_or(Aabb::new(DVec3::splat(-256.0), DVec3::splat(256.0)));
        let eye = if a.get("eye").is_some() { Some(vec_arg(a, "eye").map_err(msg)?) } else { None };
        let look_at = if a.get("look_at").is_some() { vec_arg(a, "look_at").map_err(msg)? } else { b.center() };
        if self.scene_dirty {
            self.rebuild_scene();
        }
        let gpu = self.gpu.as_ref().ok_or("the editor has no GPU renderer")?;
        let cols = if views.len() > 1 { 2 } else { 1 };
        let (cw, ch) = (w / cols, h / cols);
        let mut sheet = image::RgbaImage::new(cw * cols, ch * cols);
        for (i, view) in (0u32..).zip(views) {
            let mut cam = Camera::new(*view);
            frame_camera(&mut cam, &b, f64::from(cw.min(ch)));
            if let (View::Perspective, Some(eye)) = (view, eye) {
                let d = look_at - eye;
                cam.position = eye;
                cam.yaw = d.y.atan2(d.x).to_degrees();
                cam.pitch = d.z.atan2(d.truncate().length()).to_degrees();
            }
            let target = gpu.renderer.target([cw, ch]);
            gpu.renderer.render(&target, &cam, self.render_mode, Some(self.session.grid), &[]);
            let pixels = gpu.renderer.read_rgba(&target).ok_or("could not read the picture back from the GPU")?;
            let img = image::RgbaImage::from_raw(cw, ch, pixels).ok_or("the GPU returned a picture of the wrong size")?;
            image::imageops::replace(&mut sheet, &img, i64::from(i % cols * cw), i64::from(i / cols * ch));
        }
        let mut png = Vec::new();
        sheet.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(msg)?;
        let layout = views.iter().map(|v| v.label()).collect::<Vec<_>>().join(", ");
        let text = format!("{layout}, left to right then top to bottom; framed on {:?} to {:?}", b.min.to_array(), b.max.to_array());
        Ok(json!({"content": [
            {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png"},
            {"type": "text", "text": text},
        ]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_tokens() {
        let mut r = std::io::Cursor::new(b"{\"a\":1}\nlonger than eight\nno newline".to_vec());
        assert_eq!(read_line(&mut r, 64).as_deref(), Some("{\"a\":1}"));
        assert_eq!(read_line(&mut r, 8), None);
        let t = token();
        assert_eq!(t.len(), 32);
        assert_ne!(t, token());
        assert!(same(&t, &t.clone()) && !same(&t, "x"));
    }
}
