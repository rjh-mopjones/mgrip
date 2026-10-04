# Spec 011 - LifeGen: Civilisation Layers

**Status:** All six stages implemented at macro resolution. Calibration open (see Open questions).
**Priority:** Medium
**Depends On:** Spec 010 (macro map), `mg_core::TerrainQuery`

## Problem

Margin's Grip has terrain but nothing living on it. The Bevy prototype
(Randlebrot) had a LifeGen pipeline that turned terrain into provinces,
factions, settlements, roads and trade. None of it was ported. The world map
therefore shows only terrain layers, and nothing downstream (settlement
scenes, the deterministic world simulation, lore placement) has data to build
on.

## Goal

Port LifeGen from Randlebrot's `rb_world::lifegen` into this repo in stages,
each stage producing map layers that can be inspected on the site before the
next stage builds on it.

## Non-goals

- No runtime use yet. The Godot game does not read LifeGen data in this spec.
- No settlement scene generation (SceneGen) and no world simulation (DeterSim).
- Only the lore's territorial states are represented. Orders (guilds,
  religious orders, networks) and nomads hold no territory; the lore models
  them as presence overlays and pressure fields, which are not built.

## Ownership

- `gdextension/crates/mg_life` owns LifeGen. It depends on `mg_core` only and
  reads terrain through `TerrainQuery`. It must not import `mg_noise` types
  and must stay free of Godot dependencies.
- `mg_noise` owns the `TerrainQuery` implementation for `BiomeMap`.
- The CLI owns rendering LifeGen grids to images and exporting them.

Terrain is immutable input. LifeGen never writes back into terrain layers.

## Resolution

Randlebrot ran LifeGen on a grid of 8 cells per world unit (8192x4096). This
port runs on the macro map: 1 cell per chunk (1024x512).

Every distance and slope constant from Randlebrot is kept, expressed in world
units, and rescaled by the grid's `cells_per_world_unit`. LifeGen can move to a
finer grid later without retuning.

Consequence at macro resolution: features narrower than one chunk are lost.
The river-proximity bonus (1.5 world units) reaches only cells next to a river.

## Topology

Margin's surface is a cylinder. The macro map's east and west edges are
neighbours; its north and south edges are not. Terrain is generated that way
(`mg_noise::wrap`), so LifeGen must treat it that way.

`mg_life::Grid` holds the resolution (`cells_per_world_unit`) and whether the
grid is a ring (`wrap_width`). Every stage takes a `Grid`, and every distance
between cells and every step to a neighbouring column goes through it:

- `Grid::dx` and `Grid::distance` measure the short way round.
- `Grid::step_x` steps to a neighbouring column, wrapping on a ring and
  returning nothing off the edge of a flat grid.

`Grid::flat` gives a grid with no joined edges (tests, or a map that is only
part of the world). `Grid::ring` gives the macro map's shape.

What wraps: river distance, province seeding and tessellation, coast
detection, capital spacing, faction growth cost, settlement spacing, road
links, highway waypoints, route search, and trade distance. A province,
faction or road can span the seam.

Roads are routed in unwrapped coordinates (a column may lie beyond the east
or west edge) so a route across the seam is continuous, then folded back
onto the grid. A stored road path may therefore jump between the two edges.

## Seeds

LifeGen uses its own `civ_seed`, separate from the terrain seed, so politics
can be regenerated without changing terrain. Stage 1 is a pure function of
terrain and needs no seed. From stage 2 on, each stage derives its own random
stream from `civ_seed` (stage 2 uses `civ_seed + 2`, stage 3 `civ_seed + 3`).

## Stages

### Stage 1 - Analysis grids (implemented)

Pure functions of terrain. One `f32` per cell, range 0 to 1.

| Grid | Meaning | Inputs |
|---|---|---|
| River distance | Distance to nearest river cell (helper, not exported) | rivers |
| Habitability | How well a cell supports settlement | temperature 35%, water 30%, elevation 20%, stability 15% |
| Navigation cost | Ease of travel: 0 impassable, 1 trivial | biome traversability, slope, elevation, rivers |
| Resource desirability | Geological resource potential | tectonic 40%, rock hardness 25%, erosion 20%, fertility 15% |

