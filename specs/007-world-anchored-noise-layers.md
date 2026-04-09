# Spec 007 - World-Anchored Noise Layers

**Status:** Implemented
**Priority:** Critical
**Depends On:** None — this is a foundation fix that should precede further terrain work

---

## Problem

The terrain generator uses a single `freq_scale` parameter to multiply noise
coordinates before sampling every fBm layer. At macro scale (`freq_scale=1.0`)
the world looks coherent. At micro / runtime scale (`freq_scale=8.0`) the same
absolute world position evaluates at a completely different point in noise
space.

Concrete example:

```
Macro image:   world (500, 400) → noise coord (500 * 1.0 * 0.01) = (5.0, 4.0)
Runtime chunk: world (500, 400) → noise coord (500 * 8.0 * 0.01) = (40.0, 32.0)
```

These are different points in the noise function. They produce different
continentalness values. They disagree about whether (500, 400) is ocean or
land.

This breaks every system that tries to relate what the macro map shows to what
the runtime generates:

- clicking an ocean chunk in the map selector spawns on land
- runtime presentation classifications (`PlanetZone`, `SurfaceWaterState`,
  `AtmosphereClass`) disagree with what the macro debug images show
- LOD levels can classify the same chunk differently from each other
- any future macro-to-micro inheritance is built on an incoherent foundation

---

## Root Cause

`freq_scale` is doing two separate jobs that should not be coupled:

1. **Giving micro chunks meaningful terrain variation** — without scaling,
   a 1×1 world-unit tile spans a tiny slice of noise space and produces
   near-constant values per layer. Scaling expands that slice so local terrain
   has interesting shape.

2. **Determining the identity of a place** — ocean vs land, planetary zone,
   tectonic regime. This should not vary with render scale.

`light_level` was already correctly excluded from `freq_scale`. It evaluates
at raw world coordinates and normalises by map dimensions. It is the proof of
concept that world-anchored layers work. Every other identity layer should
follow the same pattern.

---

## Design

Split noise layers into two explicit tiers.

### Tier 1 — World-Anchored Layers

Evaluated at raw world coordinates. Never multiplied by `freq_scale`.

These define the **identity** of a place. They are stable regardless of
which LOD or `freq_scale` is used to generate a chunk. A chunk at world
position (500, 400) always gets the same values from these layers.

| Layer | Reason |
|---|---|
| `continentalness` | Primary gate for ocean vs land. Must be stable. |
| `tectonic` | Voronoi plate structure. Already CPU-only. Plate boundaries must not shift between LODs. |
| `light_level` | Already world-anchored. No change. |

### Tier 2 — Detail Layers

Evaluated at `world_coord * freq_scale`. These define the **local texture**
of a place — terrain roughness, rock variety, local moisture variation.

| Layer | Role |
|---|---|
| `peaks_valleys` | Local relief. Adds height variation on top of continentalness. |
| `rock_hardness` | Surface texture signal. |
| `humidity` | Local moisture. Terminator model still couples to world-anchored `light_level`. |

---

## What Changes

### `biome_map.rs` — coordinate dispatch

Currently:

```rust
let nwx = wx * freq_scale;
let nwy = wy * freq_scale;

let cont = cont_strat.generate(nwx, nwy, detail_level);
let tect = tect_strat.generate_full(nwx, nwy);
let humid = humid_strat.generate_terminator_model(nwx, nwy, ...);
let rock = rock_strat.generate(nwx, nwy, detail_level);
let pv_base = pv_strat.generate(nwx, nwy, detail_level);
let light = light_strat.generate(wx, wy, detail_level);   // already world-anchored
```

After:

```rust
let nwx = wx * freq_scale;  // detail coordinates
let nwy = wy * freq_scale;

// Tier 1 — world-anchored: always use (wx, wy)
let cont  = cont_strat.generate(wx, wy, detail_level);
let tect  = tect_strat.generate_full(wx, wy);
let light = light_strat.generate(wx, wy, detail_level);  // unchanged

// Tier 2 — detail: use (nwx, nwy)
let humid   = humid_strat.generate_terminator_model(nwx, nwy, ...);
let rock    = rock_strat.generate(nwx, nwy, detail_level);
let pv_base = pv_strat.generate(nwx, nwy, detail_level);
```

