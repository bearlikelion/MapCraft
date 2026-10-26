use std::collections::BTreeSet;
use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use mapcraft_geom::{Aabb, DMat4, DVec3, Plane};
use mapcraft_map::{EditError, FaceId, NodeId, NodeKind, ROOT};
use serde_json::{Value, json};

use crate::{ClipKeep, EngineError, Session, Severity, Shape, commands, execute, find_entities, validate};

/// The file in the data folder where a running editor writes its control port and token.
pub const CONTROL_FILE: &str = "control.json";

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
        description: "Starts an empty map that `save` writes to `path`, which must not exist yet; the editor keeps the open map's `sdk` when it is omitted.",
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
        description: "Path, SDK, unsaved state, node count, selection, undo/redo labels and the map's bounds.",
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
        description: "Runs an editor command by id on the selection, e.g. edit.undo, edit.delete, tools.carve, tools.group, tools.merge.",
        schema: || object(json!({"id": string()}), &["id"]),
        run: |s, a| {
            execute(s, str_arg(a, "id")?)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "select",
        description: "Selects nodes by id, replacing the selection unless `add` is true; an empty list selects nothing.",
        schema: || object(json!({"ids": ids(), "add": {"type": "boolean"}}), &["ids"]),
        run: |s, a| {
            let ids = ids_arg(s, a, "ids")?;
            if !a.get("add").and_then(Value::as_bool).unwrap_or(false) {
                s.selection.clear();
            }
            s.selection.extend(ids);
            Ok(info(s))
        },
    },
    Tool {
        name: "create_brush",
        description: "Creates a brush filling the box from `min` to `max` and selects it; `material` also becomes the default for later brushes. \
An arch spans `arc` degrees starting `start` degrees off +X with walls `thickness` deep.",
        schema: || {
            let shapes = json!({"type": "string", "enum": ["block", "wedge", "cylinder", "spike", "sphere", "arch"]});
            let n = json!({"type": "number"});
            object(
                json!({"min": vec3(), "max": vec3(), "shape": shapes, "sides": {"type": "integer"}, "material": string(), "arc": n, "start": n, "thickness": n}),
                &["min", "max"],
            )
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
                "arch" => Shape::Arch { sides, arc: num_or(a, "arc", 180.0)?, start: num_or(a, "start", 0.0)?, thickness: num_or(a, "thickness", 16.0)? },
                _ => return Err(bad("shape", "block, wedge, cylinder, spike, sphere or arch")),
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
        name: "tie_to_entity",
        description: "Makes the selected world brushes one brush entity of `classname`, such as a trigger or func_detail.",
        schema: || object(json!({"classname": string()}), &["classname"]),
        run: |s, a| {
            s.tie_to_entity(str_arg(a, "classname")?)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "set_material",
        description: "Gives the selected brushes' faces `material`: all of them, or those facing top, bottom, sides, +x, -x, +y or -y. \
It also becomes the default for later brushes.",
        schema: || {
            let faces = json!({"type": "string", "enum": ["all", "top", "bottom", "sides", "+x", "-x", "+y", "-y"]});
            object(json!({"material": string(), "faces": faces}), &["material"])
        },
        run: |s, a| {
            let side = a.get("faces").and_then(Value::as_str).unwrap_or("all");
            if !["all", "top", "bottom", "sides", "+x", "-x", "+y", "-y"].contains(&side) {
                return Err(bad("faces", "all, top, bottom, sides, +x, -x, +y or -y"));
            }
            let wanted = |n: DVec3| {
                let f = facing(n);
                side == "all" || side == f || (side == "sides" && !matches!(f, "top" | "bottom"))
            };
            let picked: BTreeSet<(NodeId, FaceId)> = s
                .selected_brushes()
                .into_iter()
                .filter_map(|id| Some((id, s.map.get(id)?.kind.brush()?)))
                .flat_map(|(id, b)| b.faces.iter().filter(|f| f.plane().is_some_and(|p| wanted(p.normal))).map(move |f| (id, f.id)))
                .collect();
            if picked.is_empty() {
                return Err(EngineError::Refused("no selected brush has faces facing that way"));
            }
            s.tex.material = str_arg(a, "material")?.into();
            let before = std::mem::replace(&mut s.face_selection, picked);
            let applied = s.apply_material();
            s.face_selection = before;
            applied?;
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
            let degrees = num_or(a, "degrees", f64::NAN).and_then(|d| if d.is_finite() { Ok(d) } else { Err(bad("degrees", "a number")) })?;
            let axis = if a.get("axis").is_some() { vec_arg(a, "axis")? } else { DVec3::Z };
            s.rotate_selection(axis, degrees)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "scale",
        description: "Scales the selection about its center by `factors` on X, Y and Z.",
        schema: || object(json!({"factors": vec3()}), &["factors"]),
        run: |s, a| {
            let f = vec_arg(a, "factors")?;
            if f.abs().min_element() < 1e-6 {
                return Err(bad("factors", "non-zero on every axis"));
            }
            let c = s.selection_center().ok_or(EngineError::NothingSelected)?;
            let m = DMat4::from_translation(c) * DMat4::from_scale(f) * DMat4::from_translation(-c);
            s.transform_selection(&m, "Scale", None)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "duplicate",
        description: "Makes `count` copies of the selection (1 when omitted, at most 256), each `delta` further on, and selects the copies.",
        schema: || object(json!({"delta": vec3(), "count": {"type": "integer", "minimum": 1, "maximum": 256}}), &["delta"]),
        run: |s, a| {
            let count = a.get("count").and_then(Value::as_u64).map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
            s.array_selection(count, &DMat4::from_translation(vec_arg(a, "delta")?))?;
            Ok(info(s))
        },
    },
    Tool {
        name: "hollow",
        description: "Turns the selected brushes into rooms whose walls are `thickness` deep (16 when omitted).",
        schema: || object(json!({"thickness": {"type": "number"}}), &[]),
        run: |s, a| {
            let t = num_or(a, "thickness", 16.0)?;
            if t <= 0.0 {
                return Err(bad("thickness", "more than 0"));
            }
            s.hollow(t)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "clip",
        description: "Cuts the selected brushes with the plane through `point` facing `normal`; `keep` is front (the side normal points to), back, or both.",
        schema: || object(json!({"point": vec3(), "normal": vec3(), "keep": {"type": "string", "enum": ["front", "back", "both"]}}), &["point", "normal"]),
        run: |s, a| {
            let (point, normal) = (vec_arg(a, "point")?, vec_arg(a, "normal")?.normalize_or_zero());
            if normal == DVec3::ZERO {
                return Err(bad("normal", "a non-zero direction"));
            }
            let keep = match a.get("keep").and_then(Value::as_str).unwrap_or("front") {
                "front" => ClipKeep::Front,
                "back" => ClipKeep::Back,
                "both" => ClipKeep::Both,
                _ => return Err(bad("keep", "front, back or both")),
            };
            s.clip(Plane { normal, dist: normal.dot(point) }.three_points(), keep)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "create_layer",
        description: "Adds a layer under the world and returns its id; new nodes still go to the world until moved with move_to.",
        schema: || object(json!({"name": string()}), &["name"]),
        run: |s, a| Ok(json!({"id": s.create_layer(str_arg(a, "name")?)?})),
    },
    Tool {
        name: "move_to",
        description: "Moves nodes into `parent` (the world, a layer, a group or a brush entity), after its other children.",
        schema: || object(json!({"ids": ids(), "parent": id()}), &["ids", "parent"]),
        run: |s, a| {
            let ids = ids_arg(s, a, "ids")?;
            let parent = id_arg(a, "parent")?;
            let end = s.map.get(parent).ok_or(EditError::Missing(parent))?.children.len();
            s.move_nodes(&ids, parent, end)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "insert_prefab",
        description: "Places an instance of another .mcmap file at `origin`; it stays linked to that file.",
        schema: || object(json!({"path": string(), "origin": vec3()}), &["path", "origin"]),
        run: |s, a| {
            let path = s.relative_path(Path::new(str_arg(a, "path")?));
            s.insert_prefab(&path, vec_arg(a, "origin")?)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "batch",
        description: "Runs many tool calls in order, stopping at the first error; `calls` holds {\"tool\": name, \"arguments\": {...}}.",
        schema: || {
            let call = json!({"type": "object", "properties": {"tool": string(), "arguments": {"type": "object"}}, "required": ["tool"]});
            object(json!({"calls": {"type": "array", "items": call}}), &["calls"])
        },
        run: |s, a| batch(a, |name, args| call(s, name, args)),
    },
    Tool {
        name: "validate",
        description: "Problems that make the map fail to compile or play; the editor adds the SDK's FGD and plugin checks.",
        schema: || object(json!({}), &[]),
        run: |s, _| Ok(issues_json(&validate(&s.map, None))),
    },
    Tool {
        name: "screenshot",
        description: "Editor only: renders the map to a PNG. `view` is all (the four views), 3d, top, front or side. \
The 3D view frames `focus` (node ids, default the whole map) or looks from `eye` at `look_at`.",
        schema: || {
            let view = json!({"type": "string", "enum": ["all", "3d", "top", "front", "side"]});
            let px = json!({"type": "integer", "minimum": 64, "maximum": 2048});
            object(json!({"view": view, "focus": ids(), "eye": vec3(), "look_at": vec3(), "width": px, "height": px}), &[])
        },
        run: needs_editor,
    },
    Tool {
        name: "frame",
        description: "Editor only: points the editor's views at `ids`, or at the selection, or at the whole map.",
        schema: || object(json!({"ids": ids()}), &[]),
        run: needs_editor,
    },
    Tool {
        name: "entity_classes",
        description: "Editor only: the SDK's entity classes (point or solid) with descriptions, optionally those containing `filter`.",
        schema: || object(json!({"filter": string()}), &[]),
        run: needs_editor,
    },
    Tool {
        name: "entity_class",
        description: "Editor only: one entity class's keys with types, defaults and choices, and its inputs and outputs.",
        schema: || object(json!({"name": string()}), &["name"]),
        run: needs_editor,
    },
    Tool {
        name: "materials",
        description: "Editor only: names of the SDK's materials containing `filter`, at most `limit` (200 when omitted).",
        schema: || object(json!({"filter": string(), "limit": {"type": "integer", "minimum": 1}}), &[]),
        run: needs_editor,
    },
    Tool {
        name: "plugin_commands",
        description: "Editor only: the SDK plugins' commands and generators with their parameters.",
        schema: || object(json!({}), &[]),
        run: needs_editor,
    },
    Tool {
        name: "run_plugin",
        description: "Editor only: runs a plugin command at `origin`; missing `params` take their defaults. \
A generator's output becomes a group, turned `yaw` degrees, that stays editable.",
        schema: || {
            object(
                json!({"plugin": string(), "command": string(), "params": {"type": "object"}, "origin": vec3(), "yaw": {"type": "number"}}),
                &["plugin", "command"],
            )
        },
        run: needs_editor,
    },
    Tool {
        name: "compile_profiles",
        description: "Editor only: the SDK's compile and run profiles (Run menu).",
        schema: || object(json!({}), &[]),
        run: needs_editor,
    },
    Tool {
        name: "compile",
        description: "Editor only: saves the map and starts a Run menu profile by name; read its progress with output.",
        schema: || object(json!({"profile": string()}), &["profile"]),
        run: needs_editor,
    },
    Tool {
        name: "output",
        description: "Editor only: the last `lines` lines of the Output panel (50 when omitted) and whether a compile is running.",
        schema: || object(json!({"lines": {"type": "integer", "minimum": 1}}), &[]),
        run: needs_editor,
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

/// Runs a batch's calls through `each`, which may be the editor's own dispatcher.
pub fn batch<E: Display>(args: &Value, mut each: impl FnMut(&str, &Value) -> Result<Value, E>) -> Result<Value, EngineError> {
    let calls = args.get("calls").and_then(Value::as_array).ok_or_else(|| bad("calls", "an array of {tool, arguments}"))?;
    let mut out = Vec::new();
    for (i, c) in calls.iter().enumerate() {
        let name = c.get("tool").and_then(Value::as_str).unwrap_or_default();
        if name == "batch" {
            return Err(EngineError::Refused("batches cannot hold batches"));
        }
        let result = each(name, c.get("arguments").unwrap_or(&Value::Null));
        out.push(result.map_err(|e| EngineError::Argument(format!("calls[{i}] ({name}) failed after {i} succeeded: {e}")))?);
    }
    Ok(Value::from(out))
}

/// A tool's answer as an MCP CallToolResult holding JSON text.
pub fn text_result<E: Display>(r: Result<Value, E>) -> Value {
    match r {
        Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}]}),
        Err(e) => json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true}),
    }
}

pub fn issues_json(issues: &[crate::Issue]) -> Value {
    issues
        .iter()
        .map(|i| {
            let severity = if i.severity == Severity::Error { "error" } else { "warning" };
            json!({"node": i.node, "face": i.face, "severity": severity, "message": i.message})
        })
        .collect()
}

/// Node ids under `key`, all of which must exist.
pub fn ids_arg(s: &Session, a: &Value, key: &str) -> Result<Vec<NodeId>, EngineError> {
    let ids: Vec<NodeId> = a
        .get(key)
        .and_then(Value::as_array)
        .and_then(|ids| ids.iter().map(|v| v.as_u64().map(NodeId)).collect())
        .ok_or_else(|| bad(key, "an array of node ids"))?;
    match ids.iter().find(|id| s.map.get(**id).is_none()) {
        Some(id) => Err(EditError::Missing(*id).into()),
        None => Ok(ids),
    }
}

pub fn str_arg<'a>(a: &'a Value, key: &str) -> Result<&'a str, EngineError> {
    a.get(key).and_then(Value::as_str).ok_or_else(|| bad(key, "a string"))
}

pub fn vec_arg(a: &Value, key: &str) -> Result<DVec3, EngineError> {
    let v: Option<Vec<f64>> = a.get(key).and_then(Value::as_array).and_then(|v| v.iter().map(Value::as_f64).collect());
    match v.as_deref() {
        Some(&[x, y, z]) if x.is_finite() && y.is_finite() && z.is_finite() => Ok(DVec3::new(x, y, z)),
        _ => Err(bad(key, "[x, y, z] in inches, Z up")),
    }
}

/// The number under `key`, or `default` when it is missing.
pub fn num_or(a: &Value, key: &str, default: f64) -> Result<f64, EngineError> {
    match a.get(key) {
        None => Ok(default),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(key, "a number")),
    }
}

