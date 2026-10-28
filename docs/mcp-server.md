# MCP server

`mapcraft-cli mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on standard input and output, so an AI agent such as Claude Code can build and edit maps.
While the editor is open, every tool acts on the map open in it, live and undoable; otherwise tools act on a map held in memory by the server.
[AI agents](ai-agents.md) covers what an agent can do and how to set one up.

## Setup

Build the CLI from the source tree (see [Building from source](building.md)):

```sh
cargo build --release -p mapcraft-cli
```

Register it with the agent, for example:

```sh
claude mcp add mapcraft -- /path/to/mapcraft-cli mcp
```

Other clients take the same command in their MCP server settings.
Open a map in MapCraft first, so its SDK gives the agent entity classes, materials and generators.

## Tools

| Tool | What it does |
| --- | --- |
| `new`, `open`, `save` | Start a map at a path that must not exist yet, open one, save it (optionally to a new path) |
| `info` | Path, SDK, unsaved state, node count, bounds, selection, undo and redo labels |
| `list_nodes`, `get_node`, `find` | Walk the scene tree, read a node's key/values or brush faces, search entities |
| `select` | Select nodes by id |
| `create_brush` | Block, wedge, cylinder, spike, sphere or arch filling a box |
| `create_entity`, `set_property` | Place point entities and edit key/values (node 1 is worldspawn), or rename a group or layer with key `name` |
| `tie_to_entity` | Turn the selected brushes into a brush entity, such as a trigger or zone |
| `set_material` | Give the selected brushes' faces a material and texture scale, all of them or those facing one way |
| `move`, `rotate`, `scale`, `duplicate` | Transform the selection, or copy it in a row |
| `hollow`, `clip` | Make rooms with walls of a given thickness, cut brushes with a plane |
| `pick_faces`, `face_texture`, `extrude` | Pick faces as the Face tool does, set their texture offset, scale, rotation and anti-tiling (`anti_tile`: `off`, `macro` or `hex`), push them out or slide them |
| `move_vertices`, `bevel` | Move brush corners and bevel edges, as the Vertex tool does |
| `create_path`, `set_curve` | Draw a path, and change a sweep, spin or path array's settings; `run_command` runs the sweep, spin and array themselves |
| `displace`, `sculpt`, `heightmap` | Make terrain from picked faces and shape it with sculpt dabs or a grayscale grid; `sculpt` modes `paint` and `blur` and `heightmap` `channel` `blend` paint a second material's share instead of heights |
| `blend` | Give the picked displacements a second material (`material`, `""` removes it) and fill its share by `value`, `invert`, `slope`, `height` or `noise` |
| `zone_from_selection` | Cover the selection's top with a zone of a given class |
| `create_layer`, `move_to`, `tag`, `cordon` | Organize the scene tree, tags and cordons |
| `insert_prefab`, `save_prefab`, `make_prefab_local` | Reuse prefabs, save the selection as one, and make instances local |
| `replace_materials` | Rename a material across the map or the selection |
| `list_commands`, `run_command` | Every editor command by id: undo, redo, delete, carve, group, merge and the rest |
| `batch` | Many of the calls above in one round trip, stopping at the first error |
| `validate` | Check for Problems, with the SDK's FGD and plugin checks while the editor is open |

These need the editor open:

| Tool | What it does |
| --- | --- |
| `screenshot` | Renders the four views, or one, as an image the agent can look at |
| `frame` | Points the editor's views at what the agent is working on |
| `entity_classes`, `entity_class` | The SDK's entity classes, and one class's keys, choices, inputs and outputs |
| `materials` | The SDK's material names |
| `plugin_commands`, `run_plugin` | The SDK plugins' generators and commands, and running one at a point |
| `compile_profiles`, `compile`, `output` | Run a Run menu profile and follow its output |

Coordinates are in inches (39.37 per meter) with Z up, as in the editor.
The first time an agent runs a compile profile, the editor asks the user to allow the SDK's programs, as the Run menu does.

## The editor connection

The editor listens on a loopback port and writes the port and a random token to `control.json` in its data folder.
`mapcraft-cli mcp` reads that file and must send the token before any call, so web pages and other users' programs cannot drive the editor.
The file is removed when the editor closes.
If the connection drops mid-call, the tool reports that the call may not have run, and the agent should check with `info` before retrying.

## Unsaved work

- Edits stay unsaved until the agent calls `save`.
- `open` and `new` refuse to drop unsaved changes unless they are passed `discard: true`.
- In the editor, the usual autosave and recovery cover the agent's work.
- Without the editor, a map still unsaved when the client disconnects is written next to itself as `<name>.unsaved.mcmap`, or to `untitled.unsaved.mcmap` in the working folder for a map that never had a path.
