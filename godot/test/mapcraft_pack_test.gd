class_name MapCraftPackTest
extends SceneTree

const MAP: String = "res://course.mcmap"
const WRITER_PACK: String = "res://pack/course.mcpck"
const BOX: String = "res://Assets/Models/box.glb"
const ENTITY_MAP: String = "res://addons/mapcraft/sdks/surfsup.tres"
const DIR: String = "user://pack_test/"
const GOOD_MATERIAL: String = "materials/ramps-orange1.png"
const BAD_MATERIAL: String = "materials/checker.png"
const BAD_JPEG: String = "materials/checker.jpg"
const BAD_MODEL: String = "models/props/crate.glb"
const GLB_JSON: int = 0x4E4F534A
const GLB_BIN: int = 0x004E4942

var _failures: PackedStringArray = PackedStringArray()
var _messages: Messages = Messages.new()
var _picture: PackedByteArray = PackedByteArray()


func _initialize() -> void:
	OS.add_logger(_messages)
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(DIR))
	var text: String = FileAccess.get_file_as_string(MAP)
	var copy: String = text.replace("Assets/Models/box.glb", "props/crate.glb")
	_save(DIR + "course.mcmap", copy.to_utf8_buffer())
	_picture = _png(8, 4)
	_check_writer_pack()
	_check_containers()
	_check_names()
	_check_pbr_materials()
	_check_pictures()
	_check_pixel_budget()
	_check_models()
	_check_map_packs()
	OS.remove_logger(_messages)
	for file: String in DirAccess.get_files_at(DIR):
		DirAccess.remove_absolute(ProjectSettings.globalize_path(DIR + file))
	for failure: String in _failures:
		printerr("FAIL: ", failure)
	print("mapcraft pack test: %d failures" % _failures.size())
	quit(1 if _failures.size() > 0 else 0)


func _check_pbr_materials() -> void:
	var descriptor: Dictionary = {
		"version": 1, "albedo": [0.2, 0.4, 0.6, 1.0], "roughness": 0.3,
		"metallic": 0.7, "emission": [0.1, 0.0, 0.0],
		"textures": {"albedo": "textures/stone.png", "normal": "textures/normal.png"},
	}
	var entries: Array = [
		["materials/ramps-orange1.json", JSON.stringify(descriptor).to_utf8_buffer()],
		["textures/stone.png", _picture], ["textures/normal.png", _picture],
	]
	var bytes: PackedByteArray = _pack(entries)
	bytes.encode_u32(4, 2)
	var map: MapCraftMap = _build(bytes)
	var world: MeshInstance3D = map.get_node_or_null("WorldMesh") as MeshInstance3D
	var material: StandardMaterial3D = _surface_material(world, "ramps-orange1") as StandardMaterial3D
	_expect(material != null, "v2 PBR descriptor builds a material")
	if material != null:
		_expect(is_equal_approx(material.roughness, 0.3), "PBR roughness preserved")
		_expect(is_equal_approx(material.metallic, 0.7), "PBR metallic preserved")
		_expect(material.normal_enabled, "PBR normal mapping enabled")
		_expect(material.albedo_texture is ImageTexture, "PBR texture decoded from pack")
	_expect(not _messages.has("mcpck"), "valid v2 pack logs nothing")
	map.free()
	bytes.encode_u32(4, 1)
	map = _build(bytes)
	_expect(_messages.has("version 1"), "v1 refuses v2 entries")
	map.free()
	entries.pop_back()
	bytes = _pack(entries)
	bytes.encode_u32(4, 2)
	map = _build(bytes)
	_expect(_messages.has("missing texture"), "missing PBR dependency is diagnosed")
	map.free()


