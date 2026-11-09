//! `mapcraft-cli mcp`: an MCP server on stdin and stdout that edits the open editor's map, or one held here.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use mapcraft_engine::{Session, control};
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_render::{NoStyle, RenderMode, Renderer, Shot, Slope, build_scene, headless_renderer, named_views, render_sheet};
use serde_json::{Value, json};

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Builds and edits MapCraft .mcmap level maps. \
While the MapCraft editor is open, tools act on the map open in it, live and undoable, and the editor-only tools work; \
otherwise they act on a map held in memory here. \
Units are inches (39.37 per meter) and Z is up. Most edits act on the selection: create or select nodes, then move, rotate, set_material or run_command. \
Check your work with screenshot and validate. Use entity_classes and materials to learn what the map's SDK offers, and plugin_commands for its generators. \
Changes stay in memory until save; a map held here that is unsaved when the client disconnects is written beside itself as <name>.unsaved.mcmap.";

pub fn run() -> Result<(), String> {
    let mut server = Server { session: Session::default(), editor: None, find_editor: true, renderer: None };
    let served = server.serve();
    let kept = keep_unsaved(&mut server.session);
    served.and(kept)
}

/// A connection to a running editor, found through the control file it writes in the data folder.
struct Editor {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Editor {
    fn connect() -> Option<Self> {
        Self::connect_to(&mapcraft_sdk_manager::data_dir()?.join(control::CONTROL_FILE))
    }

    fn connect_to(file: &std::path::Path) -> Option<Self> {
        let v: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
        let stream = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(1)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(180))).ok()?;
        let mut writer = stream.try_clone().ok()?;
        writeln!(writer, "{}", json!({"token": v.get("token")?})).ok()?;
        Some(Self { reader: BufReader::new(stream), writer })
    }

    /// Sends a tools/call's params and returns the editor's CallToolResult.
    fn call(&mut self, params: &Value) -> Result<Value, String> {
        writeln!(self.writer, "{params}").map_err(|e| e.to_string())?;
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err("the editor closed the connection".into()),
            Ok(_) => serde_json::from_str(&line).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    }
}

struct Server {
    session: Session,
    editor: Option<Editor>,
    find_editor: bool,
    renderer: Option<Renderer>,
}

impl Server {
    fn serve(&mut self) -> Result<(), String> {
        let mut out = std::io::stdout().lock();
        for line in std::io::stdin().lock().lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
            };
            if let Some(r) = reply {
                writeln!(out, "{r}").and_then(|()| out.flush()).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Answers one request; notifications, which carry no id, get no reply.
    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let params = msg.get("params").unwrap_or(&Value::Null);
        let result = match msg.get("method").and_then(Value::as_str).unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mapcraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": control::tools()}),
            "tools/call" => self.call(params),
            method => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method `{method}`")}})),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn call(&mut self, params: &Value) -> Value {
        if self.editor.is_none() && self.find_editor {
            self.editor = Editor::connect();
        }
        match self.editor.as_mut().map(|e| e.call(params)) {
            Some(Ok(result)) => result,
            Some(Err(e)) => {
                self.editor = None;
                control::text_result(Err(format!("lost the editor connection ({e}); the call may not have run, so check with info before retrying")))
            }
            None => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
                let args = params.get("arguments").unwrap_or(&Value::Null);
                if name == "screenshot" {
                    return self.screenshot(args).unwrap_or_else(|e| control::text_result(Err(e)));
                }
                control::text_result(control::call(&mut self.session, name, args))
            }
        }
    }

    /// Renders the map held here offscreen in flat colors, with walkable and surfable slopes tinted as the editor's slope view does.
    fn screenshot(&mut self, a: &Value) -> Result<Value, String> {
        let views = named_views(a.get("view").and_then(Value::as_str).unwrap_or("all")).ok_or("`view` must be all, 3d, top, front or side")?;
        let size = |key: &str, default: u32| a.get(key).and_then(Value::as_u64).map_or(default, |n| u32::try_from(n.clamp(64, 2048)).unwrap_or(default));
        let roots = match a.get("focus") {
            Some(_) => control::ids_arg(&self.session, a, "focus").map_err(|e| e.to_string())?,
            None => self.session.map.nodes().map(|(id, _)| id).collect(),
        };
        let b = self.session.bounds_of(roots).unwrap_or(Aabb::new(DVec3::splat(-256.0), DVec3::splat(256.0)));
        let eye = if a.get("eye").is_some() { Some(control::vec_arg(a, "eye").map_err(|e| e.to_string())?) } else { None };
        let look_at = if a.get("look_at").is_some() { control::vec_arg(a, "look_at").map_err(|e| e.to_string())? } else { b.center() };
        if self.renderer.is_none() {
            self.renderer = headless_renderer();
        }
        let renderer = self.renderer.as_mut().ok_or("no GPU adapter to render with")?;
        let (map, _) = mapcraft_map::mirror::expand(&self.session.map);
        let slope = Slope { walkable: 45.573, surfable: 72.0 };
        let scene = build_scene(&map, &Default::default(), &Default::default(), &|m| renderer.material_size(m), &NoStyle, true, Some(slope));
        renderer.set_scene(&scene);
        let shot = Shot { views, bounds: b, eye, look_at, size: [size("width", 1024), size("height", 768)], mode: RenderMode::Flat, grid: None };
        let sheet = render_sheet(renderer, &shot).ok_or("could not read the picture back from the GPU")?;
        let mut png = Vec::new();
        sheet.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let layout = views.iter().map(|v| v.label()).collect::<Vec<_>>().join(", ");
        let text = format!("{layout}, left to right then top to bottom; framed on {:?} to {:?}", b.min.to_array(), b.max.to_array());
        Ok(json!({"content": [
            {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png"},
            {"type": "text", "text": text},
        ]}))
    }
}

