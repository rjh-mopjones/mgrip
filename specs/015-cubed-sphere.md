# Spec 015 - The Cubed Sphere

**Status:** Proposed
**Priority:** High
**Depends On:** Spec 014 (the sphere: geometry module, light as an angle, areas and great circles)
**Supersedes:** spec 014's latitude-longitude generation grid, and its sun at the pole

## Problem

Spec 014 put the world on a sphere, but generated it on the flat map's own
grid: 1024 columns of longitude by 512 rows of latitude. Two things went
wrong, and both were visible on the globe and the map.

1. **The poles pinwheel.** On that grid a cell near a pole is a sliver,
   many times taller than it is wide, and the last row is 1024 cells that
   are all one point. Water routed to one of eight neighbours cannot follow
   the ground through slivers; the drainage and the provinces come out as
   spokes radiating from each pole. This is the known failing of
   latitude-longitude grids, and no amount of care in the solve fixes it.
2. **The map lost its character.** With the sun exactly at the south pole,
   light is a function of latitude alone, so every system calibrated on
   light (the sea's freezing and drying, the ice, the desert, the zones)
   comes out as horizontal stripes. The old map's terminus was an arc, and
   that arc did real work: coasts and continents crossed zones instead of
   lying in bands.

Both are solved problems elsewhere. Procedural planets with erosion are
generated on a **cubed sphere**: six square faces, each a plain raster, so
erosion stays image processing on a grid and there is no singularity
anywhere (Spaceframe; Michelic 2018). NOAA's FV3 weather model runs on the
equiangular cubed sphere, whose worst cell edge ratio is about 1.3 and
worst aspect ratio 1.41. Hydrology on a sphere uses icosahedral hexagonal
grids (ISEA3H), which are equal-area with uniform adjacency, but every one
of this generator's kernels (D8 routing, box blurs, bilinear sampling,
tiles) assumes a rectangular raster, and hexagons would mean rewriting them
rather than re-indexing them. HEALPix is equal-area but built for spherical
harmonics, not neighbour-heavy simulation.

## Goal

Generate the world on an equiangular cubed sphere. Keep the flat map as it
is: a 1024 by 512 equirectangular map that wraps east to west, with night
in the north and day in the south, and a terminus that arcs across it. The
map, the chunks and the site's tiles are all read from the cube by sampling
it at a world position; nothing downstream of generation changes its
coordinates.

## Geometry

Spec 014's geometry stands: Margin is a sphere of circumference 1024 world
units, a world position is a longitude east and a distance south from the
north pole in world units, and `mg_core::sphere` is the one place that
knows that projection. This spec adds the generation grid beside it.

### The cube

Six faces, each `N` by `N` cells, `N = 296` for the macro map (six times
296 squared is 525,696 cells, the same count as 1024 by 512). The fine grid
is four cells per macro cell each way, `N = 1184`, as spec 013's fine grid
was. The cube's axis is the sphere's: the `+Z` and `−Z` faces are centred
on the north and south poles, the four side faces on the equator at
longitudes 0°, 90°, 180° and 270°. Face coordinates are equiangular: a
cell's centre is at angles `(α, β)` in `(−π/4, π/4)`, spaced evenly, and
its point on the sphere is the face's frame applied to
`(tan α, tan β, 1)`, normalised. Cell edge lengths vary by at most 1.3 to
1 across the whole sphere and no cell is a sliver.

### One module

`mg_core::cube` is the only place that knows the cube:

| Function | Gives |
|---|---|
| `cell_count()`, `index(face, u, v)`, `cell(index)` | the flat layout: face-major, row-major within a face |
| `point(index)` | a cell centre on the unit sphere |
| `cell_of(point)` | the cell a point falls in: the face from the largest axis, then `(α, β)` |
| `neighbours(index)` | the eight neighbours in `D8_OFFSETS` order, across face edges; a corner cell has seven, the missing diagonal is `None` |
| `step_distances(index)`, `area(index)` | in world units and square world units, exact |
| `halo(face, width)` | a face's raster padded with `width` cells of its neighbours, so a kernel written for a raster runs on it unchanged |
| `sample(field, point)`, `sample_smooth(field, point)` | bilinear and cubic B-spline interpolation of a cell field at a point, across edges |
| `halved()`, `doubled()` | the cube at half and twice the resolution, with the resampling |

Everything that measures or steps goes through it. Nothing else indexes
`y * width + x` on macro data.

### The flat map

The game's chunk grid and the site's map stay equirectangular, 1024 by 512,
one chunk per cell. Anything that wants macro data at a world position
turns the position into a point on the sphere (`sphere::point_at`) and
samples the cube there. Images of the whole world are made by sampling
the cube at every pixel's point. Chunks near a pole are still slivers of
ground, as in spec 014, and that is still accepted: the game never streams
past a pole and nothing lives there.

### The sun

The sub-stellar point sits at the bottom centre of the map, 45° up from
the south pole: longitude 512 world units, latitude −45°. Light is the
angle from it, as spec 014 made it. The terminator is then a great circle,
which on the flat map is an arc: at the map's centre it stands at 45°
north, at the map's edges at 45° south, with the night deepest at the top
edges and the day hottest at the bottom centre. That is the shape the old
flat-distance light drew, now as the geometry of a lit planet. The tilt is
a constant (`SUN_LATITUDE`) and a calibration: 45° reproduces the old
arc's swing; 90° is spec 014's pole sun and its stripes.