# The pack MapCraft's Rust writer made, so the addon must take it whole.
func _check_writer_pack() -> void:
	var pack: PackedByteArray = FileAccess.get_file_as_bytes(WRITER_PACK)
	var map: MapCraftMap = _build(pack)
	var texture: ImageTexture = _packed_texture(map, "ramps-orange1")
	_expect(texture != null and texture.get_size() == Vector2(16, 16), \
			"a packed picture becomes the map's material")
	_expect(texture != null and texture.resource_path.is_empty(), \
			"a packed picture is not registered under any path")
	_expect(_stock_checker(map), "materials the pack lacks still come from the game")
	var mesh: MeshInstance3D = _crate_mesh(map)
	_expect(mesh != null, "a packed model is built from the pack")
	_expect(mesh != null and mesh.mesh.resource_path.is_empty(), \
			"a packed model is not registered under any path")
	_expect(not _messages.has("mcpck"), "a good pack logs nothing")
	map.free()

	_save(DIR + "loaded.mcmap", FileAccess.get_file_as_bytes(DIR + "course.mcmap"))
	_save(DIR + "loaded.mcpck", pack)
	var scene: PackedScene = load(DIR + "loaded.mcmap") as PackedScene
	var root: Node = scene.instantiate() if scene != null else null
	var world: MeshInstance3D = root.get_node_or_null("WorldMesh") as MeshInstance3D \
			if root != null else null
	var material: StandardMaterial3D = _surface_material(world, "ramps-orange1") \
			as StandardMaterial3D
	_expect(material != null and material.albedo_texture is ImageTexture, \
			"load() of a map uses the pack beside it")
	_expect(root != null and _crate_mesh(root) != null, "load() of a map builds packed models")
	if root != null:
		root.free()

	_save(DIR + "legacy.mcmap", FileAccess.get_file_as_bytes(DIR + "course.mcmap"))
	_save(DIR + "legacy.mcpack", pack)
	var legacy: PackedScene = load(DIR + "legacy.mcmap") as PackedScene
	var legacy_root: Node = legacy.instantiate() if legacy != null else null
	_expect(legacy_root != null and _crate_mesh(legacy_root) != null, \
			"a map without a .mcpck uses an old .mcpack beside it")
	if legacy_root != null:
		legacy_root.free()


func _check_containers() -> void:
	var good: PackedByteArray = _pack([[GOOD_MATERIAL, _picture], [BAD_MATERIAL, _picture]])
	var first_offset: int = 12 + 2 + GOOD_MATERIAL.length()
	var second_offset: int = first_offset + 16 + 2 + BAD_MATERIAL.length()
	var cuts: Dictionary[int, String] = {
		0: "not a MapCraft pack",
		3: "not a MapCraft pack",
		11: "cut-short table of contents",
		13: "cut-short table of contents",
		60: "cut-short table of contents",
		good.size() - 1: "runs past the end",
	}
	for cut: int in cuts:
		_expect_refused("a pack cut to %d bytes" % cut, good.slice(0, cut), cuts[cut])
	var longer: PackedByteArray = good.duplicate()
	longer.append(0)
	_expect_refused("a pack with trailing data", longer, "data after its last entry")
	_expect_refused("a zip file", "PK\u0003\u0004 not a pack".to_utf8_buffer(), \
			"not a MapCraft pack")
	_expect_refused("a newer pack", _patched32(good, 4, 4), "version 4")
	_expect_refused("a huge entry count", _patched32(good, 8, 0xFFFFFFFF), "4294967295 entries")
	_expect_refused("overlapping entries", _patched64(good, second_offset, \
			good.decode_u64(first_offset)), "does not start where")
	_expect_refused("a gap between entries", _patched64(good, second_offset, \
			good.decode_u64(second_offset) + 1), "does not start where")
	_expect_refused("an offset that overflows", _patched64(good, second_offset, -5), \
			"does not start where")
	_expect_refused("a length that overflows", _patched64(good, second_offset + 8, -1), \
			"empty or larger than 64 MiB")
	_expect_refused("a length past the end", _patched64(good, second_offset + 8, \
			_picture.size() + 1), "runs past the end")
	_expect_refused("an empty entry", _patched64(good, first_offset + 8, 0), \
			"empty or larger than 64 MiB")


