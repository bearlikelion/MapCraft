struct Light {
    pos: vec4<f32>,
    color: vec4<f32>,
    spot: vec4<f32>,
    cone: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    ambient: vec4<f32>,
    ground: vec4<f32>,
    eye: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ground: vec4<f32>,
    minlight: vec4<f32>,
    paint: vec4<f32>,
    lights: array<Light, 64>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var orm_map: texture_2d<f32>;
@group(3) @binding(0) var tex_b: texture_2d<f32>;
@group(3) @binding(1) var samp_b: sampler;
@group(3) @binding(2) var normal_b: texture_2d<f32>;
@group(3) @binding(3) var orm_b: texture_2d<f32>;

const PI: f32 = 3.14159265;

override LAYERS: u32 = 1u;
override ANTI_TILE: u32 = 0u;

// Shared with mapcraft_geom::noise and the Godot addon's mapcraft_surface.gdshaderinc; change all three together.
const BLEND_DEPTH: f32 = 0.1;
const MACRO_CELLS: vec2<f32> = vec2<f32>(512.0, 197.0);
const MACRO_AMOUNT: f32 = 0.15;
const HEX_SCALE: f32 = 1.0;
const HEX_SHARPNESS: f32 = 7.0;
const HEX_CONTRAST: f32 = 0.6;
const PAINT_TINT: vec3<f32> = vec3<f32>(0.1, 0.9, 1.0);

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Heights are scaled by 1 - BLEND_DEPTH so weights 0 and 1 always give pure layers.
fn layer_share(w: f32, ha: f32, hb: f32) -> f32 {
    let a = ha * (1.0 - BLEND_DEPTH) + 1.0 - w;
    let b = hb * (1.0 - BLEND_DEPTH) + w;
    let m = max(a, b) - BLEND_DEPTH;
    let ba = max(a - m, 0.0);
    let bb = max(b - m, 0.0);
    return bb / (ba + bb);
}

fn mix32(x: u32) -> u32 {
    var h = x ^ (x >> 16u);
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    return h ^ (h >> 16u);
}

// The offset keeps lattice coordinates positive, so WGSL and Godot convert them the same way.
fn lattice(c: vec3<f32>) -> f32 {
    let u = vec3<u32>(vec3<i32>(c) + vec3<i32>(1048576));
    return f32(mix32(u.x ^ mix32(u.y ^ mix32(u.z))) >> 8u) / 16777216.0;
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let s = f * f * (3.0 - 2.0 * f);
    let x00 = mix(lattice(i), lattice(i + vec3<f32>(1.0, 0.0, 0.0)), s.x);
    let x10 = mix(lattice(i + vec3<f32>(0.0, 1.0, 0.0)), lattice(i + vec3<f32>(1.0, 1.0, 0.0)), s.x);
    let x01 = mix(lattice(i + vec3<f32>(0.0, 0.0, 1.0)), lattice(i + vec3<f32>(1.0, 0.0, 1.0)), s.x);
    let x11 = mix(lattice(i + vec3<f32>(0.0, 1.0, 1.0)), lattice(i + vec3<f32>(1.0, 1.0, 1.0)), s.x);
    return mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z);
}

