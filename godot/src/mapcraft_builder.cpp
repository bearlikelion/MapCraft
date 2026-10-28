#include "mapcraft_builder.h"

#include <godot_cpp/classes/area3d.hpp>
#include <godot_cpp/classes/array_mesh.hpp>
#include <godot_cpp/classes/base_material3d.hpp>
#include <godot_cpp/classes/collision_object3d.hpp>
#include <godot_cpp/classes/collision_shape3d.hpp>
#include <godot_cpp/classes/concave_polygon_shape3d.hpp>
#include <godot_cpp/classes/convex_polygon_shape3d.hpp>
#include <godot_cpp/classes/directional_light3d.hpp>
#include <godot_cpp/classes/environment.hpp>
#include <godot_cpp/classes/file_access.hpp>
#include <godot_cpp/classes/json.hpp>
#include <godot_cpp/classes/marker3d.hpp>
#include <godot_cpp/classes/mesh_instance3d.hpp>
#include <godot_cpp/classes/packed_scene.hpp>
#include <godot_cpp/classes/procedural_sky_material.hpp>
#include <godot_cpp/classes/resource_loader.hpp>
#include <godot_cpp/classes/shader.hpp>
#include <godot_cpp/classes/shader_material.hpp>
#include <godot_cpp/classes/sky.hpp>
#include <godot_cpp/classes/standard_material3d.hpp>
#include <godot_cpp/classes/static_body3d.hpp>
#include <godot_cpp/classes/surface_tool.hpp>
#include <godot_cpp/classes/texture2d.hpp>
#include <godot_cpp/classes/world_environment.hpp>
#include <godot_cpp/templates/hash_map.hpp>
#include <godot_cpp/variant/utility_functions.hpp>

