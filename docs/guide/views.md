# Views and navigation

Keys on this page are the MapCraft preset's defaults.
Every shortcut can be rebound, and the Hammer and TrenchBroom presets change some of them: see [Keyboard shortcuts](shortcuts.md).

## The four views

The editor opens with four views: 3D at the top left, Top at the top right, Front at the bottom left and Side at the bottom right.

| View | Across | Up | Seen from |
|---|---|---|---|
| Top | X | Y | above |
| Front | Y | Z | the +X side |
| Side | X | Z | the -Y side |

- Click a view's name in its header to turn it into a 3D, Top, Front or Side view, or into the UV editor (see [The UV editor](materials.md#the-uv-editor)).
- Drag the gaps between the views, or the spot where they cross, to resize them.
- The view under the pointer gets an orange outline and title; keys such as `Space` and the arrow keys act on it.
- A 2D view's header shows its two axes and its scale in map units per pixel.

## Maximize

Press `Space` to fill the window with the view under the pointer, and `Space` again to bring back all four.
The square button at the right end of each header does the same, as does **View > Maximize View Under Pointer**.

### One view layout

**View > One View Layout** (`Shift+Space`) keeps a single view filling the window, as in TrenchBroom.
`Space` then shows the next view in turn (3D, Top, Front, Side), and the status bar names the one shown.
The square button in the header, or `Shift+Space` again, brings back all four views.

## Go to Origin

**View > Go to Origin** (`Shift+Home`) frames the area around the map's origin in every view, for when you lose your place.
**View > Frame All** (`Home`) frames everything in the map, and goes to the origin too when the map is empty.

## Moving around a 2D view

- Scroll to zoom in and out around the pointer.
- Right drag or middle drag to pan.
- Double click to focus on the selection.
- The arrow keys nudge the selection one grid step along the view's axes.
- The status bar shows the pointer's position next to **Cursor**.

### Linked zoom

While zoom is linked, zooming one 2D view sets every 2D view to the same scale.
It is on by default.
Toggle it with `Shift+Z`, the chain button in a 2D view's header, or **View > Link 2D Zoom**.

## Flying the 3D camera

| Do this | To |
|---|---|
| Hold the right button and move the mouse | Look around |
| Hold the right button and press `W`, `A`, `S`, `D` | Fly forward, left, back and right |
| Hold the right button and press `Q` or `E` | Sink or rise straight down or up |
| Add `Shift` while flying | Fly three times as fast |
| Middle drag | Pan |
| Scroll | Move forward or back |

While the right button is down, single key shortcuts pause so those keys only fly; chords with `Ctrl` still work.
Set the fly speed in **Edit > Preferences** (`Ctrl+Comma`); it is 512 units per second by default.

### The axis widget

The bottom left corner of each 3D view shows which way X (red), Y (green) and Z (blue) point.
Click an axis to look along it from its positive end, at whatever is in the middle of the view, from the same distance.
Click it again to look from the negative end.

## Render modes

3D views draw in one of four modes, shared by every 3D view:

| Mode | Key | Shows |
|---|---|---|
| Textured | `Ctrl+Alt+1` | Shaded faces with their materials |
| Flat Shaded | `Ctrl+Alt+2` | Shaded faces without materials |
| Wireframe | `Ctrl+Alt+3` | Edges only |
| Lit | `Ctrl+Alt+4` | Materials with their normal, roughness, metallic and occlusion maps, lit by the map's lights |

Press `Z` to step to the next mode, or pick one from the mode menu in a 3D view's header or the **View** menu.
In Textured, Flat Shaded and Lit, only the selection's edges are drawn.
2D views always draw edges.

### Lit

Lit previews lighting and materials without compiling.
Faces are shaded the way game engines such as Godot shade them, with metallic and roughness maps, normal maps for surface detail, and ambient occlusion that darkens ambient light in creases.
Each face takes light from the map's point lights, spotlights and sun, with distance falloff, plus a faint ambient so dark corners stay readable.

