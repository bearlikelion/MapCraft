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
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    fog_depth: vec4<f32>,
    fog_height: vec4<f32>,
    tonemap: vec4<f32>,
    adjustments: vec4<f32>,
    sky_shape: vec4<f32>,
    minlight: vec4<f32>,
    paint: vec4<f32>,
    lights: array<Light, 64>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
struct MaterialParameters { tint: vec4<f32>, emission: vec4<f32>, surface: vec4<f32>, };

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var orm_map: texture_2d<f32>;
@group(1) @binding(4) var emission_map: texture_2d<f32>;
@group(1) @binding(5) var<uniform> material: MaterialParameters;
@group(3) @binding(0) var tex_b: texture_2d<f32>;
@group(3) @binding(1) var samp_b: sampler;
@group(3) @binding(2) var normal_b: texture_2d<f32>;
@group(3) @binding(3) var orm_b: texture_2d<f32>;
@group(3) @binding(4) var emission_b: texture_2d<f32>;
@group(3) @binding(5) var<uniform> material_b: MaterialParameters;

const PI: f32 = 3.14159265;

override LAYERS: u32 = 1u;
override ANTI_TILE: u32 = 0u;
// See-through surfaces show only faces turned toward the camera, as Godot draws them, so a glass brush's far side stays hidden.
override CULL_BACK: bool = false;

fn culled(i: MeshOut) -> bool {
    return CULL_BACK && dot(i.normal, globals.eye.xyz - i.world) < 0.0;
}

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

// Scales a tangent-space normal's tilt by x and the roughness and metallic maps by y and z.
fn adjusted(m: Maps, surface: vec4<f32>) -> Maps {
    var bump = m.bump;
    if surface.x != 1.0 {
        let xy = bump.xy * surface.x;
        bump = normalize(vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0))));
    }
    return Maps(m.orm * vec3<f32>(1.0, surface.y, surface.z), bump);
}

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

fn varied(c: vec4<f32>, world: vec3<f32>) -> vec4<f32> {
    if ANTI_TILE > 0u {
        return vec4<f32>(c.rgb * macro_variation(world), c.a);
    }
    return c;
}

