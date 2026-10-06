# Spec 014 - A Spherical World

**Status:** Implemented, stages 0 to 6, then superseded in part by spec 015: the latitude-longitude generation grid pinwheels at the poles and a sun at the pole stripes the map. The geometry module, light as an angle from the sun, areas and great circles stand; the grid and the sun's place are spec 015's.
**Priority:** High
**Depends On:** Spec 010 (macro map), Spec 012 (shared macro data), Spec 013 (uplift terrain)
**Supersedes:** the cylinder (`wrap.rs`), the light formula in `light_level.rs`, the "terminus is an arc" invariant, `mg_life::Grid`

## Problem

Margin is a planet, but it is generated as a sheet: 1024 by 512 cells that
join east to west and stop at the top and bottom. Light is the distance, on
that sheet, from the middle of the bottom edge. Every distance, area, blur
and neighbour in the generator is measured in cells, as if every cell were
the same size and shape.

That sheet cannot be wrapped round a globe. The site's globe view
(`site/assets/map.js`) bends it round a sphere, and the whole bottom row,
1024 cells of different terrain, meets at one point: the day pole is a
pinwheel of streaks. No projection fixes that, because the data itself is
not spherical.

The lore is clear that it should be. The Geography note: "the deep south is
a scorching hot desert and the north is a freezing tundra, only a narrow
band around the equator is comfortable". The History note has the Triad go
"to the poles", one into the Wash and one into the Shade. The sub-stellar
and anti-stellar points are the poles. The terminus is the equator.

## Goal

Generate the world on a sphere. The flat map is an equirectangular
projection of it, like a map of Earth: longitude across, latitude down,
the two side edges one meridian, the top and bottom edges the two poles.
Everything that measures distance, area, slope or direction measures on the
sphere, through one geometry module. The globe view then shows the same
data without a seam or a pinch, and the flat map keeps working exactly as a
map of a round world does.

## Geometry

Decided by the lore, not open:

- **The sub-stellar point is the south pole; the anti-stellar point is the
  north pole.** Light is a function of the angle from the sub-stellar point,
  so on the flat map the light bands run straight across, and the terminus
  is the equator ring. This replaces the "terminus is an arc" invariant in
  CLAUDE.md. The ragged edges of the terminus come from noise warps on the
  sphere, as they do now, not from the geometry.
- **The grid stays 1024 by 512 cells, one cell per chunk, equirectangular.**
  Column `x` is longitude `λ = (x + 0.5) / 1024 · 2π`, row `y` is latitude
  `φ = π/2 − (y + 0.5) / 512 · π`. Row 0 touches the north pole, row 511 the
  south pole. A cell's width is `cos φ` times an equatorial cell's; its area
  is in the same proportion. Cells in the last few rows are slivers that all
  touch the pole.
- **The planet's radius is `1024 / 2π ≈ 163` world units.** One world unit
  is one chunk of 512 blocks, as now. What a block is in metres is not
  defined anywhere and this spec does not define it (at one metre per block
  the planet would be about 83 km across). The geometry module takes the
  radius as a parameter so that a later spec can change the scale in one
  place.
- **The game's chunk grid is the same equirectangular grid.** A chunk is
  still a square of 512 by 512 blocks. Near the equator that square is the
  ground it stands for. Towards the poles it stands for less and less ground
  east to west, and at the poles the last row of chunks is the same point
  1024 times over. The poles are the sub-stellar inferno and the deep night,
  where nothing lives and no one goes, so the distortion is accepted. The
  streamer must not stream past row 0 or row 511: there is nothing beyond
  the poles, as today there is nothing beyond the top and bottom edges.

### One geometry module

`mg_core::sphere` (Godot-free, so `mg_noise`, `mg_life` and `mg_web` can all
use it) is the only place that knows the projection:

| Function | Gives |
|---|---|
| `point(x, y)` | the unit vector of a cell centre |
| `point_at(wx, wy)` | the same for a world position, continuous |
| `lonlat(x, y)` | longitude and latitude |
| `cell_of(point)` | the cell a unit vector falls in |
| `angle(a, b)` | the great-circle angle between two points |
| `distance(a, b)` | that angle in world units |
| `cell_width(y)`, `cell_area(y)` | in world units and square world units |
| `neighbours(x, y)` | the eight neighbours, wrapped east to west, and across the pole in the pole rows |
| `step_distance(from, to)` | distance between neighbours, in world units |
| `noise_point(x, y, frequency)` | the 3D point to sample noise at |

