//! `mapcraft-cli mcp`: a Model Context Protocol server on stdin and stdout, editing one map at a time.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use mapcraft_engine::{Session, control};
use serde_json::{Value, json};

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Edits MapCraft .mcmap level files headlessly, one map at a time. \
Units are inches (39.37 per meter) and Z is up. Most edits act on the selection: select or create nodes, then move, rotate or run_command. \
Changes stay in memory until save; if the client disconnects first they are written beside the map as <name>.unsaved.mcmap.";

pub fn run() -> Result<(), String> {
    let mut session = Session::default();
    let served = serve(&mut session);
    let kept = keep_unsaved(&mut session);
    served.and(kept)
}

fn serve(session: &mut Session) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(session, &msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}").and_then(|()| out.flush()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Answers one request; notifications, which carry no id, get no reply.
fn handle(session: &mut Session, msg: &Value) -> Option<Value> {
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
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            match control::call(session, name, params.get("arguments").unwrap_or(&Value::Null)) {
                Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}]}),
                Err(e) => json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true}),
            }
        }
        method => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method `{method}`")}})),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
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
        let mut s = Session::default();
        let mut ask = |msg: Value| handle(&mut s, &msg);
        let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        assert!(ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap()["result"]["tools"].as_array().unwrap().len() > 10);
        let call = |name: &str, args: Value| json!({"jsonrpc": "2.0", "id": "c", "method": "tools/call", "params": {"name": name, "arguments": args}});
        let ok = ask(call("create_brush", json!({"min": [0, 0, 0], "max": [64, 64, 64]}))).unwrap();
        assert!(ok["result"]["isError"].is_null() && ok["id"] == "c");
        assert_eq!(ask(call("select", json!({"ids": [99]}))).unwrap()["result"]["isError"], true);
        assert_eq!(ask(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).unwrap()["error"]["code"], -32601);
    }
}
