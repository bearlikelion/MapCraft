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
- [ ] Later: FGD `@AutoVisGroup` tags, vertex editing in the 3D view, extruding faces behind a brush in 3D.

## UX pass

- [x] Theme with an accent color, icon tool palette, top bar with undo and redo, selection modes, grid stepper, texture lock, current material and a Run button.
- [x] Viewport headers with axes, zoom and render mode, a 3D axis widget, the hovered view outlined, per-object colors in 2D views.
- [x] Status bar with selection, cursor, grid and mode, and a hint for the current tool.
- [x] `tools/` brushes (triggers, zones, clips) drawn see-through in their entity color; layers open unfolded.
- [x] Snapping like TrenchBroom: drags snap the grabbed corner to the grid or to a nearby brush corner, with a marker showing where it lands, and toggles for both.
- [x] Space maximizes the viewport under the pointer and restores the four views.
- [x] Linked 2D zoom (on by default, chain toggle in each 2D header).
- [x] TrenchBroom's extrude: drag a face (Extrude tool, or Shift+drag with Select), Ctrl splits off a new brush or splits inward, Ctrl+Alt stamps; coplanar faces of several brushes move together.
- [x] Godot-style keys: Q select, W move, E extrude, R rotate (2D drag or 3D rings, 15 degree steps), T scale (3D handles); B block, N entity, C clip, V vertex, F face.
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
- [ ] Later: dockable and rearrangeable panels, a lit 3D preview, a model browser.

## M3 Plugins and the SurfsUp SDK

- [x] WIT plugin API and wasmtime host with fuel and memory limits.
- [x] Plugin commands with generated parameter UI, generator groups that rerun when edited.
- [x] `godot` asset plugin (`.tres` materials with texture or color previews), `surfsup` plugin with SURGE ramp and bhop generators.
- [x] Example plugin written in Python, user plugins folder.
- [ ] Later: compiled plugin cache for faster startup, SurfsUp map validators, prefab scenes as bhop platforms.

## M4 Godot extension

- [x] C++ GDExtension: `.mcmap` importer, `MapCraftMap`, `MapCraftEntityMap`, headless import test.
- [x] SurfsUp entity map (spawn point, `sdk_zone` areas, typed settings), map pack tool, Run menu profiles to import, pack and play.
- [x] Sample course `sdks/surfsup/samples/surf_mapcraft.mcmap` built with the ramp generator; imports in Godot with spawn, start, splits, finish and kill zones.
- [ ] Play a MapCraft map end to end in SurfsUp v3 with `+map`.
- [ ] Later: prefab instance overrides, `tools/` materials (nodraw, clip), Windows and macOS builds in CI, C# example.

## M5 Counter-Strike: Source SDK

- [ ] `source` plugin: VMF, VPK, VTF, VMT.
- [ ] Displacements, overlays, Valve compilers.

## M6 Quake SDK

- [ ] `quake` plugin: Standard and Valve220 `.map`, WAD2, PAK.

## M7 Platform

- [ ] macOS, packaging, control channel and MCP, Unity importer.
