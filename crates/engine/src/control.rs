use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use mapcraft_geom::{Aabb, DVec3};
use mapcraft_map::{EditError, NodeId, NodeKind, ROOT};
use serde_json::{Value, json};

use crate::{EngineError, Session, Severity, Shape, commands, execute, find_entities, validate};

/// An action an agent calls by name with a JSON object of arguments (MCP tools).
struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    run: fn(&mut Session, &Value) -> Result<Value, EngineError>,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "new",
        description: "Starts an empty map that `save` writes to `path`, which must not exist yet.",
        schema: || object(json!({"path": string(), "sdk": string(), "discard": discard()}), &["path"]),
        run: |s, a| {
            let path = Path::new(str_arg(a, "path")?);
            if path.exists() {
                return Err(EngineError::Refused("that file already exists; use open"));
            }
            check_discard(s, a)?;
            *s = Session::default();
            s.path = Some(path.to_path_buf());
            s.sdk = a.get("sdk").and_then(Value::as_str).map(String::from);
            Ok(info(s))
        },
    },
    Tool {
        name: "open",
        description: "Opens a .mcmap file, replacing the current map.",
        schema: || object(json!({"path": string(), "discard": discard()}), &["path"]),
        run: |s, a| {
            let path = Path::new(str_arg(a, "path")?);
            check_discard(s, a)?;
            *s = Session::open(path)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "save",
        description: "Saves the map to its own path, or to `path` which then becomes its path.",
        schema: || object(json!({"path": string()}), &[]),
        run: |s, a| {
            match a.get("path").and_then(Value::as_str) {
                Some(p) => s.save_as(Path::new(p))?,
                None => s.save()?,
            }
            Ok(info(s))
        },
    },
    Tool {
        name: "info",
        description: "Path, SDK, unsaved state, node count, selection and undo/redo labels.",
        schema: || object(json!({}), &[]),
        run: |s, _| Ok(info(s)),
    },
    Tool {
        name: "list_nodes",
        description: "Children of `parent` (the world, id 0, when omitted) with id, kind, label and child count.",
        schema: || object(json!({"parent": id()}), &[]),
        run: |s, a| {
            let parent = if a.get("parent").is_some() { id_arg(a, "parent")? } else { ROOT };
            let node = s.map.get(parent).ok_or(EditError::Missing(parent))?;
            Ok(node
                .children
                .iter()
                .filter_map(|id| Some((*id, s.map.get(*id)?)))
                .map(|(id, n)| json!({"id": id, "kind": kind_name(&n.kind), "label": n.kind.label(), "children": n.children.len()}))
                .collect())
        },
    },
    Tool {
        name: "get_node",
        description: "Everything stored on a node (entity key/values, brush faces and materials), plus a brush's bounds.",
        schema: || object(json!({"id": id()}), &["id"]),
        run: |s, a| {
            let id = id_arg(a, "id")?;
            let node = s.map.get(id).ok_or(EditError::Missing(id))?;
            let mut v = serde_json::to_value(node).map_err(|_| EngineError::Refused("the node could not be written as JSON"))?;
            if let (Some(o), Some(b)) = (v.as_object_mut(), node.kind.brush().and_then(|b| b.bounds().ok())) {
                o.insert("bounds".into(), json!([b.min.to_array(), b.max.to_array()]));
            }
            Ok(v)
        },
    },
    Tool {
        name: "find",
        description: "Entities whose classname or any value contains `query`, or `key=value` to match one key; case does not matter.",
        schema: || object(json!({"query": string()}), &["query"]),
        run: |s, a| {
            let ids = find_entities(&s.map, str_arg(a, "query")?);
            Ok(ids.iter().filter_map(|id| Some(json!({"id": id, "label": s.map.get(*id)?.kind.label()}))).collect())
        },
    },
    Tool {
        name: "list_commands",
        description: "Editor commands for run_command, with whether each can run right now.",
        schema: || object(json!({}), &[]),
        run: |s, _| Ok(commands().iter().map(|c| json!({"id": c.id, "label": c.label, "menu": c.menu, "enabled": (c.enabled)(s)})).collect()),
    },
    Tool {
        name: "run_command",
        description: "Runs an editor command by id on the selection, e.g. edit.undo, edit.delete, tools.hollow, tools.carve, tools.group.",
        schema: || object(json!({"id": string()}), &["id"]),
        run: |s, a| {
            execute(s, str_arg(a, "id")?)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "select",
        description: "Selects nodes by id, replacing the selection unless `add` is true; an empty list selects nothing.",
        schema: || object(json!({"ids": {"type": "array", "items": id()}, "add": {"type": "boolean"}}), &["ids"]),
        run: |s, a| {
            let ids: Vec<NodeId> = a
                .get("ids")
                .and_then(Value::as_array)
                .and_then(|ids| ids.iter().map(|v| v.as_u64().map(NodeId)).collect())
                .ok_or_else(|| bad("ids", "an array of node ids"))?;
            if let Some(id) = ids.iter().find(|id| s.map.get(**id).is_none()) {
                return Err(EditError::Missing(*id).into());
            }
            if !a.get("add").and_then(Value::as_bool).unwrap_or(false) {
                s.selection.clear();
            }
            s.selection.extend(ids);
            Ok(info(s))
        },
    },
    Tool {
        name: "create_brush",
        description: "Creates a brush filling the box from `min` to `max` and selects it; `material` also becomes the default for later brushes.",
        schema: || {
            let shapes = json!({"type": "string", "enum": ["block", "wedge", "cylinder", "spike", "sphere"]});
            object(json!({"min": vec3(), "max": vec3(), "shape": shapes, "sides": {"type": "integer"}, "material": string()}), &["min", "max"])
        },
        run: |s, a| {
            let bounds = Aabb::new(vec_arg(a, "min")?, vec_arg(a, "max")?);
            let sides = a.get("sides").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(8);
            let shape = match a.get("shape").and_then(Value::as_str).unwrap_or("block") {
                "block" => Shape::Block,
                "wedge" => Shape::Wedge,
                "cylinder" => Shape::Cylinder(sides),
                "spike" => Shape::Spike(sides),
                "sphere" => Shape::Sphere(sides),
                _ => return Err(bad("shape", "block, wedge, cylinder, spike or sphere")),
            };
            if let Some(m) = a.get("material").and_then(Value::as_str) {
                s.tex.material = m.into();
            }
            s.create_shape(shape, bounds)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "create_entity",
        description: "Creates a point entity at `origin` and selects it.",
        schema: || object(json!({"classname": string(), "origin": vec3(), "properties": {"type": "object"}}), &["classname", "origin"]),
        run: |s, a| {
            let props = match a.get("properties") {
                None => Vec::new(),
                Some(p) => p
                    .as_object()
                    .ok_or_else(|| bad("properties", "an object of key/values"))?
                    .iter()
                    .filter(|(k, _)| *k != "classname")
                    .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), String::from)))
                    .collect(),
            };
            s.create_point_entity(str_arg(a, "classname")?, vec_arg(a, "origin")?, props)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "set_property",
        description: "Sets an entity's key/value; key `classname` changes its class. Node 0 is worldspawn.",
        schema: || object(json!({"id": id(), "key": string(), "value": string()}), &["id", "key", "value"]),
        run: |s, a| {
            s.set_property(id_arg(a, "id")?, str_arg(a, "key")?, str_arg(a, "value")?)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "move",
        description: "Moves the selection by `delta`.",
        schema: || object(json!({"delta": vec3()}), &["delta"]),
        run: |s, a| {
            s.translate_selection(vec_arg(a, "delta")?, None)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "rotate",
        description: "Rotates the selection about its center by `degrees` around `axis` (Z when omitted).",
        schema: || object(json!({"degrees": {"type": "number"}, "axis": vec3()}), &["degrees"]),
        run: |s, a| {
            let degrees = a.get("degrees").and_then(Value::as_f64).filter(|d| d.is_finite()).ok_or_else(|| bad("degrees", "a number"))?;
            let axis = if a.get("axis").is_some() { vec_arg(a, "axis")? } else { DVec3::Z };
            s.rotate_selection(axis, degrees)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "validate",
        description: "Problems that make the map fail to compile or play, without the SDK's FGD and plugin checks.",
        schema: || object(json!({}), &[]),
        run: |s, _| {
            let issues = validate(&s.map, None);
            Ok(issues
                .iter()
                .map(|i| {
                    let severity = if i.severity == Severity::Error { "error" } else { "warning" };
                    json!({"node": i.node, "face": i.face, "severity": severity, "message": i.message})
                })
                .collect())
        },
    },
];

/// Every tool as an MCP tool definition: name, description and input schema.
pub fn tools() -> Vec<Value> {
    TOOLS.iter().map(|t| json!({"name": t.name, "description": t.description, "inputSchema": (t.schema)()})).collect()
}

/// Runs a tool by name; a panic inside it becomes an error and the map is kept.
pub fn call(session: &mut Session, name: &str, args: &Value) -> Result<Value, EngineError> {
    let tool = TOOLS.iter().find(|t| t.name == name).ok_or_else(|| EngineError::UnknownCommand(name.to_string()))?;
    catch_unwind(AssertUnwindSafe(|| (tool.run)(session, args))).unwrap_or(Err(EngineError::Panic(tool.name)))
}

fn info(s: &Session) -> Value {
    json!({
        "path": s.path.as_ref().map(|p| p.display().to_string()),
        "sdk": s.sdk,
        "unsaved": s.is_dirty(),
        "nodes": s.map.len(),
        "selection": s.selection,
        "undo": s.undo_label(),
        "redo": s.redo_label(),
    })
}

fn check_discard(s: &Session, a: &Value) -> Result<(), EngineError> {
    if s.is_dirty() && !a.get("discard").and_then(Value::as_bool).unwrap_or(false) {
        return Err(EngineError::Refused("the map has unsaved changes; save it or pass discard: true"));
    }
    Ok(())
}

fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::World(_) => "world",
        NodeKind::Layer(_) => "layer",
        NodeKind::Group(_) => "group",
        NodeKind::Entity(_) => "entity",
        NodeKind::Brush(_) => "brush",
        NodeKind::Prefab(_) => "prefab",
        NodeKind::Path(_) => "path",
    }
}