### Wrapping

Currently wrapping is controlled by `use_wrap = freq_scale == 1.0` applied
uniformly to all layers.

After this change, world-anchored layers (`continentalness`, `tectonic`) must
always use wrapping regardless of `freq_scale`. Detail layers keep the current
behaviour (wrapping disabled when `freq_scale != 1.0`).

Change the wrapping gate from a single flag to per-tier control:

```rust
let world_wrap = true;           // Tier 1 always wraps
let detail_wrap = freq_scale == 1.0;  // Tier 2 only wraps at macro scale
```

Pass the appropriate wrap flag to each strategy's generate call.

### Cylindrical wrapping for continentalness

Continentalness is the only layer that currently uses cylindrical wrapping
(east-west seam closure). This wrapping must be preserved for the
world-anchored call. Since it is now always called with `(wx, wy)` and
`world_wrap = true`, the existing cylindrical wrapping path continues to work
without modification.

---

## What Does Not Change

- `light_level` coordinate handling — already correct
- `biome_splines.rs` — biome classification logic is unchanged
- The ocean gate (`elevation < SEA_LEVEL`) — unchanged
- `tile_has_fluid_surface` — unchanged
- `derive_heightmap`, `derive_temperature`, all derived layers — unchanged
- GDExtension API — unchanged
- Godot-side code — unchanged
- GPU path — already disabled when `freq_scale != 1.0`; no change needed

---

## Expected Outcomes

After this change:

- A chunk generated at `freq_scale=8.0` and a macro tile generated at
  `freq_scale=1.0` over the same world coordinates produce the same
  `continentalness` value and the same ocean/land classification
- Clicking an ocean chunk in the map selector spawns in ocean
- `PlanetZone` and `SurfaceWaterState` classifications are coherent with
  the macro debug images
- LOD0, LOD1, LOD2 representations of the same chunk agree on ocean vs land
  and on planetary zone
- Tectonic plate boundaries are stable across render scales

Local terrain detail (`peaks_valleys`, `rock_hardness`, local humidity
variation) continues to differ between macro and micro representations — this
is correct and intended. Identity is stable; detail varies by scale.

---

## Implementation Notes

Spec 007 is implemented, but the final shipped work is broader than the
original Rust-only framing because the verification and player-facing map
surfaces also had to be repaired.

### Shipped Changes

Implemented outcome:

- world-anchored identity layers now hold stable across macro and runtime
  generation, so macro-vs-runtime ocean/land agreement is materially improved
- runtime coherence tests and the presentation fixture were updated, and
  `cargo test --manifest-path gdextension/Cargo.toml` passes
- the in-game Compare Generation flow ships from the map selector, but the
  implemented UI is a four-panel diagnostic rather than the original
  three-panel concept:
  - `Macro Visual (biome.png)` for world context
  - `Runtime Local Map (LOD0)` built from the same chunk data the terrain uses
  - `Macro Colours over Runtime` as a bridge view
  - `Delta` for ocean drift plus land-biome and water-biome drift
- macro comparison semantics no longer rely on sampling `biome.png` colours to
  infer truth; the compare tool generates fresh macro semantic data at
  `freq_scale=1.0` and compares that against runtime chunk semantics
- the selector preview, compare runtime panel, and in-level `[M]` local map
  now share the same LOD0 data-driven local-map path
- `CoralReef` was removed because it created ambiguous underwater-biome
  behaviour and repeatedly confused comparison output
- the compare modal now has a legend, and water-biome drift is rendered as a
  diagnostic hatch instead of a reef-looking fill

### Top-Down Local Map Pipeline

The local map implementation is intentionally **not** a camera render. It is a
top-down terrain diagram generated from the same runtime chunk data that the
terrain mesh uses.

