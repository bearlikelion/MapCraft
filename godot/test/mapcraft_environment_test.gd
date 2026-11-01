class_name MapCraftEnvironmentTest
extends SceneTree

const PATH: String = "user://mapcraft_environment_test.mcmap"
const ENTITY_MAP: String = "res://addons/mapcraft/sdks/surfsup.tres"

var _failures: int = 0


func _initialize() -> void:
	var map: MapCraftMap = _build({
		"tonemap_mode": "1", "tonemap_exposure": "2", "tonemap_white": "3",
		"adjustment_enabled": "1", "adjustment_brightness": "1.5",
		"adjustment_contrast": "1.2", "adjustment_saturation": "0.5",
		"fog_density": "0.01", "fog_light_energy": "2", "fog_sun_scatter": "0.3",
		"fog_sky_affect": "0.4", "sky_curve": "0.3", "ground_curve": "0.2",
		"sky_energy_multiplier": "2", "ground_energy_multiplier": "3",
		"glow_enabled": "1", "glow_intensity": "1.5", "glow_bloom": "0.2",
		"glow_hdr_threshold": "0.8", "glow_blend_mode": "0",
		"fog_mode": "depth", "fog_depth_curve": "2", "fog_depth_begin": "5", "fog_depth_end": "50",
		"fog_height": "-3", "fog_height_density": "0.5", "fog_aerial_perspective": "0.25",
	})
	var world: WorldEnvironment = map.get_node_or_null("WorldEnvironment") as WorldEnvironment
	_expect(world != null, "environment node exists")
	if world != null:
		var env: Environment = world.environment
		_expect(env.tonemap_mode == Environment.TONE_MAPPER_REINHARDT, "tonemapper")
		_expect(is_equal_approx(env.tonemap_exposure, 2.0), "exposure")
		_expect(is_equal_approx(env.tonemap_white, 3.0), "white point")
		_expect(env.adjustment_enabled, "adjustments enabled")
		_expect(is_equal_approx(env.adjustment_saturation, 0.5), "saturation")
		_expect(env.fog_enabled, "fog enabled from density")
		_expect(is_equal_approx(env.fog_light_energy, 2.0), "fog energy")
		_expect(is_equal_approx(env.fog_sun_scatter, 0.3), "fog scatter")
		_expect(is_equal_approx(env.fog_sky_affect, 0.4), "fog sky affect")
		_expect(env.glow_enabled and is_equal_approx(env.glow_intensity, 1.5), "glow intensity")
		_expect(is_equal_approx(env.glow_bloom, 0.2), "glow bloom")
		_expect(is_equal_approx(env.glow_hdr_threshold, 0.8), "glow threshold")
		_expect(env.glow_blend_mode == Environment.GLOW_BLEND_MODE_ADDITIVE, "glow blend mode")
		_expect(env.fog_mode == Environment.FOG_MODE_DEPTH, "depth fog")
		_expect(is_equal_approx(env.fog_depth_curve, 2.0) and is_equal_approx(env.fog_depth_begin, 5.0) \
				and is_equal_approx(env.fog_depth_end, 50.0), "depth fog range")
		_expect(is_equal_approx(env.fog_height, -3.0) and is_equal_approx(env.fog_height_density, 0.5), \
				"height fog")
		_expect(is_equal_approx(env.fog_aerial_perspective, 0.25), "aerial perspective")
		var sky: ProceduralSkyMaterial = env.sky.sky_material as ProceduralSkyMaterial
		_expect(sky != null and is_equal_approx(sky.sky_curve, 0.3), "sky curve")
		_expect(sky != null and is_equal_approx(sky.ground_energy_multiplier, 3.0), \
				"ground energy")
	map.free()
	map = _build({"tonemap_exposure": "NaN", "sky_curve": "0", "fog_density": "inf"})
	world = map.get_node_or_null("WorldEnvironment") as WorldEnvironment
	_expect(world != null and is_equal_approx(world.environment.tonemap_exposure, 1.0), \
			"invalid exposure falls back")
	_expect(world != null and not world.environment.fog_enabled, "invalid fog falls back")
	map.free()
	map = _build({"fog_density": "0.1", "fog_enabled": "0.0", "tonemap_mode": "4"})
	world = map.get_node_or_null("WorldEnvironment") as WorldEnvironment
	_expect(world != null and not world.environment.fog_enabled, "explicit fog off")
	_expect(world != null and world.environment.tonemap_mode == Environment.TONE_MAPPER_AGX, \
			"AgX remains available in Godot")
	map.free()
	map = _build({"fog_density": "100"})
	world = map.get_node_or_null("WorldEnvironment") as WorldEnvironment
	_expect(world != null and world.environment.fog_enabled \
			and is_equal_approx(world.environment.fog_density, 1.0), "out of range fog clamps")
	map.free()
	DirAccess.remove_absolute(ProjectSettings.globalize_path(PATH))
	print("mapcraft environment test: %d failures" % _failures)
	quit(1 if _failures > 0 else 0)


func _build(properties: Dictionary) -> MapCraftMap:
	var pairs: Array = []
	for key: String in properties:
		pairs.append([key, properties[key]])
	var doc: Dictionary = {
		"format": "mapcraft.map", "schema_version": 2,
		"map": {"root": {"id": 1, "type": "world", "classname": "worldspawn",
			"children": [{"id": 2, "type": "entity", "classname": "light_environment",
				"properties": pairs}]}},
	}
	var file: FileAccess = FileAccess.open(PATH, FileAccess.WRITE)
	var map: MapCraftMap = MapCraftMap.new()
	if file == null:
		_expect(false, "cannot write test map")
		return map
	file.store_string(JSON.stringify(doc))
	file.close()
	map.map_file = PATH
	map.entity_map = load(ENTITY_MAP) as MapCraftEntityMap
	_expect(map.build() == OK, "map builds")
	return map


func _expect(value: bool, label: String) -> void:
	if not value:
		_failures += 1
		printerr("FAIL: ", label)
