# Spec 011 - LifeGen: Civilisation Layers

**Status:** In progress (stages 1 to 4 implemented)
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
- No hand-authored factions from the lore. LifeGen generates anonymous
  factions; mapping them to named lore factions is a later spec.

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

Result on seed 42, civ seed 1: 1194 provinces.

| Mean habitability | Provinces | Median area (chunks) |
|---|---|---|
| under 0.25 | 98 | 844 |
| 0.25 to 0.40 | 366 | 693 |
| 0.40 to 0.60 | 176 | 384 |
| 0.60 and over | 554 | 108 |

### Stage 3 - Factions (implemented)

Works on the province graph, not on cells.

1. **Capitals.** Provinces with mean habitability over 0.35 are ranked by
   habitability, plus 0.2 for a major river and 0.1 for a coast. The best are
   taken in order, each at least 18.75 world units from every capital already
   chosen. Target count is 50 plus one per 80 habitable provinces (at most 30
   extra). If fewer than 50 fit, a second pass runs at half the spacing.
2. **Budgets.** Each faction may hold 5 to 25 provinces, drawn at random from
   a range set by the quality of its capital.
3. **Growth.** All factions grow at once (Dijkstra over province adjacency).
   Taking a province costs its terrain cost times 5 plus 0.4 per world unit
   between province sites. The cheapest claim wins. A faction stops at its
   budget. Provinces with habitability under 0.1 cannot be claimed.
4. **Absorption.** Leftover claimable provinces next to a faction with room
   (under 25 provinces) join the smallest such neighbour, repeated until
   nothing changes. This closes holes between factions.
5. **States.** Every province ends as Claimed (by a faction), Unclaimed
   (habitable, no faction holds it) or Uninhabited (habitability under 0.1).

API: `mg_life::generate_factions(province_map, cells_per_world_unit,
civ_seed)` returns a `FactionMap`: a political state per province and a
faction table (capital province, province count, area).

Export: a "Factions" layer (capitals marked white, unclaimed land grey), and
the faction and state of every province in `map.json`.

Differences from Randlebrot:

- **Leftover provinces are Unclaimed.** Randlebrot's code made each one its
  own single-province faction and never assigned `Unclaimed`, although its
  design said they should be. This port follows the design.
- **No generated names.** Randlebrot produced names like "Kingdom of
  Valdris". Factions here are numbered until open question 3 is settled.

Result on seed 42, civ seed 1: 64 factions, 1 to 25 provinces each (median
9). 778 provinces claimed (48% of land), 416 unclaimed (52%), none
uninhabited. Capitals sit along the terminus coasts.

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

Result on seed 42, civ seed 1: 3686 settlements. Metropolis 64, City 1043,
Town 1159, Village 1045, Outpost 375, Ruins 0.

### Stage 5 - Roads

A* between settlements over navigation cost.

### Stage 6 - Trade

Trade graph between settlements along roads.

## Storage

Stages 1 to 4 take about a second on the macro map and are computed at
export time; nothing is stored. A stored LifeGen artifact
(`generate lifegen <layers_tag> <civ_seed>`) is introduced when a stage
becomes slow or when the runtime needs to load LifeGen data.

## Verification

- Unit tests in `mg_life` for each stage (stage 1: river distance field,
  river bonuses, traversability; stage 2: full land coverage, no province
  spans ocean, determinism per seed, symmetric adjacency, islets; stage 3:
  capital spacing, no capital in barren land, growth stops at barren land,
  province limit per faction, determinism per seed; stage 4: counts and
  sizes by habitability, site spacing, settlements stand in their own
  province, one metropolis per faction).
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
3. How generated factions map onto the named factions in the lore.
4. No province is Uninhabited. The 0.1 habitability threshold is below every
   province's mean on this world (the lowest are about 0.2), so factions can
   expand to the sub-stellar point and the deep night. A threshold near 0.25
   would leave about 98 provinces uninhabited.
5. Unclaimed land is one undifferentiated state. The lore has nomads and
   orders operating there; nothing represents them yet.
6. Settlement counts are high for the lore's population of about a million:
   1043 cities and 64 metropolises. Every settlement in a province over 0.7
   habitability is a city, and 554 provinces qualify. Sizes probably need to
   be rarer, or scaled to faction size.
7. No ruins are generated: no province is below 0.05 habitability. The lore's
   ruins (elevator wreckage, Himaya-era sites) are authored, not derived from
   habitability, so this class may need a different source.
