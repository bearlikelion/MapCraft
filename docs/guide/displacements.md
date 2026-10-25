# Displacements

Keys on this page are the MapCraft preset's defaults.
Every shortcut can be rebound, and the Hammer and TrenchBroom presets change some of them: see [Keyboard shortcuts](shortcuts.md).

A displacement turns a four-cornered brush face into a grid of points that each rise or sink along the face's normal.
Use them for terrain, banks and other uneven ground.
They move, rotate and flip with their brushes.

## Making a displacement

1. Draw the floor brushes with the Block tool (`B`).
2. Press `Shift+F` for the Face tool and click the faces to displace in the 3D view, usually the tops; Ctrl+click picks more.
3. In the **Displacement** section at the bottom of the inspector, choose a power.
4. Click **Make**.

| Power | Grid |
|---|---|
| 2 | 5 by 5 points |
| 3 | 9 by 9 points (the default) |
| 4 | 17 by 17 points |

Only faces with exactly four corners become displacements; others are skipped, and if none qualify the status bar says so.
Switching to the Face tool with brushes selected picks every one of their faces, so click the faces you want instead, or **Make** displaces the sides too.
A brush with a displacement shows only its displaced faces, as Source compiles it.

### Changing the power

When a picked face is already a displacement, the button reads **Set power**.
It changes those faces to the chosen power, keeping their shape as closely as the new grid allows, and makes any other picked four-cornered faces displacements too.

## Removing a displacement

Pick the faces and click **Remove** in the **Displacement** section; they go back to flat faces.

## The Sculpt tool

Press `D`; the mode, **radius** and **strength** sit on the top bar.
Sculpting works in every view: point at a displacement and a yellow ring shows the brush, then drag across it to sculpt.
In a 2D view the brush works on the topmost displacement under the pointer.
Clicking where there is no displacement explains what to do in the status bar.
A stroke pauses while the pointer is off every displacement.

| Mode | Does |
|---|---|
| **Raise** | Pulls the surface up |
| **Lower** | Pushes the surface down |
| **Smooth** | Evens out bumps |
| **Flatten** | Levels the surface toward the height where the stroke began |
| **Noise** | Roughens the surface |

Press `1` to `5` to pick Raise, Lower, Smooth, Flatten or Noise; each also switches to the Sculpt tool.

- **radius** (8 to 4096 units, 128 by default) is the size of the brush.
  Its effect is full at the center and fades smoothly to nothing at the edge.
- **strength** (1 to 4096, 256 by default) is how fast it works.
  For Raise, Lower and Noise it is the height in units per second at the center; for Smooth and Flatten a higher value reaches the target sooner.
- Hold `Ctrl` while dragging to swap Raise and Lower.
- Holding the button down without moving keeps sculpting.
- A single click makes one small dab; with `Ctrl` or `Shift` held it swaps Raise and Lower.
- A whole stroke is one undo step.

### Which faces a stroke changes

A stroke changes every visible displacement within the radius, whichever brush it belongs to, measured across the face so height does not matter.
To sculpt only some displacements, pick their faces with the Face tool first, then switch to the Sculpt tool: the picks carry over, keep their cyan outline, and limit every stroke until you switch to another tool.
Picks that include no displacement limit nothing.
To keep some displacements untouched, hide them first: select their brushes and press `H`, or select the ones to work on and press `I` to isolate them.
Press `U` to show everything again when you are done.

### Sewing

Neighboring displacements stay joined.
After each dab, any point that two displaced faces share, in the same spot and facing the same way, gets one height, their average.
Sculpt across seams freely; the edges move together.

## Heightmaps

A heightmap sets displacements from a grayscale image: black pixels go to one height, white pixels to another, and grays in between.

1. In the Face tool, pick the displaced faces to cover; the image spreads over all of them together.
2. In the **Displacement** section, set **black** and **white** to the heights those colors map to (0 and 512 by default).
   Heights are measured out from the flat face along its normal.
3. Click **Load** and choose a PNG, JPEG, TGA, BMP, TIFF or EXR image.

Only the picked displaced faces change, and their old shape is replaced.
On a floor, the image's left edge is the low X side and its top is the high Y side; on a wall, its top is up.
Color images are read as gray.
Faces sharing an edge sample the same spot of the image, so they meet without a step.