Ocean cells score 0 for habitability and navigation cost.

API: `mg_life::compute_analysis_grids(terrain, cells_per_world_unit)`.

Export: `margins_grip export site-map` renders the three grids as PNGs under
a "LifeGen" layer group.

### Stage 2 - Provinces (implemented)

Every land cell belongs to exactly one province; ocean belongs to none.

1. **Seeding.** Dart throwing over land. A dart is accepted if no existing
   seed lies within the larger of the two seeds' radii. Radius falls with
   habitability, so habitable land gets small dense provinces and barren land
   large sparse ones. Seeding stops when 20,000 land darts in a row are
   rejected.
2. **Tessellation.** All seeds grow at once (Dijkstra). A cell joins the
   province that reaches it most cheaply; step cost is the inverse of
   navigation cost, so borders tend to follow hard terrain.
3. **Islets.** Land no seed reached joins the nearest province across water.
4. **Attributes.** Per province: seed site, dominant biome, mean habitability,
   area, coastal flag, major-river flag, mean elevation, mean terrain cost.
   Province adjacency is recorded for stage 3.

API: `mg_life::generate_provinces(terrain, analysis, cells_per_world_unit,
civ_seed)`.

Export: a "Provinces" layer, the province id of every chunk in `chunks.bin`,
and the province table in `map.json`. `export site-map --civ-seed <n>`
(default 1).

Differences from Randlebrot:

- **Seed radii are 2 to 38 world units**, not 0.5 to 10. Randlebrot relied on a
  cap of 1200 provinces. On this world (89% land) its radii would produce
  about 15,800 provinces, and the cap is reached before habitable land fills
  up, which makes all provinces the same size. Scaling the radii by 3.8 lets
  seeding saturate near the same count on its own. The cap (4000) is now only
  a safety bound.
- **No border snapping to rivers and no micro-tile snapping.** Both worked at
  sub-chunk distances (0.375 and 2 world units). At one cell per chunk,
  borders are already chunk-aligned and river snapping has nothing to act on.
  Revisit if LifeGen moves to a finer grid.
- **Islets are attached** to the nearest province instead of left without one.

Result on seed 42, civ seed 1: 1181 provinces, 33 of them spanning the seam.
Small where habitable (median about 110 chunks over 0.60 habitability),
large where barren (median about 840 under 0.25).

### Stage 3 - Factions (implemented)

Works on the province graph, not on cells. A faction is a territorial state.
There are two kinds: authored states, which are the named states from the
lore, and generated minor states, which fill the rest of the habitable land
up to the target count.

**Authored states** are listed in `gdextension/data/lifegen_states.ron`
(compiled into the CLI). Each has a name, an optional capital city name, a
light-level band it lives in, a size and a list of preferences:

| Field | Meaning |
|---|---|
| `light` | Band of light level (0.0 deep night to 1.0 sub-stellar). The capital is placed inside it and growth outside it is penalised |
| `size` | CityState (1-3 provinces), Small (5-9), Medium (11-17), Large (20-25) |
| `prefers` | Coastal, MajorRiver, Mountains, Resources, Fertile: what the capital province should have |

Authored states are placed first, in file order, so earlier states get first
choice. A state's capital is the best-scoring free province in its band:
habitability, plus 0.3 for a wanted coast or major river, plus elevation,
half the resource score or half the habitability again for Mountains,
Resources or Fertile. Capital spacing is tried at 18.75 world units, then
half, then none; a state is only left out if its band holds no free province,
and is then reported as unplaced. Authored states get ids 1 to N in file
order.

The steps below then run for all factions together.

1. **Generated capitals.** Provinces with mean habitability over 0.35 are ranked by
   habitability, plus 0.2 for a major river and 0.1 for a coast. The best are
   taken in order, each at least 18.75 world units from every capital already
   chosen. Target count is 50 plus one per 80 habitable provinces (at most 30
   extra). If fewer than 50 fit, a second pass runs at half the spacing.
