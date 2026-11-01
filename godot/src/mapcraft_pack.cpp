#include "mapcraft_pack.h"

#include <godot_cpp/classes/file_access.hpp>
#include <godot_cpp/classes/json.hpp>
#include <godot_cpp/core/math.hpp>
#include <godot_cpp/templates/vector.hpp>
#include <godot_cpp/variant/array.hpp>
#include <godot_cpp/variant/dictionary.hpp>

#include <cstring>
#include <vector>

namespace godot {

namespace {

// The limits; crates/format/src/pack.rs repeats them and docs/mcpck-format.md lists them.
const int64_t MAX_PACK_SIZE = int64_t(256) << 20;
const uint32_t MAX_ENTRIES = 1024;
const int64_t MAX_NAME_LEN = 255;
const uint64_t MAX_ENTRY_SIZE = uint64_t(64) << 20;
const int64_t MAX_IMAGE_SIZE = 4096;
const int64_t MAX_TOTAL_PIXELS = int64_t(64) << 20;
const int64_t MAX_JPEG_SCANS = 64;
const int64_t MAX_GLB_JSON = int64_t(2) << 20;
const int MAX_JSON_DEPTH = 64;
const int64_t MAX_GLTF_NODES = 4096;
const int64_t MAX_GLTF_NODE_DEPTH = 64;
const int64_t MAX_GLTF_MESHES = 1024;
const int64_t MAX_GLTF_PRIMITIVES = 4096;
const int64_t MAX_GLTF_ACCESSORS = 16384;
const int64_t MAX_GLTF_BUFFER_VIEWS = 16384;
const int64_t MAX_GLTF_MATERIALS = 1024;
const int64_t MAX_GLTF_TEXTURES = 1024;
const int64_t MAX_GLTF_IMAGES = 256;
const int64_t MAX_ACCESSOR_ELEMENTS = int64_t(1) << 22;
const int64_t MAX_MESH_COMPONENTS = int64_t(1) << 24;
const int64_t MAX_GLTF_ANIMATIONS = 256;
const int64_t MAX_GLTF_CHANNELS = 16384;
const int64_t MAX_GLTF_SKINS = 256;
const int64_t MAX_ANIMATION_COMPONENTS = int64_t(1) << 24;
const char *GLTF_EXTENSIONS[] = { "KHR_materials_emissive_strength", "KHR_materials_unlit", "KHR_mesh_quantization", "KHR_texture_transform" };

const uint32_t VERSION = 3;
const char *MAP_ENTRY = "map.mcmap";
const int64_t HEADER_LEN = 12;
const uint8_t PNG_SIGNATURE[8] = { 0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A };
const uint8_t PNG_IHDR[8] = { 0, 0, 0, 13, 'I', 'H', 'D', 'R' };
const uint32_t GLB_JSON = 0x4E4F534A;
const uint32_t GLB_BIN = 0x004E4942;
// Large enough for any index and small enough that a double holds it exactly.
const int64_t ANY = INT32_MAX;

enum class Kind {
	NONE,
	PNG,
	JPEG,
	GLB,
	MATERIAL,
	MAP,
};

uint32_t le16(const uint8_t *p) {
	return uint32_t(p[0]) | uint32_t(p[1]) << 8;
}

uint32_t le32(const uint8_t *p) {
	return le16(p) | le16(p + 2) << 16;
}

uint64_t le64(const uint8_t *p) {
	return uint64_t(le32(p)) | uint64_t(le32(p + 4)) << 32;
}

int64_t be16(const uint8_t *p) {
	return int64_t(p[0]) << 8 | p[1];
}

int64_t be32(const uint8_t *p) {
	return be16(p) << 16 | be16(p + 2);
}

Kind kind_of(const uint8_t *p_name, int64_t p_len, String &r_error) {
	if (p_len < 1 || p_len > MAX_NAME_LEN) {
		r_error = "names are 1 to 255 bytes";
		return Kind::NONE;
	}
	int64_t segment = 0;
	for (int64_t i = 0; i <= p_len; i++) {
		if (i == p_len || p_name[i] == '/') {
			const int64_t n = i - segment;
			const bool dots = n > 0 && p_name[segment] == '.' && (n == 1 || (n == 2 && p_name[segment + 1] == '.'));
			if (n == 0 || dots) {
				r_error = "names cannot have empty, . or .. path segments";
				return Kind::NONE;
			}
			segment = i + 1;
			continue;
		}
		const uint8_t c = p_name[i];
		if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-' || c == '.' || c == ' ')) {
			r_error = "a character in it is not allowed in a name";
			return Kind::NONE;
		}
	}
	const String name = String::utf8(reinterpret_cast<const char *>(p_name), p_len);
	if (name == MAP_ENTRY) {
		return Kind::MAP;
	}
	Kind kind = Kind::GLB;
	String folder = "models/";
	if (name.ends_with(".png") || name.ends_with(".jpg")) {
		kind = name.ends_with(".png") ? Kind::PNG : Kind::JPEG;
		folder = "materials/";
	} else if (name.ends_with(".json")) {
		kind = Kind::MATERIAL;
		folder = "materials/";
	} else if (!name.ends_with(".glb")) {
		r_error = "only .png, .jpg and .glb entries are allowed";
		return Kind::NONE;
	}
	const String stem = name.get_basename();
	if ((kind == Kind::PNG || kind == Kind::JPEG) && name.begins_with("textures/")) { folder = "textures/"; }
	if (!stem.begins_with(folder) || stem.length() == folder.length() || stem.ends_with("/")) {
		r_error = "this kind of entry belongs under " + folder;
		return Kind::NONE;
	}
	return kind;
}

String check_material(const PackedByteArray &bytes) {
  if (bytes.size() > 16384) {
    return "material descriptor exceeds 16 KiB";
  }
  Ref<JSON> json;
  json.instantiate();
  if (json->parse(bytes.get_string_from_utf8()) != OK ||
      json->get_data().get_type() != Variant::DICTIONARY) {
    return "invalid material descriptor";
  }
  const Dictionary d = json->get_data();
  if (!d.has("version") || !d.has("albedo") || !d.has("roughness") ||
      !d.has("metallic") || !d.has("emission") || !d.has("textures")) {
    return "invalid material fields";
  }
  const Variant version = d["version"];
  if ((version.get_type() != Variant::FLOAT &&
       version.get_type() != Variant::INT) ||
      (double(version) != 1.0 && double(version) != 2.0)) {
    return "unsupported material version";
  }
  const bool v2 = double(version) == 2.0;
  const Array fields = d.keys();
  for (int i = 0; i < fields.size(); i++) {
    const String field = fields[i];
    const bool v1_field = field == "version" || field == "albedo" ||
                          field == "roughness" || field == "metallic" ||
                          field == "emission" || field == "textures";
    const bool v2_field = field == "emission_energy" ||
                          field == "normal_strength" ||
                          field == "height_scale" || field == "height_flip";
    if (!v1_field && !(v2 && v2_field)) {
      return "invalid material fields";
    }
  }
  struct Range {
    const char *field;
    double low;
    double high;
  };
  for (const Range &range : {Range{"emission_energy", 0.0, 64.0},
                             Range{"normal_strength", 0.0, 16.0},
                             Range{"height_scale", -16.0, 16.0}}) {
    if (!d.has(range.field)) {
      continue;
    }
    const Variant v = d[range.field];
    if (v.get_type() != Variant::FLOAT && v.get_type() != Variant::INT) {
      return "invalid material scalar";
    }
    const double number = v;
    if (!Math::is_finite(number) || number < range.low || number > range.high) {
      return "material scalar outside limits";
    }
  }
  if (d.has("height_flip") && d["height_flip"].get_type() != Variant::BOOL) {
    return "height_flip must be true or false";
  }
  for (const char *field : {"albedo", "emission"}) {
    const Variant v = d[field];
    if (v.get_type() != Variant::ARRAY) {
      return "material color must be an array";
    }
    const Array color = v;
    if (color.size() != (String(field) == "albedo" ? 4 : 3)) {
      return "invalid material color length";
    }
    for (int i = 0; i < color.size(); i++) {
      const Variant component = color[i];
      if (component.get_type() != Variant::FLOAT &&
          component.get_type() != Variant::INT) {
        return "invalid material color component";
      }
      const double number = component;
      if (!Math::is_finite(number) || number < 0 ||
          number > (String(field) == "albedo" ? 1.0 : 64.0)) {
        return "material color outside limits";
      }
    }
  }
  for (const char *field : {"roughness", "metallic"}) {
    const Variant v = d[field];
    if (v.get_type() != Variant::FLOAT && v.get_type() != Variant::INT) {
      return "invalid material scalar";
    }
    const double number = v;
    if (!Math::is_finite(number) || number < 0 || number > 1) {
      return "material scalar outside limits";
    }
  }
  const Variant raw = d["textures"];
  if (raw.get_type() != Variant::DICTIONARY) {
    return "material textures must be an object";
  }
  const Dictionary textures = raw;
  const Array keys = textures.keys();
  for (int i = 0; i < keys.size(); i++) {
    const String key = keys[i];
    if (key != "albedo" && key != "normal" && key != "roughness" &&
        key != "metallic" && key != "emission" && !(v2 && key == "height")) {
      return "unsupported texture slot";
    }
    const Variant value = textures[key];
    if (value.get_type() != Variant::STRING) {
      return "texture name must be text";
    }
    const String name = value;
    if (!name.begins_with("textures/")) {
      return "material texture must be under textures/";
    }
    const CharString utf8 = name.utf8();
    String problem;
    const Kind kind =
        kind_of(reinterpret_cast<const uint8_t *>(utf8.get_data()),
                utf8.length(), problem);
    if (kind != Kind::PNG && kind != Kind::JPEG) {
      return "invalid material texture name";
    }
  }
  return String();
}

// Walks a JPEG's markers to its frame size, counting scans, since each progressive scan makes the decoder pass over the whole picture.
String jpeg_size(const uint8_t *p, int64_t n, int64_t &r_width, int64_t &r_height) {
	bool sized = false;
	int64_t scans = 0;
	int64_t at = 2;
	while (at < n) {
		if (p[at] != 0xFF) {
			return "is a JPEG with a broken marker";
		}
		while (at < n && p[at] == 0xFF) {
			at++;
		}
		if (at >= n) {
			break;
		}
		const uint8_t marker = p[at++];
		if (marker == 0xD9) {
			break;
		}
		if (marker == 0x00 || marker == 0x01 || (marker >= 0xD0 && marker <= 0xD8)) {
			return "is a JPEG with a misplaced marker";
		}
		if (marker == 0xC3 || (marker >= 0xC5 && marker <= 0xCB) || (marker >= 0xCD && marker <= 0xCF)) {
			return "is a JPEG of a kind the game cannot decode";
		}
		const int64_t len = n - at >= 2 ? be16(p + at) : 0;
		if (len < 2 || len > n - at) {
			return "is a cut-short JPEG";
		}
		if (marker >= 0xC0 && marker <= 0xC2) {
			if (sized) {
				return "is a JPEG with more than one frame";
			}
			if (len < 8) {
				return "is a cut-short JPEG";
			}
			r_height = be16(p + at + 3);
			r_width = be16(p + at + 5);
			sized = true;
		}
		at += len;
		if (marker == 0xDA) {
			if (!sized) {
				return "is a JPEG that starts a scan before its frame";
			}
			if (++scans > MAX_JPEG_SCANS) {
				return "is a JPEG with over " + itos(MAX_JPEG_SCANS) + " scans";
			}
			// Scan data ends at the first 0xFF that is not a stuffed zero or a restart marker.
			while (at < n) {
				const bool escaped = at + 1 < n && (p[at + 1] == 0x00 || (p[at + 1] >= 0xD0 && p[at + 1] <= 0xD7));
				if (p[at] == 0xFF && !escaped) {
					break;
				}
				at += p[at] == 0xFF ? 2 : 1;
			}
		}
	}
	return sized ? String() : String("is a JPEG without a frame header");
}

// Checks a picture's header against the caps before anything decodes it, and charges its pixels.
String check_image(const uint8_t *p, int64_t n, Kind p_expected, int64_t &r_pixels) {
	Kind kind = Kind::NONE;
	int64_t width = 0;
	int64_t height = 0;
	if (n >= 8 && memcmp(p, PNG_SIGNATURE, 8) == 0) {
		if (n < 16 || memcmp(p + 8, PNG_IHDR, 8) != 0) {
			return "is a PNG without its header";
		}
		if (n < 24) {
			return "is a cut-short PNG";
		}
		kind = Kind::PNG;
		width = be32(p + 16);
		height = be32(p + 20);
	} else if (n >= 2 && p[0] == 0xFF && p[1] == 0xD8) {
		kind = Kind::JPEG;
		const String error = jpeg_size(p, n, width, height);
		if (!error.is_empty()) {
			return error;
		}
	} else {
		return "is neither a PNG nor a JPEG";
	}
	if (kind != p_expected) {
		return "holds a different kind of picture than its name says";
	}
	if (width < 1 || height < 1 || width > MAX_IMAGE_SIZE || height > MAX_IMAGE_SIZE) {
		return "is " + itos(width) + "x" + itos(height) + ", pictures are 1x1 to 4096x4096";
	}
	r_pixels += width * height;
	if (r_pixels > MAX_TOTAL_PIXELS) {
		return "takes the map's pictures past 64 million pixels";
	}
	return String();
}

// A JSON number that is a whole number from 0 to p_max; Godot reads every JSON number as a float.
bool whole(const Variant &p_value, int64_t p_max, int64_t &r_out) {
	if (p_value.get_type() == Variant::INT) {
		const int64_t v = p_value;
		r_out = v;
		return v >= 0 && v <= p_max;
	}
	if (p_value.get_type() != Variant::FLOAT) {
		return false;
	}
	const double v = p_value;
	if (!(v >= 0.0 && v <= double(p_max)) || v != Math::floor(v)) {
		return false;
	}
	r_out = int64_t(v);
	return true;
}

bool optional(const Dictionary &p_object, const char *p_key, int64_t p_default, int64_t p_max, int64_t &r_out) {
	if (!p_object.has(p_key)) {
		r_out = p_default;
		return true;
	}
	return whole(p_object[p_key], p_max, r_out);
}

bool list(const Dictionary &p_object, const char *p_key, int64_t p_max, Array &r_items) {
	r_items = Array();
	if (!p_object.has(p_key)) {
		return true;
	}
	const Variant value = p_object[p_key];
	if (value.get_type() != Variant::ARRAY) {
		return false;
	}
	r_items = value;
	return r_items.size() <= p_max;
}

String not_a_list(const char *p_key, int64_t p_max) {
	return String("has ") + p_key + " that is not a list of at most " + itos(p_max);
}

bool allowed_extension(const Variant &p_name) {
	if (p_name.get_type() != Variant::STRING) {
		return false;
	}
	const String name = p_name;
	for (const char *known : GLTF_EXTENSIONS) {
		if (name == known) {
			return true;
		}
	}
	return false;
}

// Every extensions object anywhere may only name allowed extensions, since Godot acts on some without checking extensionsUsed.
String check_extensions(const Variant &p_value, int p_depth) {
	if (p_depth > MAX_JSON_DEPTH) {
		return "nests its JSON over " + itos(MAX_JSON_DEPTH) + " deep";
	}
	if (p_value.get_type() == Variant::ARRAY) {
		const Array items = p_value;
		for (int64_t i = 0; i < items.size(); i++) {
			const String error = check_extensions(items[i], p_depth + 1);
			if (!error.is_empty()) {
				return error;
			}
		}
	} else if (p_value.get_type() == Variant::DICTIONARY) {
		const Dictionary fields = p_value;
		const Variant used = fields.get("extensions", Variant());
		if (used.get_type() == Variant::DICTIONARY) {
			const Array names = Dictionary(used).keys();
			for (int64_t i = 0; i < names.size(); i++) {
				if (!allowed_extension(names[i])) {
					return "uses the glTF extension \"" + String(names[i]).c_escape() + "\", which packed models cannot";
				}
			}
		} else if (used.get_type() != Variant::NIL) {
			return "has extensions that are not an object";
		}
		const Array values = fields.values();
		for (int64_t i = 0; i < values.size(); i++) {
			const String error = check_extensions(values[i], p_depth + 1);
			if (!error.is_empty()) {
				return error;
			}
		}
	}
	return String();
}

struct View {
	int64_t offset = 0;
	int64_t length = 0;
	int64_t stride = 0;
};

// Mirrors gltf() in crates/format/src/pack.rs: everything in a model's JSON that could reach outside the file or run away with memory.
String check_gltf(const Variant &p_doc, const uint8_t *p_bin, int64_t p_bin_len, int64_t &r_pixels) {
	if (p_doc.get_type() != Variant::DICTIONARY) {
		return "has JSON that is not an object";
	}
	const Dictionary root = p_doc;
	String error = check_extensions(p_doc, 0);
	if (!error.is_empty()) {
		return error;
	}
	Array items;
	for (const char *key : { "extensionsUsed", "extensionsRequired" }) {
		if (!list(root, key, ANY, items)) {
			return not_a_list(key, ANY);
		}
		for (int64_t i = 0; i < items.size(); i++) {
			if (!allowed_extension(items[i])) {
				return "lists the glTF extension " + JSON::stringify(items[i]) + ", which packed models cannot use";
			}
		}
	}
	if (root.has("cameras") && (root["cameras"].get_type() != Variant::ARRAY || !Array(root["cameras"]).is_empty())) {
		return "has cameras, which packed models cannot";
	}

	Array buffers;
	if (!list(root, "buffers", 1, buffers)) {
		return not_a_list("buffers", 1);
	}
	int64_t buffer_len = 0;
	if (!buffers.is_empty()) {
		if (buffers[0].get_type() != Variant::DICTIONARY) {
			return "has a buffer that is not an object";
		}
		const Dictionary buffer = buffers[0];
		if (buffer.has("uri")) {
			return "has a buffer outside the file (uri)";
		}
		if (!buffer.has("byteLength") || !whole(buffer["byteLength"], p_bin_len, buffer_len)) {
			return "has a buffer longer than its BIN chunk";
		}
	}

	Vector<View> views;
	if (!list(root, "bufferViews", MAX_GLTF_BUFFER_VIEWS, items)) {
		return not_a_list("bufferViews", MAX_GLTF_BUFFER_VIEWS);
	}
	for (int64_t i = 0; i < items.size(); i++) {
		if (items[i].get_type() != Variant::DICTIONARY) {
			return "has a buffer view that is not an object";
		}
		const Dictionary view = items[i];
		View v;
		int64_t buffer = 0;
		const bool inside = !buffers.is_empty() && view.has("buffer") && whole(view["buffer"], 0, buffer) && optional(view, "byteOffset", 0, buffer_len, v.offset) &&
				view.has("byteLength") && whole(view["byteLength"], buffer_len - v.offset, v.length) && v.length > 0;
		if (!inside) {
			return "has a buffer view outside its buffer";
		}
		if (view.has("byteStride") && (!whole(view["byteStride"], 252, v.stride) || v.stride < 4)) {
			return "has a buffer view with a bad byteStride";
		}
		views.push_back(v);
	}

	Vector<int64_t> components;
	Vector<int64_t> counts;
	Vector<int64_t> widths;
	if (!list(root, "accessors", MAX_GLTF_ACCESSORS, items)) {
		return not_a_list("accessors", MAX_GLTF_ACCESSORS);
	}
	for (int64_t i = 0; i < items.size(); i++) {
		if (items[i].get_type() != Variant::DICTIONARY) {
			return "has an accessor that is not an object";
		}
		const Dictionary accessor = items[i];
		if (accessor.has("sparse")) {
			return "has a sparse accessor, which packed models cannot";
		}
		int64_t view_index = 0;
		if (!accessor.has("bufferView") || !whole(accessor["bufferView"], views.size() - 1, view_index)) {
			return "has an accessor without a buffer view";
		}
		const View &view = views[view_index];
		int64_t component_type = 0;
		whole(accessor.get("componentType", Variant()), ANY, component_type);
		int64_t size = 0;
		if (component_type == 5120 || component_type == 5121) {
			size = 1;
		} else if (component_type == 5122 || component_type == 5123) {
			size = 2;
		} else if (component_type == 5125 || component_type == 5126) {
			size = 4;
		} else {
			return "has an accessor of an unknown component type";
		}
		const Variant type = accessor.get("type", Variant());
		const String type_name = type.get_type() == Variant::STRING ? String(type) : String();
		int64_t n = 0;
		if (type_name == "SCALAR") {
			n = 1;
		} else if (type_name == "VEC2") {
			n = 2;
		} else if (type_name == "VEC3") {
			n = 3;
		} else if (type_name == "VEC4") {
			n = 4;
		} else if (type_name == "MAT4") {
			n = 16;
		} else {
			return "has an accessor of a type packed models cannot use";
		}
		int64_t count = 0;
		if (!accessor.has("count") || !whole(accessor["count"], MAX_ACCESSOR_ELEMENTS, count) || count < 1) {
			return "has an accessor with no or too many elements";
		}
		int64_t offset = 0;
		if (!optional(accessor, "byteOffset", 0, view.length, offset)) {
			return "has an accessor that reads past its buffer view";
		}
		const int64_t element = size * n;
		// Every term is capped above, so this stays far inside 64 bits.
		const int64_t end = (view.stride > 0 ? view.stride : element) * (count - 1) + offset + element;
		if (end > view.length) {
			return "has an accessor that reads past its buffer view";
		}
		components.push_back(count * n);
		counts.push_back(count);
		widths.push_back(n);
	}

	if (!list(root, "images", MAX_GLTF_IMAGES, items)) {
		return not_a_list("images", MAX_GLTF_IMAGES);
	}
	for (int64_t i = 0; i < items.size(); i++) {
		if (items[i].get_type() != Variant::DICTIONARY) {
			return "has an image that is not an object";
		}
		const Dictionary picture = items[i];
		if (picture.has("uri")) {
			return "has a picture outside the file (uri)";
		}
		int64_t view_index = 0;
		if (!picture.has("bufferView") || !whole(picture["bufferView"], views.size() - 1, view_index)) {
			return "has a picture without a buffer view";
		}
		const Variant mime = picture.get("mimeType", Variant());
		const String mime_type = mime.get_type() == Variant::STRING ? String(mime) : String();
		if (mime_type != "image/png" && mime_type != "image/jpeg") {
			return "has a picture that is not image/png or image/jpeg";
		}
		const View &view = views[view_index];
		error = check_image(p_bin + view.offset, view.length, mime_type == "image/png" ? Kind::PNG : Kind::JPEG, r_pixels);
		if (!error.is_empty()) {
			return "has a picture that " + error;
		}
	}

	Array meshes;
	if (!list(root, "meshes", MAX_GLTF_MESHES, meshes)) {
		return not_a_list("meshes", MAX_GLTF_MESHES);
	}
	int64_t primitives = 0;
	int64_t total = 0;
	for (int64_t i = 0; i < meshes.size(); i++) {
		if (meshes[i].get_type() != Variant::DICTIONARY) {
			return "has a mesh that is not an object";
		}
		if (!list(meshes[i], "primitives", MAX_GLTF_PRIMITIVES, items)) {
			return not_a_list("primitives", MAX_GLTF_PRIMITIVES);
		}
		for (int64_t j = 0; j < items.size(); j++) {
			if (++primitives > MAX_GLTF_PRIMITIVES) {
				return "has over " + itos(MAX_GLTF_PRIMITIVES) + " mesh primitives";
			}
			if (items[j].get_type() != Variant::DICTIONARY) {
				return "has a mesh primitive that is not an object";
			}
			const Dictionary primitive = items[j];
			if (primitive.has("targets")) {
				return "has morph targets, which packed models cannot";
			}
			const Variant attributes = primitive.get("attributes", Variant());
			if (attributes.get_type() != Variant::DICTIONARY) {
				return "has a mesh primitive without attributes";
			}
			Array used = Dictionary(attributes).values();
			if (primitive.has("indices")) {
				used.push_back(primitive["indices"]);
			}
			for (int64_t k = 0; k < used.size(); k++) {
				int64_t accessor = 0;
				if (!whole(used[k], components.size() - 1, accessor)) {
					return "has a mesh primitive with a missing accessor";
				}
				total += components[accessor];
			}
			if (total > MAX_MESH_COMPONENTS) {
				return "has meshes over 16 million values";
			}
		}
	}

	Array nodes;
	if (!list(root, "nodes", MAX_GLTF_NODES, nodes)) {
		return not_a_list("nodes", MAX_GLTF_NODES);
	}
	std::vector<int64_t> parent(nodes.size(), -1);
	for (int64_t i = 0; i < nodes.size(); i++) {
		if (nodes[i].get_type() != Variant::DICTIONARY) {
			return "has a node that is not an object";
		}
		if (!list(nodes[i], "children", MAX_GLTF_NODES, items)) {
			return not_a_list("children", MAX_GLTF_NODES);
		}
		for (int64_t j = 0; j < items.size(); j++) {
			int64_t child = 0;
			if (!whole(items[j], nodes.size() - 1, child)) {
				return "has a node with a missing child";
			}
			if (parent[child] != -1) {
				return "has a node with two parents";
			}
			parent[child] = i;
		}
	}
	// Godot walks parent chains without a limit, so a loop would hang it.
	for (int64_t start = 0; start < nodes.size(); start++) {
		int64_t depth = 0;
		for (int64_t at = parent[start]; at != -1; at = parent[at]) {
			if (++depth > MAX_GLTF_NODE_DEPTH) {
				return "has nodes in a loop or nested over " + itos(MAX_GLTF_NODE_DEPTH) + " deep";
			}
		}
	}

	Array skins;
	if (!list(root, "skins", MAX_GLTF_SKINS, skins)) {
		return not_a_list("skins", MAX_GLTF_SKINS);
	}
	for (int64_t i = 0; i < nodes.size(); i++) {
		const Dictionary node = nodes[i];
		int64_t skin = 0;
		if (node.has("skin") && !whole(node["skin"], skins.size() - 1, skin)) {
			return "has a node with a missing skin";
		}
	}
	for (int64_t i = 0; i < skins.size(); i++) {
		if (skins[i].get_type() != Variant::DICTIONARY) {
			return "has a skin that is not an object";
		}
		const Dictionary skin = skins[i];
		if (!list(skin, "joints", MAX_GLTF_NODES, items)) {
			return not_a_list("joints", MAX_GLTF_NODES);
		}
		std::vector<bool> seen(nodes.size(), false);
		bool joints_ok = !items.is_empty();
		for (int64_t j = 0; j < items.size() && joints_ok; j++) {
			int64_t joint = 0;
			joints_ok = whole(items[j], nodes.size() - 1, joint) && !seen[joint];
			if (joints_ok) {
				seen[joint] = true;
			}
		}
		if (!joints_ok) {
			return "has a skin with missing or repeated joints";
		}
		int64_t at = 0;
		if (skin.has("skeleton") && !whole(skin["skeleton"], nodes.size() - 1, at)) {
			return "has a skin with a missing skeleton";
		}
		if (skin.has("inverseBindMatrices") &&
				(!whole(skin["inverseBindMatrices"], components.size() - 1, at) || counts[at] != items.size() || widths[at] != 16)) {
			return "has a skin whose inverseBindMatrices are not one MAT4 per joint";
		}
	}

	Array animations;
	if (!list(root, "animations", MAX_GLTF_ANIMATIONS, animations)) {
		return not_a_list("animations", MAX_GLTF_ANIMATIONS);
	}
	int64_t channels = 0;
	int64_t keys = 0;
	for (int64_t i = 0; i < animations.size(); i++) {
		if (animations[i].get_type() != Variant::DICTIONARY) {
			return "has an animation that is not an object";
		}
		Array samplers;
		if (!list(animations[i], "samplers", MAX_GLTF_CHANNELS, samplers)) {
			return not_a_list("samplers", MAX_GLTF_CHANNELS);
		}
		if (!list(animations[i], "channels", MAX_GLTF_CHANNELS, items)) {
			return not_a_list("channels", MAX_GLTF_CHANNELS);
		}
		for (int64_t j = 0; j < items.size(); j++) {
			if (++channels > MAX_GLTF_CHANNELS) {
				return "has over " + itos(MAX_GLTF_CHANNELS) + " animation channels";
			}
			if (items[j].get_type() != Variant::DICTIONARY) {
				return "has an animation channel that is not an object";
			}
			const Dictionary channel = items[j];
			const Variant target_value = channel.get("target", Variant());
			if (target_value.get_type() != Variant::DICTIONARY) {
				return "has an animation target that is not an object";
			}
			const Dictionary target = target_value;
			const Variant path_value = target.get("path", Variant());
			const String path = path_value.get_type() == Variant::STRING ? String(path_value) : String();
			int64_t width = 0;
			if (path == "translation" || path == "scale") {
				width = 3;
			} else if (path == "rotation") {
				width = 4;
			} else if (path == "weights") {
				return "has morph target animation, which packed models cannot";
			} else {
				return "has an animation channel with an unknown path";
			}
			int64_t index = 0;
			if (!target.has("node") || !whole(target["node"], nodes.size() - 1, index)) {
				return "has an animation channel with a missing node";
			}
			if (!channel.has("sampler") || !whole(channel["sampler"], samplers.size() - 1, index)) {
				return "has an animation channel with a missing sampler";
			}
			if (samplers[index].get_type() != Variant::DICTIONARY) {
				return "has an animation sampler that is not an object";
			}
			const Dictionary sampler = samplers[index];
			int64_t per_key = 1;
			if (sampler.has("interpolation")) {
				const Variant mode = sampler["interpolation"];
				const String name = mode.get_type() == Variant::STRING ? String(mode) : String();
				if (name == "CUBICSPLINE") {
					per_key = 3;
				} else if (name != "LINEAR" && name != "STEP") {
					return "has an animation sampler with an unknown interpolation";
				}
			}
			int64_t input = 0;
			int64_t output = 0;
			if (!sampler.has("input") || !whole(sampler["input"], components.size() - 1, input) || widths[input] != 1 || !sampler.has("output") ||
					!whole(sampler["output"], components.size() - 1, output)) {
				return "has an animation sampler with missing or wrong accessors";
			}
			if (widths[output] != width || counts[output] != counts[input] * per_key) {
				return "has an animation sampler whose values do not match its times";
			}
			keys += counts[input] + components[output];
			if (keys > MAX_ANIMATION_COMPONENTS) {
				return "has animations over 16 million values";
			}
		}
	}

	if (!list(root, "materials", MAX_GLTF_MATERIALS, items)) {
		return not_a_list("materials", MAX_GLTF_MATERIALS);
	}
	if (!list(root, "textures", MAX_GLTF_TEXTURES, items)) {
		return not_a_list("textures", MAX_GLTF_TEXTURES);
	}
	return String();
}

// Checks a binary glTF before Godot sees it, since Godot sizes its JSON buffer from the header and follows uris to files.
String check_glb(const PackedByteArray &p_bytes, int64_t &r_pixels) {
	const uint8_t *p = p_bytes.ptr();
	const int64_t n = p_bytes.size();
	if (n < 4 || memcmp(p, "glTF", 4) != 0) {
		return "is not a GLB model";
	}
	if (n < 8 || le32(p + 4) != 2) {
		return "is not glTF 2.0";
	}
	if (n < 12 || int64_t(le32(p + 8)) != n) {
		return "has the wrong length in its header";
	}
	if (n < 16) {
		return "is a cut-short GLB";
	}
	const int64_t json_len = le32(p + 12);
	if (n < 20 || le32(p + 16) != GLB_JSON) {
		return "does not start with its JSON chunk";
	}
	if (json_len > MAX_GLB_JSON) {
		return "has a JSON chunk over 2 MiB";
	}
	const int64_t json_end = 20 + json_len;
	if (json_end > n) {
		return "has a JSON chunk past its end";
	}
	int64_t bin_start = n;
	int64_t bin_len = 0;
	if (json_end != n) {
		if (n - json_end < 4) {
			return "has a cut-short BIN chunk";
		}
		bin_len = le32(p + json_end);
		if (n - json_end < 8 || le32(p + json_end + 4) != GLB_BIN) {
			return "has a second chunk that is not BIN";
		}
		if (json_end + 8 + bin_len != n) {
			return "has a BIN chunk that does not end with the file";
		}
		bin_start = json_end + 8;
	}
	// The same text Godot's glTF importer parses, so both see the same document.
	Ref<JSON> json;
	json.instantiate();
	if (json->parse(String::utf8(reinterpret_cast<const char *>(p + 20), json_len)) != OK) {
		return "has broken JSON: " + json->get_error_message();
	}
	return check_gltf(json->get_data(), p + bin_start, bin_len, r_pixels);
}

} // namespace

