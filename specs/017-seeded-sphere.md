# Spec 017 - The Seeded Sphere

**Status:** Implemented, stages 1 to 3 (2026-10-07). Open: polar insets on the flat map, the knit
**Priority:** High
**Depends On:** Spec 015 (generation on the cubed sphere), spec 013 (the grown landscape), spec 011 (LifeGen)
**Supersedes:** spec 016 (polar caps on the flat sheet; not merged), and the part of spec 015 that grew the world from flat

## Constraints

Three things are fixed, by decision on 2026-10-07:

1. **The planet is a sphere, and the sphere is the source of truth.** The
   flat map is a picture of it. Nothing is generated on a flat raster.
2. **The land masses of the flat map stay.** The continents, coasts and
   ranges around the terminus that the flat generation produced (main's
   `generate_macro_map`, seed 42) are kept, because they are good ground
   for play. Fine valleys may change; coasts and ranges may not.
3. **Provinces on a sphere are provinces on a sphere.** No province may
   radiate from a pole. LifeGen runs on the cube's cells, where a cell at
   a pole is an ordinary cell.

## Why the earlier attempts failed

- Spec 014 and 015 regenerated the world from flat on the sphere. The
  continents kept their outlines (the same noise, the same seeds) but
  every valley, coast and range was eroded afresh, and the map lost its
  character.
- Spec 016 kept the flat sheet and glued caps beyond it. The caps came
  from a different process than the land they met, and the seam showed.
- In every version LifeGen still ran on the 1024 by 512 latitude-longitude
  raster, where a polar cell is a sliver. The province pinwheel was that
  raster, never the terrain. Spec 015 left it for later; it should have
  been first.

## The seed

The flat map's land is exported once as a seed file (`export seed-land
<layers-tag> <file>` on main's generator): continentalness, tectonic,
plate ids, rock hardness, peaks and valleys and the grown heightmap, one
32-bit float per cell of the 1024 by 512 map (`mg_noise::seed_land`). The
sphere branch never runs the flat generator; it reads this file.

The flat map maps onto the sphere by the equirectangular projection spec
014 gave it: column to longitude, row to latitude, pole to pole. In the
band the noise layers already agree, because both sample the same 3D noise
at the same point of the sphere. The seed matters for what the flat
generation decided on its own: the plates (flat Voronoi, not the sphere's
Euler plates) and above all the heightmap.

## Generation (stage 1)

`MacroMap::generate_seeded(seed, seed_land)` is spec 015's generation with
the seed in place of flat land:

1. **Base layers.** The cube's own sampling, then the seed's value laid
   over it with weight `hold`: 1 up to latitude 70°, fading to 0 by 85°.
   Plate ids are taken from the seed outright where it holds. Beyond 85°
   the sphere's own layers stand, and they are proper noise on the sphere,
   not the sheet's pinched rows.
2. **The landscape** runs in two phases over the whole sphere at once.
   First the coarse pass grows the planet from flat with every held cell
   (`hold = 1`) pinned at the seed's height, so the poles grow against the
   real continents and drain through them. Then the fine pass starts from
   the seed where it holds, blended into the grown ground where it fades,
   and erodes freely for its forty steps: the knit. The band changes only
   as much as forty steps of uplift and erosion change a mature landscape,
   which is little; the poles arrive as part of the same world.
3. Refinement, climate, rivers, wind, sand and biomes run as spec 015
   has them. The sun stands 45° up from the south pole, so the terminus
   arcs across the map as it does on the flat map.

The result is checked by eye against main's map over the terminus before
anything else is built: `generate layers <seed> <tag> --seed-land <file>`
writes the usual artifact, and its `macromap.png` is set beside main's.

## Stage 2: LifeGen on the cube

`mg_life` already measures distance and area through a `Grid`. It still
indexes cells as `(x, y)` in the basin fill, site scoring, settlement
placement and road search. Stage 2 gives it a cube-backed grid whose cells
are indices with neighbours across face edges, and runs every stage on the
macro cube's cells. Provinces are then drainage basins of the sphere, and
at a pole they are the size and shape of provinces anywhere. The site map
reads province ids from the cube's faces on the globe and from a sampled
band on the flat map.

## Stage 3: the flat map as a band

The flat map shows rows between 75° north and south; the rows beyond are
the poles stretched across the sheet and are not shown. The globe draws the
sheet to 50° and the cube's own polar faces beyond, as `cap-north.png` and
`cap-south.png` with their province ids, so the poles are the cube's ground
with the cube's provinces. Chunk coordinates in the band are what they are
now; the game never streams past 75°. Polar insets on the flat map are not
drawn yet.

## Open questions

- **How far the knit goes.** Forty fine steps is spec 013's own fine pass.
  Fewer keeps more of the seed; more lets the sphere have its way. Judged
  from the side by side.
- **Where the hold fades.** 70° to 85° keeps every inhabited row exactly
  and lets the sphere own the poles.
