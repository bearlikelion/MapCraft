# {{name}} SDK for MapCraft

This folder is a MapCraft SDK: it tells the editor which entities {{name}} has, where its materials are and how to compile a map.

## Files

- `sdk.toml`: the manifest, with comments on every key.
- `fgd/entities.fgd`: the entities mappers can place.
- `plugins/`: WebAssembly plugins listed in `sdk.toml`, as `plugins/<id>.wasm` (none yet).
- `samples/`: example maps; the SDK Manager's Test button opens the first one.

## Next steps

1. Rename the classes in `fgd/entities.fgd` to the ones your game reads, and add the rest.
2. Point `textures.roots` at the folders your game keeps its textures in.
3. Add a compile profile that builds and launches a map.
4. Save a small map into `samples/` and run Check in File > SDK Manager.
5. Package it with `mapcraft-cli sdk package <this folder>` and follow the MapCraftSDKs README to publish it.

Never ship the game's own textures, models or FGDs here unless their license allows it; point at the user's game install instead.
