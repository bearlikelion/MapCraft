//! `mapcraft-cli mcp`: an MCP server on stdin and stdout that edits the open editor's map, or one held here.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use mapcraft_engine::{Session, control};
use serde_json::{Value, json};

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Builds and edits MapCraft .mcmap level maps. \
While the MapCraft editor is open, tools act on the map open in it, live and undoable, and the editor-only tools work; \
otherwise they act on a map held in memory here. \
Units are inches (39.37 per meter) and Z is up. Most edits act on the selection: create or select nodes, then move, rotate, set_material or run_command. \
Check your work with screenshot and validate. Use entity_classes and materials to learn what the map's SDK offers, and plugin_commands for its generators. \
Changes stay in memory until save; a map held here that is unsaved when the client disconnects is written beside itself as <name>.unsaved.mcmap.";

pub fn run() -> Result<(), String> {
    let mut server = Server { session: Session::default(), editor: None, find_editor: true };
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
                control::text_result(control::call(&mut self.session, name, params.get("arguments").unwrap_or(&Value::Null)))
            }
        }
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
        let mut s = Server { session: Session::default(), editor: None, find_editor: false };
        let mut ask = |msg: Value| s.handle(&msg);
        let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        assert!(ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap()["result"]["tools"].as_array().unwrap().len() > 10);
        let call = |name: &str, args: Value| json!({"jsonrpc": "2.0", "id": "c", "method": "tools/call", "params": {"name": name, "arguments": args}});
        let ok = ask(call("create_brush", json!({"min": [0, 0, 0], "max": [64, 64, 64]}))).unwrap();
        assert!(ok["result"]["isError"].is_null() && ok["id"] == "c");
        assert_eq!(ask(call("select", json!({"ids": [99]}))).unwrap()["result"]["isError"], true);
        assert_eq!(ask(call("screenshot", json!({}))).unwrap()["result"]["isError"], true);
        assert_eq!(ask(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).unwrap()["error"]["code"], -32601);
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
        let mut s = Server { session: Session::default(), editor: Editor::connect_to(&file), find_editor: false };
        let r = s.call(&json!({"name": "screenshot", "arguments": {}}));
        assert_eq!(r["content"][0]["text"], "screenshot");
        editor.join().unwrap();
        let lost = s.call(&json!({"name": "info"}));
        assert_eq!(lost["isError"], true);
        assert!(s.editor.is_none(), "a dropped editor is forgotten so the next call looks again");
        assert!(s.call(&json!({"name": "info"}))["isError"].is_null(), "and without one the map here answers");
    }
}
