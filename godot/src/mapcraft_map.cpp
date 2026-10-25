#include "mapcraft_map.h"

#include <godot_cpp/core/class_db.hpp>
#include <godot_cpp/variant/utility_functions.hpp>

#include "mapcraft_builder.h"

namespace godot {

void MapCraftMap::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_map_file", "path"), &MapCraftMap::set_map_file);
	ClassDB::bind_method(D_METHOD("get_map_file"), &MapCraftMap::get_map_file);
	ClassDB::bind_method(D_METHOD("set_entity_map", "map"), &MapCraftMap::set_entity_map);
	ClassDB::bind_method(D_METHOD("get_entity_map"), &MapCraftMap::get_entity_map);
	ClassDB::bind_method(D_METHOD("set_units_per_meter", "units"), &MapCraftMap::set_units_per_meter);
	ClassDB::bind_method(D_METHOD("get_units_per_meter"), &MapCraftMap::get_units_per_meter);
	ClassDB::bind_method(D_METHOD("set_material_root", "root"), &MapCraftMap::set_material_root);
	ClassDB::bind_method(D_METHOD("get_material_root"), &MapCraftMap::get_material_root);
	ClassDB::bind_method(D_METHOD("set_collision", "enabled"), &MapCraftMap::set_collision);
	ClassDB::bind_method(D_METHOD("get_collision"), &MapCraftMap::get_collision);
	ClassDB::bind_method(D_METHOD("set_build_on_ready", "enabled"), &MapCraftMap::set_build_on_ready);
	ClassDB::bind_method(D_METHOD("get_build_on_ready"), &MapCraftMap::get_build_on_ready);
	ClassDB::bind_method(D_METHOD("build"), &MapCraftMap::build);
	ClassDB::bind_method(D_METHOD("clear"), &MapCraftMap::clear);

	ADD_PROPERTY(PropertyInfo(Variant::STRING, "map_file", PROPERTY_HINT_FILE, "*.mcmap"), "set_map_file", "get_map_file");
	ADD_PROPERTY(PropertyInfo(Variant::OBJECT, "entity_map", PROPERTY_HINT_RESOURCE_TYPE, "MapCraftEntityMap"), "set_entity_map", "get_entity_map");
	ADD_PROPERTY(PropertyInfo(Variant::FLOAT, "units_per_meter", PROPERTY_HINT_RANGE, "1,1000,0.01"), "set_units_per_meter", "get_units_per_meter");
	ADD_PROPERTY(PropertyInfo(Variant::STRING, "material_root", PROPERTY_HINT_DIR), "set_material_root", "get_material_root");
	ADD_PROPERTY(PropertyInfo(Variant::BOOL, "collision"), "set_collision", "get_collision");
	ADD_PROPERTY(PropertyInfo(Variant::BOOL, "build_on_ready"), "set_build_on_ready", "get_build_on_ready");

	ADD_SIGNAL(MethodInfo("entity_built", PropertyInfo(Variant::OBJECT, "node", PROPERTY_HINT_RESOURCE_TYPE, "Node"),
			PropertyInfo(Variant::STRING, "classname"), PropertyInfo(Variant::DICTIONARY, "properties")));
}

void MapCraftMap::_notification(int p_what) {
	if (p_what == NOTIFICATION_READY && build_on_ready && !map_file.is_empty()) {
		build();
	}
}

void MapCraftMap::set_map_file(const String &p_path) {
	map_file = p_path;
}

String MapCraftMap::get_map_file() const {
	return map_file;
}

void MapCraftMap::set_entity_map(const Ref<MapCraftEntityMap> &p_map) {
	entity_map = p_map;
}

Ref<MapCraftEntityMap> MapCraftMap::get_entity_map() const {
	return entity_map;
}

void MapCraftMap::set_units_per_meter(double p_units) {
	units_per_meter = p_units;
}

double MapCraftMap::get_units_per_meter() const {
	return units_per_meter;
}

void MapCraftMap::set_material_root(const String &p_root) {
	material_root = p_root;
}

String MapCraftMap::get_material_root() const {
	return material_root;
}

void MapCraftMap::set_collision(bool p_enabled) {
	collision = p_enabled;
}

bool MapCraftMap::get_collision() const {
	return collision;
}

void MapCraftMap::set_build_on_ready(bool p_enabled) {
	build_on_ready = p_enabled;
}

bool MapCraftMap::get_build_on_ready() const {
	return build_on_ready;
}

void MapCraftMap::clear() {
	while (get_child_count(true) > 0) {
		Node *child = get_child(0, true);
		remove_child(child);
		child->queue_free();
	}
}

Error MapCraftMap::build() {
	clear();
	MapCraftBuildOptions options;
	options.units_per_meter = units_per_meter;
	options.material_root = material_root;
	options.entity_map = entity_map;
	options.collision = collision;
	options.own_nodes = false;
	options.on_entity = [this](Node *p_node, const String &p_classname, const Dictionary &p_properties) {
		emit_signal("entity_built", p_node, p_classname, p_properties);
	};
	String error;
	Error err = mapcraft_build_file(map_file, this, options, error);
	if (err != OK) {
		UtilityFunctions::push_error("MapCraft: ", error);
	}
	return err;
}

} // namespace godot
