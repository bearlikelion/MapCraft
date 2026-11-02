# The .mclight lightmap format

A `.mclight` file holds a map's baked lighting, made by MapCraft's baker (`crates/light`, `mapcraft-cli bake`).
It sits beside the `.mcmap` it was baked from and goes into the map's `.mcpck` package.
Each drawn brush face has its own rectangle of luxels in an atlas, stored the way Godot's directional (spherical harmonics) lightmaps store them, so an importer can hand the texels to `LightmapGIData` unchanged.

All numbers are little endian.

## Header

| Bytes | Type | Field |
|---|---|---|
| 4 | `MCLM` | Magic |
| 4 | u32 | Version, currently 2; readers refuse any other, since version 1 stored the L1 bands along the map's axes |
| 8 | u64 | Fingerprint of the map it was baked from |
| 4 | u32 | Slice size: slices are this many texels square, at most 8192 |
| 4 | u32 | Slice count, at most 64 |
| 4 | u32 | Face count, at most 4,000,000 |

The fingerprint is FNV-1a over every brush's node id, face points, material and `lightmapscale`, and the key/values of worldspawn and every `light*` entity.
The baker then adds the settings it baked at: luxel size, slice size, bounces, rays and sun samples.
A map whose fingerprint differs, or one baked again at other settings, has changed since the bake.

## Face table

One 48-byte entry per face that has a lightmap:

| Bytes | Type | Field |
|---|---|---|
| 8 | u64 | Brush node id |
| 4 | u32 | Face index in the brush |
| 4 | u32 | Slice |
| 32 | 8 x f32 | Two rows `(a, b, c, d)` taking a world point to its UV in the slice |

A vertex at map position `(x, y, z)` gets `u = a x + b y + c z + d` from the first row and `v` from the second, both from 0 to 1 across the slice.
A face whose light is the same everywhere has rows `(0, 0, 0, d)`, pointing every vertex at the middle of one texel.
Faces that are hidden between brushes, tool faces and displacements have no entry.

## Texels

An 8-byte u64 gives the length of the zlib stream that follows (what Godot's `PackedByteArray.decompress` with `COMPRESSION_DEFLATE` reads).
It inflates to `slice count x 4 x size x size` u32 texels: for each slice, four layers, each row by row from the top.

Each texel is RGBE9995: red in bits 0 to 8, green 9 to 17, blue 18 to 26, and a shared exponent in bits 27 to 31, worth `mantissa x 2^(exponent - 24)` (Godot's `Image.FORMAT_RGBE9995`).

The four layers of a slice are Godot's L1 spherical harmonics, in Godot's order:

1. L0: the light as is, linear.
2. L1 along Y, 3. along Z, 4. along X: each band divided by 8 times L0 and shifted by 0.5 into 0 to 1.

The axes are Godot's, Y up, as the importer places the map: Godot's X is the map's X, its Y the map's Z (up), and its Z the map's -Y.

Godot shows a face with normal `n` (in those axes) as `L0 + 4 L0 (y' n.y + z' n.z + x' n.x)`, where `y'` is `(layer 2 - 0.5) x 2` and so on, before multiplying by the surface's albedo.
Values are in Godot's light units: a white face square to a sun of energy 1 reads 1.
