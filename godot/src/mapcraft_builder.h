#pragma once

#include <godot_cpp/classes/node3d.hpp>
#include <godot_cpp/classes/packed_scene.hpp>
#include <godot_cpp/variant/dictionary.hpp>

#include <functional>
#include <memory>

#include "mapcraft_entity_map.h"

namespace godot {

struct MapCraftBuildOptions {
	double units_per_meter = 39.37;
	String material_root = "res://Assets/Textures";
	Ref<MapCraftEntityMap> entity_map;
	bool collision = true;
	// World brushes collide as one triangle mesh, so seams between ramp pieces cannot catch players.
	bool trimesh_collision = false;
	bool own_nodes = true;
	String source_path;
	// Imports take a .mcmap's baked lighting from its .mclight's own import, which keeps the texels without a GPU.
	bool imported_lighting = false;
	// Prefab maps this build sits inside, outermost first, and the prefab copies the whole map has left.
	PackedStringArray prefabs_above;
	std::shared_ptr<int> prefab_budget;
	std::function<void(Node *, const String &, const Dictionary &)> on_entity;
};

// MapCraft maps are Z up in inches, Godot is Y up in meters: (x, y, z) -> (x, z, -y) / units_per_meter.
Error mapcraft_build(const Dictionary &p_doc, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error);

// Reads a .mcmap, or the map inside a self-contained .mcpck.
Error mapcraft_read_map(const String &p_path, Dictionary &r_doc, String &r_error);
Error mapcraft_build_file(const String &p_path, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error);

// Writes a .mclight's texels as a CompressedTexture2DArray (.ctexarray) file.
Error mapcraft_save_lighting(const String &p_source, const String &p_path, String &r_error);

Error mapcraft_pack(const Dictionary &p_doc, const String &p_name, const MapCraftBuildOptions &p_options, Ref<PackedScene> &r_scene, String &r_error);

} // namespace godot
