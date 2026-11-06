#include "mapcraft_import_plugin.h"

#include <godot_cpp/classes/editor_file_system.hpp>
#include <godot_cpp/classes/control.hpp>
#include <godot_cpp/classes/editor_interface.hpp>
#include <godot_cpp/classes/file_access.hpp>
#include <godot_cpp/classes/json.hpp>
#include <godot_cpp/classes/packed_scene.hpp>
#include <godot_cpp/classes/resource_loader.hpp>
#include <godot_cpp/classes/resource_saver.hpp>
#include <godot_cpp/variant/utility_functions.hpp>

#include "mapcraft_builder.h"
#include "mapcraft_map.h"

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

bool is_map(const String &p_path) {
	const String extension = p_path.get_extension().to_lower();
	return extension == "mcmap" || extension == "mcpck";
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
	return 10;
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
	options.imported_lighting = true;
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

String MapCraftLightingImportPlugin::_get_importer_name() const {
	return "mapcraft.lighting";
}

String MapCraftLightingImportPlugin::_get_visible_name() const {
	return "MapCraft Baked Lighting";
}

PackedStringArray MapCraftLightingImportPlugin::_get_recognized_extensions() const {
	PackedStringArray out;
	out.push_back("mclight");
	return out;
}

String MapCraftLightingImportPlugin::_get_save_extension() const {
	return "ctexarray";
}

String MapCraftLightingImportPlugin::_get_resource_type() const {
	return "CompressedTexture2DArray";
}

int32_t MapCraftLightingImportPlugin::_get_preset_count() const {
	return 1;
}

String MapCraftLightingImportPlugin::_get_preset_name(int32_t p_preset_index) const {
	return "Default";
}

TypedArray<Dictionary> MapCraftLightingImportPlugin::_get_import_options(const String &p_path, int32_t p_preset_index) const {
	return TypedArray<Dictionary>();
}

bool MapCraftLightingImportPlugin::_get_option_visibility(const String &p_path, const StringName &p_option_name, const Dictionary &p_options) const {
	return true;
}

float MapCraftLightingImportPlugin::_get_priority() const {
	return 1.0;
}

// Before maps, which load it.
int32_t MapCraftLightingImportPlugin::_get_import_order() const {
	return IMPORT_ORDER_DEFAULT;
}

int32_t MapCraftLightingImportPlugin::_get_format_version() const {
	return 1;
}

Error MapCraftLightingImportPlugin::_import(const String &p_source_file, const String &p_save_path, const Dictionary &p_options,
		const TypedArray<String> &p_platform_variants, const TypedArray<String> &p_gen_files) const {
	String error;
	Error err = mapcraft_save_lighting(p_source_file, p_save_path + String(".") + _get_save_extension(), error);
	if (err != OK) {
		ERR_PRINT(String("MapCraft: ") + p_source_file + String(" ") + error);
	}
	return err;
}

void MapCraftContextMenu::_popup_menu(const PackedStringArray &p_paths) {
	for (const String &path : p_paths) {
		if (is_map(path)) {
			add_context_menu_item("Save as Editable Scene...", chosen);
			return;
		}
	}
}

void MapCraftEditorPlugin::save_requested(const Array &p_paths) {
	for (int i = 0; i < p_paths.size(); i++) {
		if (is_map(p_paths[i])) {
			saving = p_paths[i];
			save_dialog->set_current_path(saving.get_basename() + ".tscn");
			save_dialog->popup_file_dialog();
			return;
		}
	}
}

void MapCraftEditorPlugin::save_chosen(const String &p_path) {
	const Error err = MapCraftMap::save_as_scene(saving, p_path);
	if (err != OK) {
		UtilityFunctions::push_error("MapCraft: could not save ", saving, " as ", p_path, " (", UtilityFunctions::error_string(err), ")");
		return;
	}
	EditorInterface::get_singleton()->get_resource_filesystem()->update_file(p_path);
	EditorInterface::get_singleton()->open_scene_from_path(p_path);
}

void MapCraftEditorPlugin::_notification(int p_what) {
	if (p_what == NOTIFICATION_ENTER_TREE) {
		lighting_importer.instantiate();
		add_import_plugin(lighting_importer);
		importer.instantiate();
		add_import_plugin(importer);
		save_dialog = memnew(EditorFileDialog);
		save_dialog->set_title("Save Map as Editable Scene");
		save_dialog->set_file_mode(EditorFileDialog::FILE_MODE_SAVE_FILE);
		save_dialog->set_access(EditorFileDialog::ACCESS_RESOURCES);
		save_dialog->add_filter("*.tscn", "Text Scene");
		save_dialog->add_filter("*.scn", "Binary Scene");
		save_dialog->connect("file_selected", callable_mp(this, &MapCraftEditorPlugin::save_chosen));
		EditorInterface::get_singleton()->get_base_control()->add_child(save_dialog);
		context_menu.instantiate();
		context_menu->chosen = callable_mp(this, &MapCraftEditorPlugin::save_requested);
		add_context_menu_plugin(EditorContextMenuPlugin::CONTEXT_SLOT_FILESYSTEM, context_menu);
	} else if (p_what == NOTIFICATION_EXIT_TREE && importer.is_valid()) {
		remove_import_plugin(importer);
		importer.unref();
		remove_import_plugin(lighting_importer);
		lighting_importer.unref();
		remove_context_menu_plugin(context_menu);
		context_menu.unref();
		save_dialog->queue_free();
		save_dialog = nullptr;
	}
}

} // namespace godot
