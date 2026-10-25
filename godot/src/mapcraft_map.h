#pragma once

#include <godot_cpp/classes/node3d.hpp>

#include "mapcraft_entity_map.h"

namespace godot {

// Built nodes are not saved with the scene.
class MapCraftMap : public Node3D {
	GDCLASS(MapCraftMap, Node3D)

	String map_file;
	Ref<MapCraftEntityMap> entity_map;
	double units_per_meter = 39.37;
	String material_root = "res://Assets/Textures";
	bool collision = true;
	bool build_on_ready = true;

protected:
	static void _bind_methods();
	void _notification(int p_what);

public:
	void set_map_file(const String &p_path);
	String get_map_file() const;
	void set_entity_map(const Ref<MapCraftEntityMap> &p_map);
	Ref<MapCraftEntityMap> get_entity_map() const;
	void set_units_per_meter(double p_units);
	double get_units_per_meter() const;
	void set_material_root(const String &p_root);
	String get_material_root() const;
	void set_collision(bool p_enabled);
	bool get_collision() const;
	void set_build_on_ready(bool p_enabled);
	bool get_build_on_ready() const;

	Error build();
	void clear();
};

} // namespace godot