Across the pole: a cell in row 0 has, as its northern neighbours, the cells
of row 0 half a world away (`x + 512`). The same for row 511. Flow, blurs
and searches then cross the poles instead of stopping at them.

`wrap.rs` goes. `mg_life::Grid` becomes a wrapper over the sphere, or goes
too; stages keep taking a grid argument and keep measuring through it.
Nothing else does modular arithmetic on `x`, clamps `y`, counts cells as
area, or uses `1 / √2` as a diagonal.

## What changes

Every item below is a flat-sheet assumption found in the code today. Each
moves onto the sphere through the geometry module.

### Noise

All 2D noise sampled on cylinder coordinates becomes 3D noise sampled at
`noise_point`. The seam blend in `light_level.rs` and the flat fallbacks in
the strategies go.

- `strategy/continentalness.rs`, `humidity.rs`, `rock_hardness.rs`,
  `peaks_valleys.rs`: fBm.
- `strategy/light_level.rs`: warps and scatter.
- `strategy/tectonic.rs`: the plate lattice, warps, boundary perturbation,
  `interior_noise`.
- `landscape.rs` `crest`, `biome_map.rs` `sea_margin_drift`, `rivers.rs`
  `meander_warp`, `wind.rs` `wander`.
- `derived/mod.rs` `derive_micro_heightmap`, used in chunks, which today
  jumps at `x = 1024`.
- `biome_map.rs:457-488`: the chunk-frequency layers that only wrap at
  frequency 1 are anchored over anyway; they sample the sphere too.

3D noise costs about twice what 2D does, but noise is a small share of
the macro pass. Measured on seed 42 with the climate pass included: 64 s
before, 62 s after. (The 35 s in spec 013 predates the climate pass.)
Budget: no slower than before. After stage 3: 59 s. A blur whose reach
grows towards the poles cost 40% more until its row pass was done by
running sums; anything else that reaches along rows should do the same.

### Tectonics

Plates become a Voronoi of points on the sphere, nearest by great-circle
angle. Each plate rotates about its own random axis (an Euler pole), which
is how plates move: the relative velocity at a boundary point `p` between
plates `a` and `b` is `(ω_a − ω_b) × p`, and the boundary normal is the
tangent direction towards the other plate's centre. Convergence and shear
follow from that as now. The flat `PlateRegistry` (centres, nearest tests,
2D velocities, unused hotspots) goes.

As built: 10 to 13 plates per seed, seeded by rejection at least 0.45
radians apart, each its own Voronoi cell (the flat model's two levels, a
lattice of cells grouped into plates, had fewer cells than plates and was
one level too many). The warps move the sample along the ground by up to
160 world units before the plates are looked up. Boundary distances are
in the flat model's lattice unit (204 world units), so its falloffs hold.

### Light

`light_level.rs`: `sun = angle(point, south pole)`, and light is
`(1 + cos(sun)) / 2`: full under the sun, half at the terminator (the
equator), none at the anti-stellar point, symmetric about the terminator.
The warps displace the point along its meridian before the angle is
measured, so the terminus stays ragged. The zone tables are unchanged.

