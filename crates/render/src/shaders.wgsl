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
    for (var k = 0u; k < u32(globals.ambient.w); k++) {
        let l = globals.lights[k];
        var dir = l.pos.xyz;
        var atten = 1.0;
        if l.pos.w > 0.0 {
            let d = max(distance(l.pos.xyz, i.world), 1.0);
            dir = (l.pos.xyz - i.world) / d;
            let r = l.spot.w;
            switch i32(l.color.w) {
                case 0: { atten = max(1.0 - d / r, 0.0); }
                case 1: { atten = min(r / d, 1.0); }
                case 2: { atten = min(r * r / (d * d), 1.0); }
                default: {}
            }
            if any(l.spot.xyz != vec3<f32>(0.0)) {
                atten *= smoothstep(l.cone.x, l.cone.y, dot(-dir, l.spot.xyz));
            }
        }
        let nl = dot(n, dir);
        if nl <= 0.0 {
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
    return vec4<f32>(max(sum, vec3<f32>(0.0)) + ambient, c.a);
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
    // Pull edges slightly toward the camera so they win the depth test against their own faces.
    o.clip.z = o.clip.z - 0.0001 * o.clip.w;
    o.color = v.color;
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    return i.color;
}