fn macro_variation(world: vec3<f32>) -> f32 {
    let n = 0.6 * value_noise(world / MACRO_CELLS.x) + 0.4 * value_noise(world / MACRO_CELLS.y);
    return 1.0 + MACRO_AMOUNT * (2.0 * n - 1.0);
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn turned(turn: vec2<f32>, v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(turn.x * v.x - turn.y * v.y, turn.y * v.x + turn.x * v.y);
}

struct Hex {
    // A copy's texture coordinates in xy, and the cosine and sine of its turn in zw.
    t0: vec4<f32>,
    t1: vec4<f32>,
    t2: vec4<f32>,
    weight: vec3<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
    aspect: vec2<f32>,
};

fn hex_corner(q: vec2<f32>, id: vec2<f32>, aspect: vec2<f32>) -> vec4<f32> {
    let u = vec2<u32>(vec2<i32>(id) + vec2<i32>(1048576));
    let a = mix32(u.x ^ mix32(u.y));
    let b = mix32(a);
    let angle = unit(a) * 2.0 * PI;
    let turn = vec2<f32>(cos(angle), sin(angle));
    let center = vec2<f32>(id.x + id.y * 0.5, id.y * 0.8660254) / HEX_SCALE;
    return vec4<f32>((turned(turn, q - center) + center) / aspect + vec2<f32>(unit(b), unit(mix32(b))), turn);
}

// Hex tiling from Mikkelsen, "Practical Real-Time Hex-Tiling" (JCGT 2022).
fn hex_tiles(uv: vec2<f32>, size: vec2<f32>) -> Hex {
    // Square texels with a repeat's area, so turns stay rigid on wide textures and a hex is about one repeat.
    let aspect = sqrt(size / size.yx);
    let q = uv * aspect;
    let p = q * HEX_SCALE;
    // Skewed, each triangle of the grid is half a unit square, the upper half when x + y > 1.
    let s = vec2<f32>(p.x - p.y * 0.57735027, p.y * 1.15470054);
    let cell = floor(s);
    let f = s - cell;
    let up = select(0.0, 1.0, f.x + f.y > 1.0);
    let weight = vec3<f32>(abs(1.0 - f.x - f.y), mix(f, 1.0 - f, up));
    let t0 = hex_corner(q, cell + vec2<f32>(up, up), aspect);
    let t1 = hex_corner(q, cell + vec2<f32>(1.0 - up, up), aspect);
    let t2 = hex_corner(q, cell + vec2<f32>(up, 1.0 - up), aspect);
    return Hex(t0, t1, t2, weight, dpdx(q), dpdy(q), aspect);
}

// Derivatives turn with the copy, so mip levels and anisotropy match across tile edges.
fn hex_tap(t: texture_2d<f32>, s: sampler, h: Hex, c: vec4<f32>) -> vec4<f32> {
    return textureSampleGrad(t, s, c.xy, turned(c.zw, h.dx) / h.aspect, turned(c.zw, h.dy) / h.aspect);
}

// Green points up the picture, against v, so turning xy by the copy's own turn undoes it on the face.
fn hex_normal(t: texture_2d<f32>, s: sampler, h: Hex, c: vec4<f32>) -> vec3<f32> {
    let n = hex_tap(t, s, h, c).xyz * 2.0 - 1.0;
    return vec3<f32>(turned(c.zw, n.xy), n.z);
}

struct Layer {
    color: vec4<f32>,
    hex: Hex,
};

fn layer(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> Layer {
    var l: Layer;
    if ANTI_TILE != 2u {
        l.color = textureSample(t, s, uv);
        return l;
    }
    l.hex = hex_tiles(uv, vec2<f32>(textureDimensions(t, 0)));
    let c0 = hex_tap(t, s, l.hex, l.hex.t0);
    let c1 = hex_tap(t, s, l.hex, l.hex.t1);
    let c2 = hex_tap(t, s, l.hex, l.hex.t2);
    // Brighter copies win, so one tile shows at a time and edges stay crisp.
    let w = mix(vec3<f32>(1.0), vec3<f32>(luma(c0.rgb), luma(c1.rgb), luma(c2.rgb)), HEX_CONTRAST) * pow(l.hex.weight, vec3<f32>(HEX_SHARPNESS));
    l.hex.weight = w / (w.x + w.y + w.z);
    l.color = c0 * l.hex.weight.x + c1 * l.hex.weight.y + c2 * l.hex.weight.z;
    return l;
}

struct Maps {
    orm: vec3<f32>,
    bump: vec3<f32>,
};

fn layer_maps(normal: texture_2d<f32>, orm: texture_2d<f32>, s: sampler, uv: vec2<f32>, h: Hex) -> Maps {
    if ANTI_TILE != 2u {
        return Maps(textureSample(orm, s, uv).rgb, textureSample(normal, s, uv).xyz * 2.0 - 1.0);
    }
    let w = h.weight;
    let bump = hex_normal(normal, s, h, h.t0) * w.x + hex_normal(normal, s, h, h.t1) * w.y + hex_normal(normal, s, h, h.t2) * w.z;
    let heaviest = select(select(h.t2, h.t1, w.y >= w.z), h.t0, w.x >= max(w.y, w.z));
    return Maps(hex_tap(orm, s, h, heaviest).rgb, bump);
}

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) tangent: vec4<f32>,
    @location(5) blend_uv: vec2<f32>,
    @location(6) blend: f32,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) world: vec3<f32>,
    @location(4) tangent: vec4<f32>,
    @location(5) blend_uv: vec2<f32>,
    @location(6) blend: f32,
};

