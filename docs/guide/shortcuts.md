# Keyboard shortcuts

Press `F1` in MapCraft for the full list, with a search box.
Every menu item, panel, tool and toggle has a shortcut you can change, and no two actions share one in any preset.
The UV editor's keys work only with the pointer over a UV editor, where they win over the same keys elsewhere.
The Vertex tool's keys (select modes, Merge and Bevel) work only in the Vertex tool, where they win the same way.

## Change a shortcut

1. Open **Edit > Preferences** (`Ctrl+Comma`), or press **Edit in Preferences** in the `F1` list.
2. Pick a preset under **Shortcuts**: MapCraft, Hammer or TrenchBroom.
3. Open **Key bindings** and type a chord such as `Ctrl+Shift+K` next to an action, or clear it to unbind the action.
4. **Reset** returns an action to its preset's chord.

Chords you type win over the preset, so they survive a preset switch.
A chord MapCraft cannot read turns red.
Menus and tooltips always show the chord that is bound right now.

## All shortcuts

These are the MapCraft preset's keys.
The tables are generated from the editor's code, so they match the version the site was built from.

<!-- shortcuts:start -->

### File

| Action | Shortcut |
|---|---|
| New Map | `Ctrl+N` |
| Open | `Ctrl+O` |
| Save | `Ctrl+S` |
| Save As | `Ctrl+Shift+S` |
| SDK Settings | `Ctrl+Shift+Comma` |
| SDK Manager | *none* |
| Quit | `Ctrl+Q` |

### Edit

| Action | Shortcut |
|---|---|
| Preferences | `Ctrl+Comma` |
| Undo | `Ctrl+Z` |
| Redo | `Ctrl+Shift+Z` |
| Delete | `Delete` |
| Duplicate | `Ctrl+D` |
| Duplicate Linked | `Alt+D` |
| Select All | `Ctrl+A` |
| Select None | `Ctrl+Shift+A` |
| Select Groups | `Alt+1` |
| Select Objects | `Alt+2` |
| Select Solids | `Alt+3` |

### Tools

| Action | Shortcut |
|---|---|
| Select Tool | `Q` |
| Move Tool | `W` |
| Extrude Tool | `E` |
| Rotate Tool | `R` |
| Scale Tool | `T` |
| Block Tool | `B` |
| Entity Tool | `N` |
| Clip Tool | `C` |
| Vertex Tool | `V` |
| Face Tool | `Shift+F` |
| Sculpt Tool | `D` |
| Path Tool | `P` |

### Tool options

| Action | Shortcut |
|---|---|
| Smart Edit | `Ctrl+Alt+E` |
| Next Block Shape | `Alt+B` |
| Next Clip Side | `Alt+C` |

### Brushes and organizing

| Action | Shortcut |
|---|---|
| Transform | `Ctrl+M` |
| Array | `Ctrl+Shift+D` |
| Replace Materials | *none* |
| Check for Problems | `Alt+P` |
| New Layer | `Ctrl+Shift+N` |
| Edit Linked Copy Here | *none* |
| Make Linked Copy Real | *none* |
| Carve | `Ctrl+Shift+C` |
| Hollow | `Ctrl+H` |
| Zone from Selection | *none* |
| Snap Selection to Grid | `Ctrl+B` |
| Group | `Ctrl+G` |
| Ungroup | `Ctrl+U` |
| Make Mirror Group | `Ctrl+Shift+M` |
| Make Mirror Real | *none* |
| Sweep Along Path | `Ctrl+Alt+S` |
| Spin | `Ctrl+Alt+N` |
| Array Along Path | `Ctrl+Alt+D` |
| Make Curve Real | *none* |
| Tie to Entity | `Ctrl+T` |
| Move to World | `Ctrl+Shift+W` |
| Rotate 90° Clockwise | `Ctrl+R` |
| Rotate 90° Counter-Clockwise | `Ctrl+Shift+R` |
| Flip X | `Ctrl+L` |
| Flip Y | `Ctrl+I` |
| Flip Z | `Ctrl+Alt+L` |
| Merge Brushes | `Ctrl+J` |
| Apply Current Material | `Shift+T` |
| Texture Lock | `Shift+L` |

### Prefabs

| Action | Shortcut |
|---|---|
| Save Selection as Prefab | `Ctrl+Shift+P` |
| Insert Prefab | `Ctrl+Alt+P` |
| Make Prefab Local | `Ctrl+Shift+U` |
| Reload Prefabs | `Ctrl+Alt+R` |

### View

