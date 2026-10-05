# Spec 013 - Terrain From Uplift and Erosion

**Status:** In progress. Stage 0 (sandbox) and stage 1 (uplift terrain in the generator, rivers only, stopgap biomes) done. Stages 2 to 6 not started.
**Priority:** High
**Depends On:** Spec 010 (macro map), Spec 012 (drainage, river courses)
**Supersedes:** the noise heightmap, the noise-driven biome inputs, and the "every river reaches the sea" invariant

## Goal

Every feature of the land should be there for a reason another system can
read. Mountains stand where the crust is pushed up. Valleys are where rivers
cut. Ridges are the ground between valleys. Biomes, provinces, settlements
and roads are then placed on that land using the same fields that shaped it.

Today each layer is its own noise (continents, peaks, humidity, an erosion
"amount") and biomes are classified from those. Erosion scratches valleys
into noise hills, and the rivers were, until spec 012, a separate solve. The
result is terrain that looks the same everywhere.

Reference: the Randlebrot uplift-erosion prototype (Braun and Willett 2013;
Cordonnier et al. 2016), where land starts flat, is lifted, and rivers cut it
until the two balance.

## The chain

Each stage is computed from the ones before it. Nothing downstream of
tectonics has its own noise.

| # | Stage | Reads | Gives |
|---|---|---|---|
| 1 | Tectonics | seed | coastline, uplift rate, rock hardness |
| 2 | Climate, first pass | light level, coastline | temperature at sea level, run-off |
| 3 | Landscape | uplift, hardness, run-off, zone | elevation, drainage, sediment |
| 4 | Climate, second pass | elevation | temperature, run-off (colder with height, rain shadows) |
| 5 | Water | drainage, climate | rivers, lakes, floodplains, basins |
| 6 | Biomes | elevation, slope, temperature, moisture, water, sediment | biome per cell |
| 7 | LifeGen | all of the above | provinces, settlements, roads |

Stages 3 and 4 depend on each other. They are run alternately for a small
fixed number of rounds: landscape, climate, landscape again.

Kept as they are: the continent outline, the light field and its arc-shaped
terminus, the east-west wrap, the macro pack, river courses, the map.

## Stage 1: uplift

Found while building the sandbox: the tectonic layer marks plate boundaries
as thin lines. Used as uplift on its own it raises narrow ridges and leaves
the rest of the land flat. Uplift needs three parts, each a share of one
rate:

- **Interiors:** whole continents rising, most in the middle (from
  continentalness above sea level).
- **Mountain belts:** along the ridges of the peaks-and-valleys layer.
- **Faults:** along plate boundaries, spread from lines into belts.

Also found: about 40% of the world lies below sea level but only 11% is
liquid sea. The rest is night-side sea ice and day-side dried sea bed. Both
are still where water drains to: the dried beds are the terminal basins.

## Stage 3: landscape

Land starts barely above sea level. Each step (spec 012's drainage solve is
step 2 to 4):

1. Uplift: `h += dt * U`.
2. Priority-flood fill; the order cells are reached in is lowest to highest.
3. Receivers by steepest descent on the filled surface.
4. Run-off gathered downstream.
5. Stream power erosion, implicit, from the sea upwards:
   `h[i] = (h[i] + dt*U[i] + f*h[r]) / (1 + f)`, `f = K*dt*A[i]^m / distance`.
6. Slope creep: each cell moves a little towards the mean of its neighbours.

Run to steady state, when uplift and erosion balance and the land stops
changing.

### Three regimes

What does the cutting depends on where on the planet a cell is. This is what
makes the landscape Margin's and not an island demo: the shape of the ground
tells you which side of the world you are on.

| Zone | Agent | Landform |
|---|---|---|
| Terminus | Rain and rivers | Dendritic valleys, sharp ridges between them |
| Night side | Ice | Wide U-shaped valleys, fjords at the coast, few rivers |
| Day side | Wind, almost no water | Tectonic relief left sharp and uneroded, dune fields, salt pans |

- **Rivers** cut by the stream power law above.
- **Ice** cuts in proportion to ice flux, not `A^m`: it widens and flattens
  valley floors and does not need a continuous downhill path. To be designed
  in detail at that stage.
- **Wind** moves loose sediment downwind and rounds nothing. Uneroded uplift
  must be bounded another way: uplift slows as the land rises
  (`U_effective = U * (1 - h / h_limit)`), standing in for the crust sagging
  under load.

The regimes blend across the margins by light level and temperature.

### Lakes

