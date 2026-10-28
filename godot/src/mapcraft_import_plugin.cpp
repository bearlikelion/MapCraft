#include "mapcraft_import_plugin.h"

#include <godot_cpp/classes/file_access.hpp>
#include <godot_cpp/classes/json.hpp>
#include <godot_cpp/classes/packed_scene.hpp>
#include <godot_cpp/classes/resource_loader.hpp>
#include <godot_cpp/classes/resource_saver.hpp>

#include "mapcraft_builder.h"

namespace godot {

namespace {

Dictionary option(const String &p_name, const Variant &p_default, PropertyHint p_hint = PROPERTY_HINT_NONE, const String &p_hint_string = "") {
	Dictionary d;
	d["name"] = p_name;
	d["default_value"] = p_default;
	d["property_hint"] = p_hint;
	d["hint_string"] = p_hint_string;
	return d;
}

} // namespace

String MapCraftImportPlugin::_get_importer_name() const {
	return "mapcraft.map";
}

String MapCraftImportPlugin::_get_visible_name() const {
	return "MapCraft Map";
}

PackedStringArray MapCraftImportPlugin::_get_recognized_extensions() const {
	PackedStringArray out;
	out.push_back("mcmap");
	return out;
}

String MapCraftImportPlugin::_get_save_extension() const {
	return "scn";
}

String MapCraftImportPlugin::_get_resource_type() const {
	return "PackedScene";
}

int32_t MapCraftImportPlugin::_get_preset_count() const {
	return 1;
}

String MapCraftImportPlugin::_get_preset_name(int32_t p_preset_index) const {
	return "Default";
}

TypedArray<Dictionary> MapCraftImportPlugin::_get_import_options(const String &p_path, int32_t p_preset_index) const {
	TypedArray<Dictionary> out;
	out.push_back(option("units_per_meter", 39.37, PROPERTY_HINT_RANGE, "1,1000,0.01"));
	out.push_back(option("material_root", "res://Assets/Textures", PROPERTY_HINT_DIR));
	out.push_back(option("entity_map", "", PROPERTY_HINT_FILE, "*.tres,*.res"));
	out.push_back(option("collision", true));
	out.push_back(option("trimesh_collision", false));
	return out;
}

bool MapCraftImportPlugin::_get_option_visibility(const String &p_path, const StringName &p_option_name, const Dictionary &p_options) const {
	return true;
}

float MapCraftImportPlugin::_get_priority() const {
	return 1.0;
}

// After the scene importer, so models that props instance are imported first.
int32_t MapCraftImportPlugin::_get_import_order() const {
	return IMPORT_ORDER_SCENE + 1;
}

// Bump when imported scenes change, so projects reimport maps made by an older build.
int32_t MapCraftImportPlugin::_get_format_version() const {
	return 6;
}

Error MapCraftImportPlugin::_import(const String &p_source_file, const String &p_save_path, const Dictionary &p_options,
		const TypedArray<String> &p_platform_variants, const TypedArray<String> &p_gen_files) const {
	Variant parsed = JSON::parse_string(FileAccess::get_file_as_string(p_source_file));
	if (parsed.get_type() != Variant::DICTIONARY) {
		ERR_PRINT(String("MapCraft: ") + p_source_file + String(" is not valid JSON"));
		return ERR_PARSE_ERROR;
	}
	Dictionary doc = parsed;

	MapCraftBuildOptions options;
	options.units_per_meter = p_options.get("units_per_meter", 39.37);
	options.material_root = p_options.get("material_root", "res://Assets/Textures");
	options.collision = p_options.get("collision", true);
	options.trimesh_collision = p_options.get("trimesh_collision", false);
	options.source_path = p_source_file;
	String map_path = p_options.get("entity_map", "");
	if (!map_path.is_empty()) {
		options.entity_map = ResourceLoader::get_singleton()->load(map_path);
	}

	Ref<PackedScene> scene;
	String error;
	Error err = mapcraft_pack(doc, p_source_file.get_file().get_basename(), options, scene, error);
	if (err != OK) {
		ERR_PRINT(String("MapCraft: ") + p_source_file + String(": ") + error);
		return err;
	}
	return ResourceSaver::get_singleton()->save(scene, p_save_path + String(".") + _get_save_extension());
}

void MapCraftEditorPlugin::_notification(int p_what) {
	if (p_what == NOTIFICATION_ENTER_TREE) {
		importer.instantiate();
		add_import_plugin(importer);
	} else if (p_what == NOTIFICATION_EXIT_TREE && importer.is_valid()) {
		remove_import_plugin(importer);
		importer.unref();
	}
}

} // namespace godot
