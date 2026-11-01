# The `.mcpck` format

A `.mcpck` file carries the pictures and models a map uses that the game does not ship.
It sits beside the map as `<map name>.mcpck`, and the Godot addon reads it whenever it loads `<map name>.mcmap` from a file.
A version 3 pack can also carry the map itself, so one file is the whole level.
Packs travel through the Steam Workshop, so they come from strangers: the format is small, uncompressed, and checked before anything in it is used.

## Layout

Every integer is little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | Magic, the bytes `MCPK` |
| 4 | 4 | Version, `1`, `2` or `3` |
| 8 | 4 | Entry count |
| 12 | | Table of contents, one record per entry |

Each table of contents record is:

| Size | Field |
|---|---|
| 2 | Name length in bytes |
| name length | Name |
| 8 | Offset of the entry's data from the start of the file |
| 8 | Length of the entry's data |

The entries' data follows the table of contents back to back, in table order: the first starts right after the table, each starts where the one before it ends, and the last ends at the end of the file.
Nothing is compressed: PNG, JPEG and GLB are compressed already, and leaving out a decompressor leaves out decompression bombs.
MapCraft writes entries sorted by name, so the same entries always make the same file; readers accept any order.
Readers refuse versions other than the ones they know.

## Entries

| Name | Holds | Looked up by |
|---|---|---|
| `materials/<material>.png` or `materials/<material>.jpg` | The picture for a material | The material name a face uses |
| `models/<path>.glb` | A binary glTF model | A point entity's `model` value, `.glb` or `.gltf` |

- A face's material name is looked up as `materials/<name>`, ignoring ASCII case and the extension.
  A picture in the pack wins over the game's own material of that name, for this map only, and becomes a `StandardMaterial3D` with the picture, mipmapped, as its albedo.
- A `model` value is trimmed, its backslashes become slashes and a leading `res://` or `/` is dropped; when it then ends in `.glb` or `.gltf` in any case, `models/<path>.glb` is looked up ignoring ASCII case.
  `Props\Crate.GLB` and `props/crate.gltf` both find `models/Props/Crate.glb`.
  The exporter packs a `.gltf` and the buffers and pictures beside it as one `.glb`.
  A model in the pack wins over the game's file at that path, for this map only.
- A point entity's `animation` key names one of its model's animations, which the addon plays when the map loads: it copies that animation into an `AnimationLibrary` named `mapcraft` as `default` and sets the model's `AnimationPlayer` to autoplay `mapcraft/default`.
  Its `animation_loop` key sets the copy's loop mode: `linear` (the default), `pingpong` or `none`.
  The name is tried as written, then as Godot sanitizes animation names.
- Version 2 additionally supports `materials/<material>.json` PBR descriptors and `textures/<path>.png` or `.jpg` dependencies.
- Version 3 additionally supports `map.mcmap`, the exact bytes of a `.mcmap`, at the top level and at most once.
- Nothing else may be in a pack: no Godot resources (`.tres`, `.res`, `.scn`, `.tscn`, `.pck`), scripts, shaders, SVG, WebP, glTF text files or native libraries.

## Self-contained maps (version 3)

The writer selects version 3 only when `map.mcmap` is present, and versions 1 and 2 refuse that entry.
The map entry has the same 64 MiB cap as any entry, and MapCraft refuses one that is not JSON with `"format": "mapcraft.map"`.
The Godot addon builds a pack's map when it is given the `.mcpck` itself: `load("<name>.mcpck")` or a `MapCraftMap` node whose `map_file` is the pack.
It parses the map like a `.mcmap` read from a file, never through `ResourceLoader`, and that map takes pictures and models from the same pack instead of looking for a pack beside it.
A pack without a map, opened this way, builds nothing and logs that it holds no map.

## PBR descriptors (version 2)

The writer selects version 2 when a material descriptor or `textures/` entry is present; image/model-only exports remain version 1.
Both readers accept versions 1 to 3; version 1 refuses version 2 entry kinds.
A descriptor replaces the legacy picture for its material name, so those two entries cannot coexist.
A descriptor is UTF-8 JSON, at most 16 KiB.
Descriptor version 1 has these required fields and no others:

```json
{
  "version": 1,
  "albedo": [1.0, 1.0, 1.0, 1.0],
  "roughness": 1.0,
  "metallic": 0.0,
  "emission": [0.0, 0.0, 0.0],
  "textures": {"albedo": "textures/stone.png", "normal": "textures/stone_normal.png"}
}
```