A hollow that fills with water is a lake, not ground to be levelled. A lake
with an outlet spills and the river continues. A hollow where evaporation
exceeds inflow has no outlet: a **terminal lake**, shrinking to a salt pan
towards the day side.

This replaces the invariant that every river reaches the sea. New rule: a
river ends in the sea, a lake, or a terminal lake or salt pan on the day-side
margin. It still never just stops on dry ground.

## Stage 4: climate from the land

- Temperature falls with height.
- Wind at the surface blows from the night side to the day side (cold air
  sinks and runs towards the heat). Air forced up a slope drops its moisture
  there and is dry beyond it.
- Run-off is rainfall less evaporation, which rises with temperature.

## Stage 6: biomes from physical fields

Biome classification reads elevation, slope, temperature, moisture, distance
to water, and sediment. The noise layers it reads today (peaks and valleys,
erosion amount, humidity as noise) are retired. No green palette, as before.

## Below chunk scale

One macro cell is one chunk, 512 blocks. A valley narrower than that cannot
exist on the macro grid. Chunks and map tiles need a second stage that, from
the river courses and the macro slope:

- lowers the ground along each river into a bed, banks and a valley floor
  sized to the river
- adds detail whose character depends on slope, drainage and regime (gullied
  where water runs, smooth on ice, rippled on dunes), in place of one uniform
  noise

It must stay a function of world position, so neighbouring chunks agree.

## LifeGen

- Provinces follow drainage basins: watersheds are the borders.
- Settlements at confluences, fords, river mouths and lake shores.
- Roads along valley floors, across ranges at passes (issue #5).
- Fertility from deposited sediment.

## Stages of work

Each stage ends with something visible on the map.

0. **Sandbox.** The landscape step running on Margin's real coastline, uplift
   and light, in the browser, with the parameters on sliders. Used to choose
   the parameters before anything is replaced.
1. **Uplift terrain** in the generator, rivers only. Biomes by a stopgap rule
   from elevation and climate.
2. **Three regimes and lakes.**
3. **Climate coupling.**
4. **Biomes** from physical fields; retire the noise layers.
5. **Below chunk scale:** channels, valley sides, regime detail.
6. **LifeGen** on basins, passes and floodplains.

## Stage 1 as built

`mg_noise/src/landscape.rs` grows the macro heightmap from flat land.

- **Two grids.** The land is grown for 400 steps at half resolution, where a
  step is four times cheaper, then carried up and refined for 40 steps at
  full resolution, which adds the smaller valleys. Macro generation takes
  about 10 seconds.
- **Parameters** chosen in the sandbox on 2026-10-05: erodibility 0.021,
  uplift 0.008, uplift stops at height 1.0, river strength (m) 0.46, slope
  creep 0.08, time step 2; uplift mix interiors 0.55, mountain belts 0.75,
  faults 0.70; run-off kept on the night side 7%, on the day side 2%.
- **Sea** is every body of at least 12 cells below sea level, liquid or not.
  Its cells keep their depth. The coast is where the ground crosses sea
  level; the old rule that nudged the coast by rock hardness is gone, so
  rivers and coast now agree exactly.
- **Stopgap biomes.** Classification is unchanged except for two inputs: it
  reads the real height of the ground instead of a height made up from
  continentalness and the peaks layer, and the real slope ("flatness")
  instead of a derived erosion amount. Height bands were reset to the new
  land (lowland to 0.20, upland to 0.45, highland to 0.85, alpine above).
- The noise heightmap is still generated: tiles that are not the macro map
  start from it before being anchored.

Seed 42: half the land lies below 0.38 (76 blocks) and a tenth above 0.86
(172 blocks); the highest point is at the uplift limit. 105 river courses.

LifeGen was not recalibrated and it shows: 1079 provinces, 44% of them
uninhabited (17% before), 20 cities (102 before). The taller, steeper land
scores as less habitable. That is stage 6's work.

## Costs

- The world is replaced. Coastline and terminus stay; every mountain, valley,
  biome boundary and province moves. LifeGen needs recalibrating and the
  lore states re-placing.
- Running to steady state takes tens of seconds at full size. It happens
  once, when the macro pack is made; the game loads the pack.
- Biome classification is rewritten.

## Open questions

1. Grid for the landscape solve: the macro grid (1024 by 512), or a finer
   one stored in the pack? Finer gives narrower valleys and a larger pack.
2. How ice erosion is modelled.
3. How many climate and landscape rounds are enough.
4. Whether sea level, and so the coastline, should also respond (deltas
   building out, fjords cutting in).