namespace godot {

namespace {

const char *MATERIAL_EXTENSIONS[] = { "tres", "res", "material" };
const char *IMAGE_EXTENSIONS[] = { "png", "jpg", "jpeg", "webp", "tga", "bmp" };
const double DEFAULT_TEXTURE_SIZE = 64.0;
const int MAX_DEPTH = 256;
// The editor's MACRO_AMOUNT in mapcraft_geom::noise.
const double MACRO_AMOUNT = 0.15;
const char *SHADER_DIR = "res://addons/mapcraft/shaders/";

// Column form of (x, y, z) -> (x, z, -y), a rotation, so windings keep their sense.
const Basis TO_GODOT(Vector3(1, 0, 0), Vector3(0, 0, -1), Vector3(0, 1, 0));

using Surfaces = HashMap<String, Ref<SurfaceTool>>;

enum class AntiTile {
	OFF,
	MACRO,
	HEX,
};

const struct {
	BaseMaterial3D::Feature feature;
	const char *name;
} FEATURE_NAMES[] = {
	{ BaseMaterial3D::FEATURE_EMISSION, "emission" },
	{ BaseMaterial3D::FEATURE_RIM, "rim" },
	{ BaseMaterial3D::FEATURE_CLEARCOAT, "clearcoat" },
	{ BaseMaterial3D::FEATURE_ANISOTROPY, "anisotropy" },
	{ BaseMaterial3D::FEATURE_HEIGHT_MAPPING, "heightmap" },
	{ BaseMaterial3D::FEATURE_SUBSURFACE_SCATTERING, "subsurface scattering" },
	{ BaseMaterial3D::FEATURE_SUBSURFACE_TRANSMITTANCE, "transmittance" },
	{ BaseMaterial3D::FEATURE_BACKLIGHT, "backlight" },
	{ BaseMaterial3D::FEATURE_REFRACTION, "refraction" },
	{ BaseMaterial3D::FEATURE_DETAIL, "detail" },
};

const struct {
	BaseMaterial3D::Flags flag;
	const char *name;
} FLAG_NAMES[] = {
	{ BaseMaterial3D::FLAG_DISABLE_DEPTH_TEST, "no depth test" },
	{ BaseMaterial3D::FLAG_ALBEDO_FROM_VERTEX_COLOR, "vertex color" },
	{ BaseMaterial3D::FLAG_USE_POINT_SIZE, "point size" },
	{ BaseMaterial3D::FLAG_FIXED_SIZE, "fixed size" },
	{ BaseMaterial3D::FLAG_UV1_USE_TRIPLANAR, "triplanar" },
	{ BaseMaterial3D::FLAG_UV1_USE_WORLD_TRIPLANAR, "triplanar" },
	{ BaseMaterial3D::FLAG_AO_ON_UV2, "ambient occlusion on UV2" },
	{ BaseMaterial3D::FLAG_ALBEDO_TEXTURE_FORCE_SRGB, "forced sRGB" },
	{ BaseMaterial3D::FLAG_DONT_RECEIVE_SHADOWS, "no shadows" },
	{ BaseMaterial3D::FLAG_DISABLE_AMBIENT_LIGHT, "no ambient light" },
	{ BaseMaterial3D::FLAG_USE_SHADOW_TO_OPACITY, "shadow to opacity" },
	{ BaseMaterial3D::FLAG_PARTICLE_TRAILS_MODE, "particle trails" },
	{ BaseMaterial3D::FLAG_ALBEDO_TEXTURE_MSDF, "MSDF" },
	{ BaseMaterial3D::FLAG_DISABLE_FOG, "no fog" },
};

Vector3 read_vec3(const Variant &p_value) {
	if (p_value.get_type() != Variant::ARRAY) {
		return Vector3();
	}
	Array a = p_value;
	if (a.size() < 3) {
		return Vector3();
	}
	return Vector3(double(a[0]), double(a[1]), double(a[2]));
}

bool parse_vec3(const String &p_text, Vector3 &r_out) {
	PackedStringArray parts = p_text.strip_edges().split(" ", false);
	if (parts.size() != 3) {
		return false;
	}
	for (int i = 0; i < 3; i++) {
		if (!parts[i].is_valid_float()) {
			return false;
		}
		r_out[i] = parts[i].to_float();
	}
	return true;
}

// Hammer order: yaw about Z, pitch about Y, roll about X, with +X forward.
Basis map_angles(const Vector3 &p_degrees) {
	return Basis(Vector3(0, 0, 1), Math::deg_to_rad(p_degrees.y)) * Basis(Vector3(0, 1, 0), Math::deg_to_rad(p_degrees.x)) *
			Basis(Vector3(1, 0, 0), Math::deg_to_rad(p_degrees.z));
}

Basis godot_basis(const Basis &p_map) {
	return TO_GODOT * p_map * TO_GODOT.transposed();
}

// Colors are "R G B" from 0 to 255, as FGD color255 keys store them.
Color color_key(const Dictionary &p_properties, const String &p_key, const Color &p_default) {
	Vector3 rgb;
	if (!parse_vec3(p_properties.get(p_key, ""), rgb)) {
		return p_default;
	}
	return Color(rgb.x / 255.0, rgb.y / 255.0, rgb.z / 255.0);
}

double number_key(const Dictionary &p_properties, const String &p_key, double p_default) {
	String text = String(p_properties.get(p_key, "")).strip_edges();
	return text.is_valid_float() ? text.to_float() : p_default;
}

// Vectors named like directions are turned into Godot axes.
Variant typed_value(const String &p_key, const String &p_value) {
	String v = p_value.strip_edges();
	if (v == "true" || v == "false") {
		return v == "true";
	}
	if (v.is_valid_int()) {
		return v.to_int();
	}
	if (v.is_valid_float()) {
		return v.to_float();
	}
	Vector3 vec;
	if (parse_vec3(v, vec)) {
		return p_key.ends_with("direction") ? TO_GODOT.xform(vec) : vec;
	}
	return p_value;
}

// Metadata names must be identifiers.
String meta_name(const String &p_key) {
	String out;
	for (int i = 0; i < p_key.length(); i++) {
		char32_t c = p_key[i];
		bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_' || (i > 0 && c >= '0' && c <= '9');
		out += ok ? String::chr(c) : String("_");
	}
	return out.is_empty() ? String("_") : out;
}

Vector3 newell_normal(const PackedVector3Array &p_points) {
	Vector3 n;
	for (int i = 0; i < p_points.size(); i++) {
		const Vector3 &a = p_points[i];
		const Vector3 &b = p_points[(i + 1) % p_points.size()];
		n += Vector3((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
	}
	return n.normalized();
}

AntiTile anti_tile_of(const Dictionary &p_face) {
	Dictionary attrs = p_face.get("attrs", Dictionary());
	const String mode = String(attrs.get("anti_tile", "")).strip_edges();
	if (mode.nocasecmp_to("macro") == 0) {
		return AntiTile::MACRO;
	}
	return mode.nocasecmp_to("hex") == 0 ? AntiTile::HEX : AntiTile::OFF;
}

// Rust's eq_ignore_ascii_case, which leaves non-ASCII letters unfolded.
bool equal_ignore_ascii_case(const String &p_a, const String &p_b) {
	if (p_a.length() != p_b.length()) {
		return false;
	}
	const auto fold = [](char32_t c) { return c >= 'A' && c <= 'Z' ? char32_t(c - 'A' + 'a') : c; };
	for (int64_t i = 0; i < p_a.length(); i++) {
		if (fold(p_a[i]) != fold(p_b[i])) {
			return false;
		}
	}
	return true;
}

// The map crate's blend_layer rule.
String blend_layer_of(const String &p_material, const Dictionary &p_disp) {
	const Variant layer = p_disp.get("blend_material", Variant());
	if (layer.get_type() != Variant::STRING) {
		return String();
	}
	const String name = layer;
	if (name.is_empty() || equal_ignore_ascii_case(name, p_material) || equal_ignore_ascii_case(name.substr(0, 6), "tools/")) {
		return String();
	}
	return name;
}

String anti_tile_suffix(AntiTile p_mode) {
	switch (p_mode) {
		case AntiTile::MACRO:
			return "#macro";
		case AntiTile::HEX:
			return "#hex";
		default:
			return "";
	}
}

// Escaped so a material name holding '|' or '#' cannot land on another face's surface.
String surface_key(const String &p_a, const String &p_b, AntiTile p_mode) {
	const auto escape = [](const String &p_name) {
		return p_name.replace("\\", "\\\\").replace("|", "\\|").replace("#", "\\#");
	};
	return escape(p_a) + (p_b.is_empty() ? String() : String("|") + escape(p_b)) + anti_tile_suffix(p_mode);
}

// Why the shader wrapper would drop part of a material's look, or empty when it would not.
String wrap_refusal(const Ref<Material> &p_material) {
	if (p_material.is_null()) {
		return "not found";
	}
	StandardMaterial3D *m = Object::cast_to<StandardMaterial3D>(p_material.ptr());
	if (m == nullptr) {
		return p_material->get_class();
	}
	if (m->get_shading_mode() == BaseMaterial3D::SHADING_MODE_UNSHADED) {
		return "unshaded";
	}
	if (m->get_shading_mode() != BaseMaterial3D::SHADING_MODE_PER_PIXEL) {
		return "per-vertex shading";
	}
	if (m->get_cull_mode() != BaseMaterial3D::CULL_BACK) {
		return "cull mode";
	}
	if (m->get_transparency() != BaseMaterial3D::TRANSPARENCY_DISABLED) {
		return "transparency";
	}
	if (m->get_blend_mode() != BaseMaterial3D::BLEND_MODE_MIX) {
		return "blend mode";
	}
	if (m->get_depth_draw_mode() == BaseMaterial3D::DEPTH_DRAW_DISABLED) {
		return "depth draw";
	}
	if (m->get_diffuse_mode() != BaseMaterial3D::DIFFUSE_BURLEY) {
		return "diffuse mode";
	}
	if (m->get_specular_mode() != BaseMaterial3D::SPECULAR_SCHLICK_GGX) {
		return "specular mode";
	}
	if (m->get_billboard_mode() != BaseMaterial3D::BILLBOARD_DISABLED) {
		return "billboard";
	}
	if (m->get_distance_fade() != BaseMaterial3D::DISTANCE_FADE_DISABLED) {
		return "distance fade";
	}
	if (m->is_proximity_fade_enabled()) {
		return "proximity fade";
	}
	if (m->is_grow_enabled()) {
		return "grow";
	}
	if (!m->get_flag(BaseMaterial3D::FLAG_USE_TEXTURE_REPEAT)) {
		return "no texture repeat";
	}
	for (const auto &known : FLAG_NAMES) {
		if (m->get_flag(known.flag)) {
			return known.name;
		}
	}
	for (int i = 0; i < BaseMaterial3D::FEATURE_MAX; i++) {
		const BaseMaterial3D::Feature feature = BaseMaterial3D::Feature(i);
		if (feature == BaseMaterial3D::FEATURE_NORMAL_MAPPING || feature == BaseMaterial3D::FEATURE_AMBIENT_OCCLUSION || !m->get_feature(feature)) {
			continue;
		}
		for (const auto &known : FEATURE_NAMES) {
			if (known.feature == feature) {
				return known.name;
			}
		}
		return "feature " + itos(i);
	}
	return String();
}

String filter_group(BaseMaterial3D::TextureFilter p_filter) {
	switch (p_filter) {
		case BaseMaterial3D::TEXTURE_FILTER_NEAREST:
			return "nearest";
		case BaseMaterial3D::TEXTURE_FILTER_NEAREST_WITH_MIPMAPS:
		case BaseMaterial3D::TEXTURE_FILTER_NEAREST_WITH_MIPMAPS_ANISOTROPIC:
			return "nearest_mipmap";
		default:
			return "linear";
	}
}

Vector4 channel_vector(BaseMaterial3D::TextureChannel p_channel) {
	switch (p_channel) {
		case BaseMaterial3D::TEXTURE_CHANNEL_RED:
			return Vector4(1, 0, 0, 0);
		case BaseMaterial3D::TEXTURE_CHANNEL_GREEN:
			return Vector4(0, 1, 0, 0);
		case BaseMaterial3D::TEXTURE_CHANNEL_BLUE:
			return Vector4(0, 0, 1, 0);
		case BaseMaterial3D::TEXTURE_CHANNEL_ALPHA:
			return Vector4(0, 0, 0, 1);
		default:
			return Vector4(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0);
	}
}

// Missing maps stay null, so the uniforms' defaults make them no-ops.
void copy_layer(const Ref<ShaderMaterial> &p_to, StandardMaterial3D *p_from, const String &p_suffix) {
	auto set = [&](const char *p_name, const Variant &p_value) {
		p_to->set_shader_parameter(String(p_name) + p_suffix, p_value);
	};
	set("albedo", p_from->get_albedo());
	set("albedo_texture", p_from->get_texture(BaseMaterial3D::TEXTURE_ALBEDO));
	set("uv1_scale", p_from->get_uv1_scale());
	set("uv1_offset", p_from->get_uv1_offset());
	set("roughness", p_from->get_roughness());
	set("roughness_texture", p_from->get_texture(BaseMaterial3D::TEXTURE_ROUGHNESS));
	set("roughness_channel", channel_vector(p_from->get_roughness_texture_channel()));
	set("metallic", p_from->get_metallic());
	set("metallic_texture", p_from->get_texture(BaseMaterial3D::TEXTURE_METALLIC));
	set("metallic_channel", channel_vector(p_from->get_metallic_texture_channel()));
	set("specular", p_from->get_specular());
	if (p_from->get_feature(BaseMaterial3D::FEATURE_NORMAL_MAPPING)) {
		set("normal_texture", p_from->get_texture(BaseMaterial3D::TEXTURE_NORMAL));
		set("normal_scale", p_from->get_normal_scale());
	}
	if (p_from->get_feature(BaseMaterial3D::FEATURE_AMBIENT_OCCLUSION)) {
		set("ao_texture", p_from->get_texture(BaseMaterial3D::TEXTURE_AMBIENT_OCCLUSION));
		set("ao_channel", channel_vector(p_from->get_ao_texture_channel()));
		set("ao_light_affect", p_from->get_ao_light_affect());
	}
}

// A convex hull, or with `triangles` a triangle soup such as a displaced surface.
struct Shape {
	PackedVector3Array points;
	bool triangles = false;
};

void own(Node *p_node, Node *p_owner) {
	p_node->set_owner(p_owner);
	if (!p_node->get_scene_file_path().is_empty()) {
		return;
	}
	for (int i = 0; i < p_node->get_child_count(); i++) {
		own(p_node->get_child(i), p_owner);
	}
}

class Builder {
	Node3D *root;
	const MapCraftBuildOptions &options;
	double scale;
	Vector<AABB> cordons;
	HashMap<String, Ref<Material>> materials;
	HashMap<String, Vector2> sizes;
	HashMap<String, Ref<Material>> surface_materials;
	HashMap<String, Ref<Material>> variants;
	bool warned_missing_shaders = false;
	Surfaces world_surfaces;
	Vector<Shape> world_hulls;
	PackedVector3Array world_triangles;
	int brush_count = 0;
	bool has_environment = false;

	Vector3 to_godot(const Vector3 &p_map) const {
		return TO_GODOT.xform(p_map) * scale;
	}

	bool outside_cordons(const AABB &p_map_bounds) const {
		if (cordons.is_empty()) {
			return false;
		}
		for (const AABB &c : cordons) {
			if (c.grow(0.001).intersects(p_map_bounds)) {
				return false;
			}
		}
		return true;
	}

	Ref<Material> material(const String &p_name, Vector2 &r_size);
	Ref<Material> variant(const String &p_a, const String &p_b, AntiTile p_mode, Vector2 &r_size);
	bool add_brush(const Dictionary &p_brush, Surfaces *p_surfaces, Vector<Shape> *p_hulls, bool p_world = false);
	void add_displacement(const Dictionary &p_face, const Array &p_grid, Surfaces *p_surfaces, PackedVector3Array &r_triangles);
	Ref<SurfaceTool> surface_for(Surfaces &p_surfaces, const String &p_key, const Ref<Material> &p_material);
	void walk(const Dictionary &p_node, int p_depth);
	void add_entity(const Dictionary &p_entity);
	void add_prefab(const Dictionary &p_prefab);
	void add_environment(const Dictionary &p_properties);
	MeshInstance3D *make_mesh(Surfaces &p_surfaces, const Vector3 &p_offset);
	void add_shapes(Node *p_parent, const Vector<Shape> &p_hulls, const Vector3 &p_offset);
	void add(Node *p_parent, Node *p_child, bool p_readable_name);

public:
	Builder(Node3D *p_root, const MapCraftBuildOptions &p_options) :
			root(p_root), options(p_options), scale(1.0 / (p_options.units_per_meter > 0.0 ? p_options.units_per_meter : 39.37)) {}

	Error build(const Dictionary &p_doc, String &r_error);
};

void Builder::add(Node *p_parent, Node *p_child, bool p_readable_name) {
	p_parent->add_child(p_child, p_readable_name);
}

Ref<Material> Builder::material(const String &p_name, Vector2 &r_size) {
	if (materials.has(p_name)) {
		r_size = sizes[p_name];
		return materials[p_name];
	}
	ResourceLoader *loader = ResourceLoader::get_singleton();
	String base = options.material_root.path_join(p_name);
	Ref<Material> mat;
	for (const char *ext : MATERIAL_EXTENSIONS) {
		if (!p_name.is_empty() && mat.is_null() && loader->exists(base + "." + ext)) {
			mat = loader->load(base + "." + ext);
		}
	}
	for (const char *ext : IMAGE_EXTENSIONS) {
		if (!p_name.is_empty() && mat.is_null() && loader->exists(base + "." + ext)) {
			Ref<Texture2D> texture = loader->load(base + "." + ext);
			if (texture.is_valid()) {
				Ref<StandardMaterial3D> generated;
				generated.instantiate();
				generated->set_texture(BaseMaterial3D::TEXTURE_ALBEDO, texture);
				mat = generated;
			}
		}
	}
	Vector2 size(DEFAULT_TEXTURE_SIZE, DEFAULT_TEXTURE_SIZE);
	Ref<BaseMaterial3D> base_mat = mat;
	if (base_mat.is_valid()) {
		Ref<Texture2D> albedo = base_mat->get_texture(BaseMaterial3D::TEXTURE_ALBEDO);
		if (albedo.is_valid() && albedo->get_width() > 0 && albedo->get_height() > 0) {
			size = albedo->get_size();
		}
	}
	materials[p_name] = mat;
	sizes[p_name] = size;
	r_size = size;
	return mat;
}

Ref<Material> Builder::variant(const String &p_a, const String &p_b, AntiTile p_mode, Vector2 &r_size) {
	Ref<Material> base = material(p_a, r_size);
	if ((p_b.is_empty() && p_mode == AntiTile::OFF) || base.is_null()) {
		return base;
	}
	const String key = surface_key(p_a, p_b, p_mode);
	if (variants.has(key)) {
		return variants[key];
	}
	variants[key] = base;
	Vector2 size_b;
	const Ref<Material> layer_b = p_b.is_empty() ? Ref<Material>() : material(p_b, size_b);
	String refused = p_a;
	String refusal = wrap_refusal(base);
	if (refusal.is_empty() && !p_b.is_empty()) {
		refused = p_b;
		refusal = wrap_refusal(layer_b);
	}
	if (!refusal.is_empty()) {
		UtilityFunctions::push_warning("MapCraft: ", refused, " cannot blend or hide tiling (", refusal, "); drawn as is");
		return base;
	}
	StandardMaterial3D *a = Object::cast_to<StandardMaterial3D>(base.ptr());
	StandardMaterial3D *b = Object::cast_to<StandardMaterial3D>(layer_b.ptr());
	const String filter = filter_group(a->get_texture_filter());
	const String filter_b = b != nullptr ? filter_group(b->get_texture_filter()) : filter;
	if (filter_b != filter) {
		UtilityFunctions::push_warning("MapCraft: ", p_b, " is drawn with ", p_a, "'s filtering in this blend");
	}
	const String path = String(SHADER_DIR) + String(b != nullptr ? "mapcraft_blend_" : "mapcraft_single_") + filter + ".gdshader";
	Ref<Shader> shader;
	if (ResourceLoader::get_singleton()->exists(path)) {
		shader = ResourceLoader::get_singleton()->load(path);
	}
	if (shader.is_null()) {
		if (!warned_missing_shaders) {
			UtilityFunctions::push_warning("MapCraft: ", path, " is missing; update the MapCraft addon");
			warned_missing_shaders = true;
		}
		return base;
	}
	Ref<ShaderMaterial> wrapped;
	wrapped.instantiate();
	wrapped->set_shader(shader);
	wrapped->set_next_pass(base->get_next_pass());
	copy_layer(wrapped, a, "_a");
	if (b != nullptr) {
		copy_layer(wrapped, b, "_b");
		// UVs are in layer A's texels; layer B keeps its own density.
		wrapped->set_shader_parameter("uv_ratio_b", r_size / size_b);
	}
	wrapped->set_shader_parameter("macro_amount", p_mode == AntiTile::OFF ? 0.0 : MACRO_AMOUNT);
	// Three blended taps of big nearest texels read as double images.
	wrapped->set_shader_parameter("hex_tiling", p_mode == AntiTile::HEX && filter == "linear" && filter_b == "linear");
	wrapped->set_shader_parameter("map_units", options.units_per_meter > 0.0 ? options.units_per_meter : 39.37);
	variants[key] = wrapped;
	return wrapped;
}

Ref<SurfaceTool> Builder::surface_for(Surfaces &p_surfaces, const String &p_key, const Ref<Material> &p_material) {
	surface_materials[p_key] = p_material;
	if (!p_surfaces.has(p_key)) {
		Ref<SurfaceTool> st;
		st.instantiate();
		st->begin(Mesh::PRIMITIVE_TRIANGLES);
		p_surfaces[p_key] = st;
	}
	return p_surfaces[p_key];
}

void Builder::add_displacement(const Dictionary &p_face, const Array &p_grid, Surfaces *p_surfaces, PackedVector3Array &r_triangles) {
	const int n = int(Math::round(Math::sqrt(double(p_grid.size()))));
	if (n < 2 || n * n != p_grid.size()) {
		return;
	}
	Vector<Vector3> map_points;
	PackedVector3Array points;
	for (int k = 0; k < p_grid.size(); k++) {
		Vector3 m = read_vec3(p_grid[k]);
		map_points.push_back(m);
		points.push_back(to_godot(m));
	}
	Vector<int> order;
	for (int j = 0; j + 1 < n; j++) {
		for (int i = 0; i + 1 < n; i++) {
			const int a = j * n + i, b = a + 1, c = a + n + 1, d = a + n;
			// Same alternating diagonals as the editor, reversed because Godot's front is clockwise.
			const int tris[6] = { a, c, b, a, d, c };
			const int flipped[6] = { a, d, b, b, d, c };
			for (int t : (i + j) % 2 == 0 ? tris : flipped) {
				order.push_back(t);
			}
		}
	}
	for (int k : order) {
		r_triangles.push_back(points[k]);
	}
	String name = p_face.get("material", "");
	if (p_surfaces == nullptr || name.to_lower().begins_with("tools/")) {
		return;
	}
	const AntiTile mode = anti_tile_of(p_face);
	const Dictionary disp = p_face.get("disp", Dictionary());
	const String layer = blend_layer_of(name, disp);
	Vector2 size;
	const Ref<Material> mat = variant(name, layer, mode, size);
	Ref<SurfaceTool> st = surface_for(*p_surfaces, surface_key(name, layer, mode), mat);
	Vector<float> weights;
	// A blend that fell back draws the face's own material, which may read COLOR itself.
	if (!layer.is_empty() && mat != materials[name]) {
		const Array blend = disp.get("blend", Array());
		weights.resize(p_grid.size());
		for (int k = 0; k < p_grid.size(); k++) {
			const double w = k < blend.size() ? double(blend[k]) : 0.0;
			weights.write[k] = Math::is_finite(w) ? CLAMP(w, 0.0, 1.0) : 0.0;
		}
	}
	Dictionary u = p_face.get("u", Dictionary());
	Dictionary v = p_face.get("v", Dictionary());
	Vector3 u_axis = read_vec3(u.get("axis", Array()));
	Vector3 v_axis = read_vec3(v.get("axis", Array()));
	double u_scale = u.get("scale", 1.0), v_scale = v.get("scale", 1.0);
	double u_offset = u.get("offset", 0.0), v_offset = v.get("offset", 0.0);
	u_scale = u_scale == 0.0 ? 1.0 : u_scale;
	v_scale = v_scale == 0.0 ? 1.0 : v_scale;
	for (int k : order) {
		const int i = k % n, j = k / n;
		Vector3 du = points[j * n + MIN(i + 1, n - 1)] - points[j * n + MAX(i - 1, 0)];
		Vector3 dv = points[MIN(j + 1, n - 1) * n + i] - points[MAX(j - 1, 0) * n + i];
		const Vector3 &m = map_points[k];
		st->set_normal(du.cross(dv).normalized());
		st->set_uv(Vector2((m.dot(u_axis) / u_scale + u_offset) / size.x, (m.dot(v_axis) / v_scale + v_offset) / size.y));
		if (!weights.is_empty()) {
			st->set_color(Color(1, 1, 1, weights[k]));
		}
		st->add_vertex(points[k]);
	}
}

bool Builder::add_brush(const Dictionary &p_brush, Surfaces *p_surfaces, Vector<Shape> *p_hulls, bool p_world) {
	const bool trimesh = p_world && (options.trimesh_collision || (options.entity_map.is_valid() && options.entity_map->get_trimesh_collision()));
	Array faces = p_brush.get("faces", Array());
	bool displaced = false;
	for (int i = 0; i < faces.size(); i++) {
		displaced = displaced || Dictionary(faces[i]).has("disp_vertices");
	}
	AABB bounds;
	bool first = true;
	for (int i = 0; i < faces.size(); i++) {
		Array polygon = Dictionary(faces[i]).get("polygon", Array());
		for (int j = 0; j < polygon.size(); j++) {
			Vector3 p = read_vec3(polygon[j]);
			if (first) {
				bounds = AABB(p, Vector3());
				first = false;
			} else {
				bounds.expand_to(p);
			}
		}
	}
	if (first || outside_cordons(bounds)) {
		return false;
	}

	PackedVector3Array hull;
	PackedVector3Array triangles;
	for (int i = 0; i < faces.size(); i++) {
		Dictionary face = faces[i];
		Array polygon = face.get("polygon", Array());
		Array grid = face.get("disp_vertices", Array());
		// A displaced brush is only its displaced faces, as Source compiles it.
		if (displaced) {
			add_displacement(face, grid, p_surfaces, triangles);
			continue;
		}
		if (polygon.size() < 3) {
			continue;
		}
		Vector<Vector3> map_points;
		PackedVector3Array points;
		for (int j = 0; j < polygon.size(); j++) {
			Vector3 p = read_vec3(polygon[j]);
			map_points.push_back(p);
			points.push_back(to_godot(p));
		}
		hull.append_array(points);
		// The editor marks faces hidden between touching brushes; they would only add seams.
		if (bool(face.get("covered", false))) {
			continue;
		}
		for (int k = 1; trimesh && k + 1 < points.size(); k++) {
			triangles.push_back(points[0]);
			triangles.push_back(points[k + 1]);
			triangles.push_back(points[k]);
		}
		String name = face.get("material", "");
		// Hammer's tools/ materials (nodraw, clip, trigger) collide but are never drawn.
		if (p_surfaces == nullptr || name.to_lower().begins_with("tools/")) {
			continue;
		}

		const AntiTile mode = anti_tile_of(face);
		Vector2 size;
		Ref<SurfaceTool> st = surface_for(*p_surfaces, surface_key(name, String(), mode), variant(name, String(), mode, size));
		Dictionary u = face.get("u", Dictionary());
		Dictionary v = face.get("v", Dictionary());
		Vector3 u_axis = read_vec3(u.get("axis", Array()));
		Vector3 v_axis = read_vec3(v.get("axis", Array()));
		double u_scale = u.get("scale", 1.0), v_scale = v.get("scale", 1.0);
		double u_offset = u.get("offset", 0.0), v_offset = v.get("offset", 0.0);
		if (u_scale == 0.0) {
			u_scale = 1.0;
		}
		if (v_scale == 0.0) {
			v_scale = 1.0;
		}
		Vector3 normal = newell_normal(points);
		auto vertex = [&](int k) {
			const Vector3 &m = map_points[k];
			st->set_normal(normal);
			st->set_uv(Vector2((m.dot(u_axis) / u_scale + u_offset) / size.x, (m.dot(v_axis) / v_scale + v_offset) / size.y));
			st->add_vertex(points[k]);
		};
		// Polygons are counter-clockwise seen from outside; Godot treats clockwise as front.
		for (int k = 1; k + 1 < points.size(); k++) {
			vertex(0);
			vertex(k + 1);
			vertex(k);
		}
	}
	if (trimesh) {
		world_triangles.append_array(triangles);
		return true;
	}
	if (p_hulls != nullptr && hull.size() >= 4) {
		p_hulls->push_back(Shape{ hull, false });
	}
	if (p_hulls != nullptr && triangles.size() >= 3) {
		p_hulls->push_back(Shape{ triangles, true });
	}
	return true;
}

MeshInstance3D *Builder::make_mesh(Surfaces &p_surfaces, const Vector3 &p_offset) {
	Ref<ArrayMesh> mesh;
	mesh.instantiate();
	for (KeyValue<String, Ref<SurfaceTool>> &kv : p_surfaces) {
		kv.value->generate_tangents();
		kv.value->commit(mesh);
		int surface = mesh->get_surface_count() - 1;
		mesh->surface_set_name(surface, kv.key);
		const Ref<Material> *mat = surface_materials.getptr(kv.key);
		if (mat != nullptr && mat->is_valid()) {
			mesh->surface_set_material(surface, *mat);
		}
	}
	MeshInstance3D *mi = memnew(MeshInstance3D);
	mi->set_name("Mesh");
	mi->set_mesh(mesh);
	mi->set_position(-p_offset);
	return mi;
}

void Builder::add_shapes(Node *p_parent, const Vector<Shape> &p_hulls, const Vector3 &p_offset) {
	for (const Shape &hull : p_hulls) {
		PackedVector3Array points;
		for (const Vector3 &p : hull.points) {
			points.push_back(p - p_offset);
		}
		CollisionShape3D *cs = memnew(CollisionShape3D);
		cs->set_name("Brush" + itos(++brush_count));
		if (hull.triangles) {
			Ref<ConcavePolygonShape3D> shape;
			shape.instantiate();
			shape->set_faces(points);
			shape->set_backface_collision_enabled(true);
			cs->set_shape(shape);
		} else {
			Ref<ConvexPolygonShape3D> shape;
			shape.instantiate();
			shape->set_points(points);
			cs->set_shape(shape);
		}
		add(p_parent, cs, false);
	}
}

void Builder::walk(const Dictionary &p_node, int p_depth) {
	if (p_depth > MAX_DEPTH) {
		return;
	}
	String type = p_node.get("type", "");
	if (type == "brush") {
		add_brush(p_node, &world_surfaces, &world_hulls, true);
		return;
	}
	if (type == "entity") {
		add_entity(p_node);
		return;
	}
	if (type == "prefab") {
		add_prefab(p_node);
		return;
	}
	if (type == "layer" && bool(p_node.get("omit_from_export", false))) {
		return;
	}
	Array children = p_node.get("children", Array());
	for (int i = 0; i < children.size(); i++) {
		walk(children[i], p_depth + 1);
	}
}

void Builder::add_entity(const Dictionary &p_entity) {
	String classname = p_entity.get("classname", "");
	Array pairs = p_entity.get("properties", Array());
	Dictionary properties;
	for (int i = 0; i < pairs.size(); i++) {
		Array pair = pairs[i];
		if (pair.size() == 2) {
			properties[String(pair[0])] = String(pair[1]);
		}
	}
	Array brushes = p_entity.get("children", Array());
	bool solid = !brushes.is_empty();
	Dictionary config;
	if (options.entity_map.is_valid()) {
		config = options.entity_map->entity_config(classname, solid);
	} else {
		config["node"] = solid ? "area" : "marker";
	}
	String kind = config.get("node", "");
	if (kind == "skip") {
		return;
	}
	if (kind == "world") {
		for (int i = 0; i < brushes.size(); i++) {
			add_brush(brushes[i], &world_surfaces, &world_hulls, true);
		}
		return;
	}
	if (kind == "environment") {
		add_environment(properties);
		return;
	}

	Node3D *node = nullptr;
	if (kind == "scene" || kind == "model") {
		String path = config.get("scene", "");
		if (kind == "model") {
			path = String(properties.get(config.get("key", "model"), "")).strip_edges().replace("\\", "/");
			if (!path.is_empty() && !path.begins_with("res://")) {
				path = "res://" + path.trim_prefix("/");
			}
		}
		Ref<PackedScene> scene;
		if (!path.is_empty()) {
			scene = ResourceLoader::get_singleton()->load(path);
		}
		if (scene.is_valid()) {
			node = Object::cast_to<Node3D>(scene->instantiate());
		}
		if (node == nullptr) {
			UtilityFunctions::push_warning("MapCraft: ", classname, " has no usable scene ", path, "; using a Node3D");
		}
	}
	if (node == nullptr) {
		if (kind == "area") {
			node = memnew(Area3D);
		} else if (kind == "static") {
			node = memnew(StaticBody3D);
		} else if (kind == "node" || kind == "scene" || kind == "model") {
			node = memnew(Node3D);
		} else {
			node = memnew(Marker3D);
		}
	}

	if (CollisionObject3D *body = Object::cast_to<CollisionObject3D>(node)) {
		if (config.has("collision_layer")) {
			body->set_collision_layer(int64_t(config["collision_layer"]));
		}
		if (config.has("collision_mask")) {
			body->set_collision_mask(int64_t(config["collision_mask"]));
		}
	}

	String name = config.get("name", "");
	if (name.is_empty()) {
		name = properties.get("targetname", "");
	}
	if (name.is_empty()) {
		name = classname;
	}
	node->set_name(name.validate_node_name());

	if (solid) {
		Surfaces surfaces;
		Vector<Shape> hulls;
		bool visible = kind == "static" || bool(config.get("visible", false));
		for (int i = 0; i < brushes.size(); i++) {
			add_brush(brushes[i], visible ? &surfaces : nullptr, &hulls);
		}
		if (hulls.is_empty()) {
			memdelete(node);
			return;
		}
		AABB box(hulls[0].points[0], Vector3());
		for (const Shape &h : hulls) {
			for (const Vector3 &p : h.points) {
				box.expand_to(p);
			}
		}
		Vector3 center = box.get_center();
		node->set_position(center);
		if (!surfaces.is_empty()) {
			node->add_child(make_mesh(surfaces, center));
		}
		if (Object::cast_to<CollisionObject3D>(node) != nullptr) {
			add_shapes(node, hulls, center);
		}
	} else {
		Vector3 origin;
		parse_vec3(properties.get("origin", ""), origin);
		if (outside_cordons(AABB(origin, Vector3()))) {
			memdelete(node);
			return;
		}
		Vector3 angles;
		if (!parse_vec3(properties.get("angles", ""), angles) && properties.has("angle")) {
			angles.y = String(properties["angle"]).to_float();
		}
		Basis basis = godot_basis(map_angles(angles));
		if (kind == "model") {
			double size = String(properties.get("scale", properties.get("modelscale", "1"))).to_float();
			basis.scale(Vector3(1, 1, 1) * (size > 0.0 ? size : 1.0));
		} else {
			// Godot nodes look down -Z; the entity's forward is +X in map space.
			basis = basis * Basis(Vector3(0, 1, 0), -Math_PI / 2.0);
		}
		node->set_transform(Transform3D(basis, to_godot(origin)));
	}

	if (bool(config.get("properties", true))) {
		Array strings = config.get("strings", Array());
		Array keys = properties.keys();
		for (int i = 0; i < keys.size(); i++) {
			String key = keys[i];
			if (key == "origin" || key == "angles" || key == "angle" || key == "classname") {
				continue;
			}
			Variant value = strings.has(key) ? Variant(properties[key]) : typed_value(key, properties[key]);
			node->set_meta(meta_name(key), value);
		}
	}
	Dictionary meta = config.get("meta", Dictionary());
	Array meta_keys = meta.keys();
	for (int i = 0; i < meta_keys.size(); i++) {
		node->set_meta(meta_name(meta_keys[i]), meta[meta_keys[i]]);
	}

	add(root, node, true);
	if (options.on_entity) {
		options.on_entity(node, classname, properties);
	}
}

void Builder::add_prefab(const Dictionary &p_prefab) {
	Vector3 origin = read_vec3(p_prefab.get("origin", Array()));
	if (outside_cordons(AABB(origin, Vector3()))) {
		return;
	}
	String path = p_prefab.get("path", "");
	if (!path.is_absolute_path()) {
		path = options.source_path.get_base_dir().path_join(path).simplify_path();
	}
	Ref<PackedScene> scene = ResourceLoader::get_singleton()->load(path);
	Node3D *instance = scene.is_valid() ? Object::cast_to<Node3D>(scene->instantiate()) : nullptr;
	if (instance == nullptr) {
		UtilityFunctions::push_warning("MapCraft: cannot load prefab ", path);
		return;
	}
	instance->set_transform(Transform3D(godot_basis(map_angles(read_vec3(p_prefab.get("angles", Array())))), to_godot(origin)));
	add(root, instance, true);
}

void Builder::add_environment(const Dictionary &p_properties) {
	has_environment = true;
	Color horizon = color_key(p_properties, "sky_horizon_color", Color(0.72, 0.8, 0.88));
	Ref<ProceduralSkyMaterial> sky_material;
	sky_material.instantiate();
	sky_material->set_sky_top_color(color_key(p_properties, "sky_top_color", Color(0.3, 0.48, 0.75)));
	sky_material->set_sky_horizon_color(horizon);
	sky_material->set_ground_horizon_color(horizon);
	sky_material->set_ground_bottom_color(color_key(p_properties, "ground_color", Color(0.2, 0.21, 0.23)));
	Ref<Sky> sky;
	sky.instantiate();
	sky->set_material(sky_material);

	Ref<Environment> environment;
	environment.instantiate();
	environment->set_background(Environment::BG_SKY);
	environment->set_sky(sky);
	environment->set_ambient_source(Environment::AMBIENT_SOURCE_SKY);
	environment->set_ambient_light_energy(number_key(p_properties, "ambient_energy", 1.0));
	environment->set_reflection_source(Environment::REFLECTION_SOURCE_SKY);
	environment->set_tonemapper(Environment::TONE_MAPPER_FILMIC);
	double fog = number_key(p_properties, "fog_density", 0.0);
	if (fog > 0.0) {
		environment->set_fog_enabled(true);
		environment->set_fog_density(fog);
		environment->set_fog_light_color(color_key(p_properties, "fog_color", horizon));
	}
	WorldEnvironment *world = memnew(WorldEnvironment);
	world->set_name("WorldEnvironment");
	world->set_environment(environment);
	add(root, world, false);

	DirectionalLight3D *sun = memnew(DirectionalLight3D);
	sun->set_name("Sun");
	Vector3 angles(50.0, 135.0, 0.0);
	parse_vec3(p_properties.get("angles", ""), angles);
	// The sun shines down its -Z, which points along the entity's facing like a marker's.
	sun->set_transform(Transform3D(godot_basis(map_angles(angles)) * Basis(Vector3(0, 1, 0), -Math_PI / 2.0), Vector3()));
	sun->set_color(color_key(p_properties, "sun_color", Color(1.0, 0.96, 0.9)));
	sun->set_param(Light3D::PARAM_ENERGY, number_key(p_properties, "sun_energy", 1.0));
	sun->set_shadow(number_key(p_properties, "shadows", 1.0) != 0.0);
	sun->set_param(Light3D::PARAM_SHADOW_MAX_DISTANCE, 1000.0);
	add(root, sun, false);
}

Error Builder::build(const Dictionary &p_doc, String &r_error) {
	if (String(p_doc.get("format", "")) != "mapcraft.map") {
		r_error = "not a MapCraft map";
		return ERR_FILE_UNRECOGNIZED;
	}
	Dictionary map = p_doc.get("map", Dictionary());
	Array cordon_list = map.get("cordons", Array());
	for (int i = 0; i < cordon_list.size(); i++) {
		Dictionary c = cordon_list[i];
		if (bool(c.get("active", false))) {
			Dictionary b = c.get("bounds", Dictionary());
			Vector3 lo = read_vec3(b.get("min", Array()));
			cordons.push_back(AABB(lo, read_vec3(b.get("max", Array())) - lo));
		}
	}
	walk(map.get("root", Dictionary()), 0);
	if (!has_environment && options.entity_map.is_valid() && options.entity_map->get_default_environment()) {
		add_environment(Dictionary());
	}

	if (!world_surfaces.is_empty()) {
		MeshInstance3D *mesh = make_mesh(world_surfaces, Vector3());
		mesh->set_name("WorldMesh");
		add(root, mesh, false);
	}
	if (!world_triangles.is_empty()) {
		world_hulls.push_back(Shape{ world_triangles, true });
	}
	if (options.collision && !world_hulls.is_empty()) {
		StaticBody3D *body = memnew(StaticBody3D);
		body->set_name("WorldCollision");
		add(root, body, false);
		add_shapes(body, world_hulls, Vector3());
	}
	if (options.own_nodes) {
		for (int i = 0; i < root->get_child_count(); i++) {
			own(root->get_child(i), root);
		}
	}
	return OK;
}

} // namespace

Error mapcraft_build(const Dictionary &p_doc, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error) {
	ERR_FAIL_NULL_V(p_root, ERR_INVALID_PARAMETER);
	Builder builder(p_root, p_options);
	return builder.build(p_doc, r_error);
}

Error mapcraft_pack(const Dictionary &p_doc, const String &p_name, const MapCraftBuildOptions &p_options, Ref<PackedScene> &r_scene, String &r_error) {
	MapCraftBuildOptions options = p_options;
	if (options.entity_map.is_null()) {
		// Maps name their SDK; an addon entity map with that name is used when none is set.
		String sdk = p_doc.get("sdk", "");
		String candidate = String("res://addons/mapcraft/sdks").path_join(sdk + ".tres");
		if (!sdk.is_empty() && ResourceLoader::get_singleton()->exists(candidate)) {
			options.entity_map = ResourceLoader::get_singleton()->load(candidate);
		}
	}
	Node3D *root = memnew(Node3D);
	root->set_name(p_name.validate_node_name());
	Error err = mapcraft_build(p_doc, root, options, r_error);
	if (err == OK) {
		r_scene.instantiate();
		err = r_scene->pack(root);
	}
	memdelete(root);
	return err;
}

Error mapcraft_build_file(const String &p_path, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error) {
	if (!FileAccess::file_exists(p_path)) {
		r_error = String("cannot open ") + p_path;
		return ERR_FILE_NOT_FOUND;
	}
	Variant parsed = JSON::parse_string(FileAccess::get_file_as_string(p_path));
	if (parsed.get_type() != Variant::DICTIONARY) {
		r_error = p_path + String(" is not valid JSON");
		return ERR_PARSE_ERROR;
	}
	MapCraftBuildOptions options = p_options;
	if (options.source_path.is_empty()) {
		options.source_path = p_path;
	}
	return mapcraft_build(parsed, p_root, options, r_error);
}

} // namespace godot
