# Spec 010 — Randlebrot-Style Macromap Replication

**Status:** Proposed
**Priority:** High
**Depends On:** Spec 009

## Problem

The current `mgrip` macro world image does not yet reproduce the look or the
production shape of Randlebrot's macromap pipeline.

Right now the project is mixing several concerns together:

- trying to port the Randlebrot visual style
- trying to repair broken runtime and compare behavior
- trying to settle macro-truth contracts at the same time

That is too much scope for one spec.

This spec should concentrate on one thing:

> replicate the Randlebrot macromap pipeline and artifact as closely as
> practical in `mgrip`.

At the same time, the desired visual target is valid.

The macro outcome should feel materially closer to Randlebrot's
`debug_layers/biome.png`. In `mgrip`, the equivalent exported artifact should
be named `macromap.png`:

- stronger relief and ridge composition
- connected river hierarchy
- cleaner coastlines and ocean read
- broader, more coherent planetary climate bands
- a composed cartographic result rather than stacked debug layers

This project already has a stronger contract from Spec 007 on the runtime side:

- runtime truth comes from runtime `LOD0` chunk data
- the runtime local map is a terrain-first top-down raster, not a macro atlas

This spec intentionally defines only the macro-side artifact contract:

- `macromap.png` replaces `biome.png` as the authoritative persisted macro
  artifact
- macro-facing review and future macro-side tooling derive from `macromap.png`

This spec is therefore not "fix the whole terrain/runtime stack."

It is:

> reproduce the Randlebrot macromap-generation pipeline closely enough that
> `mgrip` can ship an authoritative `macromap.png` that looks and is produced
> like the Randlebrot macromap.

## Current Reality

### What is missing now

**Root cause 1 — There is no single authoritative macro artifact yet.**

The port introduces a composited terrain render, but the artifact contract is
not finished. If macro rendering, ocean-mask logic, and compare behavior do not
anchor to one stable persisted macro artifact, downstream behavior will remain
confused and fragile.

**Root cause 2 — The production shape is not locked to Randlebrot yet.**

The current work references Randlebrot visually, but without explicitly locking
to its real generation pipeline:

- macro world pass
- retained global river network
- meso tile pass
- global normalization
- stitched macro image
- final `2x` downscale

Without that, the project can drift into a similar-looking but materially
different pipeline.

### What the visual target is

The Randlebrot reference image demonstrates the intended macro qualities:

- coherent large-scale landform structure
- anti-aliased connected rivers
- believable coastline and ocean depth treatment
- strong banded climate readability
- an integrated atlas-like composition

### What must remain true

From the project invariants:

- Margin must not drift into generic Earth aesthetics
- no green vegetation palette anywhere
- nightside must remain frozen and dark
- the terminus must remain the habitable ring
- the dayside must remain hot and harsh

## Goals

1. Replicate the Randlebrot macro-map production pipeline closely enough to
   reproduce its visual structure and layer behavior.
2. Make `macromap.png` the one authoritative persisted macro artifact.
3. Use GPU-backed generation for the macromap pipeline when GPU is available.
4. Preserve Margin's tidally locked climate identity.
5. Produce a final macro output that is visibly closer to the Randlebrot
   reference than the current `mgrip` macro image.

## Non-Goals

- repairing runtime level generation in this spec
- repairing compare-generation semantics in this spec
- updating runtime presentation fixtures in this spec
- making the runtime local map look like the macro image
- introducing Earth-like green vegetation rendering
- refactoring unrelated Godot ownership boundaries
- inventing new map UX beyond the current selector, compare, and local-map
  flows

Runtime health, compare health, and fixture repair should be handled after the
macromap pipeline itself is stable.

## Design

### Section 1 — Repair runtime and build paths first

This spec does not lead with runtime repair.

It is acceptable for the implementation plan to stage temporary breakage or
follow-up cleanup outside this spec, as long as the result of this spec is a
clear and correct Randlebrot-style macromap pipeline.

### Section 2 — One authoritative macro artifact

The macro pipeline should distinguish between:

1. **generator internals**
2. **the persisted macro artifact**
3. **runtime/local-map views**

The authoritative persisted macro artifact under this spec is `macromap.png`.

#### Required artifact contract

For work delivered under this spec, the macro layers artifact must have this
explicit output contract:

- `macromap.png`
  - required
  - the one primary persisted macro image artifact
  - visible macro context for compare and review
  - authoritative macro artifact for compare and macro-facing UI