| Action | Shortcut |
|---|---|
| Maximize View | `Space` |
| Link 2D Zoom | `Shift+Z` |
| Next Render Mode | `Z` |
| Textured | `Ctrl+Alt+1` |
| Flat Shaded | `Ctrl+Alt+2` |
| Wireframe | `Ctrl+Alt+3` |
| Lit | `Ctrl+Alt+4` |
| Color Brushes by Object | `Ctrl+Alt+C` |
| Frame All | `Home` |
| Go to Origin | `Shift+Home` |
| One View Layout | `Shift+Space` |
| Focus Selection | `F` |
| Find Entities | `Ctrl+F` |
| Snap to Grid | `Shift+W` |
| Snap to Corners | `Shift+Q` |
| Lock Selected | `Ctrl+Alt+K` |
| Unlock All | `Ctrl+Alt+Shift+K` |
| Hide Selected | `H` |
| Isolate Selected | `I` |
| Show All | `U` |
| Smaller Grid | `[` |
| Larger Grid | `]` |

### Panels

| Action | Shortcut |
|---|---|
| Scene Panel | `Ctrl+1` |
| Tags Panel | `Ctrl+2` |
| Materials Panel | `Ctrl+3` |
| Entities Panel | `Ctrl+4` |
| Models Panel | `Ctrl+6` |
| Output Panel | `Ctrl+5` |

### Run

| Action | Shortcut |
|---|---|
| Run Last Profile | `F9` |
| Cancel Run | `Shift+F9` |

### Help

| Action | Shortcut |
|---|---|
| Keyboard Shortcuts | `F1` |

### UV editor

| Action | Shortcut |
|---|---|
| UV Grab | `G` |
| UV Rotate | `R` |
| UV Scale | `S` |
| Unwrap | `U` |
| Project from View | *none* |
| Cube Projection | *none* |
| Reset UVs | *none* |

### Vertex tool

| Action | Shortcut |
|---|---|
| Vertex Select Mode | `1` |
| Edge Select Mode | `2` |
| Face Select Mode | `3` |
| Merge Vertices | `M` |
| Bevel Edges | `Ctrl+B` |

### Other

| Action | Shortcut |
|---|---|
| Sculpt: Raise | `1` |
| Sculpt: Lower | `2` |
| Sculpt: Smooth | `3` |
| Sculpt: Flatten | `4` |
| Sculpt: Noise | `5` |

### Mouse and fixed keys

These cannot be rebound.

| Action | Keys |
|---|---|
| Copy, or copy the active face in the Face tool | `Ctrl+C` |
| Cut | `Ctrl+X` |
| Paste, or paste a face on the picked faces in the Face tool | `Ctrl+V` |
| Cancel the current drag or tool step | `Escape` |
| Apply the clip | `Enter` |
| Extrude a face | `Shift+Drag` |
| Extrude into a new brush | `Ctrl+Shift+Drag` |
| Stamp a copy along a face | `Ctrl+Alt+Shift+Drag` |
| Pick a face's material | `Alt+Click` |
| Apply or wrap the picked material | `Alt+Right Click, Alt+Shift+Right Click` |
| Fly the 3D camera | `Right Drag + WASD` |
| Lock a UV grab or scale to U or V | `X, Y` |
| Snap a UV grab, rotate or scale | `Ctrl` |
| Apply a UV grab, rotate or scale | `Click, Enter` |
| Cancel a UV grab, rotate or scale | `Escape, Right Click` |
| Change a bevel's segments while sizing it | `Scroll` |
| Apply a bevel being sized | `Click, Enter` |
| Cancel a bevel being sized | `Escape, Right Click` |
| Move a 3D vertex handle up and down | `Shift+Drag` |
| Add a third clip point that tilts the cut | `Click in the 3D view` |

### Hammer and TrenchBroom presets

These actions change when you pick a preset; everything else keeps the keys above.

| Action | Hammer | TrenchBroom |
|---|---|---|
| Select Tool | `Shift+S` | `Q` |
| Block Tool | `Shift+B` | `B` |
| Entity Tool | `Shift+E` | *none* |
| Clip Tool | `Shift+X` | `C` |
| Vertex Tool | `Shift+V` | `V` |
| Face Tool | `Shift+A` | `Shift+F` |
| Carve | `Ctrl+Shift+C` | `Ctrl+K` |
| Hollow | `Ctrl+H` | `Ctrl+Shift+K` |
| Ungroup | `Ctrl+U` | `Ctrl+Shift+G` |
| Flip Y | `Ctrl+I` | *none* |
| Tie to Entity | `Ctrl+T` | *none* |
| Isolate Selected | `I` | `Ctrl+I` |
| Hide Selected | `H` | `Ctrl+Alt+I` |
| Show All | `U` | `Ctrl+Shift+I` |
| Smaller Grid | `[` | `Minus` |
| Larger Grid | `]` | `Plus` |

<!-- shortcuts:end -->