A map without light entities gets a stand-in rig, so materials can still be judged: a warm sun from high above at +X +Y (where Textured's light comes from), a weaker cool fill from the opposite side, and sky and ground ambient that is bluish from above and brownish from below.
Once the map has a light, Lit uses only the map's lights.

Lit reads the usual Source and Quake light keys, so it works with any SDK whose lights follow either convention:

| Key | Read as |
|---|---|
| classname | `light` or `light_*` is a light, `light_spot` a spotlight and `light_environment` the sun; any entity with `_light` is a light too |
| `_light` | Source color and brightness, `r g b brightness`; a lone number is a Quake brightness |
| `light`, `_color` | Quake brightness (300 when missing) and color, `r g b` from 0 to 255 or 0 to 1 |
| `wait`, `delay` | Quake falloff distance scale and falloff curve (linear, inverse, inverse square or none) |
| `angles`, `pitch` | Source spotlight and sun aim; pitch -90 points straight down |
| `_cone`, `_inner_cone` | Source spotlight cone, in degrees from its axis |
| `mangle`, `angle`, `_softangle` | Quake spotlight aim (yaw, pitch, roll) and cone |
| `_ambient` | Ambient color and brightness, `r g b brightness` |

Source lights fall off with the inverse square of distance.
Only the 64 lights nearest the camera shine at once, and nothing casts shadows, so a sun also lights faces indoors.
Entity boxes stay unlit.

#### Material maps

The maps come from the SDK's material plugin, and are read when a map first uses the material:

- **Godot materials** (`.tres`): `normal_texture` when `normal_enabled` is on, `roughness_texture` and `metallic_texture` with their channels, `ao_texture` when `ao_enabled` is on, and the `roughness` and `metallic` values, which multiply their maps and default to 1 and 0 as in Godot.
- **Loose image sets** with no `.tres`, named as ambientCG names them: next to `Bricks_Color.jpg`, the files `Bricks_NormalGL`, `Bricks_NormalDX`, `Bricks_Roughness`, `Bricks_Metalness`, `Bricks_AmbientOcclusion`, or a packed `Bricks_ORM` or `Bricks_ARM` (occlusion, roughness and metallic in red, green and blue).
- Normal maps are OpenGL style, green pointing up the picture, as Godot expects; a DirectX map (`NormalDX`) is flipped, and `NormalGL` wins when a set has both.
- Source and Quake materials have no such maps and draw fully rough and not metallic.

Normal and roughness maps are loaded at up to 1024 pixels on a side.

## Framing

- `Home` (**View > Frame All**) centers every view on the whole map.
- `F` (**View > Focus Selection**) centers every view on the selection, or on the whole map when nothing is selected.
- Double click a 2D view to focus on the selection, or a row in the Scene panel to select that node and focus on it.

2D views zoom to fit; the 3D view keeps its angle and backs away until the target fits.

## Grid size

The grid is 16 units by default and MapCraft remembers it between sessions.

- `[` halves it and `]` doubles it, from 0.125 up to 4096 units.
- The **-** and **+** buttons next to **Grid** on the top bar do the same.
- **Edit > Preferences** sets an exact size.

When a 2D view is zoomed out so far that grid lines would crowd together, it draws a coarser grid and shows that size in orange in its header.
Drags in that view snap to the coarser grid; zoom in to snap to the real one.

## Map limits

When the SDK's game limits how far a map may reach, such as Quake's -4096 to 4096 on every axis, every view outlines that box in red.
**Tools > Check for Problems** (`Alt+P`) reports brushes that reach past it and point entities that sit outside it, since the game's compiler or engine would fail on them.

## Snapping

Both snapping toggles are on by default and sit on the top bar.

- **Snap** (`Shift+W`, **View > Snap to Grid**) puts dragged, drawn and placed things on the grid, and turns rotation in 15 degree steps.
- **Corners** (`Shift+Q`, **View > Snap to Corners**) lets a point dragged in a 2D view catch a brush corner or entity origin within 8 pixels of it, as long as that corner is nearer than the closest grid point.

While you drag, an orange cross marks the grid point you will land on and a yellow ring marks a caught corner.
When you move a selection, the corner of it nearest where you grabbed it is the point that snaps, so grab it near the corner you want to line up.

To put objects that are already off the grid back on it, use **Tools > Snap Selection to Grid**; see [Tools](tools.md#brush-operations).

## Hide, isolate and show

| Command | Key | Does |
|---|---|---|
| **View > Hide Selected** | `H` | Hides the selection |
| **View > Isolate Selected** | `I` | Hides every brush, entity and prefab outside the selection |
| **View > Show All** | `U` | Shows everything hidden this way or with the eye icons |

The eye icon on each row of the Scene panel hides or shows that one node.
Hidden things are not drawn and cannot be clicked, box selected or snapped to.
Hiding and showing are undo steps like any edit.
Tags and cordons hide whole sets of objects at once; see [Organizing](organizing.md).

## Color by object

`Ctrl+Alt+C` (**View > Color Brushes by Object**) gives the edges of each world brush a color of its own, so neighboring brushes stand apart.
It is on by default.
The colors show in the 2D views and in the 3D Wireframe mode.
Brushes inside an entity keep the entity's color, and selected edges keep the selection color.

## The status bar

The bottom bar shows what is selected, the pointer's position in a 2D view, the grid size, the selection mode, whether texture lock is on, and the SDK.
Its right side shows a hint for the current tool, or for a few seconds a message such as an error.
**Output** at the far right shows or hides the Output panel.
