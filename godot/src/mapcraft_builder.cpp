#include "mapcraft_builder.h"

#include <godot_cpp/classes/animation.hpp>
#include <godot_cpp/classes/animation_library.hpp>
#include <godot_cpp/classes/animation_player.hpp>
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
#include <godot_cpp/classes/gltf_document.hpp>
#include <godot_cpp/classes/gltf_node.hpp>
#include <godot_cpp/classes/importer_mesh.hpp>
#include <godot_cpp/classes/importer_mesh_instance3d.hpp>
#include <godot_cpp/classes/skin.hpp>
#include <godot_cpp/classes/gltf_state.hpp>
#include <godot_cpp/classes/image.hpp>
#include <godot_cpp/classes/image_texture.hpp>
#include <godot_cpp/classes/json.hpp>
#include <godot_cpp/classes/light3d.hpp>
#include <godot_cpp/classes/lightmap_gi.hpp>
#include <godot_cpp/classes/lightmap_gi_data.hpp>
#include <godot_cpp/classes/texture2d_array.hpp>
#include <godot_cpp/templates/rb_map.hpp>
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

#include "mapcraft_pack.h"

namespace godot {

namespace {

const char *MATERIAL_EXTENSIONS[] = { "tres", "res", "material" };
const char *IMAGE_EXTENSIONS[] = { "png", "jpg", "jpeg", "webp", "tga", "bmp" };
const double DEFAULT_TEXTURE_SIZE = 64.0;
const int MAX_DEPTH = 256;
const int MAX_PREFAB_DEPTH = 16;
const int MAX_PREFABS = 256;
// mapcraft_map::MAX_SHADER_BYTES.
const int64_t MAX_SHADER_SIZE = int64_t(256) << 10;
const int MAX_SHADER_PARAMS = 256;
const int64_t MAX_NAME_LENGTH = 1024;
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

// A map's name for a file under a folder: relative, with no . or .. segment, no \ : or control character, and no segment Windows would trim.
bool inside_folder(const String &p_path) {
	for (int64_t i = 0; i < p_path.length(); i++) {
		const char32_t c = p_path[i];
		if (c < 0x20 || c == 0x7F || c == '\\' || c == ':') {
			return false;
		}
	}
	const PackedStringArray segments = p_path.split("/");
	for (int64_t i = 0; i < segments.size(); i++) {
		const String segment = segments[i];
		if (segment.is_empty() || segment.ends_with(".") || segment.ends_with(" ")) {
			return false;
		}
	}
	return true;
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

// Godot adds the emission texture to the color, then scales both by the energy.
bool emits(const StandardMaterial3D *p_material) {
	const Color color = p_material->get_emission();
	const bool lit = color.r > 0.0 || color.g > 0.0 || color.b > 0.0 || p_material->get_texture(BaseMaterial3D::TEXTURE_EMISSION).is_valid();
	return lit && p_material->get_emission_energy_multiplier() > 0.0;
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
		if (feature == BaseMaterial3D::FEATURE_EMISSION && !emits(m)) {
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

// Inside an instanced scene only nodes the builder added, which have no owner yet, are claimed; true when any were.
bool own(Node *p_node, Node *p_owner, bool p_instanced = false) {
	const bool claimed = !p_instanced || p_node->get_owner() == nullptr;
	if (claimed) {
		p_node->set_owner(p_owner);
	}
	const bool instance = !p_node->get_scene_file_path().is_empty();
	bool inside = false;
	for (int i = 0; i < p_node->get_child_count(); i++) {
		inside = own(p_node->get_child(i), p_owner, p_instanced || instance) || inside;
	}
	// PackedScene skips an instance's insides unless it is an editable instance.
	if (instance && inside) {
		p_owner->set_editable_instance(p_node, true);
	}
	return (p_instanced && claimed) || inside;
}

// Gives every mesh in a model a static body shaped as one convex hull per mesh, or as its exact triangles; p_modes overrides p_mode per mesh.
void add_model_collision(Node3D *p_model, const String &p_mode, const Dictionary &p_config, const HashMap<Node *, String> &p_modes) {
	TypedArray<Node> meshes = p_model->find_children("*", "MeshInstance3D", true, false);
	for (int64_t i = 0; i < meshes.size(); i++) {
		MeshInstance3D *mesh = Object::cast_to<MeshInstance3D>(meshes[i]);
		if (mesh == nullptr || mesh->get_mesh().is_null()) {
			continue;
		}
		const String mode = p_modes.has(mesh) ? p_modes[mesh] : p_mode;
		if (mode == "none") {
			continue;
		}
		const bool trimesh = mode == "trimesh";
		if (!trimesh && mode != "convex") {
			UtilityFunctions::push_warning("MapCraft: unsupported collision ", mode, "; using convex");
		}
		const int64_t before = mesh->get_child_count();
		if (trimesh) {
			mesh->create_trimesh_collision();
		} else {
			mesh->create_convex_collision(true, true);
		}
		for (int64_t j = before; j < mesh->get_child_count(); j++) {
			StaticBody3D *body = Object::cast_to<StaticBody3D>(mesh->get_child(j));
			// Godot hands the new nodes the model's own owner, which would keep them out of the saved map.
			TypedArray<Node> made = mesh->get_child(j)->find_children("*", "", true, false);
			made.push_back(mesh->get_child(j));
			for (int64_t k = 0; k < made.size(); k++) {
				Object::cast_to<Node>(made[k])->set_owner(nullptr);
			}
			if (body != nullptr && p_config.has("collision_layer")) {
				body->set_collision_layer(int64_t(p_config["collision_layer"]));
			}
			if (body != nullptr && p_config.has("collision_mask")) {
				body->set_collision_mask(int64_t(p_config["collision_mask"]));
			}
		}
	}
}

// One mesh's changes from a prop's mesh.<name>.origin, .angles, .scale, .hidden and .collision keys.
struct MeshEdit {
	Vector3 origin;
	Vector3 angles;
	double scale = 1.0;
	bool hidden = false;
	String collision;
};

bool has_mesh_keys(const Dictionary &p_properties) {
	Array keys = p_properties.keys();
	for (int64_t i = 0; i < keys.size(); i++) {
		if (String(keys[i]).begins_with("mesh.")) {
			return true;
		}
	}
	return false;
}

HashMap<String, MeshEdit> mesh_edits(const Dictionary &p_properties) {
	HashMap<String, MeshEdit> out;
	Array keys = p_properties.keys();
	for (int64_t i = 0; i < keys.size(); i++) {
		const String key = keys[i];
		const int64_t dot = key.rfind(".");
		if (!key.begins_with("mesh.") || dot <= 5) {
			continue;
		}
		const String name = key.substr(5, dot - 5);
		const String field = key.substr(dot + 1);
		const String value = String(p_properties[key]).strip_edges();
		if (!out.has(name)) {
			out[name] = MeshEdit();
		}
		MeshEdit &edit = out[name];
		Vector3 v;
		if (field == "origin" && parse_vec3(value, v)) {
			edit.origin = v;
		} else if (field == "angles" && parse_vec3(value, v)) {
			edit.angles = v;
		} else if (field == "scale" && value.is_valid_float() && value.to_float() > 0.0) {
			edit.scale = value.to_float();
		} else if (field == "hidden") {
			edit.hidden = value != "" && value != "0" && value != "false";
		} else if (field == "collision" && !value.is_empty()) {
			edit.collision = value.to_lower();
		}
	}
	return out;
}

// Matches MapCraft's model loader: a node's glTF name, or #<index> when it has none or shares it.
Vector<String> gltf_node_names(const Ref<GLTFState> &p_state) {
	TypedArray<GLTFNode> nodes = p_state->get_nodes();
	Vector<String> names;
	HashMap<String, int64_t> count;
	for (int64_t i = 0; i < nodes.size(); i++) {
		Ref<GLTFNode> node = nodes[i];
		const String name = node.is_valid() ? node->get_original_name().strip_edges() : String();
		names.push_back(name);
		count[name] = count.has(name) ? count[name] + 1 : 1;
	}
	for (int64_t i = 0; i < names.size(); i++) {
		if (names[i].is_empty() || count[names[i]] > 1) {
			names.set(i, "#" + itos(i));
		}
	}
	return names;
}

Transform3D in_model(Node *p_node, const Node *p_root) {
	Transform3D at;
	for (Node *n = p_node; n != nullptr && n != p_root; n = n->get_parent()) {
		if (Node3D *spatial = Object::cast_to<Node3D>(n)) {
			at = spatial->get_transform() * at;
		}
	}
	return at;
}

// Moves and frees a generated glTF scene's nodes by a prop's mesh keys; r_modes gets each mesh's own collision.
void edit_meshes(const Ref<GLTFState> &p_state, Node3D *p_root, const Dictionary &p_properties, double p_scale, HashMap<Node *, String> &r_modes) {
	const HashMap<String, MeshEdit> edits = mesh_edits(p_properties);
	const Vector<String> names = gltf_node_names(p_state);
	TypedArray<GLTFNode> nodes = p_state->get_nodes();
	const int64_t count = nodes.size();
	Vector<Node *> scene_nodes;
	Vector<Transform3D> rest;
	Vector<Transform3D> moved;
	Vector<int> state;
	for (int64_t i = 0; i < count; i++) {
		// GLTFState's own scene nodes may be freed stand-ins, but the nodes that replace them keep their unique names.
		Ref<GLTFNode> gltf = nodes[i];
		const String name = gltf.is_valid() ? gltf->get_name() : String();
		Node *found = name.is_empty() ? nullptr : (String(p_root->get_name()) == name ? p_root : p_root->find_child(name, true, false));
		scene_nodes.push_back(found);
		rest.push_back(in_model(scene_nodes[i], p_root));
		moved.push_back(Transform3D());
		state.push_back(0);
	}
	// 0 unresolved, 1 placed, 2 hidden; a pass per depth resolves parents before children.
	for (int pass = 0; pass < 64; pass++) {
		bool left = false;
		for (int64_t i = 0; i < count; i++) {
			if (state[i] != 0) {
				continue;
			}
			Ref<GLTFNode> node = nodes[i];
			const int64_t parent = node.is_valid() ? node->get_parent() : -1;
			if (parent >= 0 && parent < count && state[parent] == 0) {
				left = true;
				continue;
			}
			const bool parent_hidden = parent >= 0 && parent < count && state[parent] == 2;
			Transform3D above = parent >= 0 && parent < count ? moved[parent] : Transform3D();
			if (!edits.has(names[i])) {
				moved.set(i, above);
				state.set(i, parent_hidden ? 2 : 1);
				continue;
			}
			const MeshEdit &edit = edits[names[i]];
			if (!edit.collision.is_empty()) {
				r_modes[scene_nodes[i]] = edit.collision;
			}
			const Vector3 pivot = rest[i].origin;
			Basis turn = godot_basis(map_angles(edit.angles));
			turn.scale(Vector3(edit.scale, edit.scale, edit.scale));
			const Vector3 offset = TO_GODOT.xform(edit.origin) * p_scale;
			moved.set(i, above * Transform3D(Basis(), pivot + offset) * Transform3D(turn, Vector3()) * Transform3D(Basis(), -pivot));
			state.set(i, parent_hidden || edit.hidden ? 2 : 1);
		}
		if (!left) {
			break;
		}
	}
	for (int64_t i = 0; i < count; i++) {
		Node3D *spatial = Object::cast_to<Node3D>(scene_nodes[i]);
		if (spatial == nullptr || spatial == p_root || state[i] != 1) {
			continue;
		}
		Node *parent = spatial->get_parent();
		const int64_t j = parent != nullptr ? scene_nodes.find(parent) : -1;
		const Transform3D above = j >= 0 && j < count ? moved[j] * rest[j] : in_model(parent, p_root);
		spatial->set_transform(above.affine_inverse() * moved[i] * rest[i]);
	}
	for (int64_t i = 0; i < count; i++) {
		Ref<GLTFNode> node = nodes[i];
		const int64_t parent = node.is_valid() ? node->get_parent() : -1;
		// A freed node's pointer may still be in the list, so it is only compared, never touched.
		Node *scene = scene_nodes[i];
		const bool top = parent < 0 || parent >= count || state[parent] != 2;
		if (state[i] == 2 && top && scene != nullptr && scene != p_root && scene->get_parent() != nullptr) {
			r_modes.erase(scene);
			scene->get_parent()->remove_child(scene);
			memdelete(scene);
		}
	}
}

// In the editor glTF leaves ImporterMeshInstance3D nodes, which neither draw nor collide.
void convert_importer_meshes(Node *p_root) {
	TypedArray<Node> found = p_root->find_children("*", "ImporterMeshInstance3D", true, false);
	for (int64_t i = 0; i < found.size(); i++) {
		ImporterMeshInstance3D *source = Object::cast_to<ImporterMeshInstance3D>(found[i]);
		if (source == nullptr || source == p_root) {
			continue;
		}
		MeshInstance3D *mesh = memnew(MeshInstance3D);
		mesh->set_name(source->get_name());
		mesh->set_transform(source->get_transform());
		mesh->set_visible(source->is_visible());
		Ref<ImporterMesh> imported = source->get_mesh();
		if (imported.is_valid()) {
			mesh->set_mesh(imported->get_mesh());
		}
		mesh->set_skin(source->get_skin());
		mesh->set_skeleton_path(source->get_skeleton_path());
		Node *owner = source->get_owner();
		source->replace_by(mesh, true);
		mesh->set_owner(owner);
		memdelete(source);
	}
}

// An empty base path leaves glTF nothing outside these bytes to read.
Node3D *load_glb(const PackedByteArray &p_bytes) {
	Ref<GLTFDocument> document;
	document.instantiate();
	Ref<GLTFState> state;
	state.instantiate();
	state->set_handle_binary_image(GLTFState::HANDLE_BINARY_EMBED_AS_UNCOMPRESSED);
	if (document->append_from_buffer(p_bytes, "", state) != OK) {
		return nullptr;
	}
	Node *scene = document->generate_scene(state);
	Node3D *node = Object::cast_to<Node3D>(scene);
	if (node == nullptr && scene != nullptr) {
		memdelete(scene);
	}
	if (node != nullptr) {
		convert_importer_meshes(node);
	}
	return node;
}

// Plays a copy of the animation from its own library, since an imported model's animations are shared by every instance.
void autoplay(Node3D *p_model, const String &p_animation, const String &p_loop) {
	Animation::LoopMode mode = Animation::LOOP_LINEAR;
	if (p_loop == "pingpong") {
		mode = Animation::LOOP_PINGPONG;
	} else if (p_loop == "none") {
		mode = Animation::LOOP_NONE;
	} else if (!p_loop.is_empty() && p_loop != "linear") {
		UtilityFunctions::push_warning("MapCraft: unsupported animation_loop ", p_loop, "; using linear");
	}
	TypedArray<Node> players = p_model->find_children("*", "AnimationPlayer", true, false);
	for (int64_t i = 0; i < players.size(); i++) {
		AnimationPlayer *player = Object::cast_to<AnimationPlayer>(players[i]);
		String name = p_animation;
		if (player != nullptr && !player->has_animation(name)) {
			name = name.validate_node_name();
		}
		if (player == nullptr || !player->has_animation(name)) {
			continue;
		}
		Ref<Animation> looped = player->get_animation(name)->duplicate();
		looped->set_loop_mode(mode);
		Ref<AnimationLibrary> library;
		library.instantiate();
		library->add_animation("default", looped);
		player->add_animation_library("mapcraft", library);
		player->set_autoplay("mapcraft/default");
		return;
	}
	UtilityFunctions::push_warning("MapCraft: ", p_model->get_name(), " has no animation ", p_animation);
}

// Baked lighting from a .mclight file (docs/mclight-format.md): where each face's luxels are, and the atlas.
struct Lighting {
	struct Face {
		int slice = 0;
		float uv[8] = {};
	};
	HashMap<uint64_t, Face> faces;
	TypedArray<Image> layers;
	Ref<Texture2DArray> texture;

	static uint64_t key(uint64_t p_node, uint32_t p_face) {
		return (p_node << 16) ^ uint64_t(p_face & 0xFFFF);
	}

	Error parse(const PackedByteArray &p_bytes, String &r_error) {
		const int64_t total = p_bytes.size();
		if (total < 28 || p_bytes[0] != 'M' || p_bytes[1] != 'C' || p_bytes[2] != 'L' || p_bytes[3] != 'M') {
			r_error = "is not MapCraft baked lighting";
			return ERR_FILE_UNRECOGNIZED;
		}
		if (p_bytes.decode_u32(4) != 2) {
			r_error = "is a lighting version this addon does not read; bake the lighting again";
			return ERR_FILE_UNRECOGNIZED;
		}
		const int64_t size = p_bytes.decode_u32(16);
		const int64_t slices = p_bytes.decode_u32(20);
		const int64_t count = p_bytes.decode_u32(24);
		if (size < 1 || size > 8192 || slices < 1 || slices > 64 || count > 4000000 || 28 + count * 48 + 8 > total) {
			r_error = "has a damaged header";
			return ERR_FILE_CORRUPT;
		}
		int64_t at = 28;
		for (int64_t i = 0; i < count; i++, at += 48) {
			Face face;
			face.slice = int(p_bytes.decode_u32(at + 12));
			bool finite = face.slice < slices;
			for (int k = 0; k < 8; k++) {
				face.uv[k] = float(p_bytes.decode_float(at + 16 + k * 4));
				finite = finite && Math::is_finite(face.uv[k]);
			}
			if (!finite) {
				r_error = "has a face with a bad slice or mapping";
				return ERR_FILE_CORRUPT;
			}
			faces.insert(key(p_bytes.decode_u64(at), p_bytes.decode_u32(at + 8)), face);
		}
		const int64_t packed = int64_t(p_bytes.decode_u64(at));
		at += 8;
		const int64_t layer = size * size * 4;
		const int64_t expected = slices * 4 * layer;
		if (packed < 1 || packed > total - at || expected > (int64_t(1) << 30)) {
			r_error = "has damaged or oversized texels";
			return ERR_FILE_CORRUPT;
		}
		const PackedByteArray texels = p_bytes.slice(at, at + packed).decompress(expected, FileAccess::COMPRESSION_DEFLATE);
		if (texels.size() != expected) {
			r_error = "has texels that do not decompress";
			return ERR_FILE_CORRUPT;
		}
		for (int64_t i = 0; i < slices * 4; i++) {
			Ref<Image> image = Image::create_from_data(int32_t(size), int32_t(size), false, Image::FORMAT_RGBE9995, texels.slice(i * layer, (i + 1) * layer));
			if (image.is_null()) {
				r_error = "has a layer Godot cannot make an image of";
				return ERR_FILE_CORRUPT;
			}
			layers.push_back(image);
		}
		texture.instantiate();
		if (texture->create_from_images(layers) != OK) {
			r_error = "has layers Godot cannot make a texture of";
			return ERR_FILE_CORRUPT;
		}
		return OK;
	}
};

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
	// The layer or group being built and its brushes; trimesh triangles stay the root's so ramps meet without seams.
	Node3D *parent = nullptr;
	Surfaces world_surfaces;
	Vector<Shape> world_hulls;
	PackedVector3Array world_triangles;
	int brush_count = 0;
	bool has_environment = false;
	MapCraftPack pack;
	String pack_path;
	Lighting lighting;
	String lighting_file;
	bool lit = false;
	// Mesh instances drawn with the baked lighting, and the atlas slice each samples.
	Vector<std::pair<MeshInstance3D *, int>> lightmap_users;
	Ref<ShaderMaterial> sky_shader;
	// Brush shaders by source, so brushes with the same code share a material and a surface.
	HashMap<String, Ref<ShaderMaterial>> brush_shaders;
	HashMap<String, Node3D *> models;
	std::shared_ptr<int> prefab_budget;

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
	Ref<Material> pack_material(const String &p_name, const PackedByteArray &p_bytes, const String &p_problem);
	bool pack_model(const String &p_path, Node3D *&r_node);
	Node3D *edited_model(const String &p_path, bool p_packed, const Dictionary &p_properties, HashMap<Node *, String> &r_modes);
	Ref<Material> variant(const String &p_a, const String &p_b, AntiTile p_mode, Vector2 &r_size);
	bool add_brush(const Dictionary &p_brush, Surfaces *p_surfaces, Vector<Shape> *p_hulls, bool p_world = false);
	void add_displacement(const Dictionary &p_face, const Array &p_grid, Surfaces *p_surfaces, PackedVector3Array &r_triangles, uint64_t p_node, int p_index);
	Ref<SurfaceTool> surface_for(Surfaces &p_surfaces, const String &p_key, const Ref<Material> &p_material);
	Ref<ShaderMaterial> embedded_shader(const Dictionary &p_node, Shader::Mode p_mode);
	void set_shader_params(const Ref<ShaderMaterial> &p_material, const Variant &p_params);
	void walk(const Dictionary &p_node, int p_depth);
	void add_entity(const Dictionary &p_entity);
	void add_prefab(const Dictionary &p_prefab);
	void add_environment(const Dictionary &p_properties);
	void add_container(const Dictionary &p_node, int p_depth);
	void finish_brushes(Node3D *p_into, const String &p_mesh, const String &p_collision, bool p_root);
	MeshInstance3D *make_mesh(Surfaces &p_surfaces, const Vector3 &p_offset);
	void add_shapes(Node *p_parent, const Vector<Shape> &p_hulls, const Vector3 &p_offset);
	void add(Node *p_parent, Node *p_child, bool p_readable_name);
	void load_lighting();
	void add_lightmap();

public:
	Builder(Node3D *p_root, const MapCraftBuildOptions &p_options) :
			root(p_root), options(p_options), scale(1.0 / (p_options.units_per_meter > 0.0 ? p_options.units_per_meter : 39.37)),
			prefab_budget(p_options.prefab_budget ? p_options.prefab_budget : std::make_shared<int>(MAX_PREFABS)) {}

	~Builder() {
		for (KeyValue<String, Node3D *> &kv : models) {
			if (kv.value != nullptr) {
				memdelete(kv.value);
			}
		}
	}

	Error build(const Dictionary &p_doc, String &r_error);
};

// Clear pixels show through as they do in MapCraft's views: cut out when every pixel is fully on or off, blended otherwise.
void match_alpha(const Ref<StandardMaterial3D> &p_material, const Ref<Image> &p_image) {
	if (p_image.is_null() || p_material->get_transparency() != BaseMaterial3D::TRANSPARENCY_DISABLED) {
		return;
	}
	const Image::AlphaMode alpha = p_image->detect_alpha();
	if (alpha == Image::ALPHA_BIT) {
		p_material->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA_SCISSOR);
	} else if (alpha == Image::ALPHA_BLEND) {
		p_material->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA);
	}
}

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
	PackedByteArray packed;
	String problem;
	const bool in_pack = !p_name.is_empty() && pack.find("materials/" + p_name, packed, problem);
	if (in_pack) {
		mat = pack_material(p_name, packed, problem);
	}
	const bool look = !in_pack && inside_folder(p_name);
	if (!in_pack && !p_name.is_empty() && !look) {
		UtilityFunctions::push_warning("MapCraft: material ", p_name.c_escape(), " is not a name inside ", options.material_root, ", drawn as missing");
	}
	for (const char *ext : MATERIAL_EXTENSIONS) {
		if (look && mat.is_null() && loader->exists(base + "." + ext)) {
			mat = loader->load(base + "." + ext);
		}
	}
	for (const char *ext : IMAGE_EXTENSIONS) {
		if (look && mat.is_null() && loader->exists(base + "." + ext)) {
			Ref<Texture2D> texture = loader->load(base + "." + ext);
			if (texture.is_valid()) {
				Ref<StandardMaterial3D> generated;
				generated.instantiate();
				generated->set_texture(BaseMaterial3D::TEXTURE_ALBEDO, texture);
				match_alpha(generated, texture->get_image());
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

Ref<Material> Builder::pack_material(const String &p_name, const PackedByteArray &p_bytes, const String &p_problem) {
  if (p_problem.is_empty() && !p_bytes.is_empty() && p_bytes[0] != 0x89 &&
      p_bytes[0] != 0xFF) {
    const Dictionary descriptor =
        JSON::parse_string(p_bytes.get_string_from_utf8());
    Ref<StandardMaterial3D> generated;
    generated.instantiate();
    const Array albedo = descriptor["albedo"];
    const Array emission = descriptor["emission"];
    generated->set_albedo(Color(albedo[0], albedo[1], albedo[2], albedo[3]));
    generated->set_roughness(descriptor["roughness"]);
    generated->set_metallic(descriptor["metallic"]);
    generated->set_emission(Color(emission[0], emission[1], emission[2]));
    generated->set_emission_energy_multiplier(descriptor.get("emission_energy", 1.0));
    generated->set_feature(BaseMaterial3D::FEATURE_EMISSION, true);
    generated->set_normal_scale(descriptor.get("normal_strength", 1.0));
    if (double(albedo[3]) < 1.0) {
      generated->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA);
    }
    const Dictionary textures = descriptor["textures"];
    const Array slots = textures.keys();
    for (int i = 0; i < slots.size(); i++) {
      const String slot = slots[i];
      const String name = textures[slot];
      PackedByteArray bytes;
      String problem;
      if (!pack.find(name.get_basename(), bytes, problem) ||
          !problem.is_empty()) {
        UtilityFunctions::push_warning("MapCraft: ", pack_path, ": ", name, " ",
                                       problem);
        return Ref<Material>();
      }
      Ref<Image> image;
      image.instantiate();
      const Error result = name.ends_with(".png")
                               ? image->load_png_from_buffer(bytes)
                               : image->load_jpg_from_buffer(bytes);
      if (result != OK || image->is_empty()) {
        UtilityFunctions::push_warning("MapCraft: ", pack_path, ": ", name,
                                       " cannot be decoded");
        return Ref<Material>();
      }
      image->generate_mipmaps();
      BaseMaterial3D::TextureParam target = BaseMaterial3D::TEXTURE_ALBEDO;
      if (slot == "normal") {
        target = BaseMaterial3D::TEXTURE_NORMAL;
        generated->set_feature(BaseMaterial3D::FEATURE_NORMAL_MAPPING, true);
      }
      if (slot == "roughness") {
        target = BaseMaterial3D::TEXTURE_ROUGHNESS;
        generated->set_roughness_texture_channel(
            BaseMaterial3D::TEXTURE_CHANNEL_RED);
      }
      if (slot == "metallic") {
        target = BaseMaterial3D::TEXTURE_METALLIC;
        generated->set_metallic_texture_channel(
            BaseMaterial3D::TEXTURE_CHANNEL_RED);
      }
      if (slot == "emission") {
        target = BaseMaterial3D::TEXTURE_EMISSION;
      }
      if (slot == "height") {
        target = BaseMaterial3D::TEXTURE_HEIGHTMAP;
        generated->set_feature(BaseMaterial3D::FEATURE_HEIGHT_MAPPING, true);
        generated->set_heightmap_scale(descriptor.get("height_scale", 1.0));
        generated->set_flag(BaseMaterial3D::FLAG_INVERT_HEIGHTMAP, descriptor.get("height_flip", false));
        generated->set_heightmap_deep_parallax(true);
      }
      if (target == BaseMaterial3D::TEXTURE_ALBEDO) {
        match_alpha(generated, image);
      }
      generated->set_texture(target, ImageTexture::create_from_image(image));
    }
    return generated;
  }

	Ref<Image> image;
	image.instantiate();
	Error err = FAILED;
	if (p_problem.is_empty()) {
		err = p_bytes[0] == 0x89 ? image->load_png_from_buffer(p_bytes) : image->load_jpg_from_buffer(p_bytes);
	}
	if (err != OK || image->is_empty()) {
		UtilityFunctions::push_warning("MapCraft: ", pack_path, ": ", p_name, " ", p_problem.is_empty() ? String("cannot be decoded") : p_problem, ", drawn as missing");
		return Ref<Material>();
	}
	image->generate_mipmaps();
	Ref<StandardMaterial3D> generated;
	generated.instantiate();
	match_alpha(generated, image);
	generated->set_texture(BaseMaterial3D::TEXTURE_ALBEDO, ImageTexture::create_from_image(image));
	return generated;
}

// True when the map's pack holds this model; r_node stays null when that entry fails.
bool Builder::pack_model(const String &p_path, Node3D *&r_node) {
	const String extension = p_path.get_extension().to_lower();
	if (extension != "glb" && extension != "gltf") {
		return false;
	}
	const String stem = ("models/" + p_path.get_basename()).to_lower();
	if (!models.has(stem)) {
		PackedByteArray bytes;
		String problem;
		if (!pack.find(stem, bytes, problem)) {
			return false;
		}
		Node3D *model = problem.is_empty() ? load_glb(bytes) : nullptr;
		if (model == nullptr) {
			UtilityFunctions::push_warning("MapCraft: ", pack_path, ": ", stem, ".glb ", problem.is_empty() ? String("cannot be loaded") : problem);
		}
		models[stem] = model;
	}
	Node3D *model = models[stem];
	r_node = model != nullptr ? Object::cast_to<Node3D>(model->duplicate()) : nullptr;
	return true;
}

// Loads a model's glTF afresh, keeping which node each glTF node became, and applies the prop's mesh keys; null for animated models.
Node3D *Builder::edited_model(const String &p_path, bool p_packed, const Dictionary &p_properties, HashMap<Node *, String> &r_modes) {
	Ref<GLTFDocument> document;
	document.instantiate();
	Ref<GLTFState> state;
	state.instantiate();
	state->set_handle_binary_image(GLTFState::HANDLE_BINARY_EMBED_AS_UNCOMPRESSED);
	Error loaded = FAILED;
	if (p_packed) {
		PackedByteArray bytes;
		String problem;
		const String stem = ("models/" + p_path.trim_prefix("res://").get_basename()).to_lower();
		if (pack.find(stem, bytes, problem) && problem.is_empty()) {
			loaded = document->append_from_buffer(bytes, "", state);
		}
	} else {
		loaded = document->append_from_file(p_path, state);
	}
	if (loaded != OK || state->get_animations().size() > 0) {
		return nullptr;
	}
	Node *scene = document->generate_scene(state);
	Node3D *root = Object::cast_to<Node3D>(scene);
	if (root == nullptr) {
		if (scene != nullptr) {
			memdelete(scene);
		}
		return nullptr;
	}
	convert_importer_meshes(root);
	edit_meshes(state, root, p_properties, scale, r_modes);
	return root;
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

Ref<ShaderMaterial> Builder::embedded_shader(const Dictionary &p_node, Shader::Mode p_mode) {
	const Variant code = p_node.get("shader", Variant());
	if (code.get_type() != Variant::STRING) {
		return Ref<ShaderMaterial>();
	}
	const String source = code;
	if (source.utf8().length() > MAX_SHADER_SIZE) {
		UtilityFunctions::push_warning("MapCraft: an embedded shader is larger than 256 KiB and was skipped");
		return Ref<ShaderMaterial>();
	}
	const Variant params = p_node.get("shader_params", Variant());
	const String key = source + String("\n") + (params.get_type() == Variant::DICTIONARY ? JSON::stringify(params) : String());
	if (p_mode == Shader::MODE_SPATIAL && brush_shaders.has(key)) {
		return brush_shaders[key];
	}
	Ref<Shader> shader;
	shader.instantiate();
	shader->set_code(source);
	if (shader->get_mode() != p_mode) {
		UtilityFunctions::push_warning("MapCraft: an embedded shader is not a ", p_mode == Shader::MODE_SKY ? "sky" : "spatial", " shader and was skipped");
		return Ref<ShaderMaterial>();
	}
	Ref<ShaderMaterial> material;
	material.instantiate();
	material->set_shader(shader);
	set_shader_params(material, params);
	if (p_mode == Shader::MODE_SPATIAL) {
		brush_shaders[key] = material;
	}
	return material;
}

void Builder::set_shader_params(const Ref<ShaderMaterial> &p_material, const Variant &p_params) {
	if (p_params.get_type() != Variant::DICTIONARY) {
		return;
	}
	const Dictionary params = p_params;
	if (params.size() > MAX_SHADER_PARAMS) {
		UtilityFunctions::push_warning("MapCraft: an embedded shader has more than ", MAX_SHADER_PARAMS, " parameters; they were skipped");
		return;
	}
	HashMap<String, Dictionary> uniforms;
	const Array list = p_material->get_shader()->get_shader_uniform_list();
	for (int i = 0; i < list.size(); i++) {
		const Dictionary uniform = list[i];
		uniforms[String(uniform.get("name", ""))] = uniform;
	}
	const Array names = params.keys();
	for (int i = 0; i < names.size(); i++) {
		const String name = names[i];
		const Variant value = params[names[i]];
		const Dictionary *uniform = uniforms.getptr(name);
		const auto skip = [&](const String &p_why) {
			UtilityFunctions::push_warning("MapCraft: shader parameter ", name.left(64), " ", p_why, "; skipped");
		};
		if (uniform == nullptr) {
			skip("is not a uniform of the shader");
			continue;
		}
		const int type = int(uniform->get("type", Variant::NIL));
		const bool number = value.get_type() == Variant::FLOAT || value.get_type() == Variant::INT;
		Array numbers;
		bool vector = value.get_type() == Variant::ARRAY && Array(value).size() <= 4;
		if (vector) {
			numbers = value;
			for (int j = 0; j < numbers.size(); j++) {
				const Variant::Type t = numbers[j].get_type();
				vector = vector && (t == Variant::FLOAT || t == Variant::INT) && Math::is_finite(double(numbers[j]));
			}
		}
		const auto component = [&](int p_index) { return float(double(numbers[p_index])); };
		Variant converted;
		if (type == Variant::FLOAT && number && Math::is_finite(double(value))) {
			converted = double(value);
		} else if (type == Variant::INT && number && Math::is_finite(double(value)) && Math::abs(double(value)) < 2147483648.0) {
			converted = int64_t(double(value));
		} else if (type == Variant::BOOL && value.get_type() == Variant::BOOL) {
			converted = value;
		} else if (type == Variant::VECTOR2 && vector && numbers.size() == 2) {
			converted = Vector2(component(0), component(1));
		} else if (type == Variant::VECTOR3 && vector && numbers.size() == 3) {
			converted = Vector3(component(0), component(1), component(2));
		} else if (type == Variant::VECTOR4 && vector && numbers.size() == 4) {
			converted = Vector4(component(0), component(1), component(2), component(3));
		} else if (type == Variant::COLOR && vector && numbers.size() >= 3) {
			converted = Color(component(0), component(1), component(2), numbers.size() == 4 ? component(3) : 1.0f);
		} else if (type == Variant::OBJECT && value.get_type() == Variant::STRING && String(value).length() <= MAX_NAME_LENGTH && inside_folder(value)) {
			Vector2 size;
			const Ref<BaseMaterial3D> source = material(value, size);
			const Ref<Texture2D> texture = source.is_valid() ? source->get_texture(BaseMaterial3D::TEXTURE_ALBEDO) : Ref<Texture2D>();
			if (texture.is_null()) {
				skip("names a material with no picture");
				continue;
			}
			converted = texture;
		} else {
			skip("does not match the uniform's type");
			continue;
		}
		p_material->set_shader_parameter(name, converted);
	}
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

void Builder::add_displacement(const Dictionary &p_face, const Array &p_grid, Surfaces *p_surfaces, PackedVector3Array &r_triangles, uint64_t p_node, int p_index) {
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
	const Lighting::Face *baked = lit ? lighting.faces.getptr(Lighting::key(p_node, uint32_t(p_index))) : nullptr;
	const String slice = baked != nullptr ? "#lm" + itos(baked->slice) : String();
	Ref<SurfaceTool> st = surface_for(*p_surfaces, surface_key(name, layer, mode) + slice, mat);
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
		if (baked != nullptr) {
			const float *r = baked->uv;
			st->set_uv2(Vector2(r[0] * m.x + r[1] * m.y + r[2] * m.z + r[3], r[4] * m.x + r[5] * m.y + r[6] * m.z + r[7]));
		}
		st->add_vertex(points[k]);
	}
}

bool Builder::add_brush(const Dictionary &p_brush, Surfaces *p_surfaces, Vector<Shape> *p_hulls, bool p_world) {
	const bool trimesh = p_world && (options.trimesh_collision || (options.entity_map.is_valid() && options.entity_map->get_trimesh_collision()));
	Array faces = p_brush.get("faces", Array());
	bool displaced = false;
	for (int i = 0; i < faces.size(); i++) {
		const Dictionary face = faces[i];
		displaced = displaced || face.has("disp_vertices");
		// One clip face hides the whole brush, as in the editor.
		if (equal_ignore_ascii_case(String(face.get("material", "")), "tools/clip")) {
			p_surfaces = nullptr;
		}
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
	const uint64_t node_id = uint64_t(int64_t(p_brush.get("id", -1)));
	const Ref<ShaderMaterial> shaded = p_surfaces != nullptr ? embedded_shader(p_brush, Shader::MODE_SPATIAL) : Ref<ShaderMaterial>();
	for (int i = 0; i < faces.size(); i++) {
		Dictionary face = faces[i];
		Array polygon = face.get("polygon", Array());
		Array grid = face.get("disp_vertices", Array());
		// A displaced brush is only its displaced faces, as Source compiles it.
		if (displaced) {
			add_displacement(face, grid, p_surfaces, triangles, node_id, i);
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
		// Hammer's tools/ materials (nodraw, skip, clip, trigger) collide but are never drawn.
		if (p_surfaces == nullptr || name.to_lower().begins_with("tools/")) {
			continue;
		}

		const AntiTile mode = anti_tile_of(face);
		Vector2 size;
		Ref<Material> face_material = variant(name, String(), mode, size);
		const Lighting::Face *baked = lit ? lighting.faces.getptr(Lighting::key(node_id, uint32_t(i))) : nullptr;
		// Faces on each lightmap slice get surfaces of their own, which make_mesh gives meshes of their own.
		const String slice = baked != nullptr ? "#lm" + itos(baked->slice) : String();
		Ref<SurfaceTool> st = shaded.is_valid()
				? surface_for(*p_surfaces, "shader:" + itos(shaded->get_instance_id()) + slice, shaded)
				: surface_for(*p_surfaces, surface_key(name, String(), mode) + slice, face_material);
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
			if (baked != nullptr) {
				const float *r = baked->uv;
				st->set_uv2(Vector2(r[0] * m.x + r[1] * m.y + r[2] * m.z + r[3], r[4] * m.x + r[5] * m.y + r[6] * m.z + r[7]));
			}
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
	// Godot gives a mesh instance one lightmap slice, so lightmapped surfaces split off by slice; -1 is unlit.
	RBMap<int, Ref<ArrayMesh>> meshes;
	for (KeyValue<String, Ref<SurfaceTool>> &kv : p_surfaces) {
		const int at = kv.key.rfind("#lm");
		const int slice = at >= 0 ? kv.key.substr(at + 3).to_int() : -1;
		if (!meshes.has(slice)) {
			Ref<ArrayMesh> mesh;
			mesh.instantiate();
			meshes.insert(slice, mesh);
		}
		Ref<ArrayMesh> mesh = meshes[slice];
		kv.value->generate_tangents();
		kv.value->commit(mesh);
		int surface = mesh->get_surface_count() - 1;
		mesh->surface_set_name(surface, kv.key);
		const Ref<Material> *mat = surface_materials.getptr(kv.key);
		if (mat != nullptr && mat->is_valid()) {
			mesh->surface_set_material(surface, *mat);
		}
	}
	MeshInstance3D *first = nullptr;
	for (const KeyValue<int, Ref<ArrayMesh>> &kv : meshes) {
		MeshInstance3D *mi = memnew(MeshInstance3D);
		mi->set_mesh(kv.value);
		if (first == nullptr) {
			first = mi;
			mi->set_name("Mesh");
			mi->set_position(-p_offset);
		} else {
			mi->set_name("Lightmap" + itos(kv.key));
			first->add_child(mi);
		}
		if (kv.key >= 0) {
			lightmap_users.push_back({ mi, kv.key });
		}
	}
	if (first == nullptr) {
		first = memnew(MeshInstance3D);
		first->set_name("Mesh");
	}
	return first;
}

void Builder::load_lighting() {
	PackedByteArray bytes;
	String from = pack_path;
	if (!pack.lighting(bytes)) {
		// A plain .mcmap finds the .mclight the editor bakes beside it.
		from = options.source_path.get_basename() + ".mclight";
		if (options.source_path.is_empty() || options.source_path.get_extension().to_lower() == "mcpck" || !FileAccess::file_exists(from)) {
			return;
		}
		bytes = FileAccess::get_file_as_bytes(from);
		lighting_file = from;
	}
	String error;
	if (lighting.parse(bytes, error) != OK) {
		UtilityFunctions::push_error("MapCraft: the baked lighting in ", from, " ", error, ", so the map uses realtime lighting");
		lighting = Lighting();
		return;
	}
	lit = true;
}

void Builder::add_lightmap() {
	LightmapGI *gi = memnew(LightmapGI);
	gi->set_name("BakedLighting");
	gi->set_directional(true);
	add(root, gi, false);
	Ref<LightmapGIData> data;
	data.instantiate();
	data->set_uses_spherical_harmonics(true);
	// The L1 bands are stored divided by L0 the way Godot 4.4's baker packs them; this internal flag says so.
	data->set("_uses_packed_directional", true);
	Ref<TextureLayered> texture = lighting.texture;
	if (options.imported_lighting && !lighting_file.is_empty()) {
		Ref<TextureLayered> imported = ResourceLoader::get_singleton()->load(lighting_file, "TextureLayered");
		if (imported.is_valid()) {
			texture = imported;
		} else {
			UtilityFunctions::push_warning("MapCraft: ", lighting_file, " is not imported, so its lighting keeps texels only when imported with a GPU");
		}
	}
	TypedArray<TextureLayered> textures;
	textures.push_back(texture);
	data->set_lightmap_textures(textures);
	AABB bounds;
	bool first = true;
	for (const std::pair<MeshInstance3D *, int> &user : lightmap_users) {
		data->add_user(gi->get_path_to(user.first), Rect2(0, 0, 1, 1), user.second, -1);
		Transform3D to_root;
		for (Node *n = user.first; n != nullptr && n != root; n = n->get_parent()) {
			if (Node3D *n3 = Object::cast_to<Node3D>(n)) {
				to_root = n3->get_transform() * to_root;
			}
		}
		const AABB box = to_root.xform(user.first->get_aabb());
		bounds = first ? box : bounds.merge(box);
		first = false;
	}
	// Godot culls a lightmap by its probe bounds, and drops empty probe data's bounds, so one tetrahedron spans the map.
	bounds = bounds.grow(1.0);
	const Vector3 lo = bounds.position;
	const Vector3 hi = bounds.get_end();
	Dictionary probes;
	probes["bounds"] = bounds;
	probes["points"] = PackedVector3Array({ lo, Vector3(hi.x * 3.0 - lo.x * 2.0, lo.y, lo.z), Vector3(lo.x, hi.y * 3.0 - lo.y * 2.0, lo.z), Vector3(lo.x, lo.y, hi.z * 3.0 - lo.z * 2.0) });
	PackedColorArray sh;
	sh.resize(36);
	sh.fill(Color(0, 0, 0, 0));
	probes["sh"] = sh;
	probes["tetrahedra"] = PackedInt32Array({ 0, 1, 2, 3 });
	// One BSP node whose both sides lead to tetrahedron 0, encoded as -1.
	probes["bsp"] = PackedInt32Array({ 0, 0, 0, 0, -1, -1 });
	probes["interior"] = false;
	probes["baked_exposure"] = 1.0;
	data->set("probe_data", probes);
	gi->set_light_data(data);
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
			// One-sided like Source brushes; inside faces catch a hull crossing a ridge.
			shape->set_backface_collision_enabled(false);
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
	if (type == "layer" || type == "group") {
		add_container(p_node, p_depth);
		return;
	}
	Array children = p_node.get("children", Array());
	for (int i = 0; i < children.size(); i++) {
		walk(children[i], p_depth + 1);
	}
}

void Builder::add_container(const Dictionary &p_node, int p_depth) {
	Node3D *node = memnew(Node3D);
	String name = String(p_node.get("name", "")).validate_node_name();
	node->set_name(name.is_empty() ? String(p_node.get("type", "")).capitalize() : name);
	Node3D *outer = parent;
	Surfaces surfaces = world_surfaces;
	Vector<Shape> hulls = world_hulls;
	world_surfaces.clear();
	world_hulls.clear();
	parent = node;
	Array children = p_node.get("children", Array());
	for (int i = 0; i < children.size(); i++) {
		walk(children[i], p_depth + 1);
	}
	finish_brushes(node, "Mesh", "Collision", false);
	parent = outer;
	world_surfaces = surfaces;
	world_hulls = hulls;
	if (node->get_child_count() == 0) {
		memdelete(node);
		return;
	}
	add(parent, node, true);
}

void Builder::finish_brushes(Node3D *p_into, const String &p_mesh, const String &p_collision, bool p_root) {
	if (!world_surfaces.is_empty()) {
		MeshInstance3D *mesh = make_mesh(world_surfaces, Vector3());
		mesh->set_name(p_mesh);
		add(p_into, mesh, false);
	}
	if (p_root && !world_triangles.is_empty()) {
		world_hulls.push_back(Shape{ world_triangles, true });
	}
	if (options.collision && !world_hulls.is_empty()) {
		StaticBody3D *body = memnew(StaticBody3D);
		body->set_name(p_collision);
		add(p_into, body, false);
		add_shapes(body, world_hulls, Vector3());
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
		bool loadable = !path.is_empty();
		bool packed = false;
		if (kind == "model") {
			const String model = String(properties.get(config.get("key", "model"), "")).strip_edges().replace("\\", "/");
			const String relative = model.begins_with("res://") ? model.substr(6) : model.trim_prefix("/");
			path = "res://" + relative;
			loadable = inside_folder(relative);
			packed = loadable && pack_model(relative, node);
		}
		Ref<PackedScene> scene;
		if (loadable && !packed) {
			scene = ResourceLoader::get_singleton()->load(path);
		}
		if (scene.is_valid()) {
			node = Object::cast_to<Node3D>(scene->instantiate());
		}
		HashMap<Node *, String> mesh_modes;
		if (node != nullptr && kind == "model" && has_mesh_keys(properties)) {
			Node3D *edited = edited_model(path, packed, properties, mesh_modes);
			if (edited != nullptr) {
				memdelete(node);
				node = edited;
			}
		}
		if (node == nullptr) {
			UtilityFunctions::push_warning("MapCraft: ", classname, " has no usable scene ", path, "; using a Node3D");
		}
		const String animation = String(properties.get("animation", "")).strip_edges();
		if (node != nullptr && kind == "model" && !animation.is_empty()) {
			autoplay(node, animation, String(properties.get("animation_loop", "")).strip_edges().to_lower());
		}
		if (node != nullptr && kind == "model") {
			add_model_collision(node, String(properties.get("collision", "convex")).strip_edges().to_lower(), config, mesh_modes);
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

	add(bool(config.get("root", false)) ? root : parent, node, true);
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
		path = options.source_path.get_base_dir().path_join(path);
	}
	path = path.simplify_path();
	// Built from the file's bytes: ResourceLoader would follow a .remap or .import file beside the path, or one a mounted pack added, to any scene, scripts and all.
	const String here = options.source_path.simplify_path();
	String refusal;
	if (path.get_extension().to_lower() != "mcmap") {
		refusal = "is not a map";
	} else if (path == here || options.prefabs_above.has(path)) {
		refusal = "contains itself";
	} else if (options.prefabs_above.size() >= MAX_PREFAB_DEPTH) {
		refusal = "is nested over " + itos(MAX_PREFAB_DEPTH) + " deep";
	} else if (*prefab_budget <= 0) {
		refusal = "is past the map's " + itos(MAX_PREFABS) + " prefab copies";
	}
	Node3D *instance = nullptr;
	if (refusal.is_empty()) {
		--*prefab_budget;
		MapCraftBuildOptions nested = options;
		nested.source_path = path;
		nested.prefabs_above.push_back(here);
		nested.prefab_budget = prefab_budget;
		nested.own_nodes = false;
		nested.on_entity = nullptr;
		instance = memnew(Node3D);
		instance->set_name(path.get_file().get_basename().validate_node_name());
		if (mapcraft_build_file(path, instance, nested, refusal) != OK) {
			memdelete(instance);
			instance = nullptr;
		}
	}
	if (instance == nullptr) {
		UtilityFunctions::push_warning("MapCraft: prefab ", path, " ", refusal, "; left out");
		return;
	}
	instance->set_transform(Transform3D(godot_basis(map_angles(read_vec3(p_prefab.get("angles", Array())))), to_godot(origin)));
	add(parent, instance, true);
}

void Builder::add_environment(const Dictionary &p_properties) {
	has_environment = true;
	const auto bounded = [&](const String &key, double fallback, double minimum, double maximum) {
		if (!p_properties.has(key)) { return fallback; }
		const String raw = String(p_properties[key]).strip_edges();
		const double value = raw.is_valid_float() ? raw.to_float() : NAN;
		if (!Math::is_finite(value)) {
			UtilityFunctions::push_warning("MapCraft: light_environment ", key, " must be finite and between ", minimum, " and ", maximum, "; using ", fallback);
			return fallback;
		}
		if (value < minimum || value > maximum) {
			UtilityFunctions::push_warning("MapCraft: light_environment ", key, " ", value, " is outside ", minimum, " to ", maximum, "; using ", CLAMP(value, minimum, maximum));
			return CLAMP(value, minimum, maximum);
		}
		return value;
	};
	const auto enabled = [&](const String &key, bool fallback) {
		if (!p_properties.has(key)) { return fallback; }
		const String value = String(p_properties[key]).strip_edges().to_lower();
		if (value == "-1" || value == "auto") { return fallback; }
		if (value.is_valid_float()) { const double number = value.to_float(); return Math::is_finite(number) ? number != 0.0 : fallback; }
		return value != "0" && value != "false" && value != "off" && !value.is_empty();
	};
	const auto environment_color = [&](const String &key, const Color &fallback) {
		if (!p_properties.has(key)) { return fallback; }
		Vector3 rgb;
		if (!parse_vec3(p_properties[key], rgb) || !rgb.is_finite() || rgb.x < 0 || rgb.y < 0 || rgb.z < 0 || rgb.x > 255 || rgb.y > 255 || rgb.z > 255) {
			UtilityFunctions::push_warning("MapCraft: light_environment ", key, " must have finite color components from 0 to 255; using defaults");
			return fallback;
		}
		return Color(rgb.x / 255.0, rgb.y / 255.0, rgb.z / 255.0);
	};
	Color horizon = environment_color("sky_horizon_color", Color(0.72, 0.8, 0.88));
	Ref<ProceduralSkyMaterial> sky_material;
	sky_material.instantiate();
	sky_material->set_sky_top_color(environment_color("sky_top_color", Color(0.3, 0.48, 0.75)));
	sky_material->set_sky_horizon_color(horizon);
	sky_material->set_ground_horizon_color(horizon);
	sky_material->set_ground_bottom_color(environment_color("ground_color", Color(0.2, 0.21, 0.23)));
	sky_material->set_sky_curve(bounded("sky_curve", 0.15, 0.001, 1.0));
	sky_material->set_ground_curve(bounded("ground_curve", 0.02, 0.001, 1.0));
	sky_material->set_sky_energy_multiplier(bounded("sky_energy_multiplier", 1.0, 0.0, 64.0));
	sky_material->set_ground_energy_multiplier(bounded("ground_energy_multiplier", 1.0, 0.0, 64.0));
	Ref<Sky> sky;
	sky.instantiate();
	if (sky_shader.is_valid()) {
		sky->set_material(sky_shader);
	} else {
		sky->set_material(sky_material);
	}

	Ref<Environment> environment;
	environment.instantiate();
	environment->set_background(Environment::BG_SKY);
	environment->set_sky(sky);
	environment->set_ambient_source(Environment::AMBIENT_SOURCE_SKY);
	environment->set_ambient_light_energy(bounded("ambient_energy", 1.0, 0.0, 64.0));
	environment->set_reflection_source(Environment::REFLECTION_SOURCE_SKY);

	String tone = String(p_properties.get("tonemap_mode", "2")).strip_edges().to_lower();
	Environment::ToneMapper mapper = Environment::TONE_MAPPER_FILMIC;
	if (tone == "0" || tone == "linear") { mapper = Environment::TONE_MAPPER_LINEAR; }
	else if (tone == "1" || tone == "reinhardt") { mapper = Environment::TONE_MAPPER_REINHARDT; }
	else if (tone == "3" || tone == "aces") { mapper = Environment::TONE_MAPPER_ACES; }
	else if (tone == "4" || tone == "agx") { mapper = Environment::TONE_MAPPER_AGX; }
	else if (tone != "2" && tone != "filmic") { UtilityFunctions::push_warning("MapCraft: unsupported tonemap_mode ", tone, "; using filmic"); }
	environment->set_tonemapper(mapper);
	environment->set_tonemap_exposure(bounded("tonemap_exposure", 1.0, 0.0, 64.0));
	environment->set_tonemap_white(bounded("tonemap_white", 1.0, 0.001, 64.0));
	environment->set_adjustment_enabled(enabled("adjustment_enabled", false));
	environment->set_adjustment_brightness(bounded("adjustment_brightness", 1.0, 0.0, 8.0));
	environment->set_adjustment_contrast(bounded("adjustment_contrast", 1.0, 0.0, 8.0));
	environment->set_adjustment_saturation(bounded("adjustment_saturation", 1.0, 0.0, 8.0));
	environment->set_glow_enabled(enabled("glow_enabled", false));
	environment->set_glow_intensity(bounded("glow_intensity", 0.3, 0.0, 8.0));
	environment->set_glow_strength(bounded("glow_strength", 1.0, 0.0, 2.0));
	environment->set_glow_bloom(bounded("glow_bloom", 0.0, 0.0, 1.0));
	environment->set_glow_hdr_bleed_threshold(bounded("glow_hdr_threshold", 1.0, 0.0, 4.0));
	environment->set_glow_hdr_bleed_scale(bounded("glow_hdr_scale", 2.0, 0.0, 4.0));
	environment->set_glow_hdr_luminance_cap(bounded("glow_hdr_luminance_cap", 12.0, 0.0, 256.0));
	environment->set_glow_mix(bounded("glow_mix", 0.05, 0.0, 1.0));
	const String blend = String(p_properties.get("glow_blend_mode", "1")).strip_edges().to_lower();
	Environment::GlowBlendMode glow_blend = Environment::GLOW_BLEND_MODE_SCREEN;
	if (blend == "0" || blend == "additive") { glow_blend = Environment::GLOW_BLEND_MODE_ADDITIVE; }
	else if (blend == "2" || blend == "softlight") { glow_blend = Environment::GLOW_BLEND_MODE_SOFTLIGHT; }
	else if (blend == "3" || blend == "replace") { glow_blend = Environment::GLOW_BLEND_MODE_REPLACE; }
	else if (blend == "4" || blend == "mix") { glow_blend = Environment::GLOW_BLEND_MODE_MIX; }
	else if (blend != "1" && blend != "screen") { UtilityFunctions::push_warning("MapCraft: unsupported glow_blend_mode ", blend, "; using screen"); }
	environment->set_glow_blend_mode(glow_blend);
	const double fog = bounded("fog_density", 0.0, 0.0, 1.0);
	const double height_density = bounded("fog_height_density", 0.0, -16.0, 16.0);
	environment->set_fog_enabled(enabled("fog_enabled", fog > 0.0 || height_density != 0.0));
	const String fog_mode = String(p_properties.get("fog_mode", "0")).strip_edges().to_lower();
	const bool depth = fog_mode == "depth" || (fog_mode.is_valid_float() && fog_mode.to_float() == 1.0);
	if (!depth && fog_mode != "exponential" && !(fog_mode.is_valid_float() && fog_mode.to_float() == 0.0)) {
		UtilityFunctions::push_warning("MapCraft: unsupported fog_mode ", fog_mode, "; using exponential");
	}
	// Godot's set_fog_mode resets the density, so the density goes in after it.
	environment->set_fog_mode(depth ? Environment::FOG_MODE_DEPTH : Environment::FOG_MODE_EXPONENTIAL);
	environment->set_fog_density(fog);
	environment->set_fog_depth_curve(bounded("fog_depth_curve", 1.0, 0.00001, 16.0));
	environment->set_fog_depth_begin(bounded("fog_depth_begin", 10.0, 0.0, 4000.0));
	environment->set_fog_depth_end(bounded("fog_depth_end", 100.0, 0.0, 4000.0));
	environment->set_fog_height(bounded("fog_height", 0.0, -1024.0, 1024.0));
	environment->set_fog_height_density(height_density);
	environment->set_fog_aerial_perspective(bounded("fog_aerial_perspective", 0.0, 0.0, 1.0));
	environment->set_fog_light_color(environment_color("fog_color", horizon));
	environment->set_fog_light_energy(bounded("fog_light_energy", 1.0, 0.0, 64.0));
	environment->set_fog_sun_scatter(bounded("fog_sun_scatter", 0.0, 0.0, 1.0));
	environment->set_fog_sky_affect(bounded("fog_sky_affect", 1.0, 0.0, 1.0));
	WorldEnvironment *world = memnew(WorldEnvironment);
	world->set_name("WorldEnvironment");
	world->set_environment(environment);
	add(root, world, false);

	DirectionalLight3D *sun = memnew(DirectionalLight3D);
	sun->set_name("Sun");
	Vector3 angles(50.0, 135.0, 0.0);
	Vector3 parsed_angles;
	if (p_properties.has("angles")) {
		if (parse_vec3(p_properties["angles"], parsed_angles) && parsed_angles.is_finite() && Math::abs(parsed_angles.x) <= 360000 && Math::abs(parsed_angles.y) <= 360000) { angles = parsed_angles; }
		else { UtilityFunctions::push_warning("MapCraft: light_environment invalid sun angles; using 50 135 0"); }
	}
	// The sun shines down its -Z, which points along the entity's facing like a marker's.
	sun->set_transform(Transform3D(godot_basis(map_angles(angles)) * Basis(Vector3(0, 1, 0), -Math_PI / 2.0), Vector3()));
	sun->set_color(environment_color("sun_color", Color(1.0, 0.96, 0.9)));
	sun->set_param(Light3D::PARAM_ENERGY, bounded("sun_energy", 1.0, 0.0, 64.0));
	sun->set_shadow(number_key(p_properties, "shadows", 1.0) != 0.0);
	sun->set_param(Light3D::PARAM_SHADOW_MAX_DISTANCE, 1000.0);
	// Baked lighting already holds the sun, so it lights only what moves, such as players.
	if (lit) {
		sun->set_bake_mode(Light3D::BAKE_STATIC);
	}
	add(root, sun, false);
}

Error Builder::build(const Dictionary &p_doc, String &r_error) {
	if (String(p_doc.get("format", "")) != "mapcraft.map") {
		r_error = "not a MapCraft map";
		return ERR_FILE_UNRECOGNIZED;
	}
	if (!options.source_path.is_empty()) {
		pack_path = options.source_path.get_basename() + ".mcpck";
		if (options.source_path.get_extension().to_lower() == "mcpck") {
			pack_path = options.source_path;
		} else if (!FileAccess::file_exists(pack_path) && FileAccess::file_exists(options.source_path.get_basename() + ".mcpack")) {
			pack_path = options.source_path.get_basename() + ".mcpack";
		}
		String error;
		if (FileAccess::file_exists(pack_path) && pack.open(pack_path, error) != OK) {
			UtilityFunctions::push_error("MapCraft: ", pack_path, " ", error, ", so the map uses stock materials and models");
		}
		if (options.prefabs_above.is_empty()) {
			load_lighting();
		}
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
	const Dictionary world_node = map.get("root", Dictionary());
	if (options.prefabs_above.is_empty()) {
		sky_shader = embedded_shader(world_node, Shader::MODE_SKY);
	}
	parent = root;
	walk(world_node, 0);
	if (!has_environment && options.prefabs_above.is_empty() && options.entity_map.is_valid() && options.entity_map->get_default_environment()) {
		add_environment(Dictionary());
	}
	if (!has_environment && sky_shader.is_valid()) {
		Ref<Sky> sky;
		sky.instantiate();
		sky->set_material(sky_shader);
		Ref<Environment> environment;
		environment.instantiate();
		environment->set_background(Environment::BG_SKY);
		environment->set_sky(sky);
		environment->set_ambient_source(Environment::AMBIENT_SOURCE_SKY);
		environment->set_reflection_source(Environment::REFLECTION_SOURCE_SKY);
		WorldEnvironment *world = memnew(WorldEnvironment);
		world->set_name("WorldEnvironment");
		world->set_environment(environment);
		add(root, world, false);
		has_environment = true;
	}

	finish_brushes(root, "WorldMesh", "WorldCollision", true);
	if (lit && !lightmap_users.is_empty()) {
		add_lightmap();
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

Error mapcraft_save_lighting(const String &p_source, const String &p_path, String &r_error) {
	Lighting lighting;
	Error err = lighting.parse(FileAccess::get_file_as_bytes(p_source), r_error);
	if (err != OK) {
		return err;
	}
	Ref<FileAccess> f = FileAccess::open(p_path, FileAccess::WRITE);
	if (f.is_null()) {
		r_error = "cannot be written to " + p_path;
		return ERR_FILE_CANT_WRITE;
	}
	// Godot's layered texture header: version, layers, 2D array, then unused fields.
	f->store_buffer(String("GSTL").to_ascii_buffer());
	const uint32_t header[] = { 1, uint32_t(lighting.layers.size()), 0, 0, 0, 0, 0, 0 };
	for (uint32_t value : header) {
		f->store_32(value);
	}
	for (int64_t i = 0; i < lighting.layers.size(); i++) {
		const Ref<Image> image = lighting.layers[i];
		f->store_32(0); // Raw image data, no mipmaps.
		f->store_16(uint16_t(image->get_width()));
		f->store_16(uint16_t(image->get_height()));
		f->store_32(0);
		f->store_32(uint32_t(image->get_format()));
		f->store_buffer(image->get_data());
	}
	return OK;
}

Error mapcraft_pack(const Dictionary &p_doc, const String &p_name, const MapCraftBuildOptions &p_options, Ref<PackedScene> &r_scene, String &r_error) {
	MapCraftBuildOptions options = p_options;
	if (options.entity_map.is_null()) {
		// Maps name their SDK; an addon entity map with that name is used when none is set.
		String sdk = p_doc.get("sdk", "");
		String candidate = String("res://addons/mapcraft/sdks").path_join(sdk + ".tres");
		if (inside_folder(sdk) && !sdk.contains("/") && ResourceLoader::get_singleton()->exists(candidate)) {
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

Error mapcraft_read_map(const String &p_path, Dictionary &r_doc, String &r_error) {
	if (!FileAccess::file_exists(p_path)) {
		r_error = String("cannot open ") + p_path;
		return ERR_FILE_NOT_FOUND;
	}
	String text;
	if (p_path.get_extension().to_lower() == "mcpck") {
		MapCraftPack pack;
		String error;
		if (pack.open(p_path, error) != OK) {
			r_error = p_path + String(" ") + error;
			return ERR_FILE_CORRUPT;
		}
		PackedByteArray bytes;
		if (!pack.map(bytes)) {
			r_error = p_path + String(" holds no map");
			return ERR_FILE_UNRECOGNIZED;
		}
		text = bytes.get_string_from_utf8();
	} else {
		text = FileAccess::get_file_as_string(p_path);
	}
	Variant parsed = JSON::parse_string(text);
	if (parsed.get_type() != Variant::DICTIONARY) {
		r_error = p_path + String(" is not valid JSON");
		return ERR_PARSE_ERROR;
	}
	r_doc = parsed;
	return OK;
}

Error mapcraft_build_file(const String &p_path, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error) {
	Dictionary parsed;
	Error read = mapcraft_read_map(p_path, parsed, r_error);
	if (read != OK) {
		return read;
	}
	MapCraftBuildOptions options = p_options;
	if (options.source_path.is_empty()) {
		options.source_path = p_path;
	}
	return mapcraft_build(parsed, p_root, options, r_error);
}

} // namespace godot