- the local map is not a camera render or a screenshot of the 3D scene
- it is a data-driven raster built from the runtime `LOD0` chunk `MgBiomeMap`
- the shared renderer lives in `scripts/ui/runtime_chunk_preview_renderer.gd`
- the runtime chunk source data comes from
  `GenerationManager.generate_runtime_chunk_for_lod_with_seed(seed, chunk_coord, "LOD0")`
  which resolves to:
  - resolution `512`
  - `detail_level = 2`
  - `freq_scale = 8.0`
- that means the renderer is looking at the same runtime chunk scale used for
  the traversed terrain, not the old `LOD2` compare proxy
- the local-map image is built from three runtime data products taken from that
  `MgBiomeMap`:
  - `block_heights(HEIGHT_SCALE)` for terrain elevation
  - `is_ocean_grid()` for fluid/ocean occupancy
  - `export_layer_rgba("biome")` for biome identity colour
- the renderer produces three distinct artifacts for every chunk:
  - `image`: the player-facing top-down local map
  - `biome_image`: raw biome colour identity at runtime resolution
  - `ocean_mask_image`: binary fluid/ocean truth at runtime resolution
- ocean pixels are identified from the runtime fluid mask, then coloured as a
  blue water ramp derived from depth and hillshade
- land pixels are coloured from height, slope, contour accents, and then
  lightly tinted toward the runtime biome colour so the local map still reads
  as terrain first
- the player-facing map therefore mixes:
  - hard runtime truth for water occupancy
  - real runtime terrain relief from the height field
  - a restrained amount of biome colour for orientation
- the same renderer output is reused by:
  - the selector preview
  - the compare runtime panel
  - the in-level `[M]` local map overlay

### Coordinate and Data Linkage

The local map and the macro map are linked by world coordinates, not by
sampling one image into the other.

- compare selection starts from a world-region / meso-region pick on the macro map
- each runtime compare cell maps to a runtime `chunk_coord`
- each `chunk_coord` is converted to generator-space world origin via
  `GenerationManager.chunk_coord_to_world_origin(...)`
- runtime generation then samples that world origin at `LOD0`
- macro comparison generates the same world region at `freq_scale = 1.0`

This means macro and runtime are linked through the same absolute world-space
coordinates and generator rules, rather than by trying to visually align two
unrelated images after the fact.

### Macro Map Linkage and Compare Truth

- the visible macro panel in compare remains a crop of `biome.png` from the
  newest layers artifact because that is the user-facing macro representation
  of the world
- however, compare truth is not inferred from the PNG palette anymore
- instead, the compare tool generates fresh macro semantic data with
  `MgTerrainGen.generate_region(..., freq_scale = 1.0)` over the selected
  world region
- from that generated macro `MgBiomeMap`, compare derives:
  - macro biome identity via `export_layer_rgba("biome")`
  - macro ocean truth via `is_ocean_grid()`
- runtime truth is derived from the runtime `LOD0` chunk data in the same way:
  - runtime biome identity via `export_layer_rgba("biome")`
  - runtime ocean truth via `is_ocean_grid()`
- this means `biome.png` remains the visual macro context, but macro-vs-runtime
  scoring now comes from generated semantic data on both sides instead of a
  colour heuristic on one side and true data on the other

### What Each Compare Panel Actually Means

The compare modal now mixes context panels and scored panels. This is important
because not every visible panel is itself a source of truth.

- `Macro Visual`
  - this is a crop of `biome.png`
  - it is context for the user because this is the macro world representation
    they actually navigate with
  - it is not the sole scoring surface
- `Runtime Local Map`
  - this is the player-facing LOD0 top-down local map raster
  - it is derived from real runtime chunk data
  - it is meant to answer “what would the player traverse here?”
- `Macro Colours over Runtime`
  - this is only a bridge/intuition panel
  - it helps the eye relate macro colour regions to runtime terrain shapes
  - it is not a scoring surface
- `Delta`
  - this is the actual diagnostic panel
  - it overlays disagreement on top of runtime terrain
  - it currently distinguishes:
    - matching ocean
    - macro ocean only
    - runtime ocean only
    - water-biome drift
    - land-biome drift

### What Is Scored Versus What Is Merely Shown

Scored truth:

- macro ocean mask from generated macro `is_ocean_grid()`
- runtime ocean mask from runtime `LOD0` `is_ocean_grid()`
- macro biome colour identity from generated macro `export_layer_rgba("biome")`
- runtime biome colour identity from runtime `LOD0` `export_layer_rgba("biome")`

Shown for context / readability:

- the visible `biome.png` crop
- the player-facing runtime top-down local map
- the washed macro-over-runtime bridge panel

This distinction matters because the compare tool intentionally preserves the
user-facing macro map and player-facing local map while scoring against the
semantic generator data behind them.

### Current Limits of the Implemented Compare

The current compare is much more honest than the old macro-vs-LOD2 proxy, but
it still has known limitations:

- ocean/land agreement is the strongest signal and the main receipt for this
  spec
- exact biome mismatch is still noisier than ocean/land agreement because it
  compares exact runtime/macro biome colours rather than normalized biome
  families
- the runtime local map is a presentation raster, while the biome mismatch
  scoring still comes from the raw runtime biome image; this is correct but can
  be visually confusing if not explained
- the compare legend and hatch pattern reduce confusion, but they do not change
  the underlying scoring model
- `biome.png` must be regenerated after macro biome semantic changes or the
  visible macro context becomes stale even if the semantic compare path is fresh

Current interpretation:

- ocean/land agreement is the strongest signal and the main receipt for this
  spec
- exact biome mismatch is useful as a diagnostic, but it is still noisier than
  the ocean/land signal and should not be treated as a hard failure in the same
  way

---

## Risks

### Terrain appearance will change

World-anchored `continentalness` evaluated at `(wx, wy)` will produce
different values than the current scaled evaluation at
`(wx * freq_scale, wy * freq_scale)`. Continents and oceans will be in
different places than the current runtime. This is not a regression — the
current terrain is wrong relative to the macro map. After the fix, runtime
terrain matches the macro map.

The existing CLI golden fixture at
`testdata/runtime_presentation/seed42_v1_step256.ron` will need to be
regenerated after this change because it was built against the broken
architecture.

### Humidity terminator coupling

`humidity` uses `generate_terminator_model` which takes `light_level` as an
input but is evaluated at scaled coordinates. This existing coupling mismatch
is out of scope for this spec. Humidity drives local moisture variation and
rain shadow effects; it does not gate ocean/land or planetary zone
classification. Keeping it freq-scaled is the pragmatic choice now. A future
spec can revisit if terminus moisture zones become visually wrong.

### Within-chunk continentalness variation

At `freq_scale=8.0`, the current architecture produces continentalness
variation within a single micro chunk — which can sometimes make a chunk
appear half-ocean, half-land. After anchoring continentalness to world
coordinates, a 1×1 world-unit chunk spans a much smaller slice of the
unscaled noise space, so continentalness will be near-constant within one
chunk. This is intentional: ocean vs land is a chunk-level identity, not a
sub-chunk detail. Coast variation within a chunk comes from the
`peaks_valleys` relief layer modulating height around the sea level threshold
at the coast edge, not from within-chunk continentalness oscillation.

---

## Verification

The primary verification for this spec is a **visual scale-comparison tool**
that makes the before/after incoherence immediately legible. Unit tests are a
safety net; the comparison tool is the receipt.

---

### Compare Generation Tool

#### In-game flow

A new mode added to the map selector:

```
Quick Launch → Open Map → Compare Generation
```

The user clicks a region on the macro map. The implemented tool generates a
four-panel view:

| Panel | Content |
|---|---|
| **Macro Visual** | Cropped `biome.png` texture for the selected region |
| **Runtime Local Map** | NxN grid of LOD0 local-map previews built from runtime chunk data |
| **Macro Colours over Runtime** | Macro colours washed over the runtime terrain preview |
| **Delta** | Ocean drift plus land-biome and water-biome drift |

Default grid size is 8×8. Agreement percentages and mismatch counts are shown
as labels, and the modal includes an on-screen legend for the overlay colours.

**Before the fix:** macro can show one coastline identity while runtime
generates another at the same world position.

