class_name MapCraftLightmapTest
extends SceneTree

const MAP: String = "user://mapcraft_lightmap_test.mcmap"
const LIGHTING: String = "user://mapcraft_lightmap_test.mclight"
const ENTITY_MAP: String = "res://addons/mapcraft/sdks/surfsup.tres"
const SIZE: int = 4

var _failures: int = 0


func _initialize() -> void:
	var doc: Dictionary = JSON.parse_string(FileAccess.get_file_as_string("res://course.mcmap"))
	var root: Dictionary = doc["map"]["root"]
	root["children"].append({
		"id": 990, "type": "entity", "classname": "light_environment", "properties": [],
	})
	var brush: Dictionary = _first_brush(root, false)
	var displaced: Dictionary = _first_brush(root, true)
	_write(MAP, JSON.stringify(doc).to_utf8_buffer())

	var plain: MapCraftMap = _build()
	_expect(plain.get_node_or_null("BakedLighting") == null, "no lighting file, no LightmapGI")
	var sun: DirectionalLight3D = plain.get_node_or_null("Sun") as DirectionalLight3D
	_expect(sun != null and sun.light_bake_mode != Light3D.BAKE_STATIC, \
			"an unbaked map keeps a realtime sun")
	plain.free()

	_write(LIGHTING, _lighting(int(brush["id"]), (brush["faces"] as Array).size()))
	var map: MapCraftMap = _build()
	var gi: LightmapGI = map.get_node_or_null("BakedLighting") as LightmapGI
	_expect(gi != null, "a map with baked lighting gets a LightmapGI")
	if gi != null:
		var data: LightmapGIData = gi.light_data
		_expect(data.is_using_spherical_harmonics(), "directional lightmaps")
		_expect(data.get("_uses_packed_directional") == true, \
				"the bands are marked packed, as Godot 4.4 and later expect")
		var texture: Texture2DArray = data.get_lightmap_textures()[0]
		_expect(texture.get_layers() == 4 and texture.get_width() == SIZE, "four layers per slice")
		_expect(data.get_user_count() == 1, "one lightmapped mesh")
		var probes: Dictionary = data.get("probe_data")
		var bounds: AABB = probes.get("bounds", AABB())
		_expect(bounds.has_volume(), "probe bounds span the map, so Godot never culls the lightmap")
		var user: MeshInstance3D = gi.get_node_or_null(data.get_user_path(0)) as MeshInstance3D
		_expect(user != null, "the user path reaches its mesh")
		if user != null:
			for s: int in user.mesh.get_surface_count():
				_expect(user.mesh.surface_get_format(s) & Mesh.ARRAY_FORMAT_TEX_UV2 != 0, \
						"lightmapped surfaces carry UV2")
		_expect(map.get_node_or_null("WorldMesh") != null, "the unlit faces keep the world mesh")
	sun = map.get_node_or_null("Sun") as DirectionalLight3D
	_expect(sun != null and sun.light_bake_mode == Light3D.BAKE_STATIC, \
			"the sun lights only what moves")
	map.free()

	_expect(not displaced.is_empty(), "the course has a displacement")
	_write(LIGHTING, _lighting(int(displaced["id"]), (displaced["faces"] as Array).size()))
	var bumpy: MapCraftMap = _build()
	var bumpy_gi: LightmapGI = bumpy.get_node_or_null("BakedLighting") as LightmapGI
	_expect(bumpy_gi != null and bumpy_gi.light_data.get_user_count() == 1, \
			"a lightmapped displacement gets a mesh of its own")
	if bumpy_gi != null and bumpy_gi.light_data.get_user_count() == 1:
		var path: NodePath = bumpy_gi.light_data.get_user_path(0)
		var mesh: Mesh = (bumpy_gi.get_node(path) as MeshInstance3D).mesh
		for s: int in mesh.get_surface_count():
			_expect(mesh.surface_get_format(s) & Mesh.ARRAY_FORMAT_TEX_UV2 != 0, \
					"lightmapped displacements carry UV2")
	bumpy.free()

	_write(LIGHTING, "MCLM nonsense".to_utf8_buffer())
	var damaged: MapCraftMap = _build()
	_expect(damaged.get_node_or_null("BakedLighting") == null, \
			"damaged lighting falls back to realtime")
	damaged.free()

	DirAccess.remove_absolute(ProjectSettings.globalize_path(MAP))
	DirAccess.remove_absolute(ProjectSettings.globalize_path(LIGHTING))
	print("mapcraft lightmap test: %d failures" % _failures)
	quit(1 if _failures > 0 else 0)


func _first_brush(node: Dictionary, displaced: bool) -> Dictionary:
	if node.get("type") == "brush" \
	and JSON.stringify(node).contains("disp_vertices") == displaced:
		return node
	for child: Dictionary in node.get("children", []):
		var found: Dictionary = _first_brush(child, displaced)
		if not found.is_empty():
			return found
	return {}


# A one-slice .mclight lighting every face of brush `node` from one texel.
func _lighting(node: int, faces: int) -> PackedByteArray:
	var out: PackedByteArray = PackedByteArray()
	out.append_array("MCLM".to_ascii_buffer())
	for value: int in [2, 0, 0, SIZE, 1, faces]:
		_u32(out, value)
	for face: int in faces:
		var entry: PackedByteArray = PackedByteArray()
		entry.resize(48)
		entry.encode_u64(0, node)
		entry.encode_u32(8, face)
		entry.encode_u32(12, 0)
		entry.encode_float(28, 0.5)
		entry.encode_float(44, 0.5)
		out.append_array(entry)
	var texels: PackedByteArray = PackedByteArray()
	texels.resize(4 * SIZE * SIZE * 4)
	var packed: PackedByteArray = texels.compress(FileAccess.COMPRESSION_DEFLATE)
	var length: PackedByteArray = PackedByteArray()
	length.resize(8)
	length.encode_u64(0, packed.size())
	out.append_array(length)
	out.append_array(packed)
	return out


func _u32(out: PackedByteArray, value: int) -> void:
	var b: PackedByteArray = PackedByteArray()
	b.resize(4)
	b.encode_u32(0, value)
	out.append_array(b)


func _write(path: String, bytes: PackedByteArray) -> void:
	var file: FileAccess = FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		_expect(false, "cannot write " + path)
		return
	file.store_buffer(bytes)
	file.close()


func _build() -> MapCraftMap:
	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = MAP
	map.entity_map = load(ENTITY_MAP) as MapCraftEntityMap
	_expect(map.build() == OK, "map builds")
	return map


func _expect(value: bool, label: String) -> void:
	if not value:
		_failures += 1
		printerr("FAIL: ", label)
