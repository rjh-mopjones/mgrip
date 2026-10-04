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
   into, so rivers are unbroken, then resampled every 0.08 world units.
2. **Meander.** Every point is moved by a noise warp of the plane: a broad
   sweep (2 world units) plus a shorter wiggle (0.4). The warp depends only on
   position, so a tributary and the river it joins move together and still
   meet. Amplitudes are small enough that the warp never folds, so rivers do
   not cross themselves, and they stay near the valleys the flow solve found.
3. **Width.** Half-width runs from 0.08 world units for the smallest
   headwater to 1.0 for the largest river, by the square root of drainage,
   scaled by the river's character (seasonal, frozen and so on). Drainage
   grows along a segment from what flows in at its head to its own total, so
   a river widens downstream and a tributary is never wider than its trunk.
4. River systems that never reach Strahler order 3 are not drawn.

On any grid a river covers at least half a sample either side of its centre
line, so a river thinner than a cell still marks the cells it runs through.

On the macro map the river layer's value is the river's size as a share of
the largest possible river (0 = no river). `TerrainQuery::river_at` returns
it, and LifeGen reads major rivers from it.

## Chunk borders

`anchor_to_macro` uses the same sample positions as `BiomeMap::generate`, so
a chunk's last column and its neighbour's first column are sampled at the
same world positions. `margins_grip inspect chunk-seam <seed> <x> <y>`
reports how many border samples differ in block height; it should be zero.

## Results on seed 42

- Rivers are dendritic lines that widen downstream, on the map and in tiles.
- 44 provinces have a major river (18 before). Corazon and Furrow, the
  lore's river states, both have their capital on one.
- 0 of 512 border samples differ on every chunk border checked.
- Macro pack: 14.5 MB. World ready in 1.3 s natively and 2.0 s in the browser.

## Open questions

1. Some trunk rivers end on land a little short of open water. The flow
   solve treats cells below sea level as the sea, but some of those cells are
   drawn as land (coast, dried basin).
2. Rivers are drawn across dry dayside terrain as narrow seasonal channels.
   The river invariants say surface rivers exist only in the terminus.
3. In the game a river is a biome colour on the ground, not water.
4. The river code assumes a 1024 by 512 macro grid; other sizes panic.