2. **Budgets.** Each faction may hold 5 to 25 provinces, drawn at random from
   a range set by the quality of its capital.
3. **Growth.** All factions grow at once (Dijkstra over province adjacency).
   Taking a province costs its terrain cost times 5 plus 0.4 per world unit
   between province sites. For an authored state, a province whose light
   level lies outside the state's band costs a further 100 per unit of light
   outside it, so states spread along their band before leaving it. The
   cheapest claim wins. A faction stops at its budget. Provinces with
   habitability under 0.1 cannot be claimed.
4. **Absorption.** Leftover claimable provinces next to a faction with room
   join the smallest such neighbour, repeated until nothing changes. This
   closes holes between factions. Generated states have room up to 25
   provinces; authored states only up to their own budget, so a city-state
   stays a city-state.
5. **States.** Every province ends as Claimed (by a faction), Unclaimed
   (habitable, no faction holds it) or Uninhabited (habitability under 0.1).

API: `mg_life::generate_factions(province_map, authored_states,
cells_per_world_unit, civ_seed)` returns a `FactionMap`: a political state
per province, a faction table (name, capital name, capital province, province
count, area) and the names of any unplaced states.

Export: a "Factions" layer (capitals marked white, unclaimed land grey), the
faction and state of every province in `map.json`, and name labels for the
authored states on the map page.

Differences from Randlebrot:

- **Leftover provinces are Unclaimed.** Randlebrot's code made each one its
  own single-province faction and never assigned `Unclaimed`, although its
  design said they should be. This port follows the design.
- **Authored states.** Randlebrot had none; every faction was generated.
- **No generated names.** Randlebrot produced names like "Kingdom of
  Valdris". Generated minor states here are numbered.

Result on seed 42, civ seed 1: 64 factions (19 authored, 45 generated), 1 to
25 provinces each (median 10). 754 provinces claimed (44% of land), 427
unclaimed (56%), none uninhabited. All 19 authored states were placed. Six
factions span the seam, among them Furrow, Hollowvein Republic, Nightwall
Covenant and Shuttered Hearth.

| Authored state | Provinces | Capital light | Capital habitability |
|---|---|---|---|
| Corazon | 22 | 0.33 | 0.88 |
| Furrow | 5 | 0.34 | 0.85 |
| Tidewall | 3 | 0.31 | 0.81 |
| Vestara | 1 | 0.35 | 0.85 |
| Ashenmere | 6 | 0.30 | 0.79 |
| Cinderline | 10 | 0.52 | 0.68 |
| Breakwater | 16 | 0.29 | 0.88 |
| Ashward Dominion | 17 | 0.58 | 0.65 |
| Emberspike Regime | 12 | 0.59 | 0.64 |
| Searing Compact | 15 | 0.82 | 0.27 |
| Kermans | 5 | 0.32 | 0.78 |
| Radiant Ordinance | 7 | 0.46 | 0.74 |
| Hollowvein Republic | 7 | 0.31 | 0.78 |
| Nightwall Covenant | 16 | 0.28 | 0.86 |
| Shuttered Hearth | 5 | 0.32 | 0.76 |
| Quiet Holdings | 5 | 0.32 | 0.81 |
| Umbral Sovereignty | 20 | 0.11 | 0.46 |
| Frostdelve Communion | 7 | 0.07 | 0.41 |
| The Pale | 9 | 0.20 | 0.61 |

Several medium states ended below their size range (Furrow 5, Ashenmere 6,
Kermans 5): neighbours hemmed them in before their budget was spent.

### Stage 4 - Settlements (implemented)

Habitability decides how big settlements are, not whether they exist.
Nothing in this stage is random.

1. **Count.** Three per province; four if mean habitability is over 0.5, five
   if over 0.7; one if the province is under 3.125 square world units.
2. **Sites.** The most habitable cells of the province, each at least 7.5
   world units from the others. A province with no room for its full count
   gets fewer.
3. **Size.** From the province's mean habitability: over 0.7 City (also over
   0.5 with a major river), over 0.3 Town, over 0.15 Village, over 0.05
   Outpost, otherwise Ruins. The first site in a faction's capital province
   is a Metropolis.
