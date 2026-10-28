#ifndef MAPCRAFT_SURFACE_INC
#define MAPCRAFT_SURFACE_INC

// Shared with crates/render/src/shaders.wgsl and mapcraft_geom::noise; change all three together.
const float BLEND_DEPTH = 0.1;
const vec2 MACRO_CELLS = vec2(512.0, 197.0);
const float HEX_SCALE = 1.0;
const float HEX_SHARPNESS = 7.0;
const float HEX_CONTRAST = 0.6;

group_uniforms layer_a;
uniform vec4 albedo_a : source_color = vec4(1.0);
uniform sampler2D albedo_texture_a : source_color, hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec3 uv1_scale_a = vec3(1.0);
uniform vec3 uv1_offset_a = vec3(0.0);
uniform float roughness_a : hint_range(0.0, 1.0) = 1.0;
uniform sampler2D roughness_texture_a : hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec4 roughness_channel_a = vec4(1.0, 0.0, 0.0, 0.0);
uniform float metallic_a : hint_range(0.0, 1.0) = 0.0;
uniform sampler2D metallic_texture_a : hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec4 metallic_channel_a = vec4(1.0, 0.0, 0.0, 0.0);
uniform float specular_a : hint_range(0.0, 1.0) = 0.5;
uniform sampler2D normal_texture_a : hint_normal, MAPCRAFT_FILTER, repeat_enable;
uniform float normal_scale_a : hint_range(-16.0, 16.0) = 1.0;
uniform sampler2D ao_texture_a : hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec4 ao_channel_a = vec4(1.0, 0.0, 0.0, 0.0);
uniform float ao_light_affect_a : hint_range(0.0, 1.0) = 0.0;
#ifdef MAPCRAFT_BLEND
group_uniforms layer_b;
uniform vec4 albedo_b : source_color = vec4(1.0);
uniform sampler2D albedo_texture_b : source_color, hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec3 uv1_scale_b = vec3(1.0);
uniform vec3 uv1_offset_b = vec3(0.0);
uniform float roughness_b : hint_range(0.0, 1.0) = 1.0;
uniform sampler2D roughness_texture_b : hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec4 roughness_channel_b = vec4(1.0, 0.0, 0.0, 0.0);
uniform float metallic_b : hint_range(0.0, 1.0) = 0.0;
uniform sampler2D metallic_texture_b : hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec4 metallic_channel_b = vec4(1.0, 0.0, 0.0, 0.0);
uniform float specular_b : hint_range(0.0, 1.0) = 0.5;
uniform sampler2D normal_texture_b : hint_normal, MAPCRAFT_FILTER, repeat_enable;
uniform float normal_scale_b : hint_range(-16.0, 16.0) = 1.0;
uniform sampler2D ao_texture_b : hint_default_white, MAPCRAFT_FILTER, repeat_enable;
uniform vec4 ao_channel_b = vec4(1.0, 0.0, 0.0, 0.0);
uniform float ao_light_affect_b : hint_range(0.0, 1.0) = 0.0;
uniform vec2 uv_ratio_b = vec2(1.0);
#endif
group_uniforms anti_tiling;
uniform float macro_amount : hint_range(0.0, 0.5) = 0.0;
uniform bool hex_tiling = false;
uniform float map_units = 39.37;
group_uniforms;

varying vec3 mapcraft_pos;

struct MapcraftLayer {
	vec3 albedo;
	float roughness;
	float metallic;
	float specular;
	vec3 normal_map;
	float normal_depth;
	float ao;
	float ao_light_affect;
};

struct MapcraftHex {
	// A copy's texture coordinates in xy, and the cosine and sine of its turn in zw.
	vec4 t0;
	vec4 t1;
	vec4 t2;
	vec3 weight;
	vec2 dx;
	vec2 dy;
	vec2 aspect;
};


uint mapcraft_mix32(uint x) {
	uint h = x ^ (x >> 16u);
	h *= 2146121005u;
	h ^= h >> 15u;
	h *= 2221713035u;
	return h ^ (h >> 16u);
}


// The offset keeps lattice coordinates positive, so WGSL and Godot convert them the same way.
float mapcraft_lattice(vec3 c) {
	uvec3 u = uvec3(ivec3(c) + ivec3(1048576));
	return float(mapcraft_mix32(u.x ^ mapcraft_mix32(u.y ^ mapcraft_mix32(u.z))) >> 8u) / 16777216.0;
}


float mapcraft_value_noise(vec3 p) {
	vec3 i = floor(p);
	vec3 f = p - i;
	vec3 s = f * f * (3.0 - 2.0 * f);
	float x00 = mix(mapcraft_lattice(i), mapcraft_lattice(i + vec3(1.0, 0.0, 0.0)), s.x);
	float x10 = mix(mapcraft_lattice(i + vec3(0.0, 1.0, 0.0)), mapcraft_lattice(i + vec3(1.0, 1.0, 0.0)), s.x);
	float x01 = mix(mapcraft_lattice(i + vec3(0.0, 0.0, 1.0)), mapcraft_lattice(i + vec3(1.0, 0.0, 1.0)), s.x);
	float x11 = mix(mapcraft_lattice(i + vec3(0.0, 1.0, 1.0)), mapcraft_lattice(i + vec3(1.0, 1.0, 1.0)), s.x);
	return mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z);
}


