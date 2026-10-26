# Counter-Strike: Source

The Counter-Strike: Source SDK lets you make CS:S maps in MapCraft.
It lists the game's own materials, opens and writes Hammer `.vmf` files, and compiles with Valve's vbsp, vvis and vrad.
Nothing of the game is copied into MapCraft: materials, entity definitions and compilers are read from your own install.

## Setup

1. Install Counter-Strike: Source from Steam.
   The compilers (`vbsp.exe`, `vvis.exe`, `vrad.exe`) and the entity definitions (`cstrike.fgd`) live in the `bin` folder of the Windows version of the game, next to the `cstrike` folder.
   On Linux, install the Windows version (in Steam, force a Proton compatibility tool for the game) and install Wine, which runs the compilers.
2. Make a new map, pick **Counter-Strike: Source** in the SDK list and click **Create**.
3. In **File > SDK Settings**, set **Counter-Strike: Source game folder** to the `cstrike` folder, such as `.../steamapps/common/Counter-Strike Source/cstrike`.
   The SDK reads the game's packages from it (`cstrike_pak_dir.vpk`) and from the `hl2` folder beside it (`hl2_textures_dir.vpk`, `hl2_misc_dir.vpk`), and the entity definitions from `bin/cstrike.fgd`.

New brushes get `dev/dev_measuregeneric01b` at Hammer's usual texture scale of 0.25.

## Materials

The Materials tab lists the game's materials with a picture each, taken from the texture their `.vmt` file names (Hammer's tool picture for tool materials such as `tools/toolsnodraw`).
Loose materials in `cstrike/materials` and `cstrike/download/materials` are listed too.

- Folders that never go on brushes are left out: `models`, `vgui`, `hud`, `console`, `particle`, `sprites` and `overviews`.
- Pictures are small, about 64 pixels on a side for the stock game, so that thousands of them fit in memory.
- A material whose texture is missing or in a format MapCraft cannot read shows as gray; the Output panel counts them.
- The first listing after the computer starts reads a little from thousands of places in the packages, which takes about 15 seconds from a hard disk; later ones take about half a second.

Because the 3D view sizes each material by its picture, a texture shown from a small picture repeats more often in the 3D view than in the game.
The texture axes themselves are right, and they are what the `.vmf` and the compiled map use.

## Open a .vmf

Choose **File > Import > Valve map (VMF)** and pick a `.vmf` file, from Hammer, Hammer++ or a decompiler such as BSPSource.

- World brushes and entities come in with every key value, brush entities with their brushes, hidden objects too.
- Faces keep their material and texture axes (offset, scale and rotation included, since the axes carry the rotation).
- Material names come in lower case with forward slashes, as the game looks them up.
- Visgroups become groups named after them, nested ones as `Parent / Child`; an object in several visgroups goes into the first.
- The Output panel sums up what came in and what was skipped.

## Export and compile

**File > Export > Valve map (VMF)** writes a `.vmf` that Hammer and Hammer++ open:

- the world's key values and brushes, then every entity with its key values and brushes;
- layers and groups as nested visgroups;
- every side with its plane, material, texture axes, rotation 0 and lightmap scale 16;
- ids counted up in order, so exporting the same map twice gives the same file.

**Run > Compile** exports the map to a `build` folder beside it, runs vbsp, vvis and vrad on it (through Wine on Linux and macOS), and copies the `.bsp` into `cstrike/maps`.
Start the game and type `map <name>` in the console to play it.

## What is lost

A `.vmf` holds more than a MapCraft plugin can pass to the editor, so these are skipped on import and listed in the Output panel:

- **Displacements:** their brushes come in flat.
- **Entity outputs** (the `connections` block): triggers and buttons lose their outputs.
- **Worldspawn keys** such as `skyname`: set them again in the inspector with nothing selected.
- **Per-face lightmap scale and smoothing groups:** exports write lightmap scale 16 and no smoothing.
- **Hammer groups:** their contents come in ungrouped.
- **Cameras, cordons and Hammer's view settings.**