4. **No faction, nothing large.** In Unclaimed and Uninhabited provinces,
   cities and towns become villages and villages become outposts.

API: `mg_life::place_settlements(province_map, faction_map, analysis,
cells_per_world_unit)` returns settlements with a cell position, province
and size class.

Export: a "Settlements" layer (dots over a dimmed faction map), the
settlement list in `map.json`, and a nearest-settlement readout.

Difference from Randlebrot: its separate `SettlementTier` mapped one-to-one
onto `SizeClass`, so only the size class is kept.

Result on seed 42, civ seed 1: 3654 settlements. Metropolis 64, City 1029,
Town 1125, Village 1061, Outpost 375, Ruins 0.

### Stage 5 - Roads (implemented)

Which settlements are linked is decided on straight-line distance. Each
link is then routed over the navigation grid, so roads bend around hard
terrain and never cross open water. Nothing in this stage is random.

1. **Links.**
   - A minimum spanning tree over all settlements, so everything connects.
   - A spanning tree over faction capitals, plus one extra link from each
     capital to its nearest unlinked capital within 187.5 world units.
   - Up to two extra links from each town or larger to the nearest towns or
     larger within 37.5 world units.
2. **Kinds.** Highway between two capitals; Road if either end is a capital
   or city; Trail otherwise.
3. **Highway waypoints.** A highway is split to pass through towns or larger
   lying within 25 world units of its straight line, at least 7.5 apart.
4. **Routing.** A* (8-connected) inside a box around the two ends padded by
   25 world units. Stepping onto a cell costs the step length divided by its
   navigation ease. Links over 250 world units, and links with no land
   route, are dropped.
5. **Simplification.** Douglas-Peucker with a tolerance of 0.375 world units.

API: `mg_life::build_roads(settlements, navigation_cost, width, height,
cells_per_world_unit)` returns roads with the settlement at each end, a
kind, a simplified path and a travel cost.

Differences from Randlebrot: kinds are named Highway, Road and Trail instead
of Imperial, Provincial and Trail; each road keeps the two settlements it
joins and one cost for the whole route, instead of being flattened into
anonymous segments.

Result on seed 42, civ seed 1: 6865 roads. Highway 385 (7,496 chunks),
Road 2772 (20,418 chunks), Trail 3708 (41,375 chunks). 49 settlements are on
no road (islands).

### Stage 6 - Trade (implemented)

Every settlement sends its trade to the nearest strictly larger settlement
it can reach by road. Flows only go from smaller to larger, so they form a
directed acyclic graph. The value of a flow is the mean habitability of the
source settlement's province.

API: `mg_life::build_trade_flows(settlements, roads, province_map)`.

Difference from Randlebrot: its trade stage was a stub that treated any two
settlements near any road as connected. Here two settlements are connected
only if the road network joins them (union-find over roads).

Result on seed 42, civ seed 1: 3415 flows. 239 settlements send none (the 64
capitals, which have nothing larger to send to, and settlements with no
larger settlement on their road network).

Export for stages 5 and 6: "Roads" and "Trade" layers. The trade layer draws
each flow as a straight line from source to market; the line is schematic
and may cross water that the road does not.

## Storage

All six stages together take about a second on the macro map and are computed at
export time; nothing is stored. A stored LifeGen artifact
(`generate lifegen <layers_tag> <civ_seed>`) is introduced when a stage
becomes slow or when the runtime needs to load LifeGen data.

## Verification

- Unit tests in `mg_life` for each stage (stage 1: river distance field,
  river bonuses, traversability; stage 2: full land coverage, no province
  spans ocean, determinism per seed, symmetric adjacency, islets; stage 3:
  capital spacing, no capital in barren land, growth stops at barren land,
  province limit per faction, determinism per seed, authored states placed in
  their band and by preference, city-states stay small, growth follows the
  band, unplaceable states reported; stage 4: counts and
  sizes by habitability, site spacing, settlements stand in their own
  province, one metropolis per faction; stage 5: routes pass through gaps
  and never cross water, every settlement connected on open ground, road
  kinds, highway waypoints, simplification; stage 6: trade goes to the
  nearest larger settlement on the same road network, capitals send none;
  topology: distances and steps on a flat grid and on a ring, roads and
  highway waypoints across the seam).
