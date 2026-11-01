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
