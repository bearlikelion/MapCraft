# Roadmap

## M0 Skeleton

- [x] Workspace, rules, lints, `cargo xtask ci`, CI on Linux and Windows.

## M1 Core (headless)

- [x] `geom`: brush kernel (planes to polyhedron), CSG, clip.
- [x] `map`: document model with layers, groups, entities, brushes, prefab instances, connections, tags.
- [x] `fgd`: Valve and TrenchBroom FGD dialects.
- [x] `format`: `.mcmap` read/write.
- [x] `sdk`: `sdk.toml`, asset VFS, compile profiles.
- [x] `engine`: session, commands, undo, selection.
- [x] `mapcraft-cli new|info|validate|fgd`.

## M2 Editor

- [x] Views: four switchable panes with maximize, fly camera, ortho views, grid, snapping.
- [x] Selection: picking, marquee, Hammer selection modes, hide, isolate, show all.
- [x] Selection: drill select (click the same spot again for the object behind).
- [x] Brushes: block, wedge, cylinder and spike tools, move and resize handles, nudge, clip tool, vertex tool, carve, hollow, merge, rotate, flip, texture lock, copy and paste.
- [x] Brushes: 3D move gizmo, edge and face handles, transform dialog, arch and sphere.
- [x] Materials: browser, face tool with offset, scale, rotation, justify and align, Alt+click pick and Alt+right click apply.
- [x] Materials: thumbnails, replace materials, alignment transfer wrapped across edges (Alt+Shift+right click).
- [x] Entities: browser, FGD-driven inspector (choices, flags, colors, booleans), outputs editor, link lines.
- [x] Organization: scene tree, layers, groups, prefabs (save selection as prefab, insert, make local, nested, cycle-safe).
- [x] Organization: tags (visgroups) panel, cordons, prefab override editing.
- [x] Quality: undo and redo with coalescing, autosave, issue browser, offscreen UI smoke test.
- [x] Quality: quick fixes, crash recovery prompt, entity report, find and replace, Hammer and TrenchBroom keymap presets, preferences window.
- [x] FGD `@AutoVisGroup` sets as auto tags in the Tags panel (show, hide, select), with sets in the SurfsUp and Quake FGDs.
- [x] Vertex editing in the 3D view: drag a corner, edge or face handle across its level plane, Shift for height.
- [ ] Later: extruding faces behind a brush in 3D.

## UX pass