fn albedo(i: MeshOut) -> Albedo {
    var out: Albedo;
    let a = layer(tex, samp, i.uv);
    var c = a.color * material.tint;
    out.hex_a = a.hex;
    if LAYERS == 2u {
        let b = layer(tex_b, samp_b, i.blend_uv);
        out.share = layer_share(i.blend, luma(c.rgb), luma(b.color.rgb));
        out.hex_b = b.hex;
        c = mix(c, b.color * material_b.tint, out.share);
    }
    out.color = varied(c, i.world) * i.tint;
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
    if culled(i) {
        discard;
    }
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

// The direction toward a light from `world`, and its falloff in w.
fn light_reach(l: Light, world: vec3<f32>) -> vec4<f32> {
    var dir = l.pos.xyz;
    var atten = 1.0;
    if l.pos.w > 0.0 {
        let d = max(distance(l.pos.xyz, world), 1.0);
        dir = (l.pos.xyz - world) / d;
        let r = l.spot.w;
        switch i32(l.color.w) {
            case 0: { atten = max(1.0 - d / r, 0.0); }
            case 1: { atten = r / d; }
            case 2: { atten = r * r / (d * d); }
            case 5: { atten = r * r / ((d + r) * (d + r)); }
            default: {}
        }
        if l.cone.w <= 0.0 {
            atten = min(atten, 1.0);
        }
        if any(l.spot.xyz != vec3<f32>(0.0)) {
            atten *= smoothstep(l.cone.x, l.cone.y, dot(-dir, l.spot.xyz));
        }
    }
    return vec4<f32>(dir, atten);
}

@fragment
fn fs_lit(i: MeshOut) -> @location(0) vec4<f32> {
    if culled(i) {
        discard;
    }
    let a = albedo(i);
    let c = a.color;
    let maps = adjusted(layer_maps(normal_map, orm_map, samp, i.uv, a.hex_a), material.surface);
    var orm = maps.orm;
    var bump = maps.bump;
    if LAYERS == 2u {
        let b = adjusted(layer_maps(normal_b, orm_b, samp_b, i.blend_uv, a.hex_b), material_b.surface);
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
        let reach = light_reach(l, i.world);
        let dir = reach.xyz;
        let atten = reach.w;
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
    var emitted = (layer(emission_map, samp, i.uv).color.rgb + material.emission.rgb) * material.emission.a;
    if LAYERS == 2u {
        emitted = mix(emitted, (layer(emission_b, samp_b, i.blend_uv).color.rgb + material_b.emission.rgb) * material_b.emission.a, a.share);
    }
    let color = max(sum, vec3<f32>(0.0)) + lightmapped + ambient + emitted;
    return vec4<f32>(paint_overlay(environment_color(color, i.world), i), c.a);
}

// Godot's filmic tonemapper at white 1, which the MapCraft addon gives its environments.
fn filmic(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    let t = (x * (0.88 * x + 0.06) + 0.002) / (x * (0.88 * x + 0.6) + 0.06) - 0.01 / 0.3;
    return t / 0.578355;
}

fn tone_adjust(c: vec3<f32>) -> vec3<f32> {
    let x = max(c * globals.tonemap.y, vec3<f32>(0.0));
    let white = max(globals.tonemap.z, 0.001);
    var result = x;
    switch u32(globals.tonemap.x) {
        case 1u: { result = x * (1.0 + x / (white * white)) / (1.0 + x); }
        case 2u: { result = filmic(x) / max(filmic(vec3<f32>(white)), vec3<f32>(0.0001)); }
        case 3u: {
            result = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
        }
        default: {}
    }
    // Adjustments operate after tonemapping in display space.
    var display = to_srgb(max(result, vec3<f32>(0.0))) * globals.adjustments.x;
    display = mix(vec3<f32>(0.5), display, globals.adjustments.y);
    display = mix(vec3<f32>(luma(display)), display, globals.adjustments.z);
    return to_linear(max(display, vec3<f32>(0.0)));
}

fn fog_light(direction: vec3<f32>) -> vec3<f32> {
    var color = globals.fog_color.rgb * globals.fog_color.w;
    // Godot blends toward its blurred sky; the procedural sky's gradient stands in for it.
    let sky = mix(globals.sky_horizon.rgb, globals.sky_top.rgb, clamp(direction.z, 0.0, 1.0)) * globals.sky_shape.z;
    color = mix(color, sky, globals.fog_depth.w);
    if globals.ambient.w > 0.0 && globals.lights[0].pos.w == 0.0 {
        color += globals.lights[0].color.rgb * pow(max(dot(direction, globals.lights[0].pos.xyz), 0.0), 8.0) * globals.fog.y;
    }
    return color;
}

fn environment_color(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if globals.sky_top.w < 0.5 { return c; }
    let delta = world - globals.eye.xyz;
    let distance = length(delta);
    let direction = delta / max(distance, 0.0001);
    // Map space uses inches; Godot's fog distances and densities are in meters.
    let meters = distance / 39.37;
    var amount = 1.0 - exp(-meters * globals.fog.x);
    if globals.fog.w > 0.5 {
        let end = select(4000.0, globals.fog_depth.z, globals.fog_depth.z > 0.0);
        amount = pow(smoothstep(min(globals.fog_depth.y, end - 0.001), end, meters), globals.fog_depth.x) * globals.fog.x;
    }
    if abs(globals.fog_height.y) >= 0.0001 {
        amount = max(amount, 1.0 - exp(min(0.0, (world.z / 39.37 - globals.fog_height.x) * globals.fog_height.y)));
    }
    return tone_adjust(mix(c, fog_light(direction), clamp(amount, 0.0, 1.0)));
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
struct BakedSurface {
    orm: vec3<f32>,
    face: vec3<f32>,
    n: vec3<f32>,
    // How much brighter the live lights find the normal map than the flat face the bake lit.
    relief: f32,
};

// Baked faces carry no normals or tangents, so the frame comes from screen derivatives.
fn baked_surface(i: BakedOut, h: Hex) -> BakedSurface {
    var s: BakedSurface;
    let maps = adjusted(layer_maps(normal_map, orm_map, samp, i.uv, h), material.surface);
    s.orm = maps.orm;
    let dx = dpdx(i.world);
    let dy = dpdy(i.world);
    let raw = normalize(cross(dx, dy));
    let g = select(raw, -raw, dot(raw, globals.eye.xyz - i.world) < 0.0);
    let ux = dpdx(i.uv);
    let uy = dpdy(i.uv);
    let across_y = cross(dy, raw);
    let across_x = cross(raw, dx);
    let t = across_y * ux.x + across_x * uy.x;
    let b = across_y * ux.y + across_x * uy.y;
    let scale = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-20));
    // The picture's v grows down while its green points up.
    s.n = normalize(t * scale * maps.bump.x - b * scale * maps.bump.y + g * max(maps.bump.z, 0.0));
    s.face = g;
    var level = 0.0;
    var bumped = 0.0;
    for (var k = 0u; k < u32(globals.ambient.w); k++) {
        let l = globals.lights[k];
        let reach = light_reach(l, i.world);
        let power = luma(l.color.rgb) * reach.w;
        level += power * max(dot(g, reach.xyz), 0.0);
        bumped += power * max(dot(s.n, reach.xyz), 0.0);
    }
    s.relief = select(1.0, clamp(bumped / level, 0.0, 2.0), level > 1e-4);
    return s;
}

@fragment
fn fs_baked(i: BakedOut) -> @location(0) vec4<f32> {
    let a = layer(tex, samp, i.uv);
    let c = varied(a.color * material.tint, i.world);
    let s = baked_surface(i, a.hex);
    let light = textureSample(lightmap_tex, lightmap_samp, i.lightmap).rgb * 2.0 * s.relief;
    if c.a < 0.5 {
        discard;
    }
    let emitted = (layer(emission_map, samp, i.uv).color.rgb + material.emission.rgb) * material.emission.a;
    return vec4<f32>(environment_color(to_linear(min(to_srgb(c.rgb) * light, vec3<f32>(1.0))) + emitted, i.world), 1.0);
}

// MapCraft's baker stores linear light the way Godot applies it: the albedo times the lightmap, occluded, plus the sky's reflection.
@fragment
fn fs_baked_linear(i: BakedOut) -> @location(0) vec4<f32> {
    let a = layer(tex, samp, i.uv);
    let c = varied(a.color * material.tint, i.world);
    let s = baked_surface(i, a.hex);
    if c.a < 0.5 {
        discard;
    }
    let light = textureSample(lightmap_tex, lightmap_samp, i.lightmap).rgb * s.relief;
    let v = normalize(globals.eye.xyz - i.world);
    let rough = clamp(s.orm.g, 0.05, 1.0);
    let f0 = mix(vec3<f32>(0.04), c.rgb, s.orm.b);
    let r = normalize(mix(reflect(-v, s.n), s.n, rough * rough));
    let shine = env_brdf(f0, rough, max(dot(s.n, v), 1e-4)) * hemisphere(r);
    let emitted = (layer(emission_map, samp, i.uv).color.rgb + material.emission.rgb) * material.emission.a;
    return vec4<f32>(environment_color((c.rgb * (1.0 - s.orm.b) * light + shine) * s.orm.r + emitted, i.world), 1.0);
}

@fragment
fn fs_baked_unlit(i: BakedOut) -> @location(0) vec4<f32> {
    return vec4<f32>(varied(layer(tex, samp, i.uv).color * material.tint, i.world).rgb, 1.0);
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
    var sky = mix(globals.sky_horizon.rgb, globals.sky_top.rgb, clamp(1.0 - pow(1.0 - (1.0 - v / (PI * 0.5)), 1.0 / globals.sky_shape.x), 0.0, 1.0));
    sky *= globals.sky_shape.z;
    let sun = globals.lights[0];
    if globals.ambient.w > 0.0 && sun.pos.w == 0.0 {
        let s = acos(clamp(dot(sun.pos.xyz, d), -1.0, 1.0)) / radians(30.0);
        if s < 1.0 {
            sky = mix(sun.color.rgb, sky, clamp(1.0 - pow(1.0 - s, 1.0 / 0.15), 0.0, 1.0));
        }
    }
    let ground = mix(globals.sky_horizon.rgb, globals.sky_ground.rgb, clamp(1.0 - pow(1.0 - (v - PI * 0.5) / (PI * 0.5), 1.0 / globals.sky_shape.y), 0.0, 1.0)) * globals.sky_shape.w;
    var background = select(ground, sky, d.z >= 0.0);
    if globals.fog.x > 0.0 || abs(globals.fog_height.y) >= 0.0001 { background = mix(background, fog_light(d), globals.fog.z); }
    return vec4<f32>(tone_adjust(background), 1.0);
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