// Tinted albedo with the second layer and variation, the second layer's share of it, and each layer's hex tiles.
struct Albedo {
    color: vec4<f32>,
    share: f32,
    hex_a: Hex,
    hex_b: Hex,
};

fn albedo(i: MeshOut) -> Albedo {
    var out: Albedo;
    let a = layer(tex, samp, i.uv);
    var c = a.color;
    out.hex_a = a.hex;
    if LAYERS == 2u {
        let b = layer(tex_b, samp_b, i.blend_uv);
        out.share = layer_share(i.blend, luma(c.rgb), luma(b.color.rgb));
        out.hex_b = b.hex;
        c = mix(c, b.color, out.share);
    }
    if ANTI_TILE > 0u {
        c = vec4<f32>(c.rgb * macro_variation(i.world), c.a);
    }
    out.color = c * i.tint;
    return out;
}

// The weight overlay goes on last, so it reads the same in every render mode.
fn paint_overlay(c: vec3<f32>, i: MeshOut) -> vec3<f32> {
    return mix(c, PAINT_TINT, i.blend * globals.paint.x);
}

@vertex
fn vs_mesh(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    o.clip = globals.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.uv = v.uv;
    o.tint = v.tint;
    o.world = v.pos;
    o.tangent = v.tangent;
    o.blend_uv = v.blend_uv;
    o.blend = v.blend;
    return o;
}

@fragment
fn fs_mesh(i: MeshOut) -> @location(0) vec4<f32> {
    let shade = 0.6 + 0.4 * abs(dot(normalize(i.normal), globals.light_dir.xyz));
    let c = albedo(i).color;
    return vec4<f32>(paint_overlay(c.rgb * shade, i), c.a);
}

// Ambient light from the sky above and the ground below.
fn hemisphere(d: vec3<f32>) -> vec3<f32> {
    return mix(globals.ground.rgb, globals.ambient.rgb, d.z * 0.5 + 0.5);
}

// Karis' analytic fit of the split sum environment BRDF, for ambient specular without a probe.
fn env_brdf(f0: vec3<f32>, rough: f32, nv: f32) -> vec3<f32> {
    let r = rough * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    return f0 * (a * -1.04 + r.z) + (a * 1.04 + r.w);
}

