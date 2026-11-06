class_name MapCraftImportTest
extends SceneTree

const MAP: String = "res://course.mcmap"
const PACKED_MAP: String = "res://maps/course/course.scn"
const ENTITY_MAP: String = "res://addons/mapcraft/sdks/surfsup.tres"
const SHADERS: String = "res://addons/mapcraft/shaders/"
const UNITS_PER_METER: float = 39.37
const PREFAB_DIR: String = "user://prefab_test/"
const MOUNTED_DIR: String = "res://mounted_pack_test/"
const SCRIPTED_SCENE: String = "[gd_scene format=3]\n\n" \
		+ "[sub_resource type=\"GDScript\" id=\"1\"]\n" \
		+ "script/source = \"extends Node3D\nfunc _init():\n\tEngine.max_fps = 1234\n\"\n\n" \
		+ "[node name=\"Scripted\" type=\"Node3D\"]\nscript = SubResource(\"1\")\n"
const SCRIPTED_MATERIAL: String = "[gd_resource type=\"StandardMaterial3D\" format=3]\n\n" \
		+ "[sub_resource type=\"GDScript\" id=\"1\"]\n" \
		+ "script/source = \"extends StandardMaterial3D\nfunc _init():\n" \
		+ "\tEngine.max_fps = 1234\n\"\n\n" \
		+ "[resource]\nscript = SubResource(\"1\")\n"

var _failures: PackedStringArray = PackedStringArray()
var _entities_built: int = 0


func _initialize() -> void:
	var scene: PackedScene = load(MAP) as PackedScene
	if scene == null:
		_fail("%s did not import as a scene" % MAP)
	else:
		var root: Node3D = scene.instantiate() as Node3D
		_check_imported(root)
		root.free()
	_check_wrappers()
	_check_blend_fallbacks()
	_check_runtime_build()
	_check_tool_brushes()
	_check_runtime_load()
	_check_untrusted_prefabs()
	_check_paths_stay_in_project()
	_check_pack(OS.get_cmdline_user_args())
	_check_editable_scene(MAP)
	for failure: String in _failures:
		printerr("FAIL: ", failure)
	print("mapcraft import test: %d failures" % _failures.size())
	quit(1 if _failures.size() > 0 else 0)


