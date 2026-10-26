#include "mapcraft_entity_map.h"

#include <godot_cpp/core/class_db.hpp>

namespace godot {

void MapCraftEntityMap::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_classes", "classes"), &MapCraftEntityMap::set_classes);
	ClassDB::bind_method(D_METHOD("get_classes"), &MapCraftEntityMap::get_classes);
	ClassDB::bind_method(D_METHOD("set_point_default", "node"), &MapCraftEntityMap::set_point_default);
	ClassDB::bind_method(D_METHOD("get_point_default"), &MapCraftEntityMap::get_point_default);
	ClassDB::bind_method(D_METHOD("set_solid_default", "node"), &MapCraftEntityMap::set_solid_default);
	ClassDB::bind_method(D_METHOD("get_solid_default"), &MapCraftEntityMap::get_solid_default);
	ClassDB::bind_method(D_METHOD("set_default_environment", "enabled"), &MapCraftEntityMap::set_default_environment);
	ClassDB::bind_method(D_METHOD("get_default_environment"), &MapCraftEntityMap::get_default_environment);
	ClassDB::bind_method(D_METHOD("set_trimesh_collision", "enabled"), &MapCraftEntityMap::set_trimesh_collision);
	ClassDB::bind_method(D_METHOD("get_trimesh_collision"), &MapCraftEntityMap::get_trimesh_collision);
	ClassDB::bind_method(D_METHOD("entity_config", "classname", "solid"), &MapCraftEntityMap::entity_config);

	const String kinds = "marker,node,area,static,world,scene,model,environment,skip";
	ADD_PROPERTY(PropertyInfo(Variant::DICTIONARY, "classes"), "set_classes", "get_classes");
	ADD_PROPERTY(PropertyInfo(Variant::STRING, "point_default", PROPERTY_HINT_ENUM, kinds), "set_point_default", "get_point_default");
	ADD_PROPERTY(PropertyInfo(Variant::STRING, "solid_default", PROPERTY_HINT_ENUM, kinds), "set_solid_default", "get_solid_default");
	ADD_PROPERTY(PropertyInfo(Variant::BOOL, "default_environment"), "set_default_environment", "get_default_environment");
	ADD_PROPERTY(PropertyInfo(Variant::BOOL, "trimesh_collision"), "set_trimesh_collision", "get_trimesh_collision");
}

void MapCraftEntityMap::set_classes(const Dictionary &p_classes) {
	classes = p_classes;
}

Dictionary MapCraftEntityMap::get_classes() const {
	return classes;
}

void MapCraftEntityMap::set_point_default(const String &p_node) {
	point_default = p_node;
}

String MapCraftEntityMap::get_point_default() const {
	return point_default;
}

void MapCraftEntityMap::set_solid_default(const String &p_node) {
	solid_default = p_node;
}

String MapCraftEntityMap::get_solid_default() const {
	return solid_default;
}

void MapCraftEntityMap::set_default_environment(bool p_enabled) {
	default_environment = p_enabled;
}

bool MapCraftEntityMap::get_default_environment() const {
	return default_environment;
}

void MapCraftEntityMap::set_trimesh_collision(bool p_enabled) {
	trimesh_collision = p_enabled;
}

bool MapCraftEntityMap::get_trimesh_collision() const {
	return trimesh_collision;
}

Dictionary MapCraftEntityMap::entity_config(const String &p_classname, bool p_solid) const {
	Dictionary config;
	Variant found = classes.get(p_classname, Variant());
	if (found.get_type() == Variant::DICTIONARY) {
		config = Dictionary(found).duplicate();
	}
	if (!config.has("node")) {
		config["node"] = p_solid ? solid_default : point_default;
	}
	return config;
}

} // namespace godot