func _check_names() -> void:
	var hostile: Array[Variant] = [
		"../materials/a.png",
		"materials/../a.png",
		"materials/./a.png",
		"materials//a.png",
		"/materials/a.png",
		"materials/a.png/",
		"materials\\a.png",
		"C:/materials/a.png",
		"materials/C:a.png",
		"materials/a b.png",
		"materials/é.png",
		"materials/.png",
		"materials/a.PNG",
		"models/a.png",
		"materials/a.glb",
		"a.png",
		"",
		"materials/a".to_utf8_buffer() + PackedByteArray([0]) + ".png".to_utf8_buffer(),
	]
	for name: Variant in hostile:
		_expect_refused("the name %s" % str(name).c_escape(), \
				_pack([[GOOD_MATERIAL, _picture], [name, _picture]]), "entry 1:")
	for ext: String in ["tres", "res", "scn", "tscn", "pck", "gd", "gdshader", "svg", "so", \
			"dll", "gltf", "webp"]:
		_expect_refused("a .%s entry" % ext, \
				_pack([[GOOD_MATERIAL, _picture], ["models/a." + ext, _picture]]), \
				"only .png, .jpg and .glb")
	_expect_refused("a 256 byte name", _pack([[GOOD_MATERIAL, _picture], \
			["materials/" + "a".repeat(242) + ".png", _picture]]), "1 to 255 bytes")
	_expect_refused("the same name twice", _pack([[GOOD_MATERIAL, _picture], \
			[GOOD_MATERIAL, _picture]]), "twice")
	_expect_refused("names differing in case and extension", _pack([[GOOD_MATERIAL, _picture], \
			["materials/RAMPS-Orange1.jpg", _jpg(8, 4)]]), "twice")


func _check_pictures() -> void:
	var box: Array = _box(_png(4, 4))
	_expect_dropped("a PNG named .jpg", BAD_JPEG, _picture, "different kind")
	_expect_dropped("a JPEG named .png", BAD_MATERIAL, _jpg(8, 4), "different kind")
	_expect_dropped("a model named .png", BAD_MATERIAL, _glb(box[0], box[1]), \
			"neither a PNG nor a JPEG")
	_expect_dropped("an SVG named .png", BAD_MATERIAL, \
			"<svg xmlns='http://www.w3.org/2000/svg'/>".to_utf8_buffer(), "neither a PNG")
	_expect_dropped("a picture named .glb", BAD_MODEL, _picture, "not a GLB model")
	_expect_dropped("a decodable PNG over the cap", BAD_MATERIAL, _png(4097, 1), "4097x1")
	_expect_dropped("a decodable JPEG over the cap", BAD_JPEG, _jpg(4097, 1), "4097x1")
	_expect_dropped("a PNG claiming 100000x100000", BAD_MATERIAL, \
			_sized_png(100000, 100000), "100000x100000")
	var jpeg: PackedByteArray = _jpg(8, 4)
	var frame: int = _frame_at(jpeg)
	_put_be16(jpeg, frame + 5, 65535)
	_put_be16(jpeg, frame + 7, 65535)
	_expect_dropped("a JPEG claiming 65535x65535", BAD_JPEG, jpeg, "65535x65535")
	_expect_dropped("a JPEG with 65 scans", BAD_JPEG, _fake_jpeg(0xC2, 65), "over 64 scans")
	_expect_dropped("an arithmetic JPEG", BAD_JPEG, _fake_jpeg(0xC9, 1), "cannot decode")
	var truncated: PackedByteArray = _picture.slice(0, 20)
	_expect_dropped("a cut-short PNG", BAD_MATERIAL, truncated, "cut-short PNG")