func _check_imported(root: Node3D) -> void:
	var spawn: Marker3D = root.get_node_or_null("SpawnPoint") as Marker3D
	_expect(spawn != null, "root has a SpawnPoint Marker3D")
	if spawn != null:
		var expected: Vector3 = Vector3(-384.0, 0.0, 0.0) / UNITS_PER_METER
		_expect(spawn.position.is_equal_approx(expected), "SpawnPoint sits at the player start")
		_expect((-spawn.basis.z).is_equal_approx(Vector3.RIGHT), "SpawnPoint faces the start's yaw")

	var start: Area3D = root.get_node_or_null("StartZone") as Area3D
	_expect(_zone_kind(start) == "start", "StartZone is a start zone")
	if start != null:
		var shape: Node = start.get_child(0) if start.get_child_count() == 1 else null
		_expect(shape is CollisionShape3D, "StartZone has its brush shape")
		_expect(typeof(start.get_meta(&"course_id")) == TYPE_INT, "course_id is an int")
		_expect(start.collision_layer == 0 and start.collision_mask == 2, \
				"StartZone only sees the player layer")

	var finish: Area3D = root.get_node_or_null("FinishZone") as Area3D
	_expect(_zone_kind(finish) == "finish", "FinishZone is a finish zone")

	var teleport: Area3D = root.get_node_or_null("teleport_zone") as Area3D
	var stage: String = teleport.get_meta(&"next_stage", "") if teleport != null else ""
	_expect(stage == "dest1", "teleport names its destination")
	var filter: Variant = teleport.get_meta(&"filter_name", null) if teleport != null else null
	_expect(filter is String and filter == "7", "keys the entity map lists as strings stay text")
	_expect(root.find_child("dest1", false, false) is Marker3D, "the destination is a Marker3D")

	var booster: Area3D = root.get_node_or_null("zone_booster") as Area3D
	var direction: Vector3 = Vector3.ZERO
	if booster != null:
		direction = booster.get_meta(&"boost_direction", Vector3.ZERO)
	_expect(direction.is_equal_approx(Vector3.FORWARD), "boost_direction turns from +Y to -Z")

	_check_prop(root.get_node_or_null("crate") as Node3D)
	_check_mesh_keys(root.get_node_or_null("lidded") as Node3D, root.get_node_or_null("emptied") as Node3D)

	var mesh: MeshInstance3D = root.get_node_or_null("WorldMesh") as MeshInstance3D
	_expect(mesh != null and mesh.mesh.get_surface_count() == 8, \
			"one surface per material, blend layer and anti-tiling mode")
	var group: MeshInstance3D = root.get_node_or_null("Group/Mesh") as MeshInstance3D
	_expect(group != null and root.get_node("Group").get_class() == "Node3D", \
			"a group becomes a Node3D holding its brushes' mesh")
	if mesh != null and group != null:
		_check_checker_surface(mesh.mesh as ArrayMesh)
		_check_anti_tiling(mesh.mesh as ArrayMesh, group.mesh as ArrayMesh)
		_check_blend(mesh.mesh as ArrayMesh)
	var body: StaticBody3D = root.get_node_or_null("WorldCollision") as StaticBody3D
	var holder: CollisionShape3D = null
	if body != null and body.get_child_count() == 1:
		holder = body.get_child(0) as CollisionShape3D
	_expect(holder != null and holder.shape is ConcavePolygonShape3D, \
			"SurfsUp worlds collide as one triangle mesh")

	for node: Node in root.find_children("*", "", true, false):
		_expect(node.get_script() == null, "%s carries no script" % node.name)


# Map (x, y, z) is Godot (x, z, -y), so yaw 90 turns the model's +X toward Godot -Z.
func _check_prop(prop: Node3D) -> void:
	_expect(prop != null, "the prop is a node named by its targetname")
	if prop == null:
		return
	_expect(prop.scene_file_path == "res://Assets/Models/box.glb", "the prop instances its model")
	var expected: Vector3 = Vector3(0.0, 0.0, -128.0) / UNITS_PER_METER
	_expect(prop.position.is_equal_approx(expected), "the prop sits at its origin")
	_expect(prop.basis.x.is_equal_approx(Vector3(0.0, 0.0, -2.0)), "the prop turns and scales")
	var shapes: Array[Node] = prop.find_children("*", "CollisionShape3D", true, false)
	_expect(not shapes.is_empty() and (shapes[0] as CollisionShape3D).shape is ConvexPolygonShape3D, \
			"a prop collides as a convex hull by default, saved inside its instanced model")


func _check_mesh_keys(lidded: Node3D, emptied: Node3D) -> void:
	_expect(lidded != null and emptied != null, "props with mesh keys import")
	if lidded == null or emptied == null:
		return
	var lid: Node3D = lidded.find_child("Lid", true, false) as Node3D
	var base: Node3D = lidded.find_child("Base", true, false) as Node3D
	_expect(lid != null and base != null, "a prop with mesh keys keeps its glTF nodes")
	if lid != null and base != null:
		var height: float = (base.transform * lid.transform).origin.y
		_expect(is_equal_approx(height, 2.1), "mesh.Lid.origin lifts the lid a meter: %f" % height)
		_expect(lid.find_children("*", "CollisionShape3D", true, false).is_empty(), \
				"mesh.Lid.collision none leaves the lid without collision")
		_expect(not base.find_children("*", "CollisionShape3D", true, false).is_empty(), \
				"the base keeps the prop's collision")
	_expect(emptied.find_children("*", "MeshInstance3D", true, false).is_empty(), \
			"hiding the base frees it and the lid under it")
	_expect(emptied.find_children("*", "CollisionShape3D", true, false).is_empty(), \
			"a hidden mesh has no collision")


