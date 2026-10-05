# Spec 013 - Terrain From Uplift and Erosion

**Status:** In progress. Stage 0 (sandbox), stage 1 (uplift terrain in the generator, rivers only, stopgap biomes) stage 2 (ice, uplift ceilings, terminal lakes) and most of stage 5 (land on a finer grid, river channels) done. Stages 3, 4 and 6 not started.
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

## Fine grid (first half of stage 5) as built

Stage 5 was brought forward: on the map, close up is where the land is
looked at, and at one cell per chunk the grown terrain was a few smooth
blobs under a uniform noise texture.

- **Four cells per chunk.** After the macro land is grown, it is doubled in
  resolution twice (to 4096 by 2048), with 20 and then 8 steps of erosion,
  so the finer grids have valleys of their own. A few rounds of slope creep
  alone then close gullies a single cell wide, which shade as a rash of
  bumps. Valleys exist down to a few hundred blocks across.
- **Rivers come from the fine drainage,** so they run in the fine valleys.
  The meander warp is cut from 2.4 world units to 0.35 so it cannot carry a
  river out of its valley.
- **Water is routed by lot.** Water can only run to one of eight
  neighbours. Always taking the steepest made valleys and rivers run dead
  straight along those eight directions. The neighbour is now drawn by lot
  among the downhill ones, likelier the steeper (by slope squared), and
  differently each step. This removed most of the grid look.
- **Tiles and chunks take their ground from the fine grid.** The macro pack
  carries it as 16-bit heights (pack: 14 MB to 29 MB). Map tiles are
  generated without the ground noise the game's chunks add and are shaded
  from the fine land alone; the map's zoom stops at 64, about 16 chunks
  across, below which there is no more shape to show.
- Generating the macro map takes about 35 seconds and 2 GB.

Seed 42: 162 river courses; 39 provinces with a major river.

- **River channels.** Anchoring cuts each river into the ground of a tile
  or chunk (`rivers::carve_depths`): a bed deepest in midstream, 0.2 of
  height per world unit of half-width (about 8 blocks for the largest
  river), and a valley floor half as deep at the bank, easing out over four
  half-widths. Land is never cut below the sea. Chunk seams stay exact.
- **Meander.** A sweep of 0.25 world units and a wiggle of 0.3 at a
  wavelength of 4, enough to stop rivers running in straight stretches
  without leaving their valleys.
- **The map's coast** is drawn from `sea.png`, liquid sea at four cells per
  chunk, not from province ids at one. Land in a chunk whose middle is sea
  is given to a province next door.

Still to do in stage 5: ground detail that depends on regime, in place of
the one noise the game's chunks still add (issue #6).

Known faults: some valleys and ridges still run straight along the grid;
land at the uplift limit is a flat plateau; rivers still turn in angular
steps where the fine grid's cells show through.

## Stage 2 as built

- **Ice.** Under ice (light below 0.20, fully by 0.08) an ice stream pulls
  the ground beside it down towards its own level each step, harder the
  more run-off gathers in it (`ice_widening`, 0.25). Valleys on the night
  side widen into troughs. Ice still cuts along the same drainage as water;
  it has no flow of its own, and there are no fjords.
- **Uplift ceilings.** Land stops rising at a height set by how hard it is
  pushed up: the full limit where its share of uplift is 1.2 or more, down
  to 15% of it where there is little. Before, all land rose towards one
  limit, and wherever little water ran (most of the day side) it reached it
  and stood as one flat plateau. Now the day side keeps the shape of its
  uplift: swells and ranges, uncut.
- **Terminal lakes.** A river runs wherever its water is liquid; each
  segment carries one over its longest wet stretch. Where the stretch gives
  out before the sea or the next river (ground too dry or too cold, or a
  pond) it ends in a lake. Before, a whole river system was dropped if any
  stretch downstream was dry. A lake is kept as a stub of a river course as
  wide as the lake, so everything that draws or carves rivers handles it.