# Four 4096x4096 pictures fill the budget, so the fifth draws as missing.
func _check_pixel_budget() -> void:
	var big: PackedByteArray = _png(4096, 4096)
	var entries: Array[Array] = []
	for material_name: String in ["ramps-orange1", "checker", "glow", "nearest", "nearest_mip"]:
		entries.append(["materials/%s.png" % material_name, big])
	var map: MapCraftMap = _build(_pack(entries))
	var world: MeshInstance3D = map.get_node_or_null("WorldMesh") as MeshInstance3D
	var textures: Dictionary[Texture2D, bool] = {}
	for i: int in world.mesh.get_surface_count():
		var material: Material = world.mesh.surface_get_material(i)
		var texture: ImageTexture = null
		if material is StandardMaterial3D:
			texture = (material as StandardMaterial3D).albedo_texture as ImageTexture
		elif material is ShaderMaterial:
			texture = (material as ShaderMaterial).get_shader_parameter(&"albedo_texture_a") \
					as ImageTexture
		if texture != null and texture.get_width() == 4096:
			textures[texture] = true
	_expect(textures.size() == 4, "the pixel budget lets four 4096x4096 pictures through")
	_expect(_messages.has("past 64 million pixels"), "the fifth picture is refused")
	map.free()


func _check_models() -> void:
	var box: Array = _box(_png(4, 4))
	var map: MapCraftMap = _build(_pack([[GOOD_MATERIAL, _picture], \
			[BAD_MODEL, _glb(box[0], box[1])]]))
	var mesh: MeshInstance3D = _crate_mesh(map)
	var material: StandardMaterial3D = null
	if mesh != null:
		material = mesh.mesh.surface_get_material(0) as StandardMaterial3D
	_expect(material != null and material.albedo_texture != null, \
			"a packed model keeps its embedded picture")
	_expect(material != null and material.albedo_texture.resource_path.is_empty(), \
			"a packed model's picture is not registered under any path")
	map.free()

	for uri: String in ["file:///etc/passwd", "/etc/passwd", "res://project.godot", \
			"user://pack_test/course.mcmap", "../course.mcmap", \
			"data:application/octet-stream;base64,AAAA"]:
		_expect_bad_model("a buffer at " + uri, ["buffers", 0, "uri"], uri, \
				"buffer outside the file")
		_expect_bad_model("a picture at " + uri, ["images", 0, "uri"], uri, \
				"picture outside the file")
	var nested: Array = []
	for i: int in 100:
		nested = [nested]
	var edits: Array[Array] = [
		["an unknown required extension", ["extensionsRequired"], ["KHR_draco_mesh_compression"], \
				"KHR_draco_mesh_compression"],
		["a node extension", ["nodes", 0, "extensions"], \
				{"KHR_node_visibility": {"visible": false}}, "KHR_node_visibility"],
		["lights", ["extensions"], {"KHR_lights_punctual": {"lights": []}}, "KHR_lights_punctual"],
		["animations", ["animations"], [{"channels": [], "samplers": []}], "has animations"],
		["cameras", ["cameras"], [{"type": "perspective"}], "has cameras"],
		["skins", ["skins"], [{"joints": [0]}], "has skins"],
		["morph targets", ["meshes", 0, "primitives", 0, "targets"], [{"POSITION": 0}], \
				"morph targets"],
		["a sparse accessor", ["accessors", 0, "sparse"], {"count": 1 << 30}, "sparse accessor"],
		["an accessor past its view", ["accessors", 0, "count"], 1000, "reads past"],
		["an accessor of 2^30 elements", ["accessors", 0, "count"], 1 << 30, "too many elements"],
		["a buffer longer than the file", ["buffers", 0, "byteLength"], 1 << 30, \
				"longer than its BIN chunk"],
		["a view far past the buffer", ["bufferViews", 0, "byteOffset"], 4294967295.0, \
				"outside its buffer"],
		["a node loop", ["nodes"], [{"mesh": 0, "children": [1]}, {"children": [0]}], "in a loop"],
		["a node that is its own child", ["nodes", 0, "children"], [0], "in a loop"],
		["a webp picture", ["images", 0, "mimeType"], "image/webp", "image/png or image/jpeg"],
		["deep JSON", ["extras"], nested, "nests its JSON"],
	]
	for edit: Array in edits:
		_expect_bad_model(edit[0], edit[1], edit[2], edit[3])
	var huge: Array = _box(_sized_png(5000, 5000))
	_expect_dropped("a model with a 5000x5000 picture", BAD_MODEL, _glb(huge[0], huge[1]), \
			"picture that is 5000x5000")
	var glb: PackedByteArray = _glb(box[0], box[1])
	_expect_dropped("a GLB claiming a 4 GiB JSON chunk", BAD_MODEL, \
			_patched32(glb, 12, 0xFFFFFFF0), "JSON chunk over 2 MiB")
	_expect_dropped("a GLB with the wrong length", BAD_MODEL, \
			_patched32(glb, 8, glb.size() + 1), "wrong length")


