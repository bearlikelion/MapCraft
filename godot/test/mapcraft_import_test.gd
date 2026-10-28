class_name MapCraftImportTest
extends SceneTree

const MAP: String = "res://course.mcmap"
const PACKED_MAP: String = "res://maps/course/course.scn"
const ENTITY_MAP: String = "res://addons/mapcraft/sdks/surfsup.tres"
const SHADERS: String = "res://addons/mapcraft/shaders/"
const UNITS_PER_METER: float = 39.37

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
	_check_runtime_load()
	_check_pack(OS.get_cmdline_user_args())
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

	var mesh: MeshInstance3D = root.get_node_or_null("WorldMesh") as MeshInstance3D
	_expect(mesh != null and mesh.mesh.get_surface_count() == 9, \
			"one surface per material, blend layer and anti-tiling mode")
	if mesh != null:
		_check_checker_surface(mesh.mesh as ArrayMesh)
		_check_anti_tiling(mesh.mesh as ArrayMesh)
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
func _check_anti_tiling(mesh: ArrayMesh) -> void:
	var ramp: ShaderMaterial = _surface_material(mesh, "checker#macro") as ShaderMaterial
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
	_expect(_entities_built == 8, "entity_built fires once per entity")
	map.free()

	var sdk: MapCraftEntityMap = load(ENTITY_MAP) as MapCraftEntityMap
	var per_brush: MapCraftMap = MapCraftMap.new()
	per_brush.map_file = MAP
	per_brush.entity_map = sdk.duplicate() as MapCraftEntityMap
	per_brush.entity_map.trimesh_collision = false
	_expect(per_brush.build() == OK, "MapCraftMap builds with a convex shape per brush")
	var brushes: StaticBody3D = per_brush.get_node_or_null("WorldCollision") as StaticBody3D
	_expect(brushes != null and brushes.get_child_count() == 9, \
			"walls, detail, wedge and terrain collide")
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
	solid.free()


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
