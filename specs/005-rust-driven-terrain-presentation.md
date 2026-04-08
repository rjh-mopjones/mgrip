# Spec 005 - Rust-Driven Terrain Presentation

**Status:** Draft
**Priority:** High
**Depends On:** Spec 002a / 002b foundations stable, Spec 004 agent runtime implemented

---

## Problem

The runtime world does not convincingly express Margin's planetary identity.

The playable world currently presents too uniformly:

- the same sky appears everywhere
- terrain feels broadly similar across the planet
- liquid sea appears in dayside and polar regions where it should not exist as
  normal liquid water
- snowy regions still read as orange or scorched
- terrain form is boring and generic

The macro generator already knows much more about Margin than the runtime
renders. The current runtime is still too close to:

- generate local chunk
- build generic land mesh
- build generic water mesh
- apply one land material and one water material
- show one global environment

That is not enough for a world whose whole identity depends on large-scale
directional planetary logic.

---

## Root Cause

The runtime is not consuming the rich terrain data it already has.

Margin's Grip already generates many layered fields per chunk:
`continentalness`, `tectonic`, `humidity`, `rock_hardness`, `light_level`,
`peaks_valleys`, `heightmap`, `temperature`, `erosion`, `rivers`, `aridity`,
`precipitation_type`, `water_table`, `wind_speed`, `snowpack`,
`resource_richness`, `vegetation_density`, `soil_type`, and `biome`.

None of these are currently driving runtime presentation in a meaningful way.

---

## Design Position

### Rust computes world identity. Godot renders it.

This is the core change.

The Rust generator already owns the terrain model. Runtime classification
should happen there too — it is more performant, deterministic, and prevents
drift between what the world is and what the runtime shows.

Godot should receive a runtime-ready terrain description and render it.
GDScript should not be responsible for expensive or subtle world interpretation.

### Use many layers, not one threshold

Zone classification should not be just latitude or light level. It should
emerge from combinations of climate, hydrology, terrain relief, and biome fields.
This gives enough information to classify not just "where are we?" but "what
should this place feel like?"

### Use weighted multi-layer scoring, not single-threshold branching

For each classification family, compute a small score set and choose the
strongest match. This is better than brittle threshold trees because:

- the world is already warped and irregular
- multiple signals matter at once
- it is easier to tune visually
- it gives softer transitions

---

## Goals

- make runtime world presentation match Margin's planetary rules
- compute runtime world and presentation classifications in Rust
- make the day side, terminus, and night side feel radically different
- eliminate invalid liquid sea presentation in forbidden regions
- stop the whole planet reading as the same orange terrain
- expose snowy, frozen, terminus, coastal, and scorched regions distinctly
- improve terrain interest by attaching richer identity to local chunks
- surface runtime presentation data through the agent observation envelope
- preserve the current Godot chunk-streaming architecture

---

## Non-Goals

- full LifeGen integration
- full faction or province rendering
- rewriting all terrain generation
- replacing the existing chunk system
- moving all rendering into Rust
- survival, combat, or inventory systems

---

## Proposed Architecture

Add a new Rust-side runtime classification and presentation layer on top of
`BiomeMap`.

Each generated chunk should produce not just raw terrain fields but a
**runtime presentation summary** and optional **per-cell presentation grids**.

### Classification stability contract

Runtime presentation must be stable for a given runtime `chunk coord`
regardless of whether that chunk is currently rendered as `LOD0`, `LOD1`, or
`LOD2`.

- compute one canonical presentation summary per chunk from a fixed sampling
  basis in Rust
- do not let render LOD resolution change the dominant zone, atmosphere, water,
  landform, or palette classification for the same chunk
- lower-detail render representations may consume the same summary, but they
  must not silently recompute a different one from their reduced terrain sample

### RuntimeChunkPresentation

Required chunk-level summary:

