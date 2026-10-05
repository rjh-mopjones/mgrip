# Spec 012 - Shared Macro Data and River Courses

**Status:** Implemented
**Priority:** High
**Depends On:** Spec 010 (macro map), Spec 011 (LifeGen)

## Problem

Three things made the map and the game disagree.

1. **The game read its macro data from the home directory.** Runtime chunks
   were anchored to whichever layers artifact was newest under
   `~/.margins_grip/`. The web build has no such file, so its chunks were
   never anchored: no macro ocean mask, no rivers.
2. **Rivers were a one-cell-per-chunk mask.** The macro river raster clamped
   every river's half-width to exactly one world unit, ignoring the Strahler
   width table, then meandered each river by about ten world units. Rivers
   came out as two-chunk-wide braided blobs covering 3.4% of the world. Map
   tiles and game chunks copied that mask by nearest neighbour, so every
   river was a staircase of chunk-sized squares.
3. **Anchoring put steps in the terrain at chunk borders.** It sampled the
   macro map at cell centres, while generation and the mesh builder put a
   tile's first and last sample on its edges. About one border sample in
   eight ended up a block or more away from its neighbour.

## One macro map

`mg_noise::generate_macro_map(seed)` is the single definition of the macro
world map: the whole world at one cell per chunk, with erosion and the global
river network. Layers artifacts, macro packs and the runtime all call it.

## Macro pack

A macro pack is the part of the macro map the game needs, in one file:

- the layers that anchoring and macro sampling read (continentalness,
  tectonic, humidity, rock hardness, peaks and valleys, heightmap, rivers,
  temperature, aridity) as 32-bit floats, and biomes
- the river courses (below)
- a low-resolution probe of the terrain

`margins_grip export macro-pack <seed> <file>` writes one. The game loads
`res://data/macro/seed_<seed>.mgmacro` at startup, in native and web builds
alike, and passes it to `MgTerrainGen.prepare_macro`.

A pack is rejected if it is for another seed, or if regenerating the probe
gives different terrain (the generator has changed since the pack was made).
With no valid pack the game generates the macro map on first use: correct,
but about 7 seconds natively. Generated macro data is passed through the
pack format too, so loaded and generated data are bit-identical.

The game no longer reads `~/.margins_grip/`.

## Erosion and drainage

Rivers and valleys come from one drainage solve (`mg_noise/src/drainage.rs`),
so rivers lie in the valleys that were cut for them. Before, erosion carved
the heightmap with one flow solve and the river network was then built from
a second solve with different rules, so drawn rivers were not the channels
the erosion made.

Drainage (priority-flood, then steepest descent; Barnes et al. 2014, Braun
and Willett 2013):

1. Flood the land upwards from base level, lowest cell first, raising every
   hollow to the level at which it spills. The order cells are reached in
   runs from lowest to highest.
2. Each cell drains to its lowest neighbour on the flooded surface.
3. Each cell starts with its own run-off, and from the highest cell down
   passes what it holds to the cell it drains to.

Base level is the sea: bodies of water of at least 12 cells. Run-off is
fullest in the terminus and scaled by humidity; the night side keeps 5% of
it (ice) and the deep day side 8% (evaporation).

Erosion (`erosion_sim.rs`) runs 100 steps. Each step lifts the land where
tectonic stress is high, solves drainage, lowers every cell towards the cell
it drains to (the stream power law, solved implicitly from the sea upwards so
large time steps stay stable), and lets slopes creep. It starts from the
noise heightmap rather than from flat ground, so the relief still agrees
with the layers the biomes are classified from.

The river network is read from the drainage of the eroded ground. A river
forms where gathered run-off reaches 26 (0.005% of the world's cells, in
units of one well-watered terminus cell).

## River courses

A river is the same river at every scale. Each river segment's final course
is built once, in world space, as a line of points with a half-width at each.
Every raster of the rivers samples those courses at its own resolution:

- the macro map's river layer (one cell per chunk)
- map image tiles
- game chunks

Whether a point is in a river depends only on its world position, so scales
cannot disagree and neighbouring tiles meet exactly.

How a course is built from a river segment:

1. The segment's path is carried on to the head of the segment it flows
   into, so rivers are unbroken, then resampled every 0.05 world units.
2. **Meander.** Every point is moved by a noise warp of the plane: a broad
   sweep (2 world units) plus a shorter wiggle (0.4). The warp depends only on
   position, so a tributary and the river it joins move together and still
   meet. Amplitudes are small enough that the warp never folds, so rivers do
   not cross themselves, and they stay near the valleys the flow solve found.
