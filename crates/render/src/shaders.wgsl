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
    lights: array<Light, 64>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var orm_map: texture_2d<f32>;

const PI: f32 = 3.14159265;

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) tangent: vec4<f32>,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) world: vec3<f32>,
    @location(4) tangent: vec4<f32>,
};

@vertex
fn vs_mesh(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    o.clip = globals.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.uv = v.uv;
    o.tint = v.tint;
    o.world = v.pos;
    o.tangent = v.tangent;
    return o;
}

@fragment
fn fs_mesh(i: MeshOut) -> @location(0) vec4<f32> {
    let shade = 0.6 + 0.4 * abs(dot(normalize(i.normal), globals.light_dir.xyz));
    let c = textureSample(tex, samp, i.uv) * i.tint;
    return vec4<f32>(c.rgb * shade, c.a);
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
    let c = textureSample(tex, samp, i.uv) * i.tint;
    let orm = textureSample(orm_map, samp, i.uv).rgb;
    let bump = textureSample(normal_map, samp, i.uv).xyz * 2.0 - 1.0;
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
    return vec4<f32>(select(color, filmic(color), globals.sky_top.w > 0.0), c.a);
}

// Godot's filmic tonemapper at white 1, which the MapCraft addon gives its environments.
fn filmic(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    let t = (x * (0.88 * x + 0.06) + 0.002) / (x * (0.88 * x + 0.6) + 0.06) - 0.01 / 0.3;
    return t / 0.578355;
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