```text
RuntimeChunkPresentation
  dominant_planet_zone        PlanetZone
  dominant_atmosphere_class   AtmosphereClass
  dominant_landform_class     LandformClass
  dominant_surface_palette    SurfacePaletteClass
  average_light_level         f32
  average_temperature         f32
  average_humidity            f32
  average_aridity             f32
  average_snowpack            f32
  average_water_table         f32
  average_vegetation_density  f32
  average_erosion             f32
  average_rock_hardness       f32
  chunk_surface_water_state   SurfaceWaterState (dominant)
  chunk_material_mix          [f32; N]
  chunk_flags                 u32
  interestingness_score       f32
```

Optional reduced-resolution grids (add only if chunk summaries prove
insufficient):

```text
RuntimePresentationGrids
  zone_grid              [PlanetZone; N]
  atmosphere_grid        [AtmosphereClass; N]
  landform_grid          [LandformClass; N]
  water_state_grid       [SurfaceWaterState; N]
  surface_palette_grid   [SurfacePaletteClass; N]
  material_mix_weights   [[f32; N]; M]
```

A practical first pass is chunk-level summaries only. Reduced grids at 64x64
are the next step if chunk summaries prove too coarse. Full 512x512 grids only
if genuinely needed.

---

## Classification Families

### A. Planetary Zone

Answers: **what large-scale planetary regime does this chunk belong to?**

```text
PlanetZone
  SubstellarInferno
  ScorchBelt
  DryDaysideMargin
  InnerTerminus
  OuterTerminus
  ColdTerminus
  FrostMargin
  FrozenCoast
  DeepNightIce
  AbyssalNight
```

Primary inputs: `light_level`, `temperature`, `snowpack`, `humidity`,
`aridity`, `continentalness`, `water_table`

Secondary inputs: `heightmap`, `biome`

This should be warped by the actual generated climate fields, not
hand-authored map bands.

---

### B. Atmospheric Presentation Class

Answers: **what should the sky, ambient light, fog, and environment feel like?**

```text
AtmosphereClass
  BlastedRadiance
  HarshAmberHaze
  DryTwilight
  TemperateTwilight
  WetTwilight
  FrostTwilight
  PolarGlow
  BlackIceDark
  GeothermalNight
```

Primary inputs: `light_level`, `temperature`, `humidity`, `snowpack`,
`water_table`, `biome`

Secondary inputs: `rivers`, geothermal proxies when available

Drives in Godot: sky gradient, ambient light tint, fog density and tint,
visibility, nightside darkness profile.

---

### C. Surface Water State

Answers: **what kind of water, ice, or surface fluid presentation exists here?**

```text
SurfaceWaterState
  None
  LiquidSea
  LiquidCoast
  FrozenSea
  IceSheet
  BrineFlat
  EvaporiteBasin
  MeltwaterChannel
  LiquidRiver
  FrozenRiver
  MarshWater
```

Primary inputs: `ocean_mask`, `heightmap`, `temperature`, `snowpack`,
`water_table`, `rivers`, `aridity`, `continentalness`, `planet_zone`

Margin-specific rules:

- dayside must not present as normal liquid sea
- deep nightside reads as `IceSheet` or `FrozenSea`; `DeepNightIce` remains a
  `PlanetZone`, not a water-state value
- twilight band can carry `LiquidSea` and `LiquidRiver`
- arid basins read as `BrineFlat` or `EvaporiteBasin`
- rivers freeze based on local temperature and snowpack

This is the primary fix for invalid water presentation.

Implementation rule:

- chunk-level summaries are enough for dominant ocean/coast presentation
- river, basin, or mixed sub-chunk water presentation requires explicit Rust
  output such as masks, split surface buckets, or reduced grids; do not infer
  those states in GDScript from the coarse chunk summary alone

---

### D. Landform Class

Answers: **what kind of terrain structure is this place?**

```text
LandformClass
  FlatPlain
  Basin
  Plateau
  Ridge
  Escarpment
  BrokenHighland
  AlpineMassif
  CoastShelf
  CliffCoast
  FrozenShelf
  DuneWaste
  Badlands
  FractureBelt
  RiverCutLowland
  VolcanicField
```

Primary inputs: `heightmap`, `tectonic`, `erosion`, `peaks_valleys`,
`rock_hardness`, `continentalness`, `rivers`, `snowpack`, `aridity`

Secondary inputs: local neighborhood relief stats computed in Rust (local
relief range, slope magnitude, curvature proxy, coastline and river proximity,
relief variance)