3. **Width.** Half-width runs from 0.02 world units for the smallest
   headwater to 0.2 for the largest river (about 20 and 200 blocks across),
   by the square root of drainage,
   scaled by the river's character (seasonal, frozen and so on). Drainage
   grows along a segment from what flows in at its head to its own total, so
   a river widens downstream and a tributary is never wider than its trunk.
4. River systems that never reach Strahler order 3 are not drawn.
5. A river's final segment straightens over its last 6 world units, so the
   meander cannot swing the mouth away from the water.

## Where rivers run

A river is drawn only where its water stays liquid all the way to a body of
water. This enforces the river invariants in `CLAUDE.md`.

- **Liquid on land:** light level 0.18 to 0.62 and temperature 0 C to 42 C.
  These are the light levels at which the sea freezes over and at which
  shallow sea dries out, so rivers and seas agree on where water is liquid.
- **A body of water** is a connected stretch of at least 12 cells below sea
  level. Anything smaller is a pond. For drainage a pond counts as land, so
  rivers run through ponds and on to the sea. Before this, the largest river
  on seed 42 drained into a single cell that happened to lie below sea level.
- **Each segment** carries a surface river from the first point after which
  its whole path is liquid, provided every segment downstream is liquid from
  end to end and the last one meets a body of water whose water is liquid at
  that point. `RiverSegment::surface_from` records this. So rivers start
  where the country turns wet enough, and none stops on dry land. A river
  that starts partway along a segment starts at minimum width.
- **Mouths** are carried past the coast to open water (more than 0.05 below
  sea level, or the deepest water within 8 cells). Where the coast is drawn
  is not exactly where ground drops below sea level, so stopping at the first
  cell below sea level could leave a mouth on dry land. The site map export
  clips rivers at the coast.

River systems that drain towards the day side or the night side, or into a
frozen or dried-out sea, are not drawn at all.

On any grid a river covers at least half a sample either side of its centre
line, so a river thinner than a cell still marks the cells it runs through.

On the macro map the river layer's value is the river's size as a share of
the largest possible river (0 = no river). `TerrainQuery::river_at` returns
it, and LifeGen reads major rivers from it.

## Map tiles

`mg_noise::generate_map_tile` makes terrain for any rectangle of the world at
any resolution, anchored to the macro map. `macromap.png` is assembled from
such tiles, and the website's map renders more of them in the browser as the
view zooms in (`gdextension/crates/mg_web`, compiled to WebAssembly, reading
the macro pack).

- Tiles coarser than 12 pixels per world unit use the overview settings
  `macromap.png` is made with. Sharper tiles use the settings the game
  generates chunks with, so close up the map shows the ground a player
  would walk on.
- Every tile is coloured against one height range, that of the macro map
  (`NormalizationHints::for_macro_map`), so tiles agree on colour.
- Browser tiles are shaded with relief lit from the north-west. Relief is
  exaggerated 30 times at one pixel per world unit and less the sharper the
  tile. For shading only, tile heights rest on a smooth (cubic B-spline)
  sampling of the macro heightmap: the bilinear sampling terrain uses is
  creased along every chunk edge and shades as square facets.

Anchoring takes every base layer, and so every biome, from the macro map.
Zooming in therefore adds finer height detail and smoother boundaries, not
new features: there is nothing in the world smaller than a chunk except
noise.

## Chunk borders

`anchor_to_macro` uses the same sample positions as `BiomeMap::generate`, so
a chunk's last column and its neighbour's first column are sampled at the
same world positions. `margins_grip inspect chunk-seam <seed> <x> <y>`
reports how many border samples differ in block height; it should be zero.

## Results on seed 42

- Rivers are dendritic lines that widen downstream, on the map and in tiles.
- 202 river courses, about 1,650 world units in all, every one in the
  terminus. 25 mouths; the furthest ends 1.4 chunks from the sea as drawn.
- 29 provinces have a major river. Corazon and Furrow, the lore's river
  states, both have their capital on one.
- 0 of 512 border samples differ on every chunk border checked.
- Macro pack: 14.5 MB. World ready in 1.3 s natively and 2.0 s in the browser.

## Open questions

1. Erosion starts from noise terrain. Growing the land from uplift alone
   (as in the Randlebrot prototypes) would give cleaner mountain ranges, but
   biomes are classified from the noise layers, so terrain and biomes would
   disagree until classification reads the real elevation.
2. Hollows are filled level, so there are no lakes other than ground below
   sea level.
3. Whole river systems are dropped if any stretch downstream is too dry or
   too cold. An alternative is to let such rivers end in a terminal lake,
   which the invariants currently forbid.
4. In the game a river is a biome colour on the ground, not water.
5. The river code assumes a 1024 by 512 macro grid; other sizes panic.