The flat model's curve (`cos(dist · π/2)` with extra darkening) was kept at
first, but it gives 0.71 at the terminator, and on the sphere that put the
terminus zones between 25° and 58° north and left the night side an 8%
polar cap. Every other system is calibrated in light units (zones, rivers,
run-off, the sea's freezing and drying), so moving the curve moves them
all together. As built, the cosine of the angle from the sun is raised to
0.5 with its sign kept, so the light changes fastest at the terminator:
a plain cosine spread the terminus over 41% of the world. Seed 42 by
area, before and after: land 89% and 89%, liquid water 11% and 11%, day
55% and 50%, terminus 22% and 28%, night 23% and 22%.

`wind.rs` `dune_height` measures its own Euclidean distance from the sun;
it reads light or `angle` instead. Temperature, run-off, ice and the rim
sea are functions of light already and do not change. Humidity is rain
(spec 013 stages 3 and 4, `climate.rs`), carried by the wind, and moves
onto the sphere with the wind in the next section.

Two places used the row as a stand-in for night and day: `rivers.rs` forced
buried ice in the top fifth of rows and dry wadi in the bottom fifth. They
now say so in latitude (54° either way), which is the same thing.

### Area, distance and neighbours

- `drainage.rs`: `neighbours` from the sphere; flow accumulates `cell_area`,
  not a count; `step_distance` from the sphere. `FLAT_SLOPE` is per world
  unit.
- `erosion_sim.rs`: thresholds already in square world units use the
  accumulated area directly; stream power and ice fall use the sphere's step
  distance; `box_blurred` reaches a distance in world units, so its reach in
  cells grows towards the poles (`reach / cell_width(y)`), capped at half
  the row; `widen_valleys` and `crept` use the sphere's neighbours.
- `landscape.rs`: `spread` reach, `flatness`, the halving and doubling of
  the grid all through the module. Halving a lat-lon grid stays a lat-lon
  grid, so the coarse pass needs nothing new.
- `wind.rs`: `slope` and `drifted_sand` through the sphere's neighbours, so
  sand no longer piles up on rows 0 and 511. Surface wind is the gradient of
  light projected onto the tangent plane: from the anti-stellar point
  towards the sub-stellar point, along meridians, deflected by noise.
- `climate.rs`: moisture is carried downwind with its own D8 neighbour
  that clamps `y`; it uses the sphere's neighbours, so air crosses the
  poles rather than piling up on the edge rows. Moisture is an amount per
  cell; when it moves between rows of different widths it is scaled by the
  ratio of cell areas, so a plume does not concentrate as the cells shrink
  towards a pole. Its blurs (relief, rain smoothing) reach a distance in
  world units, as the erosion blurs do.
- Lakes (`sea_bodies`, `water_levels`): minimum sizes in square world
  units, filled through the sphere's neighbours.
- `rivers.rs`: accumulation ratio as a share of the planet's area; width
  from drained area in square world units; trees and paths to open water no
  longer stop at the pole rows; `near_courses`, `subdivide`, `resample` and
  Chaikin smoothing measure with `distance`, not in unwrapped cells. Rivers
  only exist near the equator, where cells are nearly square, so their look
  should not change.
- `rim_sea.rs`: the ring is the equator; the strait search runs round the
  sphere instead of from column 0 to column 1023. Seen on seed 42 after
  stage 2: the one strait it cuts through the land bridge east of the sea
  is a dead-straight east-west channel. The search should cost its steps
  by the sphere and prefer the shortest water, not the same row.
- `biome_map.rs` samplers (`sample_field_smooth`, `sample_field_bilinear`,
  `sample_heightmap_at`, `sample_biome_at_world`, `MacroOceanMask`,
  `FineHeights`, `doubled`, `refined_field`): all wrap `x` and clamp `y`
  today, one of them not even wrapping. They go through `cell_of` and
  `neighbours`. This also fixes the one-cell disagreement at the east edge
  between `sample_world_coord` (samples at `size / (count − 1)`) and the
  samplers (cell `i` at `wx = i`).
- `compute_slope_grid` and `BiomeMap::slope_at`: central differences through
  the neighbours, normalised by the sphere's step distance, so slope is per
  world unit everywhere.

### LifeGen

`mg_life` already measures through `Grid`, which was the point of `Grid`.
The grid becomes the sphere and the stages stop doing their own arithmetic:

- `analysis.rs`: the river distance field and basins use the sphere's
  neighbours and step distances; `compute_site_appeal` loses its raw `y`
  bounds.
- `provinces.rs`: seeds are drawn by area, not by cell, so the poles are not
  over-seeded; spacing by `distance`; `area` sums `cell_area`; the coast
  search reaches a distance, not a square of cells.
- `settlements.rs`: counts by area in square world units; spacing by
  `distance`.
- `roads.rs`: A* over cells with the sphere's neighbours and a great-circle
  heuristic; no unwrapped `(i64, i64)` coordinates; simplification tolerance
  in world units.
- `factions.rs`, `trade.rs`: `distance` and area in square world units.

Province and state areas in `map.json` become square world units. The
seed-42 figures in CLAUDE.md (89% land, 55/22/23 by zone) are cell counts
and will change twice over: once because they become area-weighted, once
because the terminus moves to the equator. They are re-measured, by area,
and CLAUDE.md is updated.

As built: `mg_life::Grid` stays, as the one place LifeGen measures
through, but it is now either the sphere or a flat patch (for the stage
tests, whose worlds are a few dozen cells and not spheres). Steps to a
neighbour, distances and a row's area share all go through it. Areas are
kept in equatorial cells (`area_cells`, and `area_chunks` in `map.json`):
what a count of cells would be if every cell were the size of one at the
equator. That keeps every threshold and the map's labels as they were.
The road search still runs on the flat, unwrapped grid: roads live in
the terminus, within about 30° of the equator, where a cell is within
15% of square, and a search that crossed a pole would never be wanted.
Only its link choice and lengths are great circles.

### `TerrainQuery`

The per-cell interface stays. It gains `sphere()`, returning the geometry,
so a stage never builds its own.

### Runtime

- Chunks are anchored to the same equirectangular grid, so `anchor_to_macro`
  and the pack format change only in that the samplers now come from the
  geometry module. Light is recomputed per chunk through the sphere.
- `chunk_streamer.gd` keeps its plain chunk arithmetic. Two things it must
  do that it does not today: never request rows below 0 or above 511, and
  treat column −1 as column 1023 (today they are separate keys that generate
  the same terrain).
- The sky (spec 006) places the star by light level; nothing changes until
  someone wants the star's direction in the sky to follow latitude, which
  the sphere now makes possible.

### Map, tiles, exports, sandbox

- `generate_map_tile` and `mg_web::render_tile` keep taking a world-unit
  rectangle: the storage is equirectangular. Their hillshade uses the cell
  width of the row for the east-west step, so slopes are not stretched
  towards the poles.
- The site-map export writes `projection: "equirectangular"` and the sun's
  position (south pole) into `map.json`; `chunks.bin` is unchanged.
- The site's globe view already projects the flat map onto a sphere. Its
  comment and the map page's copy about the world "really" being a cylinder
  go. The day pole looks like a pole.
- `inspect relief` gains a polar view: an orthographic render looking down
  on each pole, to check that the land is coherent there.
- The sandbox runs the same `erosion_step`, so it follows; its light comes
  from the sphere.
- The macro pack's version bumps, so stale packs are ignored as the rule
  already is.

## Stages

Each stage leaves the generator working and the map renderable.

| # | Stage | Visible result |
|---|---|---|
| 0 | `mg_core::sphere` with tests | nothing changes on the map |
| 1 | 3D noise on the sphere; `wrap.rs` goes | no east-west seam anywhere; the poles are coherent |
| 2 | light from the sphere | the terminus is the equator ring; zones re-measured |
| 3 | area-aware drainage, erosion, ice, wind, lakes, rivers | flow crosses the poles; nothing piles up on the edge rows |
| 4 | tectonics on the sphere | plates and belts continuous across the poles |
| 5 | LifeGen on the sphere; `Grid` goes | province areas in square world units; no polar over-seeding |
| 6 | exports, map, sandbox, runtime edges, CLAUDE.md | the globe has no pinch |

Stage 0 tests: `cell_area` sums to the sphere's area; `angle` between a cell
and its east neighbour is `cell_width / radius`; `neighbours` in row 0
include the cells half a world away; `cell_of(point(x, y))` is `(x, y)` for
every cell; `point_at` is continuous across the seam and over the poles.

Verification for the rest: `margins_grip inspect chunk-seam` still passes;
`inspect relief` and its new polar view; the site globe at both poles; the
area-weighted land, sea and zone shares printed by `export site-map`; the
macro pass within 60 s.

## Open questions

- **Planet scale.** The radius is 163 world units and a block has no size in
  metres. If the planet should feel larger, the way to do it is to make a
  macro cell cover more than one chunk, which is out of scope here. The
  radius parameter is the hook.
- **Polar resolution.** A lat-lon grid spends a sixth of its cells above 80°
  latitude, on a sliver of ground. An equal-area grid (icosahedral or
  HEALPix) with the flat map projected from it would be more honest and far
  more work, because chunks would no longer be cells. Recommendation: stay
  lat-lon, revisit only if the poles ever matter to play. Seen in
  `inspect relief`'s polar view after stage 6: the north pole is a coherent
  ice landscape; the south pole keeps a small pinwheel in its last rows,
  where cells are slivers and the drainage solve cannot route through them.
- **How ragged the terminus is.** With light a function of latitude, the
  warps alone make the band ragged. Their amplitude is a slider in the
  sandbox before it is a constant.
- **What the day pole looks like.** Today the bottom edge is dry wadi and
  desert. A single sub-stellar point might deserve its own treatment (a
  glassed plain, a permanent storm). Lore first. Found in stage 3: the
  surface wind converges on the sub-stellar point from every side, and the
  moisture it still carries has nowhere to go but down, so the climate pass
  rains it out there. The flat model piled the same moisture along its
  bottom row. A model that lifted converging air (the return flow) would
  carry it away instead; until then the pole is wetter than it should be.
- **Whether chunk 1023 and chunk −1 should be the same chunk at runtime.**
  This spec says yes; the streamer has never had to care.