Error MapCraftPack::open(const String &p_path, String &r_error) {
	Ref<FileAccess> file = FileAccess::open(p_path, FileAccess::READ);
	if (file.is_null()) {
		r_error = "cannot be opened";
		return ERR_FILE_CANT_OPEN;
	}
	if (int64_t(file->get_length()) > MAX_PACK_SIZE) {
		r_error = "is larger than 256 MiB";
		return ERR_FILE_CORRUPT;
	}
	return parse(file->get_buffer(file->get_length()), r_error);
}

Error MapCraftPack::parse(const PackedByteArray &p_data, String &r_error) {
	data = PackedByteArray();
	entries.clear();
	pixels = 0;
	const uint8_t *p = p_data.ptr();
	const int64_t total = p_data.size();
	if (total > MAX_PACK_SIZE) {
		r_error = "is larger than 256 MiB";
		return ERR_FILE_CORRUPT;
	}
	if (total < 4 || memcmp(p, "MCPK", 4) != 0) {
		r_error = "is not a MapCraft pack";
		return ERR_FILE_UNRECOGNIZED;
	}
	if (total < HEADER_LEN) {
		r_error = "has a cut-short table of contents";
		return ERR_FILE_CORRUPT;
	}
	if (le32(p + 4) < 1 || le32(p + 4) > VERSION) {
		r_error = "is version " + itos(le32(p + 4)) + ", this addon reads versions 1 to 3";
		return ERR_FILE_UNRECOGNIZED;
	}
	const uint32_t count = le32(p + 8);
	if (count > MAX_ENTRIES) {
		r_error = "has " + itos(count) + " entries, at most 1024";
		return ERR_FILE_CORRUPT;
	}
	struct Raw {
		String name;
		uint64_t offset = 0;
		uint64_t length = 0;
	};
	Vector<Raw> toc;
	int64_t at = HEADER_LEN;
	for (uint32_t i = 0; i < count; i++) {
		const int64_t len = total - at >= 2 ? int64_t(le16(p + at)) : -1;
		if (len < 0 || total - at - 2 < len + 16) {
			r_error = "has a cut-short table of contents";
			return ERR_FILE_CORRUPT;
		}
		at += 2;
		String why;
		if (kind_of(p + at, len, why) == Kind::NONE) {
			r_error = "entry " + itos(i) + ": " + why;
			return ERR_FILE_CORRUPT;
		}
		Raw raw;
		raw.name = String::utf8(reinterpret_cast<const char *>(p + at), len);
        if (le32(p + 4) == 1 && (raw.name.ends_with(".json") || raw.name.begins_with("textures/"))) {
            r_error = "version 1 cannot contain material descriptors or textures/ entries";
            return ERR_FILE_CORRUPT;
        }
		if (le32(p + 4) < 3 && raw.name == MAP_ENTRY) {
			r_error = "versions 1 and 2 cannot contain a map";
			return ERR_FILE_CORRUPT;
		}
		raw.offset = le64(p + at + len);
		raw.length = le64(p + at + len + 8);
		toc.push_back(raw);
		at += len + 16;
	}
	HashMap<String, Entry> found;
	for (const Raw &raw : toc) {
		const String key = raw.name.get_basename().to_lower();
		if (found.has(key)) {
			r_error = raw.name + String(" is in the pack twice (names ignore case and extension)");
			return ERR_FILE_CORRUPT;
		}
		if (raw.length < 1 || raw.length > MAX_ENTRY_SIZE) {
			r_error = raw.name + String(" is empty or larger than 64 MiB");
			return ERR_FILE_CORRUPT;
		}
		if (raw.offset != uint64_t(at)) {
			r_error = raw.name + String(" does not start where the entry before it ends");
			return ERR_FILE_CORRUPT;
		}
		if (int64_t(raw.length) > total - at) {
			r_error = raw.name + String(" runs past the end of the pack");
			return ERR_FILE_CORRUPT;
		}
		found.insert(key, Entry{ raw.name, at, int64_t(raw.length) });
		at += int64_t(raw.length);
	}
	if (at != total) {
		r_error = "has data after its last entry";
		return ERR_FILE_CORRUPT;
	}
	data = p_data;
	entries = found;
	return OK;
}