Albedo components, roughness and metallic are finite values from 0 to 1; emission components are finite values from 0 to 64.
Texture slots are `albedo`, `normal`, `roughness`, `metallic` and `emission`; all are optional.

Descriptor version 2 adds these optional fields and the optional `height` texture slot:

| Field | Default | Range | Godot |
|---|---|---|---|
| `emission_energy` | 1 | 0 to 64 | `emission_energy_multiplier` |
| `normal_strength` | 1 | 0 to 16 | `normal_scale` |
| `height_scale` | 1 | -16 to 16 | `heightmap_scale` |
| `height_flip` | false | true or false | `heightmap_flip_texture`: the height texture is a depth map, white lowest |

A `height` texture turns on height mapping with deep parallax.
Emission is the emission color plus the emission texture, times `emission_energy`, in Godot and in Lit mode alike.
The editor writes version 1 unless a version 2 field differs from its default or a `height` texture is set, so older addons keep reading plain descriptors.
A version 1 descriptor with a version 2 field or slot is refused.
References must name PNG/JPEG entries under `textures/` and match an entry, ignoring ASCII case but including its extension.
Normal maps use OpenGL conventions; roughness and metallic maps use the red channel.
Albedo alpha below 1 enables alpha transparency.
Textures retain the existing image limits and shared pixel budget.
Descriptors never load through `ResourceLoader`.
Invalid descriptors or their texture dependencies produce a missing material and a diagnostic.

