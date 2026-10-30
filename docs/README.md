# Technical docs

These pages are for when you want to go deeper than making maps: build MapCraft from source, add a game to it, write plugins, or read `.mcmap` files in your own tools.
If you only want to make maps, start with the [user guide](guide/README.md).

## Build and automate

- [Building from source](building.md): compile the editor, its plugins and the release packages yourself.
- [AI agents](ai-agents.md): let an AI agent such as Claude Code edit your map alongside you.
- [MCP server](mcp-server.md): the tools an AI agent can call, and how it connects to the editor.

## Add a game

- [SDK manifests](sdk-format.md): write the `sdk.toml` that describes a game to MapCraft.
- [SDK registries](sdk-registry.md): publish your SDKs so anyone can install them from the SDK Manager.
- [TrenchBroom and func_godot import](trenchbroom-import.md): how a TrenchBroom game config and func_godot's map settings turn into an `sdk.toml`.
- [Plugin API](plugin-api.md): write a WebAssembly plugin in any language.
- [Lit preview](lighting.md): the light keys and material maps the Lit render mode reads.
- [Surf mapping](surf-mapping.md): the geometry rules that make a ramp surfable.

## Files and internals

- [The .mcmap format](mcmap-format.md): the map file, for anyone writing an importer or a game tool.
- [Godot addon](../godot/README.md): the GDExtension's import plugin, runtime loader and entity maps.
- [Architecture](architecture.md): how the code fits together, from crate layers and brushes to the document model, undo and coordinates.
