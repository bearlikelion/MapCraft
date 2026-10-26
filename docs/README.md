# Technical docs

You do not need any of this to make maps.
These pages are for people who build MapCraft from source, teach it a new game, write plugins, or read `.mcmap` files in their own tools.
For making maps, start with the [user guide](guide/README.md).

## Build and automate

- [Building from source](building.md): compile the editor, its plugins and the release packages.
- [AI agents](ai-agents.md): let an AI agent such as Claude Code build in your map while you watch.
- [MCP server](mcp-server.md): every tool an AI agent can call, and how it reaches the editor.

## Teach MapCraft a game

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
