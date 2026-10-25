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
  "map": { "root": { ... }, "tags": [ ... ], "extra": { ... } },
  "view": { ... }
}
```

- `format` is always `mapcraft.map`.
- `schema_version` is an integer; readers refuse versions newer than they know.
- `sdk` is the id of the SDK the map was made with and may be missing.
- `view` holds editor state (cameras, layout) and is not part of the map.
- Unknown keys must be ignored, so newer writers can add data without breaking older readers.

## Nodes

`map.root` is a tree of nodes.
Every node has an `id` (unique integer), a `type`, and optionally `children`, `tags`, `hidden`, `locked` and `extra`.

| `type` | Fields | Children |
|---|---|---|
| `world` | `classname` (`worldspawn`), `properties` | anything except `world` |
| `layer` | `name`, `color`, `omit_from_export` | groups, entities, brushes, prefabs |
| `group` | `name`, `generator` | groups, entities, brushes, prefabs |
| `entity` | `classname`, `properties`, `connections` | brushes (a brush entity) or none (a point entity) |
| `brush` | `faces` | none |
| `prefab` | `path`, `origin`, `angles`, `overrides` | none |

`properties` is a list of `[key, value]` string pairs in file order; keys can repeat in imported data.
`connections` is a list of `{output, target, input, param, delay, times_to_fire}`; `times_to_fire` of -1 means every time.
A `generator` (`{plugin, command, params}`) marks a group whose children were produced by a plugin; importers treat them as ordinary children.
A prefab's `path` is relative to the map that contains it; `angles` are pitch, yaw and roll in degrees.

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

## Units and axes

Map space is right-handed with Z up.
One unit is one inch; there are 39.37 units per meter.
Godot importers convert a point with `(x, y, z) -> (x, z, -y) / 39.37`.