/// Writes unsaved edits beside the map instead of dropping them when the client goes away.
fn keep_unsaved(session: &mut Session) -> Result<(), String> {
    if !session.is_dirty() {
        return Ok(());
    }
    let path = session.path.as_ref().map_or_else(|| PathBuf::from("untitled.unsaved.mcmap"), |p| p.with_extension("unsaved.mcmap"));
    session.save_as(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("unsaved changes written to {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol() {
        let mut s = Server { session: Session::default(), editor: None, find_editor: false, renderer: None };
        let mut ask = |msg: Value| s.handle(&msg);
        let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        assert!(ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap()["result"]["tools"].as_array().unwrap().len() > 10);
        let call = |name: &str, args: Value| json!({"jsonrpc": "2.0", "id": "c", "method": "tools/call", "params": {"name": name, "arguments": args}});
        let ok = ask(call("create_brush", json!({"min": [0, 0, 0], "max": [64, 64, 64]}))).unwrap();
        assert!(ok["result"]["isError"].is_null() && ok["id"] == "c");
        assert_eq!(ask(call("select", json!({"ids": [99]}))).unwrap()["result"]["isError"], true);
        assert_eq!(ask(call("screenshot", json!({"view": "under"}))).unwrap()["result"]["isError"], true);
        assert_eq!(ask(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).unwrap()["error"]["code"], -32601);
    }

    #[test]
    fn screenshots_render_without_the_editor() {
        let mut s = Server { session: Session::default(), editor: None, find_editor: false, renderer: headless_renderer() };
        if s.renderer.is_none() {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        s.call(&json!({"name": "create_brush", "arguments": {"min": [0, 0, 0], "max": [256, 256, 64]}}));
        let shot = s.call(&json!({"name": "screenshot", "arguments": {"view": "top", "width": 128, "height": 128}}));
        assert_eq!(shot["content"][0]["mimeType"], "image/png", "{shot}");
    }

    #[test]
    fn calls_go_to_a_running_editor() {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let file = dir.path().join(control::CONTROL_FILE);
        std::fs::write(&file, json!({"port": port, "token": "t"}).to_string()).unwrap();
        let editor = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut lines = BufReader::new(stream).lines();
            assert_eq!(lines.next().unwrap().unwrap(), r#"{"token":"t"}"#);
            let call: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            writeln!(out, "{}", json!({"content": [{"type": "text", "text": call["name"]}]})).unwrap();
        });
        let mut s = Server { session: Session::default(), editor: Editor::connect_to(&file), find_editor: false, renderer: None };
        let r = s.call(&json!({"name": "screenshot", "arguments": {}}));
        assert_eq!(r["content"][0]["text"], "screenshot");
        editor.join().unwrap();
        let lost = s.call(&json!({"name": "info"}));
        assert_eq!(lost["isError"], true);
        assert!(s.editor.is_none(), "a dropped editor is forgotten so the next call looks again");
        assert!(s.call(&json!({"name": "info"}))["isError"].is_null(), "and without one the map here answers");
    }
}
