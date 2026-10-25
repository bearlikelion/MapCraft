class_name MapCraftImportTest
extends SceneTree

const MAP: String = "res://course.mcmap"
const PACKED_MAP: String = "res://maps/course/course.scn"
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
	_check_runtime_build()
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

	var finish: Area3D = root.get_node_or_null("FinishZone") as Area3D
	_expect(_zone_kind(finish) == "finish", "FinishZone is a finish zone")

	var teleport: Area3D = root.get_node_or_null("teleport_zone") as Area3D
	var stage: String = teleport.get_meta(&"next_stage", "") if teleport != null else ""
	_expect(stage == "dest1", "teleport names its destination")
	_expect(root.find_child("dest1", false, false) is Marker3D, "the destination is a Marker3D")

	var booster: Area3D = root.get_node_or_null("zone_booster") as Area3D
	var direction: Vector3 = Vector3.ZERO
	if booster != null:
		direction = booster.get_meta(&"boost_direction", Vector3.ZERO)
	_expect(direction.is_equal_approx(Vector3.FORWARD), "boost_direction turns from +Y to -Z")

	var mesh: MeshInstance3D = root.get_node_or_null("WorldMesh") as MeshInstance3D
	_expect(mesh != null and mesh.mesh.get_surface_count() == 2, "one surface per material")
	if mesh != null:
		_check_checker_surface(mesh.mesh as ArrayMesh)
	var body: StaticBody3D = root.get_node_or_null("WorldCollision") as StaticBody3D
	_expect(body != null and body.get_child_count() == 8, "walls, detail and wedge collide")

	for node: Node in root.find_children("*", "", true, false):
		_expect(node.get_script() == null, "%s carries no script" % node.name)


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


func _check_runtime_build() -> void:
	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = MAP
	map.entity_built.connect(_on_entity_built)
	_expect(map.build() == OK, "MapCraftMap builds at runtime")
	_expect(map.get_node_or_null("WorldMesh") != null, "the runtime build has world geometry")
	_expect(_entities_built == 7, "entity_built fires once per entity")
	map.free()


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


func _zone_kind(zone: Area3D) -> String:
	return str(zone.get_meta(&"sdk_zone", "")) if zone != null else ""


func _on_entity_built(_node: Node, _classname: String, _properties: Dictionary) -> void:
	_entities_built += 1


func _expect(condition: bool, what: String) -> void:
	if not condition:
		_fail(what)


func _fail(what: String) -> void:
	_failures.append(what)