@fragment
fn fs_lit(i: MeshOut) -> @location(0) vec4<f32> {
    let a = albedo(i);
    let c = a.color;
    let maps = layer_maps(normal_map, orm_map, samp, i.uv, a.hex_a);
    var orm = maps.orm;
    var bump = maps.bump;
    if LAYERS == 2u {
        let b = layer_maps(normal_b, orm_b, samp_b, i.blend_uv, a.hex_b);
        orm = mix(orm, b.orm, a.share);
        bump = normalize(mix(bump, b.bump, a.share));
    }
    let g = normalize(i.normal);
    let t = normalize(i.tangent.xyz - g * dot(g, i.tangent.xyz));
    let n = normalize(t * bump.x + cross(g, t) * i.tangent.w * bump.y + g * max(bump.z, 0.0));
    let v = normalize(globals.eye.xyz - i.world);
    let nv = max(dot(n, v), 1e-4);
    // Rougher where the normal turns faster than a pixel can show, so minified normal maps do not sparkle.
    let dn = fwidth(n);
    let rough = clamp(sqrt(sqrt(orm.g * orm.g * orm.g * orm.g + min(2.0 * dot(dn, dn), 0.2))), 0.05, 1.0);
    let a2 = rough * rough * rough * rough;
    let kg = (rough + 1.0) * (rough + 1.0) / 8.0;
    let f0 = mix(vec3<f32>(0.04), c.rgb, orm.b);
    let diffuse = c.rgb * (1.0 - orm.b);
    var sum = vec3<f32>(0.0);
    // Quake lights add up in gamma space as a lightmap does, where 1 shows the texture as is.
    var lightmap = vec3<f32>(0.0);
    for (var k = 0u; k < u32(globals.ambient.w); k++) {
        let l = globals.lights[k];
        let quake = l.cone.w > 0.0;
        var dir = l.pos.xyz;
        var atten = 1.0;
        if l.pos.w > 0.0 {
            let d = max(distance(l.pos.xyz, i.world), 1.0);
            dir = (l.pos.xyz - i.world) / d;
            let r = l.spot.w;
            switch i32(l.color.w) {
                case 0: { atten = max(1.0 - d / r, 0.0); }
                case 1: { atten = r / d; }
                case 2: { atten = r * r / (d * d); }
                case 5: { atten = r * r / ((d + r) * (d + r)); }
                default: {}
            }
            if !quake {
                atten = min(atten, 1.0);
            }
            if any(l.spot.xyz != vec3<f32>(0.0)) {
                atten *= smoothstep(l.cone.x, l.cone.y, dot(-dir, l.spot.xyz));
            }
        }
        let nl = dot(n, dir);
        if nl <= 0.0 {
            continue;
        }
        if quake {
            lightmap += l.color.rgb * atten * (1.0 - l.cone.z + l.cone.z * nl);
            continue;
        }
        // GGX distribution, Schlick-GGX geometry and Schlick Fresnel; light colors already carry the factor of pi.
        let h = normalize(dir + v);
        let nh = max(dot(n, h), 0.0);
        let f = f0 + (1.0 - f0) * pow(1.0 - max(dot(v, h), 0.0), 5.0);
        let d = a2 / (PI * pow(nh * nh * (a2 - 1.0) + 1.0, 2.0));
        let vis = 0.25 / ((nv * (1.0 - kg) + kg) * (nl * (1.0 - kg) + kg));
        sum += l.color.rgb * atten * nl * ((1.0 - f) * diffuse + PI * d * vis * f);
    }
    let r = normalize(mix(reflect(-v, n), n, rough * rough));
    let ambient = (diffuse * hemisphere(n) + env_brdf(f0, rough, nv) * hemisphere(r)) * orm.r;
    // A lightmap byte tops out at twice the texture's brightness.
    let lightmapped = diffuse * pow(clamp(max(lightmap, globals.minlight.rgb), vec3<f32>(0.0), vec3<f32>(2.0)), vec3<f32>(2.2));
    let color = max(sum, vec3<f32>(0.0)) + lightmapped + ambient;
    return vec4<f32>(paint_overlay(select(color, filmic(color), globals.sky_top.w > 0.0), i), c.a);
}

// Godot's filmic tonemapper at white 1, which the MapCraft addon gives its environments.
fn filmic(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    let t = (x * (0.88 * x + 0.06) + 0.002) / (x * (0.88 * x + 0.6) + 0.06) - 0.01 / 0.3;
    return t / 0.578355;
}

