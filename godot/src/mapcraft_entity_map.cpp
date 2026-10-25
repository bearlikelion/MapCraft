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
	ClassDB::bind_method(D_METHOD("entity_config", "classname", "solid"), &MapCraftEntityMap::entity_config);

	const String kinds = "marker,node,area,static,world,scene,skip";
	ADD_PROPERTY(PropertyInfo(Variant::DICTIONARY, "classes"), "set_classes", "get_classes");
	ADD_PROPERTY(PropertyInfo(Variant::STRING, "point_default", PROPERTY_HINT_ENUM, kinds), "set_point_default", "get_point_default");
	ADD_PROPERTY(PropertyInfo(Variant::STRING, "solid_default", PROPERTY_HINT_ENUM, kinds), "set_solid_default", "get_solid_default");
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
