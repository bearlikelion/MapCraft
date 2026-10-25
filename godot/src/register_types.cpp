#include <gdextension_interface.h>
#include <godot_cpp/classes/editor_plugin_registration.hpp>
#include <godot_cpp/core/class_db.hpp>
#include <godot_cpp/godot.hpp>

#include "mapcraft_entity_map.h"
#include "mapcraft_import_plugin.h"
#include "mapcraft_map.h"

using namespace godot;

static void initialize_mapcraft(ModuleInitializationLevel p_level) {
	if (p_level == MODULE_INITIALIZATION_LEVEL_SCENE) {
		GDREGISTER_CLASS(MapCraftEntityMap);
		GDREGISTER_CLASS(MapCraftMap);
	} else if (p_level == MODULE_INITIALIZATION_LEVEL_EDITOR) {
		GDREGISTER_CLASS(MapCraftImportPlugin);
		GDREGISTER_CLASS(MapCraftEditorPlugin);
		EditorPlugins::add_by_type<MapCraftEditorPlugin>();
	}
}

static void uninitialize_mapcraft(ModuleInitializationLevel p_level) {
	if (p_level == MODULE_INITIALIZATION_LEVEL_EDITOR) {
		EditorPlugins::remove_by_type<MapCraftEditorPlugin>();
	}
}

extern "C" {

GDExtensionBool GDE_EXPORT mapcraft_library_init(
		GDExtensionInterfaceGetProcAddress p_get_proc_address,
		GDExtensionClassLibraryPtr p_library,
		GDExtensionInitialization *r_initialization) {
	GDExtensionBinding::InitObject init_obj(p_get_proc_address, p_library, r_initialization);
	init_obj.register_initializer(initialize_mapcraft);
	init_obj.register_terminator(uninitialize_mapcraft);
	init_obj.set_minimum_library_initialization_level(MODULE_INITIALIZATION_LEVEL_SCENE);
	return init_obj.init();
}

}
