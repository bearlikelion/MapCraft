# Technical docs

These pages are for people who build MapCraft from source, add a game to it, write plugins, or read `.mcmap` files in their own tools.
For making maps, see the [user guide](guide/README.md).

## Build and automate

- [Building from source](building.md): compile the editor, its plugins and the release packages.
- [AI agents](ai-agents.md): have an AI agent such as Claude Code edit your map.
- [MCP server](mcp-server.md): the tools an AI agent can call, and how it connects to the editor.

## Add a game

- [SDK manifests](sdk-format.md): the `sdk.toml` that describes a game.
- [SDK registries](sdk-registry.md): publish SDKs so the SDK Manager can install them.
- [TrenchBroom and func_godot import](trenchbroom-import.md): how a TrenchBroom game config and func_godot's map settings become an `sdk.toml`.
- [Plugin API](plugin-api.md): write a WebAssembly plugin in any language.
- [Lit preview](lighting.md): the light keys and material maps the Lit render mode reads.
- [Surf mapping](surf-mapping.md): the geometry rules behind surfable ramps.

## Files and internals

- [The .mcmap format](mcmap-format.md): reference for importers and game tools.
- [Godot addon](../godot/README.md): the GDExtension's import plugin, runtime loader and entity maps.
- [Architecture](architecture.md): crate layers, brushes, the document model, undo and coordinates.

## Project

- [Roadmap](../ROADMAP.md): the milestones, what is done and what comes next.
