#pragma once

#include <godot_cpp/classes/node3d.hpp>
#include <godot_cpp/variant/dictionary.hpp>

#include <functional>

#include "mapcraft_entity_map.h"

namespace godot {

struct MapCraftBuildOptions {
	double units_per_meter = 39.37;
	String material_root = "res://Assets/Textures";
	Ref<MapCraftEntityMap> entity_map;
	bool collision = true;
	bool own_nodes = true;
	String source_path;
	std::function<void(Node *, const String &, const Dictionary &)> on_entity;
};

// MapCraft maps are Z up in inches, Godot is Y up in meters: (x, y, z) -> (x, z, -y) / units_per_meter.
Error mapcraft_build(const Dictionary &p_doc, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error);

Error mapcraft_build_file(const String &p_path, Node3D *p_root, const MapCraftBuildOptions &p_options, String &r_error);

} // namespace godot
