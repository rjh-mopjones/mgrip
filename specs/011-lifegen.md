# Spec 011 - LifeGen: Civilisation Layers

**Status:** In progress (stage 1 implemented)
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
terrain and needs no seed. `civ_seed` is introduced in stage 2.

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

### Stage 2 - Provinces

Poisson-disc seeding weighted by habitability, flood-fill tessellation over
navigation cost, borders snapped to rivers. Output: province id per cell plus
a province table (site, biome, habitability, area, political state).

### Stage 3 - Factions

Capital placement by habitability with minimum spacing, terrain-weighted
region growing over the province adjacency graph. Output: faction id per
cell, faction table. Provinces may stay unclaimed.

### Stage 4 - Settlements

Zero to three settlements per province by habitability and area, with size
class and tier. Unclaimed provinces get villages and outposts only.

### Stage 5 - Roads

A* between settlements over navigation cost.

### Stage 6 - Trade

Trade graph between settlements along roads.

## Storage

Stage 1 is cheap and is computed at export time; nothing is stored.

From stage 2 on, results are stored as a LifeGen artifact next to the layers
artifact it was built from, with `generate lifegen <layers_tag> <civ_seed>`.

## Verification

- Unit tests in `mg_life` for each stage (stage 1: river distance field,
  traversability table).
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
2. Whether macro resolution is fine enough for provinces (roughly 970 in
   Randlebrot at 8x the linear resolution).
3. How generated factions map onto the named factions in the lore.