# Sets one value in the textured box's JSON, at a path of keys and indices.
func _check_map_packs() -> void:
	var box: Array = _box(_png(4, 4))
	var entries: Array = [
		[GOOD_MATERIAL, _picture],
		["map.mcmap", FileAccess.get_file_as_bytes(DIR + "course.mcmap")],
		[BAD_MODEL, _glb(box[0], box[1])],
	]
	_save(DIR + "own.mcpck", _patched32(_pack(entries), 4, 3))
	var scene: PackedScene = load(DIR + "own.mcpck") as PackedScene
	var root: Node = scene.instantiate() if scene != null else null
	var world: Node = root.get_node_or_null("WorldMesh") if root != null else null
	var material: StandardMaterial3D = _surface_material(world, "ramps-orange1") \
			as StandardMaterial3D
	_expect(material != null and material.albedo_texture is ImageTexture, \
			"load() of a .mcpck builds its map with its own pictures")
	_expect(root != null and _crate_mesh(root) != null, "load() of a .mcpck builds its models")
	if root != null:
		root.free()

	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = DIR + "own.mcpck"
	map.entity_map = load(ENTITY_MAP) as MapCraftEntityMap
	map.build()
	_expect(_packed_texture(map, "ramps-orange1") != null, \
			"a MapCraftMap node builds a .mcpck's map")
	map.free()

	_messages.clear()
	_save(DIR + "older.mcpck", _patched32(_pack(entries), 4, 2))
	map = MapCraftMap.new()
	map.map_file = DIR + "older.mcpck"
	map.build()
	_expect(map.get_child_count() == 0 and _messages.has("cannot contain a map"), \
			"a version 2 pack cannot carry a map")
	map.free()

	_messages.clear()
	_save(DIR + "mapless.mcpck", _pack([[GOOD_MATERIAL, _picture]]))
	map = MapCraftMap.new()
	map.map_file = DIR + "mapless.mcpck"
	map.build()
	_expect(map.get_child_count() == 0 and _messages.has("holds no map"), \
			"a pack without a map builds nothing")
	map.free()


func _expect_bad_model(label: String, path: Array, value: Variant, reason: String) -> void:
	var box: Array = _box(_png(4, 4))
	var target: Variant = box[0]
	for key: Variant in path.slice(0, -1):
		target = target[key]
	target[path[-1]] = value
	_expect_dropped(label, BAD_MODEL, _glb(box[0], box[1]), reason)


# The whole pack is refused, so stock materials stay and nothing comes from it.
func _expect_refused(label: String, pack: PackedByteArray, reason: String) -> void:
	var map: MapCraftMap = _build(pack)
	_expect(_packed_texture(map, "ramps-orange1") == null and _stock_checker(map), \
			"%s: the pack is refused" % label)
	_expect(_messages.has(reason), "%s: the log says %s" % [label, reason])
	map.free()