- `manifest.layer_images`
  - must list `macromap.png`

Consumer contract:

- macro-facing review reads `macromap.png`
- future macro-side consumers should read `macromap.png`
- selector preview and in-level `[M]` local map are not changed by this spec

If `biome.png` continues to exist during migration, it is non-authoritative and
debug-only under this spec.

### Section 3 — Replicate the Randlebrot macromap pipeline

`mgrip` should replicate the Randlebrot production shape closely.

The reference pipeline in Randlebrot is:

1. Generate one macro `BiomeMap` for the full world.
   - world size `1024 x 512`
   - erosion and river-network generation happen in this pass
2. Keep the global `RiverNetwork` from that macro pass.
3. Generate a `16 x 8` grid of world tiles.
   - `64 x 64` world units per tile
   - `512 x 512` pixels per tile
   - `detail_level = 1`
   - each tile is generated against the macro map and the global river network
4. Compute one global height range from all tile heightmaps.
5. Stitch each `NoiseLayer` from the per-tile outputs into one large image.
   - full stitched size `8192 x 4096`
6. Downscale the stitched result `2x` with a box filter.
   - final exported size `4096 x 2048`
7. Save the final macro image plus the other stitched layers.

For the macro image itself, the Randlebrot hook is:

- `NoiseLayer::Biome`
- `BiomeMap::to_layer_image_with_hints(...)`
- `terrain_render::render_terrain(...)`

That means the exported top-level macro image is not a flat biome-colour PNG.
It is a composited terrain render derived from the full tile data.

`mgrip` should replicate that exact shape:

- one macro world pass
- one global river network
- one meso tile pass
- one global normalization pass
- one stitched-and-downscaled final `macromap.png`

The goal is not to invent a different macro atlas pipeline. The goal is to
port this one cleanly.

### Section 4 — GPU requirement and intentional deviation from Randlebrot

Randlebrot's current code accepts backend parameters but forces CPU in the
macro and meso generation paths because its GPU path does not yet preserve
horizontal wrapping correctly.

`mgrip` should not copy that limitation as the target behavior.

Under this spec:

- the macromap pipeline should use GPU-backed generation when GPU is available
- CPU fallback remains allowed for non-GPU environments and debugging
- GPU output must be treated as the primary path, not the emergency fallback
- GPU and CPU outputs must stay close enough that the fixed runtime fixtures
  and compare receipts remain valid

So the project should copy the Randlebrot pipeline shape, but not its current
CPU-forcing workaround.

### Section 5 — Match the Randlebrot look in the right way

The desired similarity to Randlebrot is about visual qualities, not blindly
copying every color or assumption.

The composited macro presentation should improve in these areas:

- ridge and relief coherence
- river hierarchy and continuity
- anti-aliased river rasterisation
- coastline readability
- ocean depth rendering
- integrated climate-zone composition
- reduction of obvious debug-layer / sector-artifact feel

But all of that must still be adapted to Margin:

- the terminus is the habitable band
- the nightside reads frozen and dark
- the dayside reads hot and harsh
- vegetation/moisture treatment must not become generic Earth-green

### Section 6 — Scope of the no-green rule

The "no green vegetation palette" invariant applies to all player-facing and
developer-facing world presentation outputs touched by this spec:

- `macromap.png`

This rule is about overall read, not the absence of every cool-toned pixel.
Muted teal, sickly damp tones, or desaturated fungal/wetland hints are allowed
where appropriate. Broad land areas must not read as Earth-green vegetation.

### Section 7 — Make verification part of the contract

This is high-risk generation work. The feature is incomplete unless the
macromap receipts are green.

Required verification includes:

- generating the macromap artifact successfully
- screenshot receipts for macro visual quality
- explicit checks for nightside, terminus, and dayside identity

#### Fixed macromap review regions

The following seed `42` regions are frozen by this spec for screenshot review
of the generated `macromap.png`:

- deep nightside
  - world `(256.0, 0.0)`
- dry dayside margin
  - world `(400.0, 250.0)`
- substellar inferno
  - world `(500.0, 450.0)`

Final review must also include one cold-terminus and one humid-terminus receipt
chosen from the regenerated `macromap.png` and called out explicitly in the
implementation PR or handoff notes.

#### Fixed reference-comparison geometry

