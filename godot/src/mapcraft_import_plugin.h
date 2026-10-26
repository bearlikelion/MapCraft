#pragma once

#include <godot_cpp/classes/editor_import_plugin.hpp>
#include <godot_cpp/classes/editor_plugin.hpp>

namespace godot {

// Imports .mcmap files as scenes made only of built-in nodes, so games need no extension to load them.
class MapCraftImportPlugin : public EditorImportPlugin {
	GDCLASS(MapCraftImportPlugin, EditorImportPlugin)

protected:
	static void _bind_methods() {}

public:
	String _get_importer_name() const override;
	String _get_visible_name() const override;
	PackedStringArray _get_recognized_extensions() const override;
	String _get_save_extension() const override;
	String _get_resource_type() const override;
	int32_t _get_preset_count() const override;
	String _get_preset_name(int32_t p_preset_index) const override;
	TypedArray<Dictionary> _get_import_options(const String &p_path, int32_t p_preset_index) const override;
	bool _get_option_visibility(const String &p_path, const StringName &p_option_name, const Dictionary &p_options) const override;
	float _get_priority() const override;
	int32_t _get_import_order() const override;
	int32_t _get_format_version() const override;
	Error _import(const String &p_source_file, const String &p_save_path, const Dictionary &p_options, const TypedArray<String> &p_platform_variants,
			const TypedArray<String> &p_gen_files) const override;
};

class MapCraftEditorPlugin : public EditorPlugin {
	GDCLASS(MapCraftEditorPlugin, EditorPlugin)

	Ref<MapCraftImportPlugin> importer;

protected:
	static void _bind_methods() {}
	void _notification(int p_what);
};

} // namespace godot