bool MapCraftPack::map(PackedByteArray &r_bytes) const {
	const Entry *entry = entries.getptr("map");
	if (entry == nullptr || entry->name != MAP_ENTRY) {
		return false;
	}
	r_bytes = data.slice(entry->offset, entry->offset + entry->length);
	return true;
}

bool MapCraftPack::find(const String &p_stem, PackedByteArray &r_bytes, String &r_error) {
	const Entry *entry = entries.getptr(p_stem.to_lower());
	if (entry == nullptr || entry->name == MAP_ENTRY) {
		return false;
	}
	r_bytes = data.slice(entry->offset, entry->offset + entry->length);
  if (entry->name.ends_with(".json")) {
    r_error = check_material(r_bytes);
    if (r_error.is_empty()) {
      const Dictionary descriptor =
          JSON::parse_string(r_bytes.get_string_from_utf8());
      const Dictionary textures = descriptor["textures"];
      const Array names = textures.values();
      for (int i = 0; i < names.size(); i++) {
        const String name = names[i];
        const Entry *texture = entries.getptr(name.get_basename().to_lower());
        if (texture == nullptr || texture->name.to_lower() != name.to_lower()) {
          r_error = "missing texture " + name;
          break;
        }
      }
    }
  } else if (entry->name.ends_with(".glb")) {
		r_error = check_glb(r_bytes, pixels);
	} else {
		r_error = check_image(r_bytes.ptr(), r_bytes.size(), entry->name.ends_with(".png") ? Kind::PNG : Kind::JPEG, pixels);
	}
	if (!r_error.is_empty()) {
		r_bytes = PackedByteArray();
	}
	return true;
}

} // namespace godot