- Wind is not modelled as erosion. Dunes and salt pans belong to ground
  detail (issue #6).
- Hollows in the land are still filled level; with rivers cutting every
  sill there are almost none to keep.

Seed 42: 253 river courses, 20 of them terminal lakes; 52 provinces with a
major river. With plateaus gone and more rivers, LifeGen recovers without
being recalibrated: 21% of provinces uninhabited (45% before), 52 cities.

The ceilings lower the whole world's relief, including the terminus that
the sandbox parameters were chosen for. They may want choosing again.

## Revisiting stage 2

Five shortcuts in stage 2 are being replaced, in this order. The coastline
may change (decided 2026-10-05), and every zone is to have character of its
own.

1. **Done: ceiling by run-off.** The uplift ceiling lowered the whole
   world, the terminus included. Well-watered land (run-off of 0.8 or more)
   now has only the full limit above it, as its rivers keep it down; the
   ceiling falls to the uplift-shaped one as run-off falls. The terminus is
   back to the relief its parameters were chosen for.
2. **Done: lakes that last.** Standing water cuts nothing, so a lake bed
   only rises or sinks; before, the erosion step pulled every hollow's
   floor up towards its rim and lakes vanished within a few steps. Water
   crossing a lake evaporates (0.05 of a cell's run-off per lake cell in
   the terminus, up to 3 on the day side, none under ice), so a lake can
   lose its whole river. Ground sinks where the crust pulls apart (the
   deeper troughs of the peaks layer along plate boundaries; `rifts`, 0.6),
   down to 30 blocks below the sea, which holds lakes open against the
   rivers cutting their rims. The heightmap now keeps its hollows, with a
   water level beside it; a lake is open water, ice or salt flat by the
   same rule as the sea.
3. **Done: the day side.**
   - Uplift along belts and faults runs along crests (ridged noise, crests
     about 14 world units apart): 0.35 of its strength between them, 1.75
     on them. A range has a spine, and where nothing wears it down the spine
     shows. This sharpens ranges everywhere, the terminus included.
   - In desert (light above 0.62, fully by 0.85) a channel cuts only what
     it carries beyond a flood's worth, the run-off of 0.5 square world
     units of well-watered land, and then cuts 4 times as hard. A few
     canyons are cut deep and the ground between them stays whole.
   - A canyon in flood pulls the ground beside it down towards its floor
     (`scarp_retreat`, 0.12): its walls fall back and it widens.
   - Water is drawn among downhill neighbours in proportion to slope, no
     longer to slope squared: the steeper relief had brought straight
     valleys back.

   Not done: sand seas towards the sub-stellar point (item 5), and cliffs
   as such. A cliff is narrower than the finest cell; mesas' steep sides
   are for the ground detail of issue #6.
4. **Done: ice with its own flow.**
   - **Thickness.** Ice fills everything below a copy of the ground
     smoothed over about 6 world units, so it lies deep in valleys and
     hollows and is absent from ridges. Scaled by how far the cell is under
     ice (light below 0.20, fully by 0.08).
   - **Flow.** Under ice the drainage is solved on the ice surface (ground
     plus thickness), not the ground. The surface is smoother, so ice
     ignores small features of its bed and can ride over a sill.
   - **Snow.** A cell under an ice sheet adds 0.6 to what is gathered, not
     the 7% of its rain that runs off as water: none of the snow is lost,
     it leaves as ice and as meltwater where the ice ends. Rivers in the
     cold terminus are larger for it.
   - **Digging.** Ice at least 0.004 thick digs its bed by how much ice
     gathers there and how steeply its surface falls (`ice_cutting`, 0.012
     per unit time at full strength), not towards the height of the ground
     downstream. So it can dig a basin deeper than its outlet, which holds
     a lake, and it can dig below the sea, down to 24 blocks: a fjord.
     Ground below sea level is sea to everything downstream of the
     generator (biomes, the fine grid's coast, the map).
   - **Rivers stop at the sea's surface.** A river used to cut towards the
     height of the cell it drains to, which at the coast is the sea bed,
     so with more water the whole night-side coast drowned. It now cuts
     towards sea level there, and towards the bed only of a dried-out sea.
   - Widening (stage 2) is unchanged and now sees the larger ice streams.

   Not done: ice has no thickness in the finished world, only during
   erosion; nothing is drawn as a glacier. Fjords are few, because most of
   the night-side coast faces a frozen sea, where a fjord cannot be told
   from the ice around it.
5. **To do: wind.** A wind field and a sand budget; salt where water ends.

Two more, asked for on 2026-10-06:

6. **Done: the rim sea.** One unbroken stretch of liquid water all the way
   round the world, so the whole rim can be sailed (`rim_sea.rs`). Liquid
   sea lies only in the terminus, and the continents left it as a string of
   separate seas. Before the land is grown, the cheapest route round the
   world is found through water that would be liquid (open sea costs 1 a
   cell, land 40 and more the higher it stands, anywhere the sea would
   freeze or dry 4000), closed into a ring across the east-west seam, and
   wherever it crosses land a strait is cut: 2.5 world units to either side,
   0.07 deep in the middle, shallowing to the shore. It works on
   continentalness, so the straits are sea to everything after and get
   coasts like any other. The site map export checks the result and says
   whether the rim sea is unbroken.
7. **Done: ragged ice and desert coasts.** Where the sea freezes and where
   it dries out depend on light, which drew both edges as clean arcs across
   the world. Those lines now wander by up to 0.06 in light level, about
   ten world units on the ground, with bends from 45 world units down to 6
   (`sea_margin_drift`): tongues, bays and outliers. The same drift is used
   wherever the sea is classified (biomes, tiles, river mouths, lakes, the
   map's coast), so they agree.

Not yet done under 2: closed basins. A lake that loses all its water still
routes it towards its rim, not its floor, so it has no true shore; the
river-stub terminal lakes remain for rivers that dry out.

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