# The displaced floor collides as triangles and its middle vertex rises 48 units.
func _check_displacement(body: StaticBody3D) -> void:
	var faces: PackedVector3Array = PackedVector3Array()
	if body != null:
		for child: Node in body.get_children():
			var holder: CollisionShape3D = child as CollisionShape3D
			if holder != null and holder.shape is ConcavePolygonShape3D:
				faces = (holder.shape as ConcavePolygonShape3D).get_faces()
	_expect(faces.size() == 96, "a power 2 displacement collides as 32 triangles")
	var top: float = -INF
	for p: Vector3 in faces:
		top = maxf(top, p.y)
	_expect(is_equal_approx(top, 48.0 / UNITS_PER_METER), "the displacement keeps its bump")


# Every face of the 64 unit detail block shows its 32x16 texture at one texel per unit.
func _check_checker_surface(mesh: ArrayMesh) -> void:
	var surface: int = -1
	for i: int in mesh.get_surface_count():
		if mesh.surface_get_name(i) == "checker":
			surface = i
	_expect(surface >= 0, "the checker material has its own surface")
	if surface < 0:
		return
	var material: StandardMaterial3D = mesh.surface_get_material(surface) as StandardMaterial3D
	_expect(material != null and material.albedo_texture != null, "checker.tres is the material")
	var uvs: PackedVector2Array = mesh.surface_get_arrays(surface)[Mesh.ARRAY_TEX_UV]
	var low: Vector2 = Vector2.INF
	var high: Vector2 = -Vector2.INF
	for uv: Vector2 in uvs.slice(0, 6):
		low = low.min(uv)
		high = high.max(uv)
	_expect((high - low).is_equal_approx(Vector2(2.0, 4.0)), "UVs divide by the texture size")


# Anti-tiled faces draw through the addon's wrappers unless that would lose part of their look.
func _check_anti_tiling(mesh: ArrayMesh, group: ArrayMesh) -> void:
	var ramp: ShaderMaterial = _surface_material(group, "checker#macro") as ShaderMaterial
	_expect(_wrapper(ramp) == "mapcraft_single_linear.gdshader", \
			"an anti-tiled face draws through the linear wrapper")
	if ramp != null:
		_expect(_approx(ramp.get_shader_parameter(&"macro_amount"), 0.15), \
				"the wrapper varies albedo by 15 percent")
		_expect(ramp.get_shader_parameter(&"albedo_texture_a") \
				== load("res://Assets/Textures/checker.png"), \
				"the wrapper copies the albedo texture")
		_expect(_approx(ramp.get_shader_parameter(&"uv1_scale_a"), Vector3.ONE), \
				"the wrapper copies the UV scale")
		_expect(_approx(ramp.get_shader_parameter(&"map_units"), UNITS_PER_METER), \
				"the wrapper's noise uses the map's units")
	var pixel: ShaderMaterial = _surface_material(mesh, "nearest_mip#macro") as ShaderMaterial
	_expect(_wrapper(pixel) == "mapcraft_single_nearest_mipmap.gdshader", \
			"texture_filter 2 picks the nearest mipmap wrapper")
	var sharp: ShaderMaterial = _surface_material(mesh, "nearest#hex") as ShaderMaterial
	_expect(_wrapper(sharp) == "mapcraft_single_nearest.gdshader", \
			"texture_filter 0 picks the nearest wrapper")
	var specular: Variant = sharp.get_shader_parameter(&"specular_a") if sharp != null else null
	_expect(_approx(specular, 0.0), "the wrapper copies metallic_specular")
	var sharp_hex: Variant = sharp.get_shader_parameter(&"hex_tiling") if sharp != null else null
	var sharp_macro: Variant = sharp.get_shader_parameter(&"macro_amount") if sharp != null else null
	_expect(sharp_hex is bool and not sharp_hex and _approx(sharp_macro, 0.15), \
			"a pixel texture draws hex tiling as variation")
	var hex: ShaderMaterial = _surface_material(mesh, "checker#hex") as ShaderMaterial
	_expect(_wrapper(hex) == "mapcraft_single_linear.gdshader", \
			"a hex tiled face draws through the linear wrapper")
	var tiled: Variant = hex.get_shader_parameter(&"hex_tiling") if hex != null else null
	var varied: Variant = hex.get_shader_parameter(&"macro_amount") if hex != null else null
	_expect(tiled is bool and tiled and _approx(varied, 0.15), \
			"hex tiling turns on the wrapper's hex tiles over the variation")
	for material_name: String in ["glow", "shader"]:
		var kept: Material = _surface_material(mesh, material_name + "#macro")
		var path: String = kept.resource_path if kept != null else ""
		_expect(path == "res://Assets/Textures/%s.tres" % material_name, \
				"%s keeps its own material" % material_name)