// Takes a model-space point in Godot axes; the noise lives in MapCraft's Z-up inches.
float mapcraft_macro(vec3 pos) {
	vec3 world = vec3(pos.x, -pos.z, pos.y) * map_units;
	float n = 0.6 * mapcraft_value_noise(world / MACRO_CELLS.x) + 0.4 * mapcraft_value_noise(world / MACRO_CELLS.y);
	return 1.0 + macro_amount * (2.0 * n - 1.0);
}


float mapcraft_luma(vec3 c) {
	return dot(c, vec3(0.2126, 0.7152, 0.0722));
}


float mapcraft_unit(uint h) {
	return float(h >> 8u) / 16777216.0;
}


vec2 mapcraft_turned(vec2 turn, vec2 v) {
	return vec2(turn.x * v.x - turn.y * v.y, turn.y * v.x + turn.x * v.y);
}


vec4 mapcraft_hex_corner(vec2 q, vec2 id, vec2 aspect) {
	uvec2 u = uvec2(ivec2(id) + ivec2(1048576));
	uint a = mapcraft_mix32(u.x ^ mapcraft_mix32(u.y));
	uint b = mapcraft_mix32(a);
	float angle = mapcraft_unit(a) * TAU;
	vec2 turn = vec2(cos(angle), sin(angle));
	vec2 center = vec2(id.x + id.y * 0.5, id.y * 0.8660254) / HEX_SCALE;
	return vec4((mapcraft_turned(turn, q - center) + center) / aspect + vec2(mapcraft_unit(b), mapcraft_unit(mapcraft_mix32(b))), turn);
}


// Hex tiling from Mikkelsen, "Practical Real-Time Hex-Tiling" (JCGT 2022).
MapcraftHex mapcraft_hex_tiles(vec2 uv, vec2 size) {
	// Square texels with a repeat's area, so turns stay rigid on wide textures and a hex is about one repeat.
	vec2 aspect = sqrt(size / size.yx);
	vec2 q = uv * aspect;
	vec2 p = q * HEX_SCALE;
	// Skewed, each triangle of the grid is half a unit square, the upper half when x + y > 1.
	vec2 s = vec2(p.x - p.y * 0.57735027, p.y * 1.15470054);
	vec2 cell = floor(s);
	vec2 f = s - cell;
	float up = f.x + f.y > 1.0 ? 1.0 : 0.0;
	vec3 weight = vec3(abs(1.0 - f.x - f.y), mix(f, 1.0 - f, up));
	vec4 t0 = mapcraft_hex_corner(q, cell + vec2(up, up), aspect);
	vec4 t1 = mapcraft_hex_corner(q, cell + vec2(1.0 - up, up), aspect);
	vec4 t2 = mapcraft_hex_corner(q, cell + vec2(up, 1.0 - up), aspect);
	return MapcraftHex(t0, t1, t2, weight, dFdx(q), dFdy(q), aspect);
}


// Derivatives turn with the copy, so mip levels match across tile edges.
vec4 mapcraft_hex_tap(sampler2D tex, MapcraftHex h, vec4 t) {
	return textureGrad(tex, t.xy, mapcraft_turned(t.zw, h.dx) / h.aspect, mapcraft_turned(t.zw, h.dy) / h.aspect);
}


// Brighter copies win, so one tile shows at a time and edges stay crisp.
vec4 mapcraft_hex_albedo(sampler2D tex, inout MapcraftHex h) {
	vec4 c0 = mapcraft_hex_tap(tex, h, h.t0);
	vec4 c1 = mapcraft_hex_tap(tex, h, h.t1);
	vec4 c2 = mapcraft_hex_tap(tex, h, h.t2);
	vec3 w = mix(vec3(1.0), vec3(mapcraft_luma(c0.rgb), mapcraft_luma(c1.rgb), mapcraft_luma(c2.rgb)), HEX_CONTRAST) * pow(h.weight, vec3(HEX_SHARPNESS));
	h.weight = w / (w.x + w.y + w.z);
	return c0 * h.weight.x + c1 * h.weight.y + c2 * h.weight.z;
}


// Green points up the picture, against v, so turning xy by the copy's own turn undoes it on the face.
vec3 mapcraft_hex_normal_tap(sampler2D tex, MapcraftHex h, vec4 t) {
	vec3 n = mapcraft_hex_tap(tex, h, t).rgb * 2.0 - 1.0;
	return vec3(mapcraft_turned(t.zw, n.xy), n.z);
}


vec3 mapcraft_hex_normal(sampler2D tex, MapcraftHex h) {
	vec3 n = mapcraft_hex_normal_tap(tex, h, h.t0) * h.weight.x + mapcraft_hex_normal_tap(tex, h, h.t1) * h.weight.y + mapcraft_hex_normal_tap(tex, h, h.t2) * h.weight.z;
	return n * 0.5 + 0.5;
}