The editor's **Shader Editor > PBR Material** tab writes descriptors.
The Material inspector (**Inspect Material** on a material's right click menu in the Materials browser) edits a loaded descriptor in place: it validates the values, then writes the file atomically when the pointer is released, so a refused value never reaches the file.
Put descriptors in `materials/` and referenced images in `textures/` beneath a configured custom asset folder.
File > Add Material…, the Materials browser's + button or dropping pictures on the window makes a new descriptor from picked PBR maps, each assigned to a slot by its name's last word (`albedo`, `normal`, `rough`, `metal`, `glow`, `height`, `depth` and the like).
It and File > Add Model… copy into the folder beside the saved map (`<map name>/`): `materials/<name>.json`, `textures/<name>_<slot>.png` and `models/<path>.glb`, refusing a name already taken.
The pack exporter collects descriptors for face material names, including blend layers, and includes their referenced images once.
Custom descriptors appear in the material browser and map viewport; Lit mode uses their normal map at its strength, their roughness and metallic maps, and their emission at its energy; height maps show only in Godot.
Albedo tint is also applied to textured views and thumbnails.
Saving a descriptor from the editor reloads project assets.
Shader graphs are not yet bound to descriptors.

## Names

- A name is 1 to 255 bytes of `A` to `Z`, `a` to `z`, `0` to `9`, space, `_`, `-` and `.`, with `/` between path segments.
- No segment is empty, `.` or `..`, so a name has no leading or trailing slash and no `//`.
- A name starts with `materials/` and ends in `.png` or `.jpg`, or starts with `models/` and ends in `.glb`, with at least one character in between; extensions are lower case.
  Version 2 also permits the descriptor and texture names listed above.
- Names are unique ignoring ASCII case and the extension, so `materials/Rock.png` and `materials/rock.jpg` cannot share a pack.
- Backslashes, colons, NUL and everything outside ASCII are refused, so a name can never be a drive letter, an absolute path or a way out of the pack.
- Names are only ever lookup keys: nothing is written to disk under them.

## Pictures

- A PNG starts with the PNG signature followed by its `IHDR` chunk, which gives its width and height.
- A JPEG starts with `FF D8`, and its markers are walked to its frame header: baseline, extended and progressive frames (`SOF0` to `SOF2`) are allowed, lossless, hierarchical and arithmetic ones are refused, there is one frame, every segment ends inside the file, and there are at most 64 scans.
- The content must be the kind of picture the name's extension says.
- Width and height are each 1 to 4096, read from the header before anything decodes the picture.
- All the pictures a map decodes from its pack, including the ones inside models, add up to at most 67,108,864 pixels, four 4096 by 4096 pictures.

## Models

The GLB container:

- Starts with `glTF`, version 2, and the length in its header is the entry's length.
- Has its JSON chunk first, at most 2 MiB, and then at most a BIN chunk that ends exactly at the end of the entry.

Its JSON:

- Is an object nested at most 64 deep.
- Has at most one buffer, with no `uri` and a `byteLength` that fits in the BIN chunk.
- Has buffer views on buffer 0 that lie inside the buffer, with a `byteStride` of 4 to 252 when they have one.
- Has accessors that each name a buffer view, have no `sparse`, use one of glTF's six component types and the type `SCALAR`, `VEC2`, `VEC3`, `VEC4` or `MAT4`, hold 1 to 4,194,304 elements, and read only inside their buffer view.
- Has images with no `uri`, a buffer view, and a `mimeType` of `image/png` or `image/jpeg` that matches their bytes, which follow the picture rules above.
- Has mesh primitives with no morph targets, whose attributes and `indices` name accessors; counting an accessor's elements times its components each time a primitive uses it, all primitives add up to at most 16,777,216 values.
- Has nodes whose children exist, where no node has two parents and none is its own ancestor, nested at most 64 deep.
- Has skins whose joints are distinct nodes that exist, whose `skeleton` exists, and whose `inverseBindMatrices`, when present, is a `MAT4` accessor with one element per joint; a node's `skin` exists.
- Has animations whose channels target an existing node's `translation`, `rotation` or `scale` (not `weights`), through an existing sampler whose `interpolation` is `LINEAR`, `STEP` or `CUBICSPLINE`, whose input is a `SCALAR` accessor and whose output has 3 components (4 for `rotation`) and as many elements as the input has, three times as many for `CUBICSPLINE`.
  Counting each channel's input elements plus its output values, all channels add up to at most 16,777,216 values.
  The exporter drops morph targets, and the animation channels that drive them, before it writes a model.
- Has no `cameras`.
- Names only these extensions, in `extensionsUsed`, `extensionsRequired` and every `extensions` object anywhere: `KHR_materials_emissive_strength`, `KHR_materials_unlit`, `KHR_mesh_quantization` and `KHR_texture_transform`.
- May carry `extras`, which Godot keeps as `extras` metadata on the nodes and resources it makes.

## Limits

`crates/format/src/pack.rs` and `godot/src/mapcraft_pack.cpp` each keep these in one place, with the same values.

| Limit | Value |
|---|---|
| Pack size | 256 MiB |
| Entries | 1024 |
| Name length | 255 bytes |
| Entry size | 64 MiB |
| Picture width and height | 4096 |
| Pixels decoded per map | 67,108,864 |
| JPEG scans | 64 |
| GLB JSON chunk | 2 MiB |
| JSON nesting | 64 |
| glTF nodes | 4096 |
| glTF node nesting | 64 |
| glTF meshes | 1024 |
| glTF mesh primitives | 4096 |
| glTF accessors | 16,384 |
| glTF buffer views | 16,384 |
| glTF materials | 1024 |
| glTF textures | 1024 |
| glTF images | 256 |
| glTF buffers | 1 |
| glTF animations | 256 |
| glTF animation channels | 16,384 |
| glTF skins | 256 |
| Elements per accessor | 4,194,304 |
| Mesh values per model | 16,777,216 |
| Animation values per model | 16,777,216 |

## Reading a pack

- The Godot addon looks for `<map>.mcpck` whenever it builds a map from a file: `load()`, a `MapCraftMap` node, or an import.
  When there is none it reads `<map>.mcpack`, the name older MapCraft versions wrote; MapCraft itself always writes `.mcpck`.
- The pack's size is checked, then the whole file is read into memory once.
  Nothing is extracted to disk, nothing gets a `res://` or `user://` path, and nothing enters the resource cache, so a pack cannot shadow the game's files or reach another map.
- A problem with the header, the table of contents, an offset or length, or a name refuses the whole pack with an error in the log, and the map loads with the game's own materials and models.
- An entry's content is checked when the map first uses it.
  A picture that fails draws as a missing material, and a model that fails becomes an empty `Node3D`, each with a warning in the log; the rest of the pack is still used.
- Pictures decode only through Godot's PNG and JPEG buffer loaders.
  Models load through `GLTFDocument.append_from_buffer` with an empty base path and their pictures embedded uncompressed, then `generate_scene`; each model loads once per map and every entity using it gets a copy.

## Writing a pack

MapCraft's writer is `mapcraft_format::pack` in Rust:

- `write(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, PackError>` builds a pack from `(name, bytes)` entries.
- `save(path, entries)` writes one atomically, and `path_for(map)` gives the path beside a map.
- `material_entry(material, picture)` and `model_entry(model)` give the entry name the addon looks up.
- `read(bytes)` and `load(path)` read a pack back.

The writer runs every reader check on the bytes it made, including the content checks, so MapCraft never writes a pack the addon would refuse or drop an entry from.

## Threat model

### What an attacker controls

A Workshop item is a folder its author fills: the `.mcmap`, the `.mcpck` and any other file they like.
In the pack that means every byte: the header, the names, offsets and lengths, every picture header and pixel, and every byte of a model's JSON and binary data.

### What each check stops

- Size caps checked before anything is allocated stop a header from claiming a huge entry count, entry, pack or GLB JSON chunk; Godot's own GLB reader allocates the JSON chunk's length straight from the header.
- Entries stored back to back, with bounds checked by subtraction, stop overlapping entries, entries past the end and offsets that overflow when added to a length.
- The name allowlist stops path traversal, absolute paths, drive letters, NUL truncation and look-alike Unicode, though names never become paths anyway.
- The extension allowlist and the magic byte checks mean no Godot resource, scene or script ever loads from a pack, so nothing in it can run code, and no SVG or WebP decoder ever sees its bytes.
- The picture size caps and the pixel budget stop decompression bombs: a tiny file whose header claims a vast picture.
- The JPEG scan cap stops progressive JPEGs whose thousands of scans each make the decoder pass over the whole picture.
- Refusing `uri` on buffers and images, and loading with an empty base path, stop every file read: Godot reads a buffer `uri` as a file and loads an image `uri` through `ResourceLoader`, which can load a scene or a script.
- The glTF extension allowlist stops extensions Godot acts on without checking `extensionsUsed`, such as lights and node visibility, and ones that make physics bodies or decode other picture formats.
- Animation channels can only target the model's own nodes, so no track reaches outside it, and refusing cameras means no model can take the view.
- Matching each sampler's output count to its input count keeps Godot's track builder inside its arrays, and the animation budget caps what it decodes.
- The node check stops a parent loop, which makes Godot's glTF importer loop forever.
- The accessor caps and the mesh budget stop memory bombs: Godot decodes an accessor into arrays sized by its element count, a sparse accessor or one without a buffer view makes a zero-filled array from its count alone, and one accessor shared by many primitives is decoded once for each.
- Never registering pack content stops a pack from replacing the game's assets for other maps.

### What is still trusted

- Godot's libpng and libjpeg-turbo, on pictures that passed the header checks.
- Godot's glTF importer for everything the checks let through, including checking that indices, materials, textures and node meshes exist, and building materials and textures.
- Any `GLTFDocumentExtension` the game itself registers.
- Godot's JSON parser; the addon checks the same parse of the same text that the glTF importer makes.

### Beyond the pack

Godot runs a resource's scripts while it loads the resource: a `.tscn`, `.scn`, `.tres` or `.res` can carry a built-in script whose `_init` runs before any code can look at what was loaded.
So every string in a map that becomes a `ResourceLoader` path is part of the attack surface, not only the pack.

- Prefabs: the addon used to load a prefab's path through `ResourceLoader` wherever it pointed, so a map could name a `.tscn` with a built-in script sitting beside it, or plant `<prefab>.mcmap.remap` or `.import` to turn a `.mcmap` path into one.
  The addon now builds a prefab from its `.mcmap` file's bytes with the same builder, never through `ResourceLoader`, up to 16 prefabs deep and 256 prefab copies per map, and leaves out a prefab that contains itself.
- Names inside `res://`: Godot keeps a leading `..` in a `res://` path, and an exported game looks up a `res://` file its packs lack in its working folder, which Steam sets to the game's install folder, so a model path like `../../workshop/content/<app>/<item>/x.tscn` could reach a scene in a Workshop folder.
  Before it builds any path from a model path (less a leading `res://` or `/`), a material name or an SDK name, the addon refuses one with an empty, `.` or `..` segment, a segment ending in a dot or space, or a `\`, `:` or control character.
- The map itself: `load()` and `ResourceLoader` follow a `<map>.mcmap.remap` or `<map>.mcmap.import` planted beside the map to any resource, scripts included, before the addon is ever called.
  Games should build maps downloaded from players with a `MapCraftMap` node, which reads the file itself, rather than with `load()`.

### What the addon cannot close

Materials and models the map names by path still load through `ResourceLoader` from inside `res://`, which is only safe while everything in `res://` is the game's own.
A game that mounts a player's `.pck` into `res://` with `ProjectSettings.load_resource_pack` lets that pack add `.tscn`, `.tres`, `.res`, `.remap` and `.import` files there, and a map naming one of them runs its scripts; blocking `.gd` files in the pack does not help, since scripts can be built into scenes and resources.
SurfsUp's Workshop `.pck` support does this, so it stays a risk no check in the addon can remove.
Games should stop mounting packs from untrusted sources, build Workshop maps from bytes with a `MapCraftMap` node, and ship custom pictures and models in a `.mcpck`, which never registers anything in `res://`.