# The floor spans map x 1024 to 1280 and its weights rise from 0 to 1 along x.
func _check_blend(mesh: ArrayMesh) -> void:
	var surface: int = _surface_index(mesh, "checker|stripes")
	_expect(surface >= 0, "a blended displacement has its own surface")
	if surface < 0:
		return
	var arrays: Array = mesh.surface_get_arrays(surface)
	var colors: Variant = arrays[Mesh.ARRAY_COLOR]
	var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	var follows: bool = colors is PackedColorArray and colors.size() == vertices.size() \
			and not vertices.is_empty()
	if follows:
		for i: int in vertices.size():
			var expected: float = (vertices[i].x * UNITS_PER_METER - 1024.0) / 256.0
			follows = follows and absf(colors[i].a - expected) < 0.01
	_expect(follows, "vertex alpha carries each vertex's weight")
	var blend: ShaderMaterial = mesh.surface_get_material(surface) as ShaderMaterial
	_expect(_wrapper(blend) == "mapcraft_blend_linear.gdshader", \
			"a blended displacement draws through the blend wrapper")
	if blend == null:
		return
	_expect(_approx(blend.get_shader_parameter(&"uv_ratio_b"), Vector2(2.0, 1.0)), \
			"the second layer keeps its own texel size")
	_expect(blend.get_shader_parameter(&"albedo_texture_b") \
			== load("res://Assets/Textures/stripes.png"), \
			"the wrapper copies the second layer's albedo texture")
	_expect(_approx(blend.get_shader_parameter(&"macro_amount"), 0.0), \
			"a blend without anti-tiling has no variation")


# The floor blending a ShaderMaterial or a missing material falls back to checker.tres alone.
func _check_blend_fallbacks() -> void:
	var text: String = FileAccess.get_file_as_string(MAP)
	var wrappers: Dictionary[String, String] = {
		"shader": "",
		"missing": "",
		"nearest": "mapcraft_blend_linear.gdshader",
	}
	for layer: String in wrappers:
		var copy: String = "user://blend_%s.mcmap" % layer
		var suffix: String = "#hex" if layer == "nearest" else ""
		var edited: String = text.replace("\"blend_material\": \"stripes\"", \
				"\"blend_material\": \"%s\"" % layer)
		if not suffix.is_empty():
			edited = edited.replace("\"disp\": {", \
					"\"attrs\": {\"anti_tile\": \"hex\"}, \"disp\": {")
		var file: FileAccess = FileAccess.open(copy, FileAccess.WRITE)
		file.store_string(edited)
		file.close()
		var packed: PackedScene = load(copy) as PackedScene
		var root: Node = packed.instantiate() if packed != null else null
		var world: MeshInstance3D = null
		if root != null:
			world = root.get_node_or_null("WorldMesh") as MeshInstance3D
		var mesh: ArrayMesh = world.mesh as ArrayMesh if world != null else null
		var surface: int = _surface_index(mesh, "checker|" + layer + suffix) if mesh != null else -1
		_expect(surface >= 0, "a floor blending %s imports" % layer)
		if surface >= 0:
			var material: Material = mesh.surface_get_material(surface)
			var colored: bool = (mesh.surface_get_format(surface) & Mesh.ARRAY_FORMAT_COLOR) != 0
			if wrappers[layer].is_empty():
				_expect(material != null and material.resource_path \
						== "res://Assets/Textures/checker.tres" and not colored, \
						"a floor blending %s draws its own material without weights" % layer)
			else:
				var shaded: ShaderMaterial = material as ShaderMaterial
				_expect(_wrapper(shaded) == wrappers[layer] and colored, \
						"a floor blending %s draws with its own filtering" % layer)
				var hex: Variant = shaded.get_shader_parameter(&"hex_tiling") \
						if shaded != null else null
				var varied: Variant = shaded.get_shader_parameter(&"macro_amount") \
						if shaded != null else null
				_expect(hex is bool and not hex and _approx(varied, 0.15), \
						"a blend with a pixel layer draws hex tiling as variation")
		if root != null:
			root.free()
		DirAccess.remove_absolute(ProjectSettings.globalize_path(copy))


