#include "mapcraft_loader.h"

#include <godot_cpp/classes/file_access.hpp>
#include <godot_cpp/classes/json.hpp>
#include <godot_cpp/variant/utility_functions.hpp>

#include "mapcraft_builder.h"

namespace godot {

PackedStringArray MapCraftLoader::_get_recognized_extensions() const {
	PackedStringArray out;
	out.push_back("mcmap");
	return out;
}

bool MapCraftLoader::_handles_type(const StringName &p_type) const {
	return p_type == StringName("PackedScene");
}

String MapCraftLoader::_get_resource_type(const String &p_path) const {
	return p_path.get_extension().to_lower() == "mcmap" ? "PackedScene" : "";
}

bool MapCraftLoader::_exists(const String &p_path) const {
	return FileAccess::file_exists(p_path);
}

Variant MapCraftLoader::_load(const String &p_path, const String &p_original_path, bool p_use_sub_threads, int32_t p_cache_mode) const {
	Variant parsed = JSON::parse_string(FileAccess::get_file_as_string(p_path));
	if (parsed.get_type() != Variant::DICTIONARY) {
		UtilityFunctions::push_error("MapCraft: ", p_path, " is not valid JSON");
		return ERR_PARSE_ERROR;
	}
	MapCraftBuildOptions options;
	options.source_path = p_path;
	Ref<PackedScene> scene;
	String error;
	Error err = mapcraft_pack(parsed, p_path.get_file().get_basename(), options, scene, error);
	if (err != OK) {
		UtilityFunctions::push_error("MapCraft: ", p_path, ": ", error);
		return err;
	}
	return scene;
}

} // namespace godot
