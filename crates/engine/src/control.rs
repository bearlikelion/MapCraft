use std::collections::BTreeSet;
use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use mapcraft_geom::{Aabb, DMat4, DVec3, Plane};
use mapcraft_map::{Bevel, Cordon, CurveOp, EditError, FaceId, NodeId, NodeKind, PathPoint, ROOT};
use serde_json::{Value, json};

use crate::curves::curve_of;
use crate::{ClipKeep, EngineError, ExtrudeMode, Sculpt, SculptMode, Session, Severity, Shape, commands, execute, find_entities, validate};

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
        description: "Gives the selected brushes' faces `material`: all of them, or those facing top, bottom, sides, +x, -x, +y or -y, \
and with `scale` that texture scale (units per texel). Both become the defaults for later brushes and generators.",
        schema: || object(json!({"material": string(), "faces": sides(), "scale": {"type": "number", "exclusiveMinimum": 0}}), &["material"]),
        run: |s, a| {
            let scale = match a.get("scale") {
                None => None,
                Some(v) => Some(v.as_f64().filter(|k| k.is_finite() && *k > 0.0).ok_or_else(|| bad("scale", "a number above 0"))?),
            };
            let picked = faces_facing(s, a)?;
            s.tex.material = str_arg(a, "material")?.into();
            if let Some(k) = scale {
                s.tex.scale = k;
            }
            let before = std::mem::replace(&mut s.face_selection, picked);
            let applied = s.apply_material().and_then(|_| match scale {
                Some(k) => s.edit_faces("Texture scale", None, |f| (f.u.scale, f.v.scale) = (k, k)),
                None => Ok(()),
            });
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
        name: "save_prefab",
        description: "Writes the selection to `path` as its own .mcmap file and replaces it with an instance of that file.",
        schema: || object(json!({"path": string()}), &["path"]),
        run: |s, a| {
            s.save_selection_as_prefab(Path::new(str_arg(a, "path")?))?;
            Ok(info(s))
        },
    },
    Tool {
        name: "make_prefab_local",
        description: "Turns the selected prefab instances into ordinary nodes that no longer follow their file.",
        schema: || object(json!({}), &[]),
        run: |s, _| {
            let dir = s.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_default();
            s.make_local(&mut |stored| mapcraft_format::load(&dir.join(stored)).ok().map(|f| f.map))?;
            Ok(info(s))
        },
    },
    Tool {
        name: "pick_faces",
        description: "Picks faces of the selected brushes as the Face tool does: all of them, or those facing top, bottom, sides, +x, -x, +y or -y; \
`add` keeps earlier picks. Sweeps, spins, the uv.* commands, displace, extrude and face_texture act on the picks.",
        schema: || object(json!({"faces": sides(), "add": {"type": "boolean"}}), &[]),
        run: |s, a| {
            let picked = faces_facing(s, a)?;
            if !flag(a, "add") {
                s.face_selection.clear();
            }
            s.face_selection.extend(picked);
            Ok(json!({"faces": s.face_selection.len()}))
        },
    },
    Tool {
        name: "face_texture",
        description: "Sets the picked faces' texture `offset` [u, v] and `scale` [u, v], and turns it `rotate` degrees, as the face inspector does.",
        schema: || {
            let uv = json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2});
            object(json!({"offset": uv, "scale": uv, "rotate": {"type": "number"}}), &[])
        },
        run: |s, a| {
            let pair = |key| match a.get(key) {
                None => Ok(None),
                Some(v) => match v.as_array().map(|v| v.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>()) {
                    Some(Some(p)) if p.len() == 2 && p.iter().all(|x| x.is_finite()) => Ok(Some((p[0], p[1]))),
                    _ => Err(bad(key, "[u, v]")),
                },
            };
            let (offset, scale, rotate) = (pair("offset")?, pair("scale")?, num_or(a, "rotate", 0.0)?);
            if scale.is_some_and(|(u, v)| u == 0.0 || v == 0.0) {
                return Err(bad("scale", "non-zero"));
            }
            s.edit_faces("Edit face", None, |f| {
                if let Some((u, v)) = offset {
                    (f.u.offset, f.v.offset) = (u, v);
                }
                if let Some((u, v)) = scale {
                    (f.u.scale, f.v.scale) = (u, v);
                }
                if rotate != 0.0 {
                    f.rotate_texture(rotate);
                }
            })?;
            Ok(info(s))
        },
    },
    Tool {
        name: "extrude",
        description: "Pushes each picked face `distance` along its normal. `mode` resize moves the face (the default), normal keeps its shape adding square sides, \
split adds a brush in the swept volume (or cuts when negative), stamp adds a brush between the face and its moved copy.",
        schema: || {
            let mode = json!({"type": "string", "enum": ["resize", "normal", "split", "stamp"]});
            object(json!({"distance": {"type": "number"}, "mode": mode}), &["distance"])
        },
        run: |s, a| {
            let mode = match a.get("mode").and_then(Value::as_str).unwrap_or("resize") {
                "resize" => ExtrudeMode::Resize,
                "normal" => ExtrudeMode::AlongNormal,
                "split" => ExtrudeMode::Split,
                "stamp" => ExtrudeMode::Stamp,
                _ => return Err(bad("mode", "resize, normal, split or stamp")),
            };
            let distance = num_or(a, "distance", f64::NAN).and_then(|d| if d.is_finite() { Ok(d) } else { Err(bad("distance", "a number")) })?;
            let faces: Vec<_> = s.face_selection.iter().filter_map(|(id, f)| Some((*id, s.map.get(*id)?.kind.brush()?.clone(), *f))).collect();
            if faces.is_empty() {
                return Err(EngineError::Refused("pick faces first"));
            }
            Ok(json!({"created": s.extrude(&faces, distance, mode, &[], None)?}))
        },
    },
    Tool {
        name: "move_vertices",
        description: "Moves corners of the selected brushes, each move [from, to], as the Vertex tool does.",
        schema: || object(json!({"moves": pairs()}), &["moves"]),
        run: |s, a| {
            s.move_vertices(&pairs_arg(a, "moves")?, None)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "bevel",
        description: "Bevels edges of the selected brushes, each edge [a, b] by its two corners, as the Vertex tool's Ctrl+B: \
`width` (16), `segments` (1) and `profile` (0.5; lower is concave, higher convex).",
        schema: || {
            let n = json!({"type": "number"});
            object(json!({"edges": pairs(), "width": n, "segments": {"type": "integer", "minimum": 1, "maximum": 64}, "profile": n}), &["edges"])
        },
        run: |s, a| {
            let edges = pairs_arg(a, "edges")?;
            let width = num_or(a, "width", 16.0)?;
            if width <= 0.0 {
                return Err(bad("width", "more than 0"));
            }
            let bevel = Bevel { width, segments: count_or(a, "segments", 1, 64)?, profile: num_or(a, "profile", 0.5)? };
            let job: Vec<_> = s
                .selected_brushes()
                .into_iter()
                .filter_map(|id| {
                    let b = s.map.get(id)?.kind.brush()?.clone();
                    let corners = b.vertices().ok()?;
                    let has = |p: DVec3| corners.iter().any(|c| c.distance_squared(p) < 1e-4);
                    let mine: Vec<_> = edges.iter().copied().filter(|(p, q)| has(*p) && has(*q)).collect();
                    (!mine.is_empty()).then_some((id, (b, mine)))
                })
                .collect();
            if job.is_empty() {
                return Err(EngineError::Refused("no selected brush has those edges"));
            }
            s.bevel_edges(&job, &bevel, None)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "create_path",
        description: "Makes a path through `points` for sweeps and arrays along a path, and selects it; it curves smoothly through them unless `smooth` is false.",
        schema: || object(json!({"points": {"type": "array", "items": vec3(), "minItems": 2}, "smooth": {"type": "boolean"}}), &["points"]),
        run: |s, a| {
            let pts: Vec<DVec3> = a
                .get("points")
                .and_then(Value::as_array)
                .and_then(|v| v.iter().map(vec_value).collect())
                .ok_or_else(|| bad("points", "an array of [x, y, z]"))?;
            if pts.len() < 2 {
                return Err(bad("points", "at least two points"));
            }
            let smooth = a.get("smooth").and_then(Value::as_bool).unwrap_or(true);
            let last = pts.len() - 1;
            let points = (0..=last)
                .map(|i| {
                    let t = if smooth { (pts[(i + 1).min(last)] - pts[i.saturating_sub(1)]) / 6.0 } else { DVec3::ZERO };
                    PathPoint { pos: pts[i], handle_in: pts[i] - t, handle_out: pts[i] + t }
                })
                .collect();
            Ok(json!({"id": s.create_path(points)?}))
        },
    },
    Tool {
        name: "set_curve",
        description: "Changes a sweep, spin or path array group's settings and rebuilds it: sweep `segments`, `twist`, `scale`; \
spin `angle`, `steps`, `axis`, `pivot`; array `count`, `spacing`, `yaw_only`.",
        schema: || {
            let (n, i) = (json!({"type": "number"}), json!({"type": "integer", "minimum": 1, "maximum": 4096}));
            let props = json!({"id": id(), "segments": i, "twist": n, "scale": n, "angle": n, "steps": i, "axis": vec3(), "pivot": vec3(), "count": i, "spacing": n, "yaw_only": {"type": "boolean"}});
            object(props, &["id"])
        },
        run: |s, a| {
            let group = id_arg(a, "id")?;
            let mut op =
                s.map.get(group).and_then(|n| curve_of(&n.kind)).cloned().ok_or(EngineError::Refused("that node is not a sweep, spin or path array"))?;
            match &mut op {
                CurveOp::Sweep { segments, twist, scale, .. } => {
                    *segments = count_or(a, "segments", *segments, 4096)?;
                    *twist = num_or(a, "twist", *twist)?;
                    *scale = num_or(a, "scale", *scale)?;
                }
                CurveOp::Spin { axis, pivot, angle, steps, .. } => {
                    *angle = num_or(a, "angle", *angle)?;
                    *steps = count_or(a, "steps", *steps, 4096)?;
                    if a.get("axis").is_some() {
                        *axis = vec_arg(a, "axis")?.try_normalize().ok_or_else(|| bad("axis", "a non-zero direction"))?;
                    }
                    if a.get("pivot").is_some() {
                        *pivot = vec_arg(a, "pivot")?;
                    }
                }
                CurveOp::Array { count, spacing, yaw_only, .. } => {
                    *count = count_or(a, "count", *count, 4096)?;
                    *spacing = num_or(a, "spacing", *spacing)?;
                    *yaw_only = a.get("yaw_only").and_then(Value::as_bool).unwrap_or(*yaw_only);
                }
            }
            s.set_curve(group, op, None)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "displace",
        description: "Turns the picked four-cornered faces into terrain displacements of `power` 2, 3 (the default) or 4, or flattens them again with `remove`.",
        schema: || object(json!({"power": {"type": "integer", "minimum": 2, "maximum": 4}, "remove": {"type": "boolean"}}), &[]),
        run: |s, a| {
            if flag(a, "remove") {
                s.remove_displacements()?;
                return Ok(info(s));
            }
            let power = a.get("power").map_or(Some(3), Value::as_u64).and_then(|p| u32::try_from(p).ok()).filter(|p| (2..=4).contains(p));
            Ok(json!({"faces": s.displace_faces(power.ok_or_else(|| bad("power", "2, 3 or 4"))?)?}))
        },
    },
    Tool {
        name: "sculpt",
        description: "One dab of the Sculpt tool at `center` on the picked displacements, or every visible one when none are picked: \
`mode` raise, lower, smooth, flatten (toward the center's height) or noise; `radius` (128); `amount` in units for raise, lower and noise (64), 0 to 1 for smooth and flatten (0.5).",
        schema: || {
            let mode = json!({"type": "string", "enum": ["raise", "lower", "smooth", "flatten", "noise"]});
            object(json!({"mode": mode, "center": vec3(), "radius": {"type": "number"}, "amount": {"type": "number"}}), &["center"])
        },
        run: |s, a| {
            let (mode, amount) = match a.get("mode").and_then(Value::as_str).unwrap_or("raise") {
                "raise" => (SculptMode::Raise, 64.0),
                "lower" => (SculptMode::Lower, 64.0),
                "smooth" => (SculptMode::Smooth, 0.5),
                "flatten" => (SculptMode::Flatten, 0.5),
                "noise" => (SculptMode::Noise, 64.0),
                _ => return Err(bad("mode", "raise, lower, smooth, flatten or noise")),
            };
            let center = vec_arg(a, "center")?;
            let radius = num_or(a, "radius", 128.0)?;
            if radius <= 0.0 {
                return Err(bad("radius", "more than 0"));
            }
            let dab = Sculpt { mode, center, radius, amount: num_or(a, "amount", amount)?, level: center };
            Ok(json!({"moved": s.sculpt(&dab, None)?}))
        },
    },
    Tool {
        name: "heightmap",
        description: "Shapes the picked displacements (or every visible one) from a grayscale grid stretched over them together: \
`values` row by row from the far (+Y) side, `width` values per row, 0 at `low` (0) and 1 at `high` (256).",
        schema: || {
            let n = json!({"type": "number"});
            object(json!({"width": {"type": "integer", "minimum": 2}, "values": {"type": "array", "items": n}, "low": n, "high": n}), &["width", "values"])
        },
        run: |s, a| {
            let gray: Vec<f32> = a
                .get("values")
                .and_then(Value::as_array)
                .and_then(|v| v.iter().map(|x| x.as_f64().map(|x| x as f32)).collect())
                .ok_or_else(|| bad("values", "an array of numbers"))?;
            let width =
                a.get("width").and_then(Value::as_u64).and_then(|w| usize::try_from(w).ok()).filter(|w| *w >= 2).ok_or_else(|| bad("width", "2 or more"))?;
            s.apply_heightmap(&gray, width, gray.len() / width, num_or(a, "low", 0.0)?, num_or(a, "high", 256.0)?)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "zone_from_selection",
        description: "Makes a `classname` zone covering the top of the selected brushes, `height` units tall (256 when omitted), and selects it.",
        schema: || object(json!({"classname": string(), "height": {"type": "number"}}), &["classname"]),
        run: |s, a| {
            let height = num_or(a, "height", 256.0)?;
            if height <= 0.0 {
                return Err(bad("height", "more than 0"));
            }
            s.zone_from_selection(str_arg(a, "classname")?, height)?;
            Ok(info(s))
        },
    },
    Tool {
        name: "tag",
        description: "Adds the selection to the tag `name` (a Hammer visgroup), creating the tag when it is new; `remove` takes the selection out of it.",
        schema: || object(json!({"name": string(), "remove": {"type": "boolean"}}), &["name"]),
        run: |s, a| {
            let name = str_arg(a, "name")?;
            match s.map.tags.iter().find(|t| t.name == name).map(|t| t.id) {
                Some(tag) => s.tag_selection(tag, !flag(a, "remove"))?,
                None if flag(a, "remove") => return Err(EngineError::Refused("there is no tag with that name")),
                None => _ = s.create_tag(name)?,
            }
            Ok(info(s))
        },
    },
    Tool {
        name: "cordon",
        description: "Adds or replaces the cordon `name`, a box from `min` to `max` that limits what the views show and the map exports while `active`; `remove` deletes it.",
        schema: || object(json!({"name": string(), "min": vec3(), "max": vec3(), "active": {"type": "boolean"}, "remove": {"type": "boolean"}}), &["name"]),
        run: |s, a| {
            let name = str_arg(a, "name")?;
            let mut cordons: Vec<Cordon> = s.map.cordons.iter().filter(|c| c.name != name).cloned().collect();
            if !flag(a, "remove") {
                cordons.push(Cordon { name: name.into(), bounds: Aabb::new(vec_arg(a, "min")?, vec_arg(a, "max")?), active: flag(a, "active") });
            }
            s.edit_cordons(cordons, "Edit cordons");
            Ok(info(s))
        },
    },
    Tool {
        name: "replace_materials",
        description: "Renames material `find` to `replace` on every face, or on the selection's with `selected_only`: \
whole names ignoring case, or every occurrence inside a name with `partial`. Returns how many faces changed.",
        schema: || {
            object(json!({"find": string(), "replace": string(), "partial": {"type": "boolean"}, "selected_only": {"type": "boolean"}}), &["find", "replace"])
        },
        run: |s, a| Ok(json!({"faces": s.replace_materials(str_arg(a, "find")?, str_arg(a, "replace")?, flag(a, "partial"), flag(a, "selected_only"))?})),
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
    a.get(key).and_then(vec_value).ok_or_else(|| bad(key, "[x, y, z] in inches, Z up"))
}

fn vec_value(v: &Value) -> Option<DVec3> {
    let v: Vec<f64> = v.as_array()?.iter().map(Value::as_f64).collect::<Option<_>>()?;
    match v.as_slice() {
        &[x, y, z] if x.is_finite() && y.is_finite() && z.is_finite() => Some(DVec3::new(x, y, z)),
        _ => None,
    }
}

/// A list of [a, b] point pairs, such as vertex moves or edges.
fn pairs_arg(a: &Value, key: &str) -> Result<Vec<(DVec3, DVec3)>, EngineError> {
    let pair = |p: &Value| match p.as_array()?.as_slice() {
        [x, y] => Some((vec_value(x)?, vec_value(y)?)),
        _ => None,
    };
    a.get(key).and_then(Value::as_array).and_then(|v| v.iter().map(pair).collect()).ok_or_else(|| bad(key, "an array of [[x, y, z], [x, y, z]]"))
}

/// The whole number under `key` from 1 to `max`, or `default` when it is missing.
fn count_or(a: &Value, key: &str, default: usize, max: usize) -> Result<usize, EngineError> {
    match a.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| bad(key, &format!("a whole number from 1 to {max}"))),
    }
}

fn flag(a: &Value, key: &str) -> bool {
    a.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The selected brushes' faces that look the way `faces` says.
fn faces_facing(s: &Session, a: &Value) -> Result<BTreeSet<(NodeId, FaceId)>, EngineError> {
    let side = a.get("faces").and_then(Value::as_str).unwrap_or("all");
    if !SIDES.contains(&side) {
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
    Ok(picked)
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

const SIDES: [&str; 8] = ["all", "top", "bottom", "sides", "+x", "-x", "+y", "-y"];

fn sides() -> Value {
    json!({"type": "string", "enum": SIDES})
}

fn pairs() -> Value {
    json!({"type": "array", "items": {"type": "array", "items": vec3(), "minItems": 2, "maxItems": 2}})
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

        call(&mut s, "set_material", &json!({"material": "floor", "faces": "top", "scale": 3})).unwrap();
        let faces = &s.map.get(NodeId(b)).unwrap().kind.brush().unwrap().faces;
        assert_eq!(faces.iter().filter(|f| f.material == "floor" && f.u.scale == 3.0).count(), 1);
        assert_eq!(s.tex.scale, 3.0);

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
    fn face_curve_terrain_and_prefab_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::default();
        s.path = Some(dir.path().join("t.mcmap"));
        call(&mut s, "create_brush", &json!({"min": [0, 0, -64], "max": [512, 512, 0]})).unwrap();
        assert_eq!(call(&mut s, "pick_faces", &json!({"faces": "top"})).unwrap()["faces"], 1);
        assert_eq!(call(&mut s, "displace", &json!({"power": 2})).unwrap()["faces"], 1);
        assert_eq!(call(&mut s, "sculpt", &json!({"center": [256, 256, 0], "radius": 256})).unwrap()["moved"], true);
        call(&mut s, "heightmap", &json!({"width": 2, "values": [0, 1, 0, 1], "high": 64})).unwrap();
        call(&mut s, "face_texture", &json!({"offset": [8, 0], "rotate": 90})).unwrap();

        call(&mut s, "create_brush", &json!({"min": [0, 0, 0], "max": [16, 64, 64]})).unwrap();
        call(&mut s, "pick_faces", &json!({"faces": "+x"})).unwrap();
        call(&mut s, "create_path", &json!({"points": [[16, 32, 32], [512, 32, 32], [512, 512, 32]]})).unwrap();
        call(&mut s, "run_command", &json!({"id": "tools.sweep"})).unwrap();
        let sweep = s.selection.first().unwrap().0;
        call(&mut s, "set_curve", &json!({"id": sweep, "segments": 4})).unwrap();
        assert!(s.map.get(NodeId(sweep)).unwrap().children.len() > 2);

        let b = call(&mut s, "create_brush", &json!({"min": [0, 0, 0], "max": [64, 64, 64]})).unwrap()["selection"][0].as_u64().unwrap();
        call(&mut s, "move_vertices", &json!({"moves": [[[64, 64, 64], [64, 64, 96]]]})).unwrap();
        assert_eq!(bounds(&s, b).max.z, 96.0);
        call(&mut s, "pick_faces", &json!({"faces": "-y"})).unwrap();
        call(&mut s, "extrude", &json!({"distance": 32})).unwrap();
        assert_eq!(bounds(&s, b).min.y, -32.0);
        call(&mut s, "bevel", &json!({"edges": [[[0, -32, 0], [64, -32, 0]]], "width": 8})).unwrap();
        assert!(s.map.get(NodeId(b)).unwrap().kind.brush().unwrap().faces.len() > 6);

        call(&mut s, "tag", &json!({"name": "Decor"})).unwrap();
        call(&mut s, "tag", &json!({"name": "Decor"})).unwrap();
        assert_eq!(s.map.tags.len(), 1);
        call(&mut s, "cordon", &json!({"name": "a", "min": [0, 0, 0], "max": [64, 64, 64], "active": true})).unwrap();
        assert!(s.map.cordons[0].active);
        call(&mut s, "cordon", &json!({"name": "a", "remove": true})).unwrap();
        assert!(s.map.cordons.is_empty());

        call(&mut s, "zone_from_selection", &json!({"classname": "zone_start"})).unwrap();
        assert_eq!(call(&mut s, "replace_materials", &json!({"find": "tools/trigger", "replace": "zones"})).unwrap()["faces"], 6);
        call(&mut s, "save_prefab", &json!({"path": dir.path().join("prefabs/zone.mcmap")})).unwrap();
        assert!(call(&mut s, "find", &json!({"query": "zone_start"})).unwrap().as_array().unwrap().is_empty());
        call(&mut s, "make_prefab_local", &json!({})).unwrap();
        assert_eq!(call(&mut s, "find", &json!({"query": "zone_start"})).unwrap().as_array().unwrap().len(), 1);
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
            ("set_material", json!({"material": "x", "scale": 0})),
            ("batch", json!({"calls": [{"tool": "batch", "arguments": {}}]})),
            ("move_to", json!({"ids": [0], "parent": 999})),
            ("pick_faces", json!({"faces": "up"})),
            ("extrude", json!({"distance": 8})),
            ("displace", json!({"power": 9})),
            ("sculpt", json!({"center": [0, 0, 0], "mode": "melt"})),
            ("heightmap", json!({"width": 1, "values": []})),
            ("create_path", json!({"points": [[0, 0, 0]]})),
            ("set_curve", json!({"id": 0})),
            ("bevel", json!({"edges": [[1, 2]]})),
            ("move_vertices", json!({"moves": "all"})),
            ("face_texture", json!({"scale": [0, 1]})),
            ("cordon", json!({"name": "c"})),
            ("tag", json!({"name": "none", "remove": true})),
            ("make_prefab_local", json!({})),
        ] {
            assert!(matches!(call(&mut s, name, &args), Err(e) if !matches!(e, EngineError::Panic(_))), "{name} {args}");
        }
        let huge = json!({"min": [0, 0, 0], "max": [1e308, 1e308, 1e308], "shape": "sphere", "sides": u64::MAX});
        assert!(!matches!(call(&mut s, "create_brush", &huge), Err(EngineError::Panic(_))));
    }
}
