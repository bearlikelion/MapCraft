class_name MapCraftPackMap
extends SceneTree

const USAGE: String = "usage: godot --headless --path <project> -s " \
	+ "res://addons/mapcraft/tools/pack_map.gd -- <map.mcmap> <internal.scn> <out.pck>"


func _initialize() -> void:
	var args: PackedStringArray = OS.get_cmdline_user_args()
	if args.size() != 3:
		printerr(USAGE)
		quit(2)
		return
	var error: String = pack(args[0], args[1], args[2])
	if not error.is_empty():
		printerr("pack_map: ", error)
	quit(0 if error.is_empty() else 1)


# Packs only the map scene; materials and textures it names stay paths into the game.
static func pack(map_path: String, internal_path: String, pck_path: String) -> String:
	var scene: PackedScene = load(map_path) as PackedScene
	if scene == null:
		return "%s has not been imported as a scene" % map_path
	var built: String = "user://mapcraft_pack/%s" % internal_path.get_file()
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(built.get_base_dir()))
	if ResourceSaver.save(scene, built, ResourceSaver.FLAG_COMPRESS) != OK:
		return "could not save %s" % built
	DirAccess.make_dir_recursive_absolute(pck_path.get_base_dir())
	var packer: PCKPacker = PCKPacker.new()
	if packer.pck_start(pck_path) != OK:
		return "could not start %s" % pck_path
	packer.add_file(internal_path, ProjectSettings.globalize_path(built))
	if packer.flush() != OK:
		return "could not write %s" % pck_path
	print("pack_map: wrote %s" % pck_path)
	return ""