# Headless Godot only prints shader errors, and a shader that fails to compile lists no uniforms.
func _check_wrappers() -> void:
	for kind: String in ["single", "blend"]:
		for filter: String in ["linear", "nearest", "nearest_mipmap"]:
			var wrapper: String = "mapcraft_%s_%s" % [kind, filter]
			var shader: Shader = load(SHADERS + wrapper + ".gdshader") as Shader
			var names: PackedStringArray = PackedStringArray()
			if shader != null:
				for uniform: Dictionary in shader.get_shader_uniform_list():
					names.append(uniform["name"])
			var blends: bool = "uv_ratio_b" in names
			_expect("macro_amount" in names and "hex_tiling" in names \
					and "albedo_texture_a" in names and blends == (kind == "blend"), \
					"%s compiles" % wrapper)


func _check_runtime_build() -> void:
	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = MAP
	map.entity_built.connect(_on_entity_built)
	_expect(map.build() == OK, "MapCraftMap builds at runtime")
	_expect(map.get_node_or_null("WorldMesh") != null, "the runtime build has world geometry")
	_expect(_entities_built == 10, "entity_built fires once per entity")
	map.free()

	var sdk: MapCraftEntityMap = load(ENTITY_MAP) as MapCraftEntityMap
	var per_brush: MapCraftMap = MapCraftMap.new()
	per_brush.map_file = MAP
	per_brush.entity_map = sdk.duplicate() as MapCraftEntityMap
	per_brush.entity_map.trimesh_collision = false
	_expect(per_brush.build() == OK, "MapCraftMap builds with a convex shape per brush")
	var brushes: StaticBody3D = per_brush.get_node_or_null("WorldCollision") as StaticBody3D
	var grouped: StaticBody3D = per_brush.get_node_or_null("Group/Collision") as StaticBody3D
	_expect(brushes != null and brushes.get_child_count() == 8, \
			"walls, detail and terrain collide")
	_expect(grouped != null and grouped.get_child_count() == 1, \
			"the grouped wedge collides in its group")
	_check_displacement(brushes)
	per_brush.free()

	var solid: MapCraftMap = MapCraftMap.new()
	solid.map_file = MAP
	solid.trimesh_collision = true
	_expect(solid.build() == OK, "MapCraftMap builds with trimesh collision")
	var body: StaticBody3D = solid.get_node_or_null("WorldCollision") as StaticBody3D
	var holder: CollisionShape3D = null
	if body != null and body.get_child_count() == 1:
		holder = body.get_child(0) as CollisionShape3D
	var trimesh: bool = holder != null and holder.shape is ConcavePolygonShape3D
	_expect(trimesh, "trimesh collision makes the world one triangle mesh")
	if trimesh:
		var faces: PackedVector3Array = (holder.shape as ConcavePolygonShape3D).get_faces()
		_expect(faces.size() == 86 * 3, "faces hidden between touching brushes are left out")
		_expect(not (holder.shape as ConcavePolygonShape3D).backface_collision, \
				"brush faces collide from outside only")
	solid.free()