Influences: terrain visual breakup, local material mix, future object
placement, traversal scoring.

---

### E. Surface Palette Class

Answers: **what should the terrain surface visually read as?**

```text
SurfacePaletteClass
  ScorchedStone
  AshDust
  DarkTerminusSoil
  WetTerminusGround
  FungalLowland
  CoastalSediment
  SaltCrust
  SnowCover
  BlueIce
  BlackIceRock
  ExposedStone
  IronOxideHighland
  VegetatedDarkCanopyFloor
```

Primary inputs: `biome`, `temperature`, `snowpack`, `water_table`,
`vegetation_density`, `soil_type`, `aridity`, `rock_hardness`,
`landform_class`, `planet_zone`

The main fix for "everything is orange."

This classification must enforce the world rule: **green is not dominant.** Sparse muted green is acceptable; lush Earth-green surfaces are not.

---

## Terrain Interestingness Score

Each chunk should compute a lightweight interestingness score derived from
existing terrain fields.

Possible inputs:

- local relief range
- coastline proximity
- river proximity
- landform contrast
- water state contrast
- slope variance
- zone contrast with adjacent chunks

Used for:

- spawn selection quality
- agent traversal path selection
- future POI seeding
- debug inspection of bland chunks

This score is a first-class field in `RuntimeChunkPresentation` and should
appear in the agent observation envelope.

---

## Rust-Side Implementation

### New module

```text
gdextension/crates/mg_noise/src/runtime_presentation/
  mod.rs
  zone.rs
  atmosphere.rs
  water_state.rs
  landform.rs
  surface_palette.rs
  interestingness.rs
```

### New Rust types

Enums: `PlanetZone`, `AtmosphereClass`, `SurfaceWaterState`, `LandformClass`,
`SurfacePaletteClass`

Structs: `RuntimeChunkPresentation`, `RuntimeChunkAverages`, `RuntimeChunkFlags`

### BiomeMap extensions

New methods:

- `build_runtime_chunk_presentation() -> RuntimeChunkPresentation`
- `export_runtime_zone_grid() -> Vec<PlanetZone>`
- `export_runtime_water_state_grid() -> Vec<SurfaceWaterState>`
- `export_runtime_landform_grid() -> Vec<LandformClass>`
- `export_runtime_surface_palette_grid() -> Vec<SurfacePaletteClass>`

### GDExtension API extensions

New methods on `MgBiomeMap`:

- `runtime_zone_at(x: i32, y: i32) -> i32`
- `runtime_water_state_at(x: i32, y: i32) -> i32`
- `runtime_landform_at(x: i32, y: i32) -> i32`
- `runtime_surface_palette_at(x: i32, y: i32) -> i32`
- `build_runtime_chunk_summary() -> Dictionary`

The summary dictionary should be passable directly to Godot without any
GDScript interpretation.

### Performance strategy

1. compute terrain layers in Rust
2. compute runtime presentation summaries in Rust at generation time
3. pass compact summaries to Godot via the GDExtension
4. let Godot use summaries for rendering decisions only

GDScript does not derive presentation logic. It receives it.

---

## Godot-Side Implementation

### WorldChunk extensions

Add runtime presentation fields:

```text
runtime_presentation: Dictionary
planet_zone: int          # PlanetZone enum value
atmosphere_class: int     # AtmosphereClass enum value
landform_class: int       # LandformClass enum value
surface_palette_class: int
water_state: int
interestingness_score: float
```

These are populated from the Rust-computed summary at chunk generation time.

### New world environment controller

Add `scripts/world/world_environment_controller.gd`.

Reads the current chunk's `RuntimeChunkPresentation` and updates:

- sky material or gradient
- ambient light tint and energy
- fog density and color
- world environment tint
- nightside darkness profile
- twilight look
- dayside harshness

Updates when the player crosses into a new chunk.

### Terrain rendering changes

`VoxelMeshBuilder` should stop treating all land as one visual category.

**Option A (preferred first pass):** one terrain shader with runtime parameters

Pass to shader:

- `dominant_surface_palette`
- `dominant_planet_zone`
- `average_snowpack`
- `average_temperature`
- `average_water_table`
- `average_vegetation_density`

**Option B (if Option A is insufficient):** split land surfaces into render
groups based on Rust classification.

### Water rendering changes

Water mesh selection and material should be driven by `SurfaceWaterState`.

Phase 1 scope:

- drive chunk-level ocean and coast presentation from the dominant chunk water
  state
- fix the invalid cases first: no liquid sea on scorched dayside chunks and no
  generic liquid sea on deep nightside chunks
- defer `FrozenRiver`, `BrineFlat`, `MeltwaterChannel`, and other mixed
  sub-chunk states unless Rust also exports the masks or reduced grids needed
  to render them correctly

Full-spec expected outcomes:

- no `LiquidSea` material in dayside scorch zones
- `IceSheet` or `FrozenSea` material on nightside ocean chunks
- frozen river rendering when `FrozenRiver` is the dominant river state
- `BrineFlat` or `SaltCrust` presentation in arid basins

### Anti-smoothing rule

Do not smooth the voxel heightfield to paper over boring terrain.

- keep terrain fundamentally stepped
- preserve one-block-per-pixel identity where that is the design
- soften presentation through materials, shading, palette variation, and
  distance fog — not geometry blur
- consider mild contour terracing or step shaping if slopes read as noisy

If terrain visuals become significantly smoother than the collision mesh, the
world will feel fake underfoot. Do not let that happen.

---

## Agent Runtime Integration

### Observation envelope

Once `RuntimeChunkPresentation` exists, extend `agent_observation_builder.gd`
to include presentation fields in the observation payload:

```text
observation["runtime_presentation"] = {
  "planet_zone": { "id": <int>, "name": <string> },
  "atmosphere_class": { "id": <int>, "name": <string> },
  "landform_class": { "id": <int>, "name": <string> },
  "surface_palette_class": { "id": <int>, "name": <string> },
  "water_state": { "id": <int>, "name": <string> },
  "interestingness_score": <float>,
  "average_light_level": <float>,
  "average_temperature": <float>,
  "average_snowpack": <float>,
}
```

This field should appear at the top level of every observation alongside
`current_chunk` and `player_position`.

### get_chunk_state extension

`get_chunk_state` already returns runtime chunk data. Extend it to include the
full `RuntimeChunkPresentation` dictionary when available, including both enum
ids and canonical enum names for every exposed classification field.

### find_nearest_land and water state

`find_nearest_land` currently uses the ocean mask. Once `SurfaceWaterState` is
available, it should only use water-state data for landing rejection when the
runtime has per-cell or reduced-grid water-state coverage. A chunk-level
dominant water summary is not precise enough to reject individual landing
positions. When per-cell or reduced-grid data exists, treat non-standable water
states as invalid landing positions — including `FrozenSea`, `IceSheet`, and
`BrineFlat`. Land means a surface the player can actually stand on, not just
"not ocean mask."

### Bridge state

The bridge state written by `_write_bridge_state()` should include the current
chunk's dominant `planet_zone` and `atmosphere_class` so external tools can
read them without submitting a full observation action. Include both ids and
names there as well.

---

## Pre-Implementation Diagnostic Gate

Before moving beyond Phase 1, run a structured diagnostic Codex agent session
to determine whether terrain blandness is a **relief problem**, a
**presentation uniformity problem**, or both.

### Session outline

```text
1. start session
2. teleport_to_block → representative dayside chunk (high light_level)
3. wait_for_ring_ready
4. wait_for_player_settled
5. capture_screenshot { file_name: "dayside_relief" }
6. get_chunk_state
7. teleport_to_block → representative terminus chunk
8. wait_for_ring_ready + wait_for_player_settled
9. capture_screenshot { file_name: "terminus_relief" }
10. teleport_to_block → representative nightside chunk
11. wait_for_ring_ready + wait_for_player_settled
12. capture_screenshot { file_name: "nightside_relief" }
13. end session
```

Read `steps.jsonl` and screenshots to answer:

- is visible relief range adequate at gameplay camera height?
- do ridges, basins, and slopes read clearly?
- is terrain flat because of geometry or because of palette sameness?
- is vertical scaling suppressing terrain drama?

