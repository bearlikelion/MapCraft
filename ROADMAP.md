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
- [ ] Selection: drill select.
- [x] Brushes: block, wedge, cylinder and spike tools, move and resize handles, nudge, clip tool, vertex tool, carve, hollow, merge, rotate, flip, texture lock, copy and paste.
- [ ] Brushes: 3D gizmo, edge and face handles, transform dialog, arch and sphere.
- [x] Materials: browser, face tool with offset, scale, rotation, justify and align, Alt+click pick and Alt+right click apply.
- [ ] Materials: thumbnails, replace materials, UV transfer.
- [x] Entities: browser, FGD-driven inspector (choices, flags, colors, booleans), outputs editor, link lines.
- [x] Organization: scene tree, layers, groups, prefabs (save selection as prefab, insert, make local, nested, cycle-safe).
- [ ] Organization: tags (visgroups) panel, cordons, prefab override editing.
- [x] Quality: undo and redo with coalescing, autosave, issue browser, offscreen UI smoke test.
- [ ] Quality: quick fixes, crash recovery prompt, entity report, find and replace, keymap presets, preferences window.

## M3 Plugins and the SurfsUp SDK

- [ ] WIT plugin API and wasmtime host.
- [ ] Plugin commands with generated parameter UI, generator groups.
- [ ] `godot` asset plugin, `surfsup` plugin with ramp and bhop generators.
- [ ] Example plugin written in Python.

## M4 Godot extension

- [ ] C++ GDExtension: `.mcmap` importer, `MapCraftMap`, `MapCraftEntityMap`.
- [ ] SurfsUp maps playable in SurfsUp v3.

## M5 Counter-Strike: Source SDK

- [ ] `source` plugin: VMF, VPK, VTF, VMT.
- [ ] Displacements, overlays, Valve compilers.

## M6 Quake SDK

- [ ] `quake` plugin: Standard and Valve220 `.map`, WAD2, PAK.

## M7 Platform

- [ ] macOS, packaging, control channel and MCP, Unity importer.