vec4 mapcraft_hex_heaviest(MapcraftHex h) {
	vec3 w = h.weight;
	return w.x >= max(w.y, w.z) ? h.t0 : (w.y >= w.z ? h.t1 : h.t2);
}


MapcraftLayer mapcraft_layer_a(vec2 uv) {
	MapcraftLayer l;
	if (hex_tiling) {
		MapcraftHex h = mapcraft_hex_tiles(uv, vec2(textureSize(albedo_texture_a, 0)));
		l.albedo = albedo_a.rgb * mapcraft_hex_albedo(albedo_texture_a, h).rgb;
		vec4 t = mapcraft_hex_heaviest(h);
		l.roughness = roughness_a * dot(mapcraft_hex_tap(roughness_texture_a, h, t), roughness_channel_a);
		l.metallic = metallic_a * dot(mapcraft_hex_tap(metallic_texture_a, h, t), metallic_channel_a);
		l.normal_map = mapcraft_hex_normal(normal_texture_a, h);
		l.ao = dot(mapcraft_hex_tap(ao_texture_a, h, t), ao_channel_a);
	} else {
		l.albedo = albedo_a.rgb * texture(albedo_texture_a, uv).rgb;
		l.roughness = roughness_a * dot(texture(roughness_texture_a, uv), roughness_channel_a);
		l.metallic = metallic_a * dot(texture(metallic_texture_a, uv), metallic_channel_a);
		l.normal_map = texture(normal_texture_a, uv).rgb;
		l.ao = dot(texture(ao_texture_a, uv), ao_channel_a);
	}
	l.specular = specular_a;
	l.normal_depth = normal_scale_a;
	l.ao_light_affect = ao_light_affect_a;
	return l;
}


#ifdef MAPCRAFT_BLEND
MapcraftLayer mapcraft_layer_b(vec2 uv) {
	MapcraftLayer l;
	if (hex_tiling) {
		MapcraftHex h = mapcraft_hex_tiles(uv, vec2(textureSize(albedo_texture_b, 0)));
		l.albedo = albedo_b.rgb * mapcraft_hex_albedo(albedo_texture_b, h).rgb;
		vec4 t = mapcraft_hex_heaviest(h);
		l.roughness = roughness_b * dot(mapcraft_hex_tap(roughness_texture_b, h, t), roughness_channel_b);
		l.metallic = metallic_b * dot(mapcraft_hex_tap(metallic_texture_b, h, t), metallic_channel_b);
		l.normal_map = mapcraft_hex_normal(normal_texture_b, h);
		l.ao = dot(mapcraft_hex_tap(ao_texture_b, h, t), ao_channel_b);
	} else {
		l.albedo = albedo_b.rgb * texture(albedo_texture_b, uv).rgb;
		l.roughness = roughness_b * dot(texture(roughness_texture_b, uv), roughness_channel_b);
		l.metallic = metallic_b * dot(texture(metallic_texture_b, uv), metallic_channel_b);
		l.normal_map = texture(normal_texture_b, uv).rgb;
		l.ao = dot(texture(ao_texture_b, uv), ao_channel_b);
	}
	l.specular = specular_b;
	l.normal_depth = normal_scale_b;
	l.ao_light_affect = ao_light_affect_b;
	return l;
}


// Heights are scaled by 1 - BLEND_DEPTH so weights 0 and 1 always give pure layers.
float mapcraft_share(float w, float ha, float hb) {
	float a = ha * (1.0 - BLEND_DEPTH) + 1.0 - w;
	float b = hb * (1.0 - BLEND_DEPTH) + w;
	float m = max(a, b) - BLEND_DEPTH;
	float ba = max(a - m, 0.0);
	float bb = max(b - m, 0.0);
	return bb / (ba + bb);
}
#endif


MapcraftLayer mapcraft_surface(vec2 uv, float w, vec3 pos) {
	MapcraftLayer s = mapcraft_layer_a(uv * uv1_scale_a.xy + uv1_offset_a.xy);
#ifdef MAPCRAFT_BLEND
	MapcraftLayer b = mapcraft_layer_b(uv * uv_ratio_b * uv1_scale_b.xy + uv1_offset_b.xy);
	float t = mapcraft_share(w, mapcraft_luma(s.albedo), mapcraft_luma(b.albedo));
	s.albedo = mix(s.albedo, b.albedo, t);
	s.roughness = mix(s.roughness, b.roughness, t);
	s.metallic = mix(s.metallic, b.metallic, t);
	s.specular = mix(s.specular, b.specular, t);
	// Godot rebuilds the normal's z, so mixing the encoded rgb is enough.
	s.normal_map = mix(s.normal_map, b.normal_map, t);
	s.normal_depth = mix(s.normal_depth, b.normal_depth, t);
	s.ao = mix(s.ao, b.ao, t);
	s.ao_light_affect = mix(s.ao_light_affect, b.ao_light_affect, t);
#endif
	if (macro_amount > 0.0) {
		s.albedo *= mapcraft_macro(pos);
	}
	return s;
}

#endif