### Diagnostic outcome

Answer one question:

- boring feel is primarily **geometry / relief** — re-evaluate `HEIGHT_SCALE`
  before relying on palette/landform work
- boring feel is primarily **palette / environment uniformity** — proceed with
  Phase 2
- combination of both — address relief scaling and palette in parallel

---

## Implementation Phases

### Phase 1 — Rust runtime summaries + environment and water correctness

**Rust:**

- `PlanetZone` enum and classification
- `AtmosphereClass` enum and classification
- `SurfaceWaterState` enum and classification
- `RuntimeChunkPresentation` struct (subset: zone, atmosphere, water state,
  averages)
- `build_runtime_chunk_summary()` on `MgBiomeMap`

**Godot:**

- extend `WorldChunk` with `runtime_presentation` dictionary
- add `world_environment_controller.gd`
- drive environment from current chunk's Rust presentation summary
- drive water mesh/material from `SurfaceWaterState`
- extend `agent_observation_builder.gd` to include `runtime_presentation`

**Fixes:**

- same sky everywhere
- invalid liquid sea in dayside and polar regions
- weak basic visual distinction between day side, terminus, and night side

**Codex verification session:**

```text
1. start session with --agent-runtime
2. teleport_to_block → known dayside chunk coord
3. wait_for_ring_ready
4. wait_for_player_settled
5. get_chunk_state → assert planet_zone is ScorchBelt or SubstellarInferno
6. capture_screenshot { file_name: "phase1_dayside" }
7. teleport_to_block → known nightside chunk coord
8. wait_for_ring_ready + wait_for_player_settled
9. get_chunk_state → assert planet_zone is DeepNightIce or AbyssalNight
10. assert water_state is FrozenSea or IceSheet, not LiquidSea
11. capture_screenshot { file_name: "phase1_nightside" }
12. end session
```

Run diagnostic gate after Phase 1 before proceeding.

---

### Phase 2 — Terrain palette correctness

**Rust:**

- `SurfacePaletteClass` enum and classification
- add `dominant_surface_palette` to `RuntimeChunkPresentation`

**Godot:**

- terrain shader or material selection driven by `dominant_surface_palette`
- ensure green does not dominate any surface at shader level

**Fixes:**

- orange everywhere
- snowy regions reading as scorched
- lack of strong zone-specific surface identity

**Codex verification session:**

```text
1. start session
2. teleport to snowy/frozen chunk → assert dominant_surface_palette is
   SnowCover or BlueIce
3. capture_screenshot { file_name: "phase2_frozen" }
4. teleport to dayside chunk → assert palette is ScorchedStone or AshDust
5. capture_screenshot { file_name: "phase2_scorch" }
6. inspect screenshots for dominant green — fail if green is the primary surface colour across any large area
7. end session
```

---

### Phase 3 — Terrain interest and landform identity

**Rust:**

- `LandformClass` enum and classification using neighborhood relief stats
- `interestingness_score` field in `RuntimeChunkPresentation`

**Godot:**

- terrain material or detail response based on `landform_class`
- expose `interestingness_score` in observation envelope

**Fixes:**

- boring, samey terrain feel
- terrain that reads as generic noise rather than recognisable places

**Codex verification session:**

```text
1. start session
2. request observations at 6+ diverse chunk locations
3. assert at least 4 distinct LandformClass values appear across the sample
4. assert interestingness_score varies meaningfully across locations
5. capture screenshots at low and high interestingness locations
6. end session
```

---

### Phase 4 — Reduced-resolution presentation grids (if needed)

Add only if chunk-level summaries prove too coarse.

- 64x64 `water_state_grid`
- 64x64 `landform_grid`
- 64x64 `surface_palette_grid`

This gives sub-chunk variation without requiring Godot to derive it.
Do not implement until Phase 3 reveals a concrete need.

---

## Additional Terrain Improvements

These should be folded into the same implementation direction.

### Macro-to-micro inheritance

The playable chunk should inherit more of the macro world's identity. The
runtime presentation system supports this by attaching macro climate and biome
pressure to each chunk summary. The micro chunk should know what larger kind of
place it belongs to.