pub fn bad(key: &str, what: &str) -> EngineError {
    EngineError::Argument(format!("`{key}` must be {what}"))
}

pub fn info(s: &Session) -> Value {
    let bounds = s.map.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).reduce(|a, b| a.union(&b));
    json!({
        "path": s.path.as_ref().map(|p| p.display().to_string()),
        "sdk": s.sdk,
        "unsaved": s.is_dirty(),
        "nodes": s.map.len(),
        "bounds": bounds.map(|b| [b.min.to_array(), b.max.to_array()]),
        "selection": s.selection,
        "undo": s.undo_label(),
        "redo": s.redo_label(),
    })
}

/// Refuses to replace a map with unsaved changes unless the call passed `discard: true`.
pub fn check_discard(s: &Session, a: &Value) -> Result<(), EngineError> {
    if s.is_dirty() && !a.get("discard").and_then(Value::as_bool).unwrap_or(false) {
        return Err(EngineError::Refused("the map has unsaved changes; save it or pass discard: true"));
    }
    Ok(())
}

fn needs_editor(_: &mut Session, _: &Value) -> Result<Value, EngineError> {
    Err(EngineError::Refused("this tool needs the MapCraft editor; open it and call again"))
}

fn id_arg(a: &Value, key: &str) -> Result<NodeId, EngineError> {
    a.get(key).and_then(Value::as_u64).map(NodeId).ok_or_else(|| bad(key, "a node id"))
}