# Brush 3 skips one face and brush 4 has one clip face, which hides all of it.
func _check_tool_brushes() -> void:
	var doc: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(MAP))
	var kept: Array = []
	for child: Dictionary in doc["map"]["root"]["children"]:
		if child["id"] == 3 or child["id"] == 4:
			for face: Dictionary in child["faces"]:
				face["material"] = "checker"
			child["faces"][0]["material"] = "tools/skip" if child["id"] == 3 else "tools/clip"
			kept.append(child)
	doc["map"]["root"]["children"] = kept
	var path: String = "user://tool_brushes.mcmap"
	_write_text(path, JSON.stringify(doc))
	var sdk: MapCraftEntityMap = load(ENTITY_MAP) as MapCraftEntityMap
	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = path
	map.entity_map = sdk.duplicate() as MapCraftEntityMap
	map.entity_map.trimesh_collision = false
	_expect(map.build() == OK, "a map with skip and clip builds")
	var world: MeshInstance3D = map.get_node_or_null("WorldMesh") as MeshInstance3D
	var mesh: ArrayMesh = world.mesh as ArrayMesh if world != null else null
	var drawn: int = 0
	for i: int in mesh.get_surface_count() if mesh != null else 0:
		drawn += mesh.surface_get_array_len(i)
	_expect(drawn == 5 * 6, "skip leaves out its face and clip draws nothing")
	var body: StaticBody3D = map.get_node_or_null("WorldCollision") as StaticBody3D
	_expect(body != null and body.get_child_count() == 2, "skip and clip brushes still collide")
	map.free()
	DirAccess.remove_absolute(ProjectSettings.globalize_path(path))


func _check_runtime_load() -> void:
	var copy: String = "user://runtime_course.mcmap"
	var file: FileAccess = FileAccess.open(copy, FileAccess.WRITE)
	file.store_string(FileAccess.get_file_as_string(MAP))
	file.close()
	var packed: PackedScene = load(copy) as PackedScene
	_expect(packed != null, "a .mcmap outside the project loads as a scene")
	var root: Node = packed.instantiate() if packed != null else null
	if root != null:
		var start: Area3D = root.get_node_or_null("StartZone") as Area3D
		_expect(_zone_kind(start) == "start", "the loaded map has its zones")
		_expect(root.get_node_or_null("WorldEnvironment") is WorldEnvironment, \
				"a map with no light_environment gets a sky")
		var sun: DirectionalLight3D = root.get_node_or_null("Sun") as DirectionalLight3D
		_expect(sun != null and (-sun.basis.z).y < 0.0, "the default sun shines down")
		root.free()
	DirAccess.remove_absolute(ProjectSettings.globalize_path(copy))