Contract:

- do not derive planetary identity only from the scaled micro chunk sample
- include at least one stable large-scale input computed from true world
  coordinates or parent macro/meso context
- local micro variation may refine presentation inside a chunk, but it must not
  erase the larger dayside / terminus / nightside identity

### Re-evaluate vertical punch

Confirm that the current runtime `HEIGHT_SCALE` produces enough visible relief
at gameplay camera height before Phase 3 landform work begins. If terrain looks
flat because of geometry, a classification system alone will not fix it.

### Voxel identity preservation

Prefer terracing and better-shaped steps over blur or downsampling:

- keep terrain fundamentally stepped
- avoid blurring the core heightfield
- soften presentation through materials, shading, palette variation, snow or
  ash overlays, and distance presentation
- do not add visual smoothing that the collision mesh does not match

---

## Debug Requirements

Runtime classification must be inspectable.

### Required debug outputs

- current chunk `PlanetZone` id and name
- current chunk `AtmosphereClass` id and name
- current chunk `LandformClass` id and name
- current chunk dominant `SurfacePaletteClass` id and name
- water state summary
- averages for temperature, light, humidity, snowpack, aridity
- `interestingness_score`

By the end of the full spec, all of these should be readable from the agent
observation at any step.

### Optional exported layers

- `runtime_zone`
- `runtime_atmosphere`
- `runtime_water_state`
- `runtime_landform`
- `runtime_surface_palette`

These should plug into the existing layer-debug mindset and tooling.

---

## Acceptance Criteria

### Core correctness

- runtime world presentation differs strongly across the planet
- sky and atmosphere vary by zone
- scorched dayside no longer looks like normal twilight or snowy regions
- snowy and frozen regions visibly read as snowy and frozen
- nightside water reads as ice or frozen sea, not generic liquid sea
- dayside does not show normal liquid surface ocean
- terminus looks distinct from both dayside and nightside
- green does not dominate any terrain surface; muted sparse green is acceptable

### Performance and architecture

- runtime classification is computed in Rust, not GDScript
- Godot only consumes summaries or grids and renders them
- classifications are deterministic for a given seed and location
- chunk presentation remains stable across `LOD0`, `LOD1`, and `LOD2`
- debug output exists for all classification families
- agent observation includes `runtime_presentation` at top level

### Terrain quality

- terrain no longer reads as one generic orange world
- local chunks inherit stronger macro identity
- terrain shows distinct structural character between regions

---

## Risks

- too many classification families increases tuning complexity
- score-based thresholds may still produce visible transitions at extreme biome
  boundaries
- shader complexity may grow quickly if both palette and landform drive the
  same shader
- full 512x512 grids may be heavier than necessary and should be deferred

## Mitigations

- begin with chunk-level summaries only
- tune one classification family at a time
- use agent verification sessions to catch regressions early
- keep debug overlays active during all phases
- defer reduced grids until Phase 3 reveals a concrete need

---

## Modifies

Expected Rust files:

```text
gdextension/crates/mg_noise/src/runtime_presentation/mod.rs   (new)
gdextension/crates/mg_noise/src/runtime_presentation/zone.rs  (new)
gdextension/crates/mg_noise/src/runtime_presentation/atmosphere.rs  (new)
gdextension/crates/mg_noise/src/runtime_presentation/water_state.rs (new)
gdextension/crates/mg_noise/src/runtime_presentation/landform.rs    (new)
gdextension/crates/mg_noise/src/runtime_presentation/surface_palette.rs (new)
gdextension/crates/mg_noise/src/runtime_presentation/interestingness.rs (new)
gdextension/src/lib.rs
```

Expected Godot files:

```text
scripts/world/world_environment_controller.gd     (new)
scripts/world/world.gd
scripts/world/world_chunk.gd
scripts/world/voxel_mesh_builder.gd
scripts/autoload/agent_observation_builder.gd
```

---

## Codex Phase 1 Prompt

Use this spec as full context. Implement **Phase 1 only**.