The Randlebrot reference image at
`/Users/roryhedderman/Documents/IdeaProjects/Rust/randlebrot/debug_layers/biome.png`
is `4096 x 2048`.

For strict side-by-side review, the generated `macromap.png` must be reviewed
at the same final export size:

- generated `macromap.png`
  - `4096 x 2048`
- Randlebrot reference `biome.png`
  - `4096 x 2048`

The fixed review regions above must also be compared as matched crops using the
same geometry in both images:

- map world coordinates to image coordinates at `4 px / world unit`
- take a `256 x 256` crop centered on each frozen review coordinate
- if a crop would cross an image edge, clamp it to the image bounds

This produces reproducible region receipts instead of a loose whole-image
impression.

## Modifies

### Rust files

```text
gdextension/crates/mg_noise/src/rivers.rs
gdextension/crates/mg_noise/src/biome_map.rs
gdextension/crates/mg_noise/src/terrain_render.rs
gdextension/crates/mg_noise/src/gpu/*
gdextension/src/bin/cli.rs
```

### Godot files

Only if needed to surface `macromap.png` in existing macro-facing UI:

```text
scripts/ui/map_selector.gd
scripts/ui/map_overlay.gd
```

### No ownership changes

Do not create parallel ownership paths for:

```text
scripts/autoload/generation_manager.gd
scripts/world/world.gd
scripts/world/chunk_streamer.gd
scripts/player/fps_controller.gd
```

## Verification

1. **Regenerate macro layers**
   ```sh
   cargo run --release --manifest-path gdextension/Cargo.toml --bin margins_grip -- generate layers 42 spec010-seed42
   ```

2. **Inspect outputs**
   - persisted `macromap.png`
   - any remaining debug-only `biome.png`
   - stitched base and derived layer exports
   - manifest contents

3. **Reference comparison**
   Compare the generated `macromap.png` side-by-side against the Randlebrot
   reference image:
   - `/Users/roryhedderman/Documents/IdeaProjects/Rust/randlebrot/debug_layers/biome.png`
   Required review receipts:
   - one full-image side-by-side at `4096 x 2048`
   - matched `256 x 256` crops for:
     - deep nightside
     - dry dayside margin
     - substellar inferno
   - one matched cold-terminus crop and one matched humid-terminus crop,
     explicitly labeled in the implementation PR or handoff notes

4. **Screenshot receipts**
   Use windowed screenshot probes where final rendered appearance matters.

## Acceptance Criteria

1. The layers artifact contains `macromap.png` as the one primary persisted
   macro image and lists it in the saved layer manifest.

2. The implemented macromap pipeline matches the Randlebrot production shape:
   - one macro world pass
   - one global river network
   - one meso tile pass
   - one global normalization pass
   - one stitched `8192 x 4096` intermediate
   - one `2x` box-filter downscale to the final macro image

3. `macromap.png` is the authoritative persisted macro artifact for
   macro-facing review and future macro-side tooling.

4. The macro presentation is visibly closer to the Randlebrot reference:
   - rivers read as connected drainage systems
   - coastlines and ocean depth are coherent
   - relief reads as a composed landform system
   - obvious sector/wedge artefacts are reduced or removed
   - the result stands up in side-by-side comparison against
     Randlebrot's `debug_layers/biome.png`
   - the fixed crop receipts for deep nightside, dry dayside margin, and
     substellar inferno hold up under matched `256 x 256` comparison

5. The result still reads as Margin across `macromap.png`:
   - no green vegetation palette
   - nightside remains frozen/dark
   - dayside remains hot/harsh
   - the terminus remains the habitable band

6. GPU-backed generation is the primary implemented path for macromap
    production when GPU is available, and CPU fallback does not change the
    frozen macromap review receipts materially.

7. Follow-up work for runtime health, compare health, and fixture repair is not
   blocked by uncertainty about how the macromap itself should be generated.

## Suggested Phases

### Phase 1 — Replicate the macromap pipeline

- port the Randlebrot tile-render + stitch + downscale macromap path
- make `macromap.png` the authoritative persisted macro artifact
- port the composited terrain-render hook for the macro image
- port the global normalization pass
- preserve Margin-specific climate and palette rules

### Phase 2 — Validate the macromap

- regenerate the macromap artifact
- collect screenshot receipts for the frozen review regions
- review against the Randlebrot reference

### Phase 3 — Follow-on health work

- runtime generation repair
- compare-generation repair
- fixture and audit repair
