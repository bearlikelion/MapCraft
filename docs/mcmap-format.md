# The `.mcmap` format

A `.mcmap` file is UTF-8 JSON.
This page is the reference for anyone writing an importer (the Godot extension, a Unity importer, game tools).

## Envelope

```json
{
  "format": "mapcraft.map",
  "schema_version": 1,
  "generator": "MapCraft 0.1.0",
  "sdk": "surfsup",
  "map": { "root": { ... }, "tags": [ ... ], "cordons": [ ... ], "extra": { ... } },
  "view": { ... }
}
```

- `format` is always `mapcraft.map`.
- `schema_version` is an integer; readers refuse versions newer than they know.
- `sdk` is the id of the SDK the map was made with and may be missing.
- `view` holds editor state (cameras, layout) and is not part of the map.
- `tags` is a list of `{id, name, color, parent, hidden}`; nodes list the tag ids they carry.
- `cordons` is a list of `{name, bounds: {min, max}, active}` boxes; while one is active, compilers and exporters should keep only what touches an active cordon.
- Unknown keys must be ignored, so newer writers can add data without breaking older readers.

## Nodes

`map.root` is a tree of nodes.
Every node has an `id` (unique integer), a `type`, and optionally `children`, `tags`, `hidden`, `locked`, `extra` and `derived`.

| `type` | Fields | Children |
|---|---|---|
| `world` | `classname` (`worldspawn`), `properties` | anything except `world` |
| `layer` | `name`, `color`, `omit_from_export` | groups, entities, brushes, prefabs |
| `group` | `name`, `generator`, `mirror` | groups, entities, brushes, prefabs |
| `entity` | `classname`, `properties`, `connections` | brushes (a brush entity) or none (a point entity) |
| `brush` | `faces` | none |
| `prefab` | `path`, `origin`, `angles`, `overrides` | none |

`properties` is a list of `[key, value]` string pairs in file order; keys can repeat in imported data.
`connections` is a list of `{output, target, input, param, delay, times_to_fire}`; `times_to_fire` of -1 means every time.
A `generator` (`{plugin, command, params, origin, angles}`) marks a group whose children were produced by a plugin; importers treat them as ordinary children.
A prefab's `path` is relative to the map that contains it; `angles` are pitch, yaw and roll in degrees.

## Mirror groups

A group with a `mirror` (`{axis, offset, texture_lock}`) is kept symmetric: its children are the edited half, and MapCraft copies them across the plane where coordinate `axis` (`"x"`, `"y"` or `"z"`) equals `offset`.
The writer emits that copy as ordinary children after the originals, each subtree flagged `"derived": true`, with ids and face ids numbered after the map's own.
Importers need not know about mirroring: they treat derived nodes like any others and get the whole symmetric map.
MapCraft drops derived nodes when it reads a file and builds them again from the originals.

How the copy is made:

- Brushes are reflected exactly, so a brush that touches the plane meets its copy with no gap.
  A brush that crosses the plane adds only the part of its reflection it does not already fill, so nothing overlaps and a brush centered on the plane adds nothing.
- With `texture_lock` the copies' textures are mirrored with them; without it they keep the originals' texture axes.
- Point entities get the mirrored `origin`, and their `angles` (or `angle`) turn to face as the reflection does: yaw is reflected across an X or Y plane, and pitch is negated across a Z plane.
  Point entities and prefab instances lying on the plane are not copied, so a light or spawn on the middle line is not doubled.
- Prefab instances are copied as instances at the mirrored origin with mirrored angles; their contents are turned, not flipped.
- A mirror group inside another one is copied as a mirror group across the reflected plane, with its own derived half.

## Faces

```json
{
  "id": 7,
  "points": [[256, 256, -16], [256, -256, -16], [256, -256, 240]],
  "material": "ramps-orange1",
  "u": {"axis": [0, 1, 0], "offset": 0, "scale": 1},
  "v": {"axis": [0, 0, -1], "offset": 0, "scale": 1},
  "rotation": 0,
  "polygon": [[256, 256, -16], [256, 256, 240], [256, -256, 240], [256, -256, -16]]
}
```

- `points` define the face plane. Seen from outside the brush they run clockwise, so the outward normal is `normalize((p0 - p1) x (p2 - p1))`.
- `polygon` is the face's convex polygon, counter-clockwise seen from outside (so it is front-facing for a right-handed, counter-clockwise renderer). It is written by MapCraft for importers and ignored when MapCraft reads the file. Faces of invalid brushes have no polygon.
- Texture coordinates in texels: `u = dot(p, u.axis) / u.scale + u.offset`, likewise for `v`. Divide by the texture's width and height for normalized UVs. This is the Valve220 and VMF convention.
- `rotation` is informational; the axes already include it.
- `attrs` holds per-face game attributes such as `lightmapscale` or `smoothing_groups` as strings.
- Face `id`s are unique within a map and stable across saves.

## Displacements

A face with four corners may carry a `disp`: Hammer-style displacement that turns it into a grid of raised vertices.

```json
"disp": {"power": 2, "start": [1024, -128, 0], "heights": [0, 0, 0, 0, 0, 0, 8, 16, 8, 0, ...]},
"disp_vertices": [[1024, -128, 0], [1088, -128, 0], ...]
```

- `power` is 2 to 4; the grid has `2^power + 1` vertices on each side.
- `start` picks the corner the grid starts at: the polygon corner nearest it.
- Vertex `(i, j)` is entry `j * size + i`: `i` steps from the start corner toward the next corner counter-clockwise from outside, and `j` steps from the start corner toward the previous one.
- `heights` has one value per vertex in that order, measured along the face's outward normal.
- `disp_vertices` is the displaced grid in map space, row by row, written for importers and ignored on read. Triangulate cell `(i, j)` with corners `a = (i, j)`, `b = (i + 1, j)`, `c = (i + 1, j + 1)`, `d = (i, j + 1)` as `a b c` and `a c d` when `i + j` is even, else `a b d` and `b c d`; these are counter-clockwise from outside.
- A brush with any displaced face counts only as its displaced faces, as in Source: importers draw and collide with the displaced surfaces and skip the brush's other faces. Collide with them as triangles, not as a convex hull.

## Units and axes

Map space is right-handed with Z up.
One unit is one inch; there are 39.37 units per meter.
Godot importers convert a point with `(x, y, z) -> (x, z, -y) / 39.37`.