# One entry draws as missing while the rest of the pack is used.
func _expect_dropped(label: String, entry: String, data: PackedByteArray, reason: String) -> void:
	var map: MapCraftMap = _build(_pack([[GOOD_MATERIAL, _picture], [entry, data]]))
	_expect(_packed_texture(map, "ramps-orange1") != null, \
			"%s: the rest of the pack is used" % label)
	if entry.begins_with("models/"):
		_expect(_crate_mesh(map) == null, "%s: the model is left out" % label)
	else:
		_expect(_surface_material(map.get_node_or_null("WorldMesh"), "checker") == null, \
				"%s: the picture draws as missing" % label)
	_expect(_messages.has(reason), "%s: the log says %s" % [label, reason])
	map.free()


func _build(pack: PackedByteArray) -> MapCraftMap:
	_save(DIR + "course.mcpck", pack)
	_messages.clear()
	var map: MapCraftMap = MapCraftMap.new()
	map.map_file = DIR + "course.mcmap"
	map.entity_map = load(ENTITY_MAP) as MapCraftEntityMap
	map.build()
	return map


# Lays a pack out by hand, so a test can break it in ways the MapCraft writer never would.
func _pack(entries: Array) -> PackedByteArray:
	var offset: int = 12
	for entry: Array in entries:
		offset += 18 + _name_bytes(entry[0]).size()
	var out: StreamPeerBuffer = StreamPeerBuffer.new()
	out.put_data("MCPK".to_ascii_buffer())
	out.put_u32(1)
	out.put_u32(entries.size())
	for entry: Array in entries:
		var name: PackedByteArray = _name_bytes(entry[0])
		var data: PackedByteArray = entry[1]
		out.put_u16(name.size())
		out.put_data(name)
		out.put_u64(offset)
		out.put_u64(data.size())
		offset += data.size()
	for entry: Array in entries:
		out.put_data(entry[1])
	return out.data_array


func _name_bytes(name: Variant) -> PackedByteArray:
	return name if name is PackedByteArray else (name as String).to_utf8_buffer()


# Box with a picture in its BIN chunk, as [json, bin].
func _box(picture: PackedByteArray) -> Array:
	var bytes: PackedByteArray = FileAccess.get_file_as_bytes(BOX)
	var json_len: int = bytes.decode_u32(12)
	var json: Dictionary = JSON.parse_string(bytes.slice(20, 20 + json_len).get_string_from_utf8())
	var bin: PackedByteArray = bytes.slice(28 + json_len)
	var views: Array = json["bufferViews"]
	views.append({"buffer": 0, "byteOffset": bin.size(), "byteLength": picture.size()})
	bin.append_array(picture)
	json["buffers"][0]["byteLength"] = bin.size()
	json["images"] = [{"bufferView": views.size() - 1, "mimeType": "image/png"}]
	json["textures"] = [{"source": 0}]
	json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = {"index": 0}
	return [json, bin]


func _glb(json: Dictionary, bin: PackedByteArray) -> PackedByteArray:
	var text: PackedByteArray = JSON.stringify(json).to_utf8_buffer()
	var out: StreamPeerBuffer = StreamPeerBuffer.new()
	out.put_data("glTF".to_ascii_buffer())
	out.put_u32(2)
	out.put_u32(28 + text.size() + bin.size())
	out.put_u32(text.size())
	out.put_u32(GLB_JSON)
	out.put_data(text)
	out.put_u32(bin.size())
	out.put_u32(GLB_BIN)
	out.put_data(bin)
	return out.data_array


func _png(width: int, height: int) -> PackedByteArray:
	var image: Image = Image.create_empty(width, height, false, Image.FORMAT_RGB8)
	image.fill(Color(0.9, 0.5, 0.1))
	return image.save_png_to_buffer()


func _jpg(width: int, height: int) -> PackedByteArray:
	var image: Image = Image.create_empty(width, height, false, Image.FORMAT_RGB8)
	image.fill(Color(0.2, 0.5, 0.9))
	return image.save_jpg_to_buffer()


