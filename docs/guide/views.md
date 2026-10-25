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

- Click a view's name in its header to turn it into a 3D, Top, Front or Side view.
- Drag the gaps between the views, or the spot where they cross, to resize them.
- The view under the pointer gets an orange outline and title; keys such as `Space` and the arrow keys act on it.
- A 2D view's header shows its two axes and its scale in map units per pixel.

## Maximize

Press `Space` to fill the window with the view under the pointer, and `Space` again to bring back all four.
The square button at the right end of each header does the same, as does **View > Maximize View Under Pointer**.

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

3D views draw in one of three modes, shared by every 3D view:

| Mode | Key | Shows |
|---|---|---|
| Textured | `Ctrl+Alt+1` | Shaded faces with their materials |
| Flat Shaded | `Ctrl+Alt+2` | Shaded faces without materials |
| Wireframe | `Ctrl+Alt+3` | Edges only |

Press `Z` to step to the next mode, or pick one from the mode menu in a 3D view's header or the **View** menu.
In Textured and Flat Shaded, only the selection's edges are drawn.
2D views always draw edges.

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
