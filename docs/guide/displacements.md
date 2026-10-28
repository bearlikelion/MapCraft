# Terrain

MapCraft makes terrain, such as hills, banks and mountains, from displacements: a four-cornered face becomes a grid of points that each move up or down.
Shape them with the Sculpt tool or a grayscale heightmap image, and paint a second material into them.

Displacements move, rotate and flip with their brushes.

![Sculpted rock mountains around a surf course, in the Lit 3D view](img/terrain-mountains.webp)

## Making a displacement

1. Draw the floor with the Block tool (`B`).
2. Press `Shift+F` for the Face tool and click the faces to displace, usually the tops. Ctrl+click picks more.
3. Unfold the **Displacement** section at the bottom of the inspector, choose a **Power** and click **Make**.

| Power | Grid |
|---|---|
| 2 | 5 by 5 points |
| 3 | 9 by 9 points (the default) |
| 4 | 17 by 17 points |

Only faces with exactly four corners can become displacements; the status bar says when none can.
A brush with a displacement shows only its displaced faces, as Source does.

![The Face tool with a platform's top face picked, and the Displacement section in the inspector with Power 3 and Make](img/terrain-make.webp#wide)

> [!WARNING]
> Switching to the Face tool with brushes selected picks every face of them. Click only the faces you want, or **Make** displaces the sides too.

### Changing the power

When a picked face is already a displacement, the button reads **Set power**.
It changes those faces to the new power and keeps their shape as closely as the new grid allows.

## Removing a displacement

Pick the faces and click **Remove** in the **Displacement** section.
They go back to flat faces.

## The Sculpt tool

Press `D`.
Point at a displacement and a yellow ring shows the brush; drag across it to sculpt.
It works in every view, and in a 2D view it shapes the topmost displacement under the pointer.

Pick a mode on the top bar, or press `1` to `7`:

| Mode | Key | Does |
|---|---|---|
| **Raise** | `1` | Pulls the surface up |
| **Lower** | `2` | Pushes the surface down |
| **Smooth** | `3` | Evens out bumps |
| **Flatten** | `4` | Levels the surface toward the height where the stroke began |
| **Noise** | `5` | Roughens the surface |
| **Paint** | `6` | Paints the current material into the surface; see [Painting a blend](#painting-a-blend) |
| **Blur** | `7` | Softens the edges of a painted blend |

- **radius** (8 to 4096 units, 128 by default) is the brush size, faded toward its edge as the falloff sets.
- **falloff** is how it fades: **Smooth** (the default) eases out, **Linear** fades evenly, **Sharp** fades fast near the center, and **Constant** works fully all the way to the edge.
- **strength** (1 to 4096, 256 by default) is how fast the shaping modes work: for Raise, Lower and Noise, the units per second at the center.
- Hold `Ctrl` while dragging to swap Raise and Lower.
- Holding the button still keeps sculpting; a single click makes one small dab.
- A stroke pauses while the pointer is off every displacement, and a click off them explains what to do in the status bar.
- A whole stroke is one undo step.

![The Sculpt tool raising hills on a displacement, with the yellow brush ring in the 3D and Top views and the modes, radius and strength on the top bar](img/terrain-sculpt.webp#wide)

### Which faces a stroke changes

A stroke changes every visible displacement inside the ring, whichever brush it belongs to.
To limit it:

- **Pick first:** pick faces with the Face tool, then switch to Sculpt. The picks keep their cyan outline and limit every stroke until you switch tools. Picks with no displacement in them limit nothing.
- **Hide the rest:** select brushes and press `H` to hide them, or `I` to isolate the ones you want. Press `U` to show everything again.

### Seams

Neighboring displacements stay joined.
Where two displaced faces share a point, both use one height, so sculpting across a seam leaves no gap.

## Heightmaps

A heightmap shapes displacements from a grayscale picture: black is low, white is high, grays in between.

1. In the Face tool, pick the displaced faces to cover. The picture spreads over all of them together.
2. In the **Displacement** section, set **Black height** and **White height** to the heights those colors become (0 and 512 by default).
3. Click **Load heightmap** and choose a PNG, JPEG, TGA, BMP, TIFF or EXR picture.

Only the picked faces change, and their old shape is replaced.
Heights are measured out from the flat face.
On a floor, the picture's left edge is the low X side and its top is the high Y side; on a wall, its top is up.
Color pictures are read as gray, and faces that share an edge meet without a step.

![A new displacement grid on the platform, with the Heightmap row in the inspector: black 0, white 512 and Load](img/terrain-heightmap.webp#wide)

## Blending textures

A displacement can blend a second material into its own, such as grass over the flat parts of a rock hill.
Each vertex holds the second material's share, from none of it to all of it, and the Textured and Lit views draw the mix.
Where the two meet, the brighter parts of each picture show through first, so the edge follows the textures instead of a smooth fade.

### Painting a blend

1. Make the second material current: Alt+click a face that shows it, or click it in the Materials panel while nothing is selected or picked.
   A click in the panel also puts the material on the selected brushes and picked faces, which leaves them nothing to blend.
2. Press `D` for the Sculpt tool, then `6` for **Paint**.
3. Drag across a displacement.

The first dab on a face makes the current material its second material, and the stroke paints it in under the ring.
Painting never moves a point, and a whole stroke is one undo step.

- **flow** (1 to 100 %, 50 % by default) is how much one dab paints, and how fast Blur softens.
- **opacity** (1 to 100 %, 100 % by default) is the most one stroke changes a point, so going back over a spot in the same stroke never paints past it; release and paint again to go further.
- **spacing** (5 to 100 % of the radius, 25 % by default) is the distance between dabs along a stroke, so a fast stroke paints as much as a slow one, and holding still paints nothing more.
- **Blur** (`7`) softens the edges of a blend for as long as you hold the button, on any displacement.
- Hold `Ctrl` as you start a stroke to paint the face's own material back over its second material for that whole stroke.
- Alt+click a blended face to make the material that shows most there the current one.
- The eye button, **Show weights**, tints each point cyan by its share of the second material while Paint or Blur is on.
- The status bar reads the blend under the pointer, such as `rock031 + grass: 62%`.

What a dab does depends on the current material:

- On a face whose second material is the current one, it paints more of it.
- With the face's own material current, it paints the own material back, but only where the face already blends; on a face with no second material it paints nothing.
- On a face that blends a different material, it paints nothing; pick that face and use **Tools > Displacement > Blend Current Material** to switch it.
- Tool materials, the SDK's skies and materials the game cannot blend cannot be painted.

When a stroke paints nothing, the status bar says why once.
Faces picked with the Face tool limit the stroke, as they limit sculpting.

### Filling a blend

1. Make the second material current, as in [Painting a blend](#painting-a-blend), before you pick anything.
2. In the Face tool, pick the displaced faces.
3. In the inspector's **Displacement** section, click the bucket on the **Blend material** row, or use **Tools > Displacement > Blend Current Material**.
4. Choose how **Blend fill** sets each point's share, then click **Fill blend**:
   - **Amount** gives every point the same share, 100 % by default.
   - **Slope** starts to show the second material where the ground is steeper than **From** degrees and shows only it from **To**; set **From** above **To** to put it on flat ground instead.
   - **Height** does the same between two heights in map units, which start at the picked faces' lowest and highest points.
   - **Noise** scatters it in patches about **Size** units wide; another **Seed** gives other patches, and **From** and **To** set how much of the noise shows it.
5. **Invert blend** swaps the two materials' shares, and **Clear blend** shows only each face's own material again.

**Load blend mask…** stretches a grayscale picture over the picked faces, white for the second material, as **Load heightmap** does for heights.
The trash button on the **Blend material** row, or **Tools > Displacement > Remove Blend Material**, takes the second material away and keeps the shares.
**Tools > Displacement > Fill Blend** always fills 100 %, and **Invert Blend** and **Clear Blend** match the buttons.
A face's own material, tool materials, the SDK's skies and materials the game's plugin says cannot blend, such as Godot shader materials, cannot be a second material.
AI agents can do the same with the `blend` and `heightmap` tools; see [MCP server](../mcp-server.md).

### Seams and games

Neighboring faces with the same material and second material share one value where they meet, while a blend meets a different material with a hard edge.
Merging, clipping or carving a brush so that a displaced face changes shape flattens that face and drops its blend, and the status bar says how many faces that hit.

Godot draws the blend as the views do, when both materials are plain `StandardMaterial3D`s; see [Blended terrain](godot.md#blended-terrain).
Source exports the blend as alphas, which the game draws only through a WorldVertexTransition material; see [Displacements](cstrike.md#displacements).