## What changes

### The macro map

`BiomeMap` today is one struct for both the macro map and a chunk's tile,
indexed by `width` and `height`. It splits:

- `MacroMap`: every per-cell layer over the cube's cells, with the cube
  beside it. `generate_macro_map(seed)` builds this.
- `BiomeMap` stays what chunks and map tiles are: a raster of samples in
  world coordinates, anchored to the macro map by sampling it.

The macro pack stores the `MacroMap` (format `MGMP06`), fine heights per
face included. `to_biome_map` goes; anchoring and tiles take a `&MacroMap`.

### The macro pass

Stage by stage of spec 013's chain, on the cube:

- **Base layers** (noise, tectonics, light): one value per cell at the
  cell's point. Already sphere-sampled; only the loop changes.
- **Rim sea**: the cheapest crossing is found on an equirectangular raster
  of continentalness sampled from the cube, as now, and the straits are
  written back to the cells under them. The ring it opens is the one round
  the terminus, whatever its tilt.
- **Landscape** (uplift, erosion, drainage): `solve_drainage` takes the
  neighbour table; flow gathers by cell area; stream power divides by the
  true step. Blurs run per face on a haloed raster. The coarse pass halves
  the cube (296 to 148) and the fine pass doubles it twice (to 1184). Ice
  flow, valley widening and creep use the neighbour table.
- **Rivers**: the tree is built over cell indices and the paths are the
  cells' world positions, so the course machinery (meanders, smoothing,
  courses, rasterising into tiles) is unchanged. Rivers cross face edges as
  they cross anything else.
- **Wind and climate**: the wind is a tangent vector at each cell's point;
  the eight directions are the cell's neighbours, by the angle between the
  wind and each step. Sand and moisture carried between cells scale by the
  ratio of the two areas, as spec 014 made them.
- **Biomes, flatness, sand**: per cell, with slopes from the neighbour
  table.

### Everything that reads the macro map

- Anchoring (`anchor_to_macro`), map tiles (`generate_map_tile`,
  `mg_web::render_tile`), `FineHeights::sample`, lake levels, the ocean
  mask: sample the cube at the world position's point.
- The layers artifact, the site-map export and `inspect relief`: images by
  sampling at each pixel; `chunks.bin` the same, one sample per chunk.
- LifeGen keeps running on the 1024 by 512 raster sampled from the cube,
  through its sphere grid (spec 014 stage 5). Its provinces still radiate
  at the poles, which are now ordinary night and day interiors; running
  LifeGen on the cube's cells is a later spec if that ever matters.
- The sandbox runs the real erosion step, so it moves to the cube too, at
  the coarse resolution, and renders an equirectangular image.
- The site's globe view needs nothing: it already projects the flat map.

### Seams

There are no east-west seams and no poles in generation any more, but
there are twelve face edges and eight corners. `inspect chunk-seam` stays
the test for chunk borders; a new `inspect cube-seam <seed>` samples the
macro heightmap along every face edge from both sides and reports the
largest step, as the chunk test does. Both must report zero.

## Stages

| # | Stage | Visible result |
|---|---|---|
| 0 | `mg_core::cube` with tests: layout, points, neighbours, areas, halos, sampling, halving and doubling | nothing changes on the map |
| 1 | `MacroMap` on the cube: base layers, light with the tilted sun, the pack, images by sampling | the terminus arcs across the map again; the land is still spec 014's lat-lon land |
| 2 | landscape, drainage, erosion, rivers on the cube | no spokes at the poles in `inspect relief`'s polar view |
| 3 | wind, climate, sand, biomes on the cube | the full macro pass on the cube; shares by area re-measured |
| 4 | anchoring, tiles, the sandbox, the exports | the game, the site and the sandbox read the cube |
| 5 | `inspect cube-seam`, CLAUDE.md, spec 014 marked superseded where it is | done |

Stage 0 tests: every cell is found under its own centre; the areas sum to
the sphere's; the largest edge ratio is under 1.35; every cell has eight
neighbours except the twenty-four at corners, which have seven; a face's
halo matches its neighbours' cells; a constant field samples constant
everywhere, a linear-in-point field samples within a tolerance across
every edge; halving then doubling a smooth field returns it closely.

## Open questions

- **The tilt.** 45° is set to match the old arc. It also decides how much
  of the world is night: the anti-stellar point moves up the map as the
  tilt grows. Measure the shares by area at 30°, 45° and 60° before fixing
  it.
- **Resolution.** 296 cells a side matches today's cell count. The fine
  grid at 1184 a side is 8.4 million cells, as now. Both could change
  independently of the map's 1024 by 512.
- **LifeGen on the cube.** Not in this spec. Provinces are seeded by area
  already; the radiating borders at the poles are the lat-lon raster
  LifeGen reads, not the land.
- **The runtime's polar chunks.** Still slivers of ground. A chunk grid on
  the cube would end that, and would change what a chunk coordinate is for
  every script. Not this spec.
