# AI agents (MCP)

`mapcraft-cli mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server, so an AI agent such as Claude Code can open, edit, check and save `.mcmap` files.
It runs headless on stdin and stdout and edits one map at a time through the same engine, commands and undo as the editor.

## Setting it up

Build the CLI with `cargo build --release -p mapcraft-cli`, then register it with your agent, for example:

```sh
claude mcp add mapcraft -- /path/to/mapcraft-cli mcp
```

Other clients take the same command in their MCP server settings.

## Tools

| Tool | What it does |
| --- | --- |
| `new`, `open`, `save` | Start a map at a path, open one, save it (optionally to a new path) |
| `info` | Path, SDK, unsaved state, node count, selection, undo and redo labels |
| `list_nodes`, `get_node`, `find` | Walk the scene tree, read a node's key/values or brush faces, search entities |
| `select` | Select nodes by id |
| `create_brush` | Block, wedge, cylinder, spike or sphere filling a box, with an optional material |
| `create_entity`, `set_property` | Place point entities and edit key/values (node 0 is worldspawn) |
| `move`, `rotate` | Transform the selection |
| `list_commands`, `run_command` | Every editor command by id: undo, redo, delete, duplicate, hollow, carve, group, tie to entity and the rest |
| `validate` | Check for Problems without the SDK's FGD and plugin checks |

Coordinates are in inches (39.37 per meter) with Z up, as in the editor.

## Unsaved work

Edits stay in memory until the agent calls `save`.
`open` and `new` refuse to drop unsaved changes unless they are passed `discard: true`.
If the agent disconnects with changes still unsaved, they are written next to the map as `<name>.unsaved.mcmap`, or to `untitled.unsaved.mcmap` in the working folder for a map that never had a path.

The server edits files, not the open editor: reopen a map in MapCraft to see what the agent did.