**After the fix:** ocean agreement is high, and the remaining disagreements are
mostly concentrated near coastline drift or biome-family drift instead of whole
region identity failure.

#### CLI command

```bash
margins_grip compare-scale <seed> <wx> <wy> <grid_size> <output_dir>
```

Generates the macro/runtime/diff artifacts plus a sidecar JSON:

```
output_dir/
  macro.png          # cropped macro biome image
  micro_grid.png     # NxN runtime local-map grid
  diff.png           # ocean + biome drift overlay
  agreement.json     # per-cell and overall agreement stats
```

`agreement.json` format:

```json
{
  "seed": 42,
  "origin": [200, 100],
  "grid_size": 8,
  "overall_agreement": 0.953,
  "cells": [
    { "chunk": [200, 100], "macro_ocean": true, "micro_ocean": true, "agree": true },
    ...
  ]
}
```

This is what the agentic harness reads and asserts against.

#### Agentic harness assertion

The agent triggers the comparison via CLI, reads `agreement.json`, and fails
the session if `overall_agreement < 0.95` for a known-ocean region:

```python
# pseudocode — actual implementation in agentic test script
result = run("margins_grip compare-scale 42 200 100 8 /tmp/compare_out")
data = json.load("/tmp/compare_out/agreement.json")
assert data["overall_agreement"] >= 0.95, f"Scale coherence below threshold: {data}"
```

On failure, the diff PNG is saved as evidence in the session artifact directory.

---

### Unit tests

These cover specific known coordinates programmatically and run in CI.

**Test 1 — Cross-scale ocean coherence**

```rust
#[test]
fn ocean_classification_is_stable_across_freq_scales() {
    for (wx, wy) in REFERENCE_OCEAN_COORDS {
        let macro_map = BiomeMap::generate(SEED, wx, wy, 64.0, 64.0, 256, 256, 0, false, false, 1.0);
        let micro_map = BiomeMap::generate(SEED, wx, wy, 1.0, 1.0, 512, 512, 2, false, false, 8.0);
        assert_eq!(macro_ocean(macro_map, wx, wy), micro_ocean(micro_map));
    }
}
```

Choose `REFERENCE_OCEAN_COORDS` from known ocean positions visible in the
macro biome image for seed 42. Minimum 5 coordinates.

**Test 2 — Cross-LOD classification stability**

```rust
#[test]
fn runtime_presentation_is_stable_across_lod() {
    for (wx, wy) in REFERENCE_COORDS {
        let lod0 = build_presentation(SEED, wx, wy, 512, 2, 8.0);
        let lod2 = build_presentation(SEED, wx, wy, 65, 0, 8.0);
        assert_eq!(lod0.planet_zone, lod2.planet_zone);
        assert_eq!(lod0.water_state, lod2.water_state);
    }
}
```

---

### Regenerate the golden fixture

After all tests pass, regenerate the RON fixture:

```bash
margins_grip inspect layer-presentation-grid post_fix 256 > \
  testdata/runtime_presentation/seed42_v1_step256.ron
```

Commit the new fixture alongside a `compare-scale` diff PNG for seed 42 at
a known ocean region as the canonical visual receipt of the fix.

---

## Modifies

### Phase 1 — Rust fix (Codex)

```text
gdextension/crates/mg_noise/src/biome_map.rs
  - change continentalness generate() call from (nwx, nwy) to (wx, wy)
  - change tectonic generate_full() call from (nwx, nwy) to (wx, wy)
  - split use_wrap into world_wrap=true and detail_wrap=(freq_scale==1.0)
  - pass world_wrap to continentalness and tectonic strategy calls
  - pass detail_wrap to peaks_valleys, rock_hardness, humidity strategy calls

gdextension/crates/mg_noise/src/strategy/continentalness.rs
  - accept wrap flag as parameter (or verify existing wrap parameter path)

gdextension/crates/mg_noise/src/strategy/tectonic.rs
  - accept wrap flag as parameter; confirm Voronoi period wrapping works at
    world scale

gdextension/crates/mg_noise/src/runtime_presentation/mod.rs (tests)
  - add cross-scale and cross-LOD coherence tests

testdata/runtime_presentation/seed42_v1_step256.ron
  - regenerate after implementation
```