@group(2) @binding(0) var lightmap_tex: texture_2d<f32>;
@group(2) @binding(1) var lightmap_samp: sampler;

struct BakedIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) lightmap: vec2<f32>,
};

// MeshOut's fields first, so the sky face shader reads a baked face too.
struct BakedOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) world: vec3<f32>,
    @location(4) tangent: vec4<f32>,
    @location(5) lightmap: vec2<f32>,
    @location(6) blend: f32,
};

@vertex
fn vs_baked(v: BakedIn) -> BakedOut {
    var o: BakedOut;
    o.clip = globals.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = vec3<f32>(0.0, 0.0, 1.0);
    o.uv = v.uv;
    o.tint = vec4<f32>(1.0);
    o.world = v.pos;
    o.tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    o.lightmap = v.lightmap;
    o.blend = 0.0;
    return o;
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// Quake engines multiply in sRGB at twice the lightmap, clamp, and cut fence textures where alpha is low.
@fragment
fn fs_baked(i: BakedOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    let light = textureSample(lightmap_tex, lightmap_samp, i.lightmap).rgb * 2.0;
    if c.a < 0.5 {
        discard;
    }
    return vec4<f32>(to_linear(min(to_srgb(c.rgb) * light, vec3<f32>(1.0))), 1.0);
}

@fragment
fn fs_baked_unlit(i: BakedOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(tex, samp, i.uv).rgb, 1.0);
}

// Quake's sky on a face: the view direction flattened toward a dome picks the spot, and the near
// layer's clear pixels show the far layer behind it.
@fragment
fn fs_sky_face(i: MeshOut) -> @location(0) vec4<f32> {
    var d = i.world - globals.eye.xyz;
    d.z = d.z * 3.0;
    let uv = normalize(d).xy * (189.0 / 64.0);
    let far = textureSample(tex, samp, uv);
    let near = textureSample(normal_map, samp, uv);
    return vec4<f32>(mix(far.rgb, near.rgb, near.a), 1.0);
}

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0);
    var o: SkyOut;
    o.clip = vec4<f32>(p, 1.0, 1.0);
    o.ndc = p;
    return o;
}

// Godot's ProceduralSkyMaterial at its default curves, with the first sun's glow.
@fragment
fn fs_sky(i: SkyOut) -> @location(0) vec4<f32> {
    let p = globals.inv_view_proj * vec4<f32>(i.ndc, 0.5, 1.0);
    let d = normalize(p.xyz / p.w - globals.eye.xyz);
    let v = acos(clamp(d.z, -1.0, 1.0));
    var sky = mix(globals.sky_horizon.rgb, globals.sky_top.rgb, clamp(1.0 - pow(1.0 - (1.0 - v / (PI * 0.5)), 1.0 / 0.15), 0.0, 1.0));
    let sun = globals.lights[0];
    if globals.ambient.w > 0.0 && sun.pos.w == 0.0 {
        let s = acos(clamp(dot(sun.pos.xyz, d), -1.0, 1.0)) / radians(30.0);
        if s < 1.0 {
            sky = mix(sun.color.rgb, sky, clamp(1.0 - pow(1.0 - s, 1.0 / 0.15), 0.0, 1.0));
        }
    }
    let ground = mix(globals.sky_horizon.rgb, globals.sky_ground.rgb, clamp(1.0 - pow(1.0 - (v - PI * 0.5) / (PI * 0.5), 1.0 / 0.02), 0.0, 1.0));
    return vec4<f32>(filmic(select(ground, sky, d.z >= 0.0)), 1.0);
}

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = globals.view_proj * vec4<f32>(v.pos, 1.0);
    // Pull edges slightly toward the camera so they win the reversed depth test against their own faces.
    o.clip.z = min(o.clip.z * 1.0005 + 0.000001 * o.clip.w, o.clip.w);
    o.color = v.color;
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    return i.color;
}