/// Which way a face looks, by the largest part of its normal.
fn facing(n: DVec3) -> &'static str {
    let a = n.abs();
    if a.z >= a.x && a.z >= a.y {
        if n.z > 0.0 { "top" } else { "bottom" }
    } else if a.x >= a.y {
        if n.x > 0.0 { "+x" } else { "-x" }
    } else if n.y > 0.0 {
        "+y"
    } else {
        "-y"
    }
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

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn string() -> Value {
    json!({"type": "string"})
}

fn id() -> Value {
    json!({"type": "integer", "minimum": 0})
}

fn ids() -> Value {
    json!({"type": "array", "items": id()})
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

    fn bounds(s: &Session, id: u64) -> Aabb {
        s.map.get(NodeId(id)).unwrap().kind.brush().unwrap().bounds().unwrap()
    }

    #[test]
    fn tools_edit_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.mcmap");
        let p = path.to_str().unwrap();
        let mut s = Session::default();
        assert!(tools().iter().all(|t| t["inputSchema"]["type"] == "object"));

        call(&mut s, "new", &json!({"path": p})).unwrap();
        call(&mut s, "create_brush", &json!({"min": [0, 0, 0], "max": [256, 256, 128], "material": "dev/grid"})).unwrap();
        call(&mut s, "hollow", &json!({"thickness": 8})).unwrap();
        let out = call(&mut s, "create_entity", &json!({"classname": "info_player_start", "origin": [128, 128, 32], "properties": {"angle": 90}})).unwrap();
        let ent = out["selection"][0].as_u64().unwrap();
        call(&mut s, "set_property", &json!({"id": ent, "key": "targetname", "value": "spawn"})).unwrap();
        assert_eq!(call(&mut s, "find", &json!({"query": "targetname=spawn"})).unwrap()[0]["id"], ent);
        assert_eq!(call(&mut s, "get_node", &json!({"id": ent})).unwrap()["kind"]["properties"][0], json!(["angle", "90"]));
        assert!(call(&mut s, "validate", &json!({})).unwrap().as_array().unwrap().is_empty());
        assert_eq!(call(&mut s, "info", &json!({})).unwrap()["bounds"], json!([[0.0, 0.0, 0.0], [256.0, 256.0, 128.0]]));
        assert!(matches!(call(&mut s, "open", &json!({"path": p})), Err(EngineError::Refused(_))));
        call(&mut s, "save", &json!({})).unwrap();

        let mut t = Session::default();
        let listed = call(&mut t, "open", &json!({"path": p})).and_then(|_| call(&mut t, "list_nodes", &json!({}))).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 7);
        assert!(matches!(call(&mut t, "new", &json!({"path": p})), Err(EngineError::Refused(_))));
    }

    #[test]
    fn building_tools() {
        let mut s = Session::default();
        let made = call(&mut s, "create_brush", &json!({"min": [0, 0, 0], "max": [64, 64, 64]})).unwrap();
        let b = made["selection"][0].as_u64().unwrap();

        call(&mut s, "clip", &json!({"point": [16, 0, 0], "normal": [1, 0, 0]})).unwrap();
        let kept = s.selection.first().unwrap().0;
        assert_eq!(bounds(&s, kept).min.x, 16.0);
        call(&mut s, "run_command", &json!({"id": "edit.undo"})).unwrap();

        call(&mut s, "set_material", &json!({"material": "floor", "faces": "top"})).unwrap();
        let faces = &s.map.get(NodeId(b)).unwrap().kind.brush().unwrap().faces;
        assert_eq!(faces.iter().filter(|f| f.material == "floor").count(), 1);

        call(&mut s, "scale", &json!({"factors": [2, 1, 1]})).unwrap();
        assert_eq!(bounds(&s, b).size().x, 128.0);

        call(&mut s, "duplicate", &json!({"delta": [0, 128, 0], "count": 3})).unwrap();
        assert_eq!(s.selection.len(), 3);

        let layer = call(&mut s, "create_layer", &json!({"name": "copies"})).unwrap()["id"].as_u64().unwrap();
        let copies: Vec<u64> = s.selection.iter().map(|id| id.0).collect();
        call(&mut s, "move_to", &json!({"ids": copies, "parent": layer})).unwrap();
        assert_eq!(s.map.get(NodeId(layer)).unwrap().children.len(), 3);

        call(&mut s, "select", &json!({"ids": [b]})).unwrap();
        call(&mut s, "tie_to_entity", &json!({"classname": "trigger_multiple"})).unwrap();
        assert_eq!(call(&mut s, "find", &json!({"query": "trigger_multiple"})).unwrap().as_array().unwrap().len(), 1);

        let out = call(
            &mut s,
            "batch",
            &json!({"calls": [
                {"tool": "create_brush", "arguments": {"min": [0, 0, -16], "max": [512, 512, 0]}},
                {"tool": "create_brush", "arguments": {"min": [0, 0, 0], "max": [64, 64, 64], "shape": "arch", "sides": 8}},
                {"tool": "move", "arguments": {"delta": "up"}},
            ]}),
        );
        assert!(matches!(out, Err(EngineError::Argument(m)) if m.starts_with("calls[2] (move) failed after 2 succeeded")));
        assert!(call(&mut s, "screenshot", &json!({})).is_err());
    }

    #[test]
    fn hostile_arguments_are_errors() {
        let mut s = Session::default();
        for (name, args) in [
            ("nope", json!({})),
            ("create_brush", json!({"min": [0, 0], "max": [1, 1, 1]})),
            ("create_brush", json!({"min": [0, 0, 0], "max": [0, 0, 0]})),
            ("get_node", json!({"id": -1})),
            ("select", json!({"ids": [999]})),
            ("move", json!({"delta": "up"})),
            ("set_property", json!({"id": 999, "key": "a", "value": "b"})),
            ("create_entity", json!({"classname": "x", "origin": [0, 0, 0], "properties": []})),
            ("run_command", json!({"id": "edit.delete"})),
            ("save", json!({})),
            ("hollow", json!({"thickness": -1})),
            ("clip", json!({"point": [0, 0, 0], "normal": [0, 0, 0]})),
            ("scale", json!({"factors": [0, 1, 1]})),
            ("duplicate", json!({"delta": [1, 0, 0], "count": 100000})),
            ("set_material", json!({"material": "x", "faces": "up"})),
            ("batch", json!({"calls": [{"tool": "batch", "arguments": {}}]})),
            ("move_to", json!({"ids": [0], "parent": 999})),
        ] {
            assert!(matches!(call(&mut s, name, &args), Err(e) if !matches!(e, EngineError::Panic(_))), "{name} {args}");
        }
        let huge = json!({"min": [0, 0, 0], "max": [1e308, 1e308, 1e308], "shape": "sphere", "sides": u64::MAX});
        assert!(!matches!(call(&mut s, "create_brush", &huge), Err(EngineError::Panic(_))));
    }
}