Implement a Rust-driven Phase 1 runtime presentation system for Margin's Grip.
The current playable world is too uniform: same sky everywhere, liquid sea in
invalid regions, and weak region readability. The project already has many
generated layers in Rust (`light_level`, `temperature`, `humidity`, `aridity`,
`snowpack`, `water_table`, `tectonic`, `erosion`, `peaks_valleys`,
`rock_hardness`, `resource_richness`, `vegetation_density`, `soil_type`,
`biome`, etc.).

For Phase 1:

- add a new `runtime_presentation` module under
  `gdextension/crates/mg_noise/src/`
- implement `PlanetZone`, `AtmosphereClass`, and `SurfaceWaterState` enums
  using weighted multi-layer scoring
- implement `RuntimeChunkPresentation` struct with zone, atmosphere, water
  state, and layer averages
- add `build_runtime_chunk_summary()` to `MgBiomeMap` in `gdextension/src/lib.rs`
- extend `WorldChunk` in Godot to store the Rust-computed summary dictionary
- add `world_environment_controller.gd` that reads the current chunk's
  `planet_zone` and `atmosphere_class` and updates the Godot `WorldEnvironment`
- update water mesh or material selection to use the dominant chunk
  `SurfaceWaterState` for ocean/coast presentation, not just the ocean mask
- extend `agent_observation_builder.gd` to include a `runtime_presentation`
  block in every observation with enum ids and names

Do not implement `SurfacePaletteClass` or `LandformClass` yet. Do not scaffold
placeholder classification families for later phases.

Do not attempt sub-chunk river, basin, or mixed water-state rendering in Phase
1 unless Rust also exports the explicit masks or reduced grids needed to do it
correctly.

The immediate goal of Phase 1 is:

- sky varies across the planet
- no liquid sea in dayside scorch or deep nightside zones
- basic visual distinction between day side, terminus, and night side is clear

After implementing, run the diagnostic agent session described in the spec to
assess whether terrain blandness is a geometry or presentation problem before
Phase 2 begins.

## Codex Phase 2 Prompt

Using this spec as full context, implement **Phase 2 only**.

Phase 1 is complete. `PlanetZone`, `AtmosphereClass`, and `SurfaceWaterState`
are implemented and the environment controller and water rendering are live.

For Phase 2:

- add `SurfacePaletteClass` enum to the `runtime_presentation` module
- implement weighted multi-layer scoring using `biome`, `temperature`,
  `snowpack`, `water_table`, `vegetation_density`, `soil_type`, `aridity`,
  `rock_hardness`, `landform_class` (if available), and `planet_zone`
- add `dominant_surface_palette` to `RuntimeChunkPresentation`
- expose it through `build_runtime_chunk_summary()` in the GDExtension
- update Godot terrain rendering so `dominant_surface_palette` drives shader
  parameters or material selection
- ensure green does not dominate terrain surfaces at shader or material level

The goal of Phase 2 is:

- snowy and frozen regions visibly read as snowy or frozen, not orange
- scorched dayside reads as scorched, not generic terrain
- terrain no longer collapses into one orange world

Run the Phase 2 Codex verification session from the spec after implementing.

## Codex Phase 3 Prompt

Using this spec as full context, implement **Phase 3 only**.

Phases 1 and 2 are complete. Zone, atmosphere, water, and palette
classifications are live.

For Phase 3:

- add `LandformClass` enum to the `runtime_presentation` module
- implement classification using `heightmap`, `tectonic`, `erosion`,
  `peaks_valleys`, `rock_hardness`, `continentalness`, `rivers`, `snowpack`,
  `aridity`, and neighborhood-derived relief stats computed in Rust
- add `dominant_landform_class` and `interestingness_score` to
  `RuntimeChunkPresentation`
- expose both through the GDExtension
- update Godot terrain material or detail response to vary by `landform_class`
- add `interestingness_score` to the agent observation envelope
- do not smooth away voxel identity — prefer stronger material diversity and
  step shaping over blur or downsampling

The goal of Phase 3 is:

- terrain feels like recognisable places rather than generic noise
- distinct structural character is visible between regions
- `interestingness_score` varies meaningfully and is usable for spawn or path
  selection

Run the Phase 3 Codex verification session from the spec after implementing.
