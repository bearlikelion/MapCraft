shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

#define MAPCRAFT_FILTER filter_nearest
#include "res://addons/mapcraft/shaders/mapcraft_surface.gdshaderinc"


void vertex() {
	mapcraft_pos = VERTEX;
}


void fragment() {
	MapcraftLayer s = mapcraft_surface(UV, COLOR.a, mapcraft_pos);
	ALBEDO = s.albedo;
	ROUGHNESS = s.roughness;
	METALLIC = s.metallic;
	SPECULAR = s.specular;
	NORMAL_MAP = s.normal_map;
	NORMAL_MAP_DEPTH = s.normal_depth;
	AO = s.ao;
	AO_LIGHT_AFFECT = s.ao_light_affect;
}
