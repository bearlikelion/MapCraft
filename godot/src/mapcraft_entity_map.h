#pragma once

#include <godot_cpp/classes/resource.hpp>
#include <godot_cpp/variant/dictionary.hpp>

namespace godot {

class MapCraftEntityMap : public Resource {
	GDCLASS(MapCraftEntityMap, Resource)

	Dictionary classes;
	String point_default = "marker";
	String solid_default = "area";
	bool default_environment = false;
	bool trimesh_collision = false;

protected:
	static void _bind_methods();

public:
	void set_classes(const Dictionary &p_classes);
	Dictionary get_classes() const;
	void set_point_default(const String &p_node);
	String get_point_default() const;
	void set_solid_default(const String &p_node);
	String get_solid_default() const;
	void set_default_environment(bool p_enabled);
	bool get_default_environment() const;
	void set_trimesh_collision(bool p_enabled);
	bool get_trimesh_collision() const;

	Dictionary entity_config(const String &p_classname, bool p_solid) const;
};

} // namespace godot