- `margins_grip inspect layer-stats <layers_tag>` prints layer percentiles,
  LifeGen grid percentiles and province counts and areas.
- Each stage's layers are inspected on the site map page against the terrain
  layers.

## Calibration against this world

Measured with `margins_grip inspect layer-stats <layers_tag>` on seed 42.
Differences from Randlebrot that the port accounts for:

- **Tectonic layer is inverted.** Here it stores distance from the nearest
  plate boundary (1.0 = quiet interior; land median 0.977). LifeGen expects
  stress. The `TerrainQuery` implementation returns `1.0 - layer`.
- **Ocean is liquid surface water only.** `is_ocean` uses
  `tile_has_fluid_surface`, the same rule as `MacroOceanMask`. Dried dayside
  basins (`ScorchedRock`, `SaltFlat`: 97-98% below sea level) and frozen
  nightside seas are land. 89% of cells are land by this rule.
- **`White` is crossable.** It is the most common biome (half frozen sea, half
  frozen land). Traversability 0.2: harder than ordinary snow and ice (0.3),
  not a wall. Randlebrot treated it as impassable.

Resulting land-cell distributions (p5 / median / p95): habitability
0.20 / 0.36 / 0.75, navigation cost 0.09 / 0.29 / 0.85, resource
desirability 0.25 / 0.34 / 0.55.

## Open questions

1. Temperature comfort peaks at 15 C and reaches zero at -20 C and 50 C. The
   temperature model here is wider (-119 C to 123 C on land) than
   Randlebrot's (-40 C to 40 C). Habitability still concentrates in the
   terminus, but the band has not been tuned.
2. Province scale. The smallest provinces are about 30 chunks, the median
   about 290. Whether that is the right size for a province is a design call;
   it is set by the two seed radii.
3. The light bands, sizes and preferences in `lifegen_states.ron` are a first
   reading of the lore, not checked against it state by state.
4. No province is Uninhabited. The 0.1 habitability threshold is below every
   province's mean on this world (the lowest are about 0.2), so factions can
   expand to the sub-stellar point and the deep night. A threshold near 0.25
   would leave about 98 provinces uninhabited.
5. Unclaimed land is one undifferentiated state. The lore has nomads and
   orders operating there; nothing represents them yet.
6. Settlement counts are high for the lore's population of about a million:
   1029 cities and 64 metropolises. Every settlement in a province over 0.7
   habitability is a city, and 554 provinces qualify. Sizes probably need to
   be rarer, or scaled to faction size.
7. No ruins are generated: no province is below 0.05 habitability. The lore's
   ruins (elevator wreckage, Himaya-era sites) are authored, not derived from
   habitability, so this class may need a different source.
8. The lore's great railway and its neutral operators are not represented.
   Highways are the nearest equivalent.
9. Trade has no goods. A flow is one number. The economy in the lore
   (energy, food, steel, water, graphene) would need resource types per
   province and flows per resource.
10. Major rivers are rare: about 18 provinces carry the flag (a river cell
    draining over 2000 cells). No province with one fell inside Corazon's or
    Furrow's band with enough habitability, so neither capital is on a major
    river although both prefer one.
11. Fixed in terrain: light level (and humidity, which derives from it) had a
    visible seam where the world wraps, because its warp and scatter noise is
    planar. It is now crossfaded across the last 64 world units before the
    east edge. Layers artifacts made before the fix still carry the seam.
12. `BiomeMap`'s `slope_at` clamps at the east and west edges instead of
    wrapping, because a `BiomeMap` can also be a single tile that is not a
    ring. Slope in the two edge columns of the macro map is slightly off.
13. Lore relationships are not used: Vestara is not placed at a crossroads,
    rivals are not placed next to each other, and Quiet Holdings is one
    contiguous state rather than a scattered network.