# A downloaded map's prefab could name a scene with a script, or a .remap planted to point at one.
func _check_untrusted_prefabs() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(PREFAB_DIR))
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(MOUNTED_DIR))
	_write_text(PREFAB_DIR + "scripted.tscn", SCRIPTED_SCENE)
	_write_text(PREFAB_DIR + "inner.mcmap.remap", \
			"[remap]\n\npath=\"%sscripted.tscn\"\n" % PREFAB_DIR)
	# What a Workshop .pck mounted into res:// can add.
	_write_text(MOUNTED_DIR + "scripted.tscn", SCRIPTED_SCENE)
	_write_text(MOUNTED_DIR + "inner.mcmap.remap", \
			"[remap]\n\npath=\"%sscripted.tscn\"\n" % MOUNTED_DIR)
	var hostile: Dictionary[String, String] = {
		"a scene beside the map": "scripted.tscn",
		"a .remap beside the map": "inner.mcmap",
		"an absolute scene path": ProjectSettings.globalize_path(PREFAB_DIR + "scripted.tscn"),
		"a .remap a mounted pack added to res://": MOUNTED_DIR + "inner.mcmap",
	}
	for what: String in hostile:
		var map: MapCraftMap = _build_with_prefabs(PREFAB_DIR + "map.mcmap", [hostile[what]])
		_expect(Engine.max_fps != 1234, "%s cannot make a prefab run a script" % what)
		Engine.max_fps = 0
		map.free()

	_write_text(PREFAB_DIR + "piece.mcmap", FileAccess.get_file_as_string(MAP))
	_write_text(PREFAB_DIR + "self.mcmap", _prefab_map(["self.mcmap"]))
	var built: MapCraftMap = _build_with_prefabs(PREFAB_DIR + "map.mcmap", \
			["piece.mcmap", "self.mcmap"])
	var piece: Node = built.get_node_or_null("piece")
	_expect(piece != null and piece.get_node_or_null("WorldMesh") != null, \
			"a prefab builds from its map file")
	var itself: Node = built.get_node_or_null("self")
	_expect(itself != null and itself.get_node_or_null("self") == null, \
			"a prefab that contains itself stops at itself")
	built.free()

	for i: int in 18:
		_write_text(PREFAB_DIR + "fan%d.mcmap" % i, \
				_prefab_map(["fan%d.mcmap" % (i + 1), "fan%d.mcmap" % (i + 1)]))
	var fan: MapCraftMap = _build_with_prefabs(PREFAB_DIR + "map.mcmap", ["fan0.mcmap"])
	var copies: int = fan.find_children("fan*", "Node3D", true, false).size()
	_expect(copies > 16 and copies <= 256, "prefabs that fan out stop at 256 copies")
	fan.free()
	for dir: String in [PREFAB_DIR, MOUNTED_DIR]:
		for file: String in DirAccess.get_files_at(dir):
			DirAccess.remove_absolute(ProjectSettings.globalize_path(dir + file))
	DirAccess.remove_absolute(ProjectSettings.globalize_path(MOUNTED_DIR))


# res:// keeps a leading .., so a name in the map could reach a scripted file beside the game.
func _check_paths_stay_in_project() -> void:
	var outside: String = ProjectSettings.globalize_path("res://").simplify_path().get_base_dir() \
			.path_join("mapcraft_escape")
	DirAccess.make_dir_recursive_absolute(outside)
	_write_text(outside.path_join("scripted.tscn"), SCRIPTED_SCENE)
	_write_text(outside.path_join("scripted.tres"), SCRIPTED_MATERIAL)
	var text: String = FileAccess.get_file_as_string(MAP)
	var escapes: Dictionary[String, String] = {
		"a model path": text.replace("Assets/Models/box.glb", "../mapcraft_escape/scripted.tscn"),
		"a material name": text.replace("\"material\": \"glow\"", \
				"\"material\": \"../../../mapcraft_escape/scripted\""),
		"an SDK name": text.replace("\"sdk\": \"surfsup\"", \
				"\"sdk\": \"../../../../mapcraft_escape/scripted\""),
	}
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(PREFAB_DIR))
	for what: String in escapes:
		_write_text(PREFAB_DIR + "escape.mcmap", escapes[what])
		var scene: PackedScene = ResourceLoader.load(PREFAB_DIR + "escape.mcmap", "", \
				ResourceLoader.CACHE_MODE_IGNORE) as PackedScene
		var root: Node = scene.instantiate() if scene != null else null
		_expect(root != null, "a map with %s outside the project still loads" % what)
		_expect(Engine.max_fps != 1234, "%s cannot load a scripted file outside the project" % what)
		Engine.max_fps = 0
		if root != null:
			root.free()
	for file: String in DirAccess.get_files_at(outside):
		DirAccess.remove_absolute(outside.path_join(file))
	DirAccess.remove_absolute(outside)
	DirAccess.remove_absolute(ProjectSettings.globalize_path(PREFAB_DIR + "escape.mcmap"))