fn bad(key: &str, what: &str) -> EngineError {
    EngineError::Argument(format!("`{key}` must be {what}"))
}

fn str_arg<'a>(a: &'a Value, key: &str) -> Result<&'a str, EngineError> {
    a.get(key).and_then(Value::as_str).ok_or_else(|| bad(key, "a string"))
}

fn id_arg(a: &Value, key: &str) -> Result<NodeId, EngineError> {
    a.get(key).and_then(Value::as_u64).map(NodeId).ok_or_else(|| bad(key, "a node id"))
}

fn vec_arg(a: &Value, key: &str) -> Result<DVec3, EngineError> {
    let v: Option<Vec<f64>> = a.get(key).and_then(Value::as_array).and_then(|v| v.iter().map(Value::as_f64).collect());
    match v.as_deref() {
        Some(&[x, y, z]) if x.is_finite() && y.is_finite() && z.is_finite() => Ok(DVec3::new(x, y, z)),
        _ => Err(bad(key, "[x, y, z] in inches, Z up")),
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn string() -> Value {
    json!({"type": "string"})
}

fn id() -> Value {
    json!({"type": "integer", "minimum": 0})
}

fn vec3() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "[x, y, z] in inches (39.37 per meter), Z up"})
}

fn discard() -> Value {
    json!({"type": "boolean", "description": "drop unsaved changes to the open map"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_edit_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.mcmap");
        let p = path.to_str().unwrap();
        let mut s = Session::default();
        assert!(tools().iter().all(|t| t["inputSchema"]["type"] == "object"));

        call(&mut s, "new", &json!({"path": p})).unwrap();
        call(&mut s, "create_brush", &json!({"min": [0, 0, 0], "max": [256, 256, 128], "material": "dev/grid"})).unwrap();
        call(&mut s, "run_command", &json!({"id": "tools.hollow"})).unwrap();
        let out = call(&mut s, "create_entity", &json!({"classname": "info_player_start", "origin": [128, 128, 32], "properties": {"angle": 90}})).unwrap();
        let ent = out["selection"][0].as_u64().unwrap();
        call(&mut s, "set_property", &json!({"id": ent, "key": "targetname", "value": "spawn"})).unwrap();
        assert_eq!(call(&mut s, "find", &json!({"query": "targetname=spawn"})).unwrap()[0]["id"], ent);
        assert_eq!(call(&mut s, "get_node", &json!({"id": ent})).unwrap()["kind"]["properties"][0], json!(["angle", "90"]));
        assert!(call(&mut s, "validate", &json!({})).unwrap().as_array().unwrap().is_empty());
        assert!(matches!(call(&mut s, "open", &json!({"path": p})), Err(EngineError::Refused(_))));
        call(&mut s, "save", &json!({})).unwrap();

        let mut t = Session::default();
        let listed = call(&mut t, "open", &json!({"path": p})).and_then(|_| call(&mut t, "list_nodes", &json!({}))).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 7);
        assert!(matches!(call(&mut t, "new", &json!({"path": p})), Err(EngineError::Refused(_))));
    }

    #[test]
    fn hostile_arguments_are_errors() {
        let mut s = Session::default();
        for (name, args) in [
            ("nope", json!({})),
            ("create_brush", json!({"min": [0, 0], "max": [1, 1, 1]})),
            ("create_brush", json!({"min": [0, 0, 0], "max": [0, 0, 0]})),
            ("create_brush", json!({"min": [0, 0, 0], "max": [1e308, 1e308, 1e308], "shape": "sphere", "sides": u64::MAX})),
            ("get_node", json!({"id": -1})),
            ("select", json!({"ids": [999]})),
            ("move", json!({"delta": "up"})),
            ("set_property", json!({"id": 999, "key": "a", "value": "b"})),
            ("create_entity", json!({"classname": "x", "origin": [0, 0, 0], "properties": []})),
            ("run_command", json!({"id": "edit.delete"})),
            ("save", json!({})),
        ] {
            assert!(matches!(call(&mut s, name, &args), Err(e) if !matches!(e, EngineError::Panic(_))), "{name} {args}");
        }
    }
}