### Phase 2 — Compare Generation tool

```text
gdextension/src/bin/cli.rs
  - add `compare-scale <seed> <wx> <wy> <grid_size> <output_dir>` subcommand
  - generates macro.png, micro_grid.png, diff.png, agreement.json

scripts/ui/map_selector.gd
  - add "Compare Generation" button/mode

scripts/ui/compare_generation_view.gd  (new)
  - four-panel layout with legend
  - compares fresh macro semantic data against runtime chunk semantics
  - shows runtime local-map previews instead of LOD2 biome proxy images
  - overlays ocean drift plus land/water biome drift

scripts/ui/runtime_chunk_preview_renderer.gd  (new)
  - shared LOD0 local-map renderer for compare, selector preview, and in-level map

scripts/ui/map_overlay.gd
  - switch in-level local map to the shared LOD0 renderer

scripts/world/world.gd
  - developer screenshot probe for verifying the in-level map overlay
```

---

## Acceptance Criteria

### Phase 1 — Rust fix
- `ocean_classification_is_stable_across_freq_scales` passes for ≥5 known ocean coordinates
- `runtime_presentation_is_stable_across_lod` passes for ≥5 diverse world coordinates
- `default_grid_audit_passes_for_seed_42_step_256` still passes (coverage maintained)
- golden fixture regenerated and committed

### Phase 2 — Compare Generation tool
- `margins_grip compare-scale 42 <ocean_region> 8 <out>` produces `agreement.json`
  with `overall_agreement >= 0.95`
- diff PNG committed alongside golden fixture as visual receipt
- in-game Compare Generation mode accessible from map selector
- four-panel view renders without errors for any valid map region click
- agentic harness can trigger compare-scale, read agreement.json, and assert threshold

---

## Codex Prompt

Read `specs/007-world-anchored-noise-layers.md` in full before starting.

This prompt covers **Phase 1 only** (the Rust fix). The Compare Generation
tool (Phase 2) is a separate implementation task.

This spec fixes a fundamental coordinate scaling bug in the Rust terrain
generator. The generator uses `freq_scale` to multiply noise coordinates
before sampling. This is correct for detail layers (peaks, rock, local
humidity) but wrong for identity layers (continentalness, tectonic) which
should be stable across all render scales.

**Implement the following changes to
`gdextension/crates/mg_noise/src/biome_map.rs`:**

1. Change the `continentalness` strategy call from `(nwx, nwy)` to `(wx, wy)`
2. Change the `tectonic` strategy call from `(nwx * freq_scale, ...)` to
   `(wx, wy)` — check the exact call site
3. Split the single `use_wrap` flag into `world_wrap = true` (for
   continentalness and tectonic) and `detail_wrap = freq_scale == 1.0`
   (for peaks_valleys, rock_hardness, humidity)
4. Pass the correct wrap flag to each strategy's generate call
5. Confirm that `light_level` is unchanged (it already uses `wx, wy`)

**Then add tests to
`gdextension/crates/mg_noise/src/runtime_presentation/mod.rs`:**

1. A test that generates the same world coordinate at both `freq_scale=1.0`
   and `freq_scale=8.0` and asserts ocean/land agreement
2. A test that generates the same world coordinate at LOD0
   (`detail=2, freq_scale=8.0`) and LOD2 (`detail=0, freq_scale=8.0`) and
   asserts that `planet_zone` and `water_state` match

**Then regenerate the golden fixture:**

```bash
cargo run --release --bin margins_grip -- \
  inspect layer-presentation-grid <latest_tag> 256 \
  > gdextension/src/testdata/runtime_presentation/seed42_v1_step256.ron
```

Do not change `biome_splines.rs`, the GDExtension API, or any Godot-side
code. The fix is entirely inside the Rust noise generation coordinate
dispatch.

Run `cargo test` in the gdextension workspace after implementing. All
existing tests must pass plus the two new coherence tests.