func _build_with_prefabs(path: String, prefabs: Array[String]) -> MapCraftMap:
	var doc: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(MAP))
	var children: Array = doc["map"]["root"]["children"]
	for prefab: String in prefabs:
		children.append({"id": 900000 + children.size(), "type": "prefab", "path": prefab})
	_write_text(path, JSON.stringify(doc))
	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = path
	_expect(map.build() == OK, "a map with prefabs %s builds" % [prefabs])
	return map


func _prefab_map(prefabs: Array[String]) -> String:
	var children: Array[Dictionary] = []
	for prefab: String in prefabs:
		children.append({"id": 2 + children.size(), "type": "prefab", "path": prefab})
	return JSON.stringify({
		"format": "mapcraft.map",
		"schema_version": 1,
		"map": {"root": {"id": 1, "type": "world", "children": children}},
	})


func _check_pack(args: PackedStringArray) -> void:
	if args.is_empty():
		return
	_expect(ProjectSettings.load_resource_pack(args[0], false), "the map pack mounts")
	var packed: PackedScene = load(PACKED_MAP) as PackedScene
	var root: Node = packed.instantiate() if packed != null else null
	var spawn: Node = root.get_node_or_null("SpawnPoint") if root != null else null
	_expect(spawn is Marker3D, "the pack has the map")
	if root != null:
		root.free()


func _write_text(path: String, text: String) -> void:
	var file: FileAccess = FileAccess.open(path, FileAccess.WRITE)
	file.store_string(text)
	file.close()


func _check_editable_scene(map: String) -> void:
	var path: String = "user://editable_test/%s.tscn" % map.get_file().get_basename()
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	_expect(MapCraftMap.save_as_scene(map, path) == OK, "%s saves as an editable scene" % map)
	var scene: PackedScene = ResourceLoader.load(path, "", ResourceLoader.CACHE_MODE_IGNORE)
	var state: SceneState = scene.get_state() if scene != null else null
	_expect(state != null and state.get_node_instance(0) == null \
			and state.get_base_scene_state() == null, \
			"%s's editable scene owns its root instead of instancing the map" % map)
	var root: Node = scene.instantiate() if scene != null else null
	_expect(root != null and root.get_node_or_null("SpawnPoint") is Marker3D \
			and root.get_node_or_null("Group/Mesh") is MeshInstance3D, \
			"%s's editable scene keeps the map's nodes and groups" % map)
	if root != null:
		root.free()
	_expect(not FileAccess.get_file_as_string(path).contains(map.get_file()), \
			"%s's editable scene does not refer back to the map" % map)


func _surface_material(mesh: ArrayMesh, surface_name: String) -> Material:
	var surface: int = _surface_index(mesh, surface_name)
	return mesh.surface_get_material(surface) if surface >= 0 else null


func _surface_index(mesh: ArrayMesh, surface_name: String) -> int:
	for i: int in mesh.get_surface_count():
		if mesh.surface_get_name(i) == surface_name:
			return i
	return -1


# Takes Variants so a missing shader parameter fails its check instead of stopping the test.
func _approx(value: Variant, expected: Variant) -> bool:
	if typeof(value) != typeof(expected):
		return false
	if value is float:
		return is_equal_approx(value, expected)
	return value.is_equal_approx(expected)


func _wrapper(material: ShaderMaterial) -> String:
	if material == null or material.shader == null:
		return ""
	return material.shader.resource_path.get_file()


func _zone_kind(zone: Area3D) -> String:
	return str(zone.get_meta(&"sdk_zone", "")) if zone != null else ""


func _on_entity_built(_node: Node, _classname: String, _properties: Dictionary) -> void:
	_entities_built += 1


func _expect(condition: bool, what: String) -> void:
	if not condition:
		_fail(what)


func _fail(what: String) -> void:
	_failures.append(what)
