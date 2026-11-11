# {{name}} SDK for MapCraft

This folder is a MapCraft SDK for a Godot game: it tells the editor which entities {{name}} has, where its materials are and how to send a map to the Godot project.

## Files

- `sdk.toml`: the manifest, with comments on every key.
- `fgd/godot.fgd`: the entities the MapCraft Godot addon builds: the sky and sun (`light_environment`), baked lights (`light`, `light_spot`), models (`prop_static`), `func_detail`, `func_illusionary` and `info_player_start`.
- `fgd/entities.fgd`: your game's own entities.
- `godot/{{id}}.tres`: the entity map that tells the addon which Godot node each entity becomes.
- `plugins/`: WebAssembly plugins listed in `sdk.toml`, as `plugins/<id>.wasm` (none yet).
- `samples/`: example maps; the SDK Manager's Test button opens the first one.

## Set up the Godot project

1. Install the MapCraft addon: unzip `MapCraft-godot-addon.zip` from the latest MapCraft release into the project, so it lands in `addons/mapcraft`.
2. Copy `godot/{{id}}.tres` to `addons/mapcraft/sdks/{{id}}.tres` in the project.
   Without it the addon makes plain markers and areas of every entity, so the map gets no sky, sun or models.
3. In MapCraft, open File > SDK Settings and set the Godot project folder.
4. Make a map and choose Run > Send to Godot project.
   Godot imports it from the project's `maps` folder as a scene.

## Next steps

1. Add your game's entities to `fgd/entities.fgd`.
2. Open `addons/mapcraft/sdks/{{id}}.tres` in Godot and say what each of them becomes, such as a scene; the addon's README lists the keys.
3. If your materials are not in `res://Assets/Textures`, change `textures.roots` here and the `material_root` import option in Godot.
4. Save a small map into `samples/` and run Check in File > SDK Manager.
5. Package it with `mapcraft-cli sdk package <this folder>` and follow the MapCraftSDKs README to publish it.

Never ship a game's textures, models or FGDs here unless their license allows it; point at the project instead.