# A small PNG whose header claims another size, the shape of a decompression bomb.
func _sized_png(width: int, height: int) -> PackedByteArray:
	var bytes: PackedByteArray = _png(4, 4)
	_put_be16(bytes, 16, width >> 16)
	_put_be16(bytes, 18, width & 0xFFFF)
	_put_be16(bytes, 20, height >> 16)
	_put_be16(bytes, 22, height & 0xFFFF)
	return bytes


# Headers only: enough for the marker walk, never meant to decode.
func _fake_jpeg(frame: int, scans: int) -> PackedByteArray:
	var out: PackedByteArray = PackedByteArray([0xFF, 0xD8, 0xFF, frame, 0, 11, 8, 0, 8, 0, 8, 1, \
			1, 0x11, 0])
	for i: int in scans:
		out.append_array(PackedByteArray([0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0, 0x12, 0xFF, 0, 6]))
	out.append_array(PackedByteArray([0xFF, 0xD9]))
	return out


func _frame_at(jpeg: PackedByteArray) -> int:
	for i: int in jpeg.size() - 1:
		if jpeg[i] == 0xFF and (jpeg[i + 1] == 0xC0 or jpeg[i + 1] == 0xC2):
			return i
	return 0


func _put_be16(bytes: PackedByteArray, at: int, value: int) -> void:
	bytes[at] = (value >> 8) & 0xFF
	bytes[at + 1] = value & 0xFF


func _patched32(bytes: PackedByteArray, at: int, value: int) -> PackedByteArray:
	var out: PackedByteArray = bytes.duplicate()
	out.encode_u32(at, value)
	return out


func _patched64(bytes: PackedByteArray, at: int, value: int) -> PackedByteArray:
	var out: PackedByteArray = bytes.duplicate()
	out.encode_u64(at, value)
	return out


func _save(path: String, bytes: PackedByteArray) -> void:
	var file: FileAccess = FileAccess.open(path, FileAccess.WRITE)
	file.store_buffer(bytes)
	file.close()


func _packed_texture(map: MapCraftMap, surface_name: String) -> ImageTexture:
	var material: StandardMaterial3D = _surface_material(map.get_node_or_null("WorldMesh"), \
			surface_name) as StandardMaterial3D
	return material.albedo_texture as ImageTexture if material != null else null


func _stock_checker(map: MapCraftMap) -> bool:
	var material: Material = _surface_material(map.get_node_or_null("WorldMesh"), "checker")
	return material != null and material.resource_path == "res://Assets/Textures/checker.tres"


func _crate_mesh(map: Node) -> MeshInstance3D:
	var crate: Node = map.get_node_or_null("crate")
	if crate == null:
		return null
	var meshes: Array[Node] = crate.find_children("*", "MeshInstance3D", true, false)
	return meshes[0] as MeshInstance3D if not meshes.is_empty() else null


func _surface_material(world: Node, surface_name: String) -> Material:
	var mesh: ArrayMesh = (world as MeshInstance3D).mesh as ArrayMesh \
			if world is MeshInstance3D else null
	if mesh == null:
		return null
	for i: int in mesh.get_surface_count():
		if mesh.surface_get_name(i) == surface_name:
			return mesh.surface_get_material(i)
	return null


func _expect(condition: bool, what: String) -> void:
	if not condition:
		_failures.append(what)


class Messages extends Logger:
	var _lines: PackedStringArray = PackedStringArray()
	var _mutex: Mutex = Mutex.new()


	func _log_error(_function: String, _file: String, _line: int, code: String, \
			rationale: String, _editor_notify: bool, _error_type: int, \
			_script_backtraces: Array[ScriptBacktrace]) -> void:
		_mutex.lock()
		_lines.append(code + " " + rationale)
		_mutex.unlock()


	func clear() -> void:
		_mutex.lock()
		_lines.clear()
		_mutex.unlock()


	func has(text: String) -> bool:
		_mutex.lock()
		var found: bool = false
		for line: String in _lines:
			found = found or line.contains(text)
		_mutex.unlock()
		return found