- [x] Theme with an accent color, icon tool palette, top bar with undo and redo, selection modes, grid stepper, texture lock, current material and a Run button.
- [x] Viewport headers with axes, zoom and render mode, a 3D axis widget, the hovered view outlined, per-object colors in 2D views.
- [x] Status bar with selection, cursor, grid and mode, and a hint for the current tool.
- [x] `tools/` brushes (triggers, zones, clips) drawn see-through in their entity color; layers open unfolded.
- [x] Snapping like TrenchBroom: drags snap the grabbed corner to the grid or to a nearby brush corner, with a marker showing where it lands, and toggles for both.
- [x] Space maximizes the viewport under the pointer and restores the four views.
- [x] Linked 2D zoom (on by default, chain toggle in each 2D header).
- [x] TrenchBroom's extrude: drag a face (Extrude tool, or Shift+drag with Select), Ctrl splits off a new brush or splits inward, Ctrl+Alt stamps; coplanar faces of several brushes move together.
- [x] Godot-style keys: Q select, W move, E extrude, R rotate (2D drag or 3D rings, 15 degree steps), T scale (3D handles); B block, N entity, C clip, V vertex, Shift+F face, D sculpt; F focuses the selection.
- [x] SDK Settings opens by itself when a path materials need is unset, lists those paths first, flags paths that do not exist and applies on close.
- [x] Release builds find the SDKs in the source tree; `MAPCRAFT_SDK_PATH` adds more folders.
- [x] Scene tree rows with flat disclosure arrows, node kind icons and the visibility eye at the far right.
- [x] Viewports are shown with correct gamma, so grid lines, textures and colors read as intended.
- [x] Inspector with Keyvalues, Outputs and Inputs tabs, SmartEdit, and help for the hovered property.
- [x] Every menu item, panel, toolbar toggle and mode has a rebindable shortcut, shown in menus and tooltips, with no clashes in any preset; F1 lists them all.
- [x] Blocks can be drawn in the 3D view: on the ground or on any brush face, Shift drags the height.
- [x] 2D views snap to the grid they draw, which coarsens when zoomed out (the header says so), and a grid point beats a farther brush corner.
- [x] Scene tree rows drag to reorder or move into another layer, group or brush entity; a selection made in a view is revealed in the tree.
- [x] Clicking an axis on the 3D axis widget looks down that axis; clicking it again looks from the other side.
- [x] Tool output in the Output panel has terminal color codes stripped.
- [x] Faces picked with the Face tool keep their true color with a cyan outline; entering the Face tool picks the selected brushes' faces.
- [x] Plugin generators are Block tool shapes: parameters in the inspector, a live ghost under the pointer, click to place.
- [x] Clip tool: drag a line or click points in any view (on faces in 3D), with a preview of what stays and what goes.
- [x] Mirror groups: build one half and the other follows live; the file holds both halves so importers need nothing new.
- [x] Displacements on four-cornered faces, a Sculpt tool (raise, lower, smooth, flatten, noise) that keeps neighbors joined, and heightmap images.
- [x] Snap Selection to Grid, F to focus the selection, see-through zones that never fight the floor they sit on.
- [x] Edit > Selection Mode submenu showing the current mode; 3D scale handles on both sides of each axis.
- [x] Faces hidden between touching world brushes are found and marked in the file; the Godot importer leaves them out of meshes and trimesh collision, and the Face tool leaves them out of picks.
- [x] Right click menu in the views, Blender style: the tools that fit the selection (Hollow, Subtract, Brush Type, Transform, UV straight from brushes, Organize, Curves), the Face and Vertex tools' own commands, and Add with every shape on empty space.
- [x] Linked copies (`Alt+D`): copies that follow their original's contents but keep their own place, Edit Linked Copy Here to make any copy the original, Make Real to break the link; saved filled so importers need nothing new.
- [x] Playtest round: View > Go to Origin (`Shift+Home`), a one view layout where Space cycles the views, a right click shape menu on the Block icon, a brush Type row (World, detail, illusionary, wall, Group), three point clipping in the 3D view, and an Array dialog for rows, rings and spirals of copies.
- [x] SDKs set map limits (`map_size`, else the FGD's `@mapsize`): the views outline them in red and Check for Problems reports brushes and entities outside; Quake is -4096 to 4096 and CS:S -16384 to 16384.
- [x] Face tool copy and paste: Ctrl+C copies the active face, Ctrl+V gives the picked faces its material, alignment (wrapped across edges) and attributes in one undo step.
- [x] Plugin material listing runs on a worker thread, so a CS:S project opens at once and its 5,500 materials arrive in the background.
- [x] Clicking a material applies it to the selection, as in TrenchBroom (a preference turns it off).
- [x] Size labels like TrenchBroom's while drawing a block or resizing or scaling the selection, and the faces a resize, scale or extrude moves highlighted in the selection red.
- [x] Lit render mode (`Ctrl+Alt+4`): a 3D preview shaded by the map's point, spot and sun lights, read from Source and Quake light keys.
- [x] PBR in Lit: metallic-roughness shading with normal and ambient occlusion maps from Godot `.tres` materials and ambientCG image sets, and a stand-in sun and sky for maps without lights.
- [x] Blender-style UV editor: any view can show the picked faces over their tiled texture with picks shared with the Face tool, G, R and S move, turn and scale textures with axis locks and Ctrl snapping, and Tools > UV holds Unwrap, Project from View, Cube Projection and Reset.
- [x] Blender-style Vertex tool: vertex, edge and face select modes (1, 2, 3) with click and box select, Merge at center (M), and Bevel (Ctrl+B) with width, segments and profile set by the pointer and scroll, an Adjust Last Bevel window, and textures wrapped around the new faces.
- [x] glTF models: point entities show the `.glb` or `.gltf` their class or `model` key names in every view, a Models tab places them as the SDK's prop class (`[models]` in `sdk.toml`), and the Godot importer instances them.
- [x] Curves: a Bezier Path tool (P) with Blender-style points and handles edited in every view, and Sweep Along Path, Spin and Array Along Path as curve groups that rebuild in the same undo step when their path, faces or settings change, with trims running on around every bend.
- [ ] Later: dockable and rearrangeable panels.

## M3 Plugins and the SurfsUp SDK

- [x] WIT plugin API and wasmtime host with fuel and memory limits.
- [x] Plugin commands with generated parameter UI, generator groups that rerun when edited.
- [x] `godot` asset plugin (`.tres` materials with texture or color previews), `surfsup` plugin with SURGE ramp and bhop generators.
- [x] Example plugin written in Python, user plugins folder.
- [x] Importers: plugins list file formats under File > Import and read the picked file's bytes.
- [x] Surf Ramp gains drop, ridge, inside troughs, gaps, sloped ends and pyramids, and lead in and out, with warnings for walkable faces and coarse curves; bhop platforms gain shapes, path facing and jitter; new Surf Window and Surf Stage generators. See [`docs/surf-mapping.md`](docs/surf-mapping.md).
- [x] Compiled plugins are cached, so loading a plugin again takes milliseconds.
- [x] Plugin validators: a `validate` hook whose issues join Check for Problems; SurfsUp checks spawns, start and finish zones, zone sizes and teleport destinations.
- [ ] Later: prefab scenes as bhop platforms.

## M4 Godot extension

- [x] C++ GDExtension: `.mcmap` importer, `MapCraftMap`, `MapCraftEntityMap`, headless import test.
- [x] SurfsUp entity map (spawn point, `sdk_zone` areas, typed settings), map pack tool, Run menu profiles to import, pack and play.
- [x] Sample course `sdks/surfsup/samples/surf_mapcraft.mcmap` built with the ramp generator; imports in Godot with spawn, start, splits, finish and kill zones.
- [x] Displacements import as meshes with triangle collision; a Trimesh collision option makes the world one triangle mesh so ramp seams cannot catch players.
- [ ] Play a MapCraft map end to end in SurfsUp v3 with `+map`.
- [ ] Later: prefab instance overrides, `tools/` materials (nodraw, clip), Windows and macOS builds in CI, C# example.

## M5 Counter-Strike: Source SDK

- [x] `source-bsp` plugin: File > Import reads BSP maps (versions 19 to 21) with texture alignment, grouped into ramps, world, triggers, spawns, lights and props, and textures from the map and the game's VPKs.
- [ ] `source` plugin: VMF, VPK and VTF materials in the browser, MDL previews.
- [x] Displacements as a host feature.
- [ ] Overlays, Valve compilers.

## M6 Quake SDK

- [x] `quake` plugin: Standard and Valve220 `.map` import with texture alignment and TrenchBroom layers and groups, WAD2 and WAD3 textures in the browser, the palette from the user's PAK; `sdks/quake` with an FGD written from scratch.
- [x] Valve220 `.map` export through the plugin exporter hook, so the ericw-tools compile profiles run.

## M7 Platform

- [x] SDK Manager (File > SDK Manager…): installed SDKs with Configure, Check, Test, Open Folder and Remove; install and update from registries with checksums and safe unpacking; new SDKs from a template; `mapcraft-cli sdk` and the MapCraftSDKs registry format. See [`docs/sdk-registry.md`](docs/sdk-registry.md).
- [x] MCP server: `mapcraft-cli mcp` opens, edits, checks and saves maps headlessly through the engine's commands. See [`docs/guide/mcp.md`](docs/guide/mcp.md).
- [ ] macOS, packaging, editor control channel (MCP against the open map), Unity importer.
