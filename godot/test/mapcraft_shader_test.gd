class_name MapCraftShaderTest
extends SceneTree

const PATH: String = "user://mapcraft_shader_test.mcmap"
const SKY: String = "shader_type sky;\nvoid sky() { COLOR = vec3(0.1, 0.2, 0.9); }\n"
const SPATIAL: String = "shader_type spatial;\nuniform vec3 tint = vec3(1.0, 0.0, 0.0);\n" \
		+ "void fragment() { ALBEDO = tint; }\n"

const PARAMS: String = "shader_type spatial;\nuniform float speed = 1.0;\n" \
		+ "uniform int steps = 2;\nuniform bool on = false;\nuniform vec2 scroll;\n" \
		+ "uniform vec4 tint : source_color;\nuniform sampler2D albedo;\n" \
		+ "void fragment() { ALBEDO = texture(albedo, UV).rgb; }\n"

var _failures: int = 0


func _initialize() -> void:
	var map: MapCraftMap = _build(SKY, [SPATIAL, SPATIAL, ""])
	var sky: ShaderMaterial = _sky_material(map) as ShaderMaterial
	_expect(sky != null and sky.shader.code == SKY, "the world's sky shader draws the sky")
	var shaded: Array[Material] = _surface_materials(map, "shader:")
	_expect(shaded.size() == 1, "brushes with the same shader share one surface")
	_expect(not shaded.is_empty() and shaded[0] is ShaderMaterial \
			and (shaded[0] as ShaderMaterial).shader.code == SPATIAL, \
			"a brush's spatial shader draws its faces")
	_expect(_surface_materials(map, "ramps-orange1").size() == 1, \
			"a brush without a shader keeps its face material")
	map.free()

	map = _build(SPATIAL, [SKY])
	_expect(not (_sky_material(map) is ShaderMaterial), "a spatial shader on the world is skipped")
	_expect(_surface_materials(map, "shader:").is_empty(), "a sky shader on a brush is skipped")
	_expect(_surface_materials(map, "ramps-orange1").size() == 1, \
			"a skipped brush shader keeps the face material")
	map.free()

	var huge: String = SPATIAL + "//" + "x".repeat(256 * 1024)
	map = _build("", [huge])
	_expect(_surface_materials(map, "shader:").is_empty(), "an oversized shader is skipped")
	map.free()

	var good: Dictionary = {
		"speed": 2.5, "steps": 4, "on": true, "scroll": [1, 2],
		"tint": [0.5, 0.25, 1.0, 1.0], "albedo": "checker",
	}
	var garbage: Dictionary = {
		"speed": "fast", "steps": [1], "on": 1, "scroll": [1, 2, 3, 4, 5],
		"tint": ["a", 0, 0], "albedo": "../outside", "missing": 1.0,
	}
	map = _build("", [PARAMS, PARAMS, PARAMS], [good, good, garbage])
	var materials: Array[Material] = _surface_materials(map, "shader:")
	_expect(materials.size() == 2, "the same source with other parameters is its own material")
	var applied: ShaderMaterial = ShaderMaterial.new()
	var skipped: ShaderMaterial = ShaderMaterial.new()
	for material: Material in materials:
		var shader: ShaderMaterial = material as ShaderMaterial
		if shader != null and shader.get_shader_parameter("speed") != null:
			applied = shader
		elif shader != null:
			skipped = shader
	var speed: Variant = applied.get_shader_parameter("speed")
	_expect(speed is float and is_equal_approx(speed, 2.5), "a float parameter is set")
	_expect(applied.get_shader_parameter("steps") == 4, "an int parameter is set")
	_expect(applied.get_shader_parameter("on") == true, "a bool parameter is set")
	_expect(applied.get_shader_parameter("scroll") == Vector2(1, 2), "a vec2 parameter is set")
	_expect(applied.get_shader_parameter("tint") == Color(0.5, 0.25, 1.0, 1.0), \
			"a source_color parameter becomes a color")
	var texture: Texture2D = applied.get_shader_parameter("albedo") as Texture2D
	_expect(texture != null and texture.resource_path == "res://Assets/Textures/checker.png", \
			"a sampler parameter takes the named material's picture")
	_expect(skipped.shader != null and skipped.get_shader_parameter("speed") == null \
			and skipped.get_shader_parameter("albedo") == null, "garbage parameters are skipped")
	map.free()

	DirAccess.remove_absolute(ProjectSettings.globalize_path(PATH))
	print("mapcraft shader test: %d failures" % _failures)
	quit(1 if _failures > 0 else 0)


func _build(world_shader: String, brush_shaders: Array, params: Array = []) -> MapCraftMap:
	var children: Array = []
	for i: int in brush_shaders.size():
		var x: int = i * 128
		var brush: Dictionary = {
			"id": 10 + i, "type": "brush",
			"faces": [{
				"id": 100 + i, "material": "ramps-orange1",
				"u": {"axis": [1, 0, 0], "offset": 0, "scale": 1},
				"v": {"axis": [0, -1, 0], "offset": 0, "scale": 1},
				"polygon": [[x, 0, 0], [x + 64, 0, 0], [x + 64, 64, 0], [x, 64, 0]],
			}],
		}
		if not String(brush_shaders[i]).is_empty():
			brush["shader"] = brush_shaders[i]
		if i < params.size():
			brush["shader_params"] = params[i]
		children.append(brush)
	var root: Dictionary = {
		"id": 1, "type": "world", "classname": "worldspawn", "children": children,
	}
	if not world_shader.is_empty():
		root["shader"] = world_shader
	var doc: Dictionary = {"format": "mapcraft.map", "schema_version": 4, "map": {"root": root}}
	var map: MapCraftMap = MapCraftMap.new()
	var file: FileAccess = FileAccess.open(PATH, FileAccess.WRITE)
	if file == null:
		_expect(false, "cannot write test map")
		return map
	file.store_string(JSON.stringify(doc))
	file.close()
	map.map_file = PATH
	_expect(map.build() == OK, "map builds")
	return map


func _sky_material(map: MapCraftMap) -> Material:
	var world: WorldEnvironment = map.get_node_or_null("WorldEnvironment") as WorldEnvironment
	if world == null or world.environment == null or world.environment.sky == null:
		return null
	return world.environment.sky.sky_material


func _surface_materials(map: MapCraftMap, prefix: String) -> Array[Material]:
	var out: Array[Material] = []
	var world: MeshInstance3D = map.get_node_or_null("WorldMesh") as MeshInstance3D
	var mesh: ArrayMesh = world.mesh as ArrayMesh if world != null else null
	if mesh == null:
		return out
	for i: int in mesh.get_surface_count():
		if mesh.surface_get_name(i).begins_with(prefix):
			out.append(mesh.surface_get_material(i))
	return out


func _expect(value: bool, label: String) -> void:
	if not value:
		_failures += 1
		printerr("FAIL: ", label)
