# SurfsUp sample map

`surf_mapcraft.mcmap` is a short surf course made with MapCraft and the SurfsUp SDK.

- **Course** layer: the start and finish platforms and three ramps from the Surf Ramp generator, two straight and one curving 90 degrees. Select a ramp group and change its parameters in the inspector to reshape it.
- **Gameplay** layer: the player start, the start zone on the first platform, two split zones between the ramps and the finish zone on the last platform.
- **Hazards** layer: a lava floor with a kill zone above it, which sends players back to the start.

Zones use `tools/trigger`, so the editor draws them see-through in their FGD color and the Godot extension turns them into invisible areas.

Open it with File > Open, then set the SurfsUp project folder in File > SDK Settings so the materials load.
To play it, put the MapCraft Godot addon in the SurfsUp project, open the project in the Godot editor once, set the game and Maps folder in SDK Settings, and use Run > Build and play.
