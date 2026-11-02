#pragma once

#include <godot_cpp/templates/hash_map.hpp>
#include <godot_cpp/variant/packed_byte_array.hpp>
#include <godot_cpp/variant/string.hpp>

namespace godot {

// A map's .mcpck (docs/mcpck-format.md), held in memory: nothing in it is written to disk or registered as a resource.
class MapCraftPack {
	struct Entry {
		String name;
		int64_t offset = 0;
		int64_t length = 0;
	};

	PackedByteArray data;
	HashMap<String, Entry> entries;
	int64_t pixels = 0;

public:
	// Any problem with the container or a name refuses the whole pack.
	Error open(const String &p_path, String &r_error);
	Error parse(const PackedByteArray &p_data, String &r_error);
	// False when no entry has this name less its extension, ignoring case; r_error says why a present entry cannot be used.
	bool find(const String &p_stem, PackedByteArray &r_bytes, String &r_error);
	// The map a self-contained pack carries, unchecked: the builder parses it like any .mcmap.
	bool map(PackedByteArray &r_bytes) const;
	// The map's baked lighting (docs/mclight-format.md), unchecked: the builder parses it.
	bool lighting(PackedByteArray &r_bytes) const;
};

} // namespace godot
