# Spec 008 - True Top-Down Runtime Chunk Preview

**Status:** Open
**Priority:** High
**Depends On:** Spec 003 and Spec 007

---

## Problem

The project currently has more than one visual story for a chunk:

- the player traverses the actual runtime terrain mesh built from the `LOD0`
  chunk generation path
- the compare tool currently shows a generated micro-map proxy
- launcher and debug flows still rely on biome-style 2D images that are not a
  literal top-down view of the terrain the player walks on

That creates a trust problem.

When the user clicks a chunk in the launcher or opens the compare tool, the
preview should answer one question clearly:

> "What will this chunk actually look like when I spawn into it?"

Right now the answer is "approximately this biome/debug representation", which
is not good enough. Even when the underlying world identity is now coherent
after Spec 007, the preview itself is still a proxy:

- it may use a lower-fidelity LOD than the active runtime chunk
- it may show biome colors instead of real traversable shape
- it may disagree with the 3D chunk mesh, water surface, and material result

For a chunk preview, "close enough" is the wrong contract.
The preview must be derived from the same runtime terrain the player traverses.

---

## Goals

1. Replace the current micro-map proxy with a true top-down preview of the real
   runtime chunk terrain.
2. Make the compare tool evaluate the macro map against exactly the chunk the
   player would traverse, not a biome-style stand-in.
3. Use one preview pipeline for launcher and compare flows so there is one
   source of visual truth.
4. Build the preview from the same `LOD0` chunk path used by the active nearby
   runtime terrain.
5. Keep the preview deterministic and visually stable enough for screenshots,
   reviews, and agentic verification.

## Non-Goals

- replacing the macro map with a rendered full-world top-down atlas
- changing spawn ownership away from `world.gd`
- changing chunk streaming ownership away from `chunk_streamer.gd`
- inventing a gameplay minimap or discovered-map progression system
- changing world generation truth beyond what Spec 007 already established
- making the preview match transient player-camera effects like shake, fog
  exposure drift, or momentary sky transitions

---

## Core Decision

The micro preview should be a **rendered orthographic top-down image of the
actual runtime chunk terrain**, not a separately rasterized debug map.

That means:

- generate the same `LOD0` chunk data used for active nearby terrain
- build the same terrain mesh and water mesh the runtime would use
- render that chunk from above in a dedicated preview viewport with a fixed,
  neutral presentation setup
- use that rendered image anywhere the UI needs a "what this chunk really looks
  like" preview

The preview is allowed to use a dedicated lighting and camera rig for clarity,
but it must still render the real chunk geometry and surfaces.

This is the product contract:

- **Macro map**
  world-scale planning view
- **Top-down runtime preview**
  exact chunk-terrain preview for launcher and compare flows

The biome debug image is no longer the user-facing micro-map.

---

## Why Rendering Is The Right Contract

The player does not traverse:

- a biome PNG
- a height heatmap
- a low-resolution classification proxy

The player traverses:

- the `LOD0` chunk mesh
- its water surface mask
- its terrain materials
- its runtime presentation-driven palette and atmosphere context

If the preview is intended to build trust in spawn choice and map comparison,
it should come from the same rendered terrain artifact.

Any alternative 2D reconstruction can drift:

- wrong LOD
- wrong silhouette
- wrong coastline edge
- wrong material read
- wrong water appearance

An orthographic render of the actual chunk avoids that class of mismatch.

---

## Current Reality

- `GenerationManager.generate_runtime_chunk_for_lod(...)` already defines the
  runtime chunk LOD contracts
- `LOD0` is the active near-runtime chunk:
  - `resolution = 512`
  - `detail_level = 2`
  - `freq_scale = 8.0`
- `WorldChunk` already treats `MgBiomeMap` as the source for runtime mesh and
  presentation metadata
- `VoxelMeshBuilder` already builds the terrain and water meshes from that
  chunk data
- the compare flow currently uses a generated micro proxy instead of a true
  render

So the missing piece is not terrain truth.
It is a reusable preview renderer that consumes that truth.

### Performance Reality

This spec deliberately replaces a cheap proxy with an expensive truth-based
preview.

Current compare generation cost is roughly:

- `8 x 8` cells
- each cell generated at `LOD2`
- `resolution = 65`
- `detail_level = 0`

Moving compare generation to a true `LOD0` preview means:

- `8 x 8` cells
- each cell generated at `LOD0`
- `resolution = 512`
- `detail_level = 2`

That is a very large increase in chunk-generation and render work.
Treat this as a first-class implementation constraint, not a cleanup detail.

This spec therefore requires:

- serial or low-concurrency preview generation for compare grids
- yielding between cells so the UI remains responsive
- a reusable preview renderer rather than dozens of live preview scenes
- a smaller default compare grid for the rendered preview path

Truth is more important than immediacy here, but the UI must still remain
usable while previews are being generated.

---

## Architecture Direction

### 1. One Reusable Preview Pipeline

Introduce a dedicated runtime chunk preview renderer used by:

- the launcher / map selector
- the compare generation view

Suggested ownership:

```text
scripts/ui/runtime_chunk_preview_renderer.gd
scenes/ui/runtime_chunk_preview_renderer.tscn   (optional)
```

This renderer owns:

- generating the `LOD0` chunk preview source
- building the preview scene contents
- rendering an orthographic top-down image to a texture
- caching results per `(seed, chunk coord)` when practical
- owning the lifecycle of the preview `SubViewport`, camera, and temporary mesh
  nodes used for capture

It should not own spawn decisions, world streaming, or player placement.

The renderer should accept an explicit `seed` parameter.
It must not implicitly read `GameState.world_seed`, because compare and launcher
flows may be operating before world runtime startup or against a seed loaded
from macro-map artifacts.

### 2. Preview Must Use The Runtime LOD0 Path

The preview must use the same chunk configuration as active nearby terrain:

- `resolution = 512`
- `detail_level = 2`
- `freq_scale = 8.0`

Do not use `LOD1` or `LOD2` for the user-facing preview.

Those LODs remain valid for horizon streaming, but they are not the source of
truth for "what the player will walk on right after spawning here".

### 3. Render Real Mesh, Not A Replacement Representation

The preview scene should render:

- the actual land mesh
- the actual water mesh
- the same runtime chunk material setup, or a deliberately simplified preview
  material path that still reads from the same chunk mesh and runtime
  presentation bundle

If preview-specific presentation tuning is needed, keep these rules:

- geometry must remain the real geometry
- water presence must remain the real water presence
- terrain silhouette must remain the real terrain silhouette
- do not substitute a 2D biome texture for the rendered chunk

### 4. Fixed Preview Camera And Environment

Use a dedicated orthographic camera above the chunk.

Requirements:

- camera looks straight down
- chunk is framed consistently
- lighting is stable and neutral enough that terrain shape reads clearly
- preview should avoid runtime haze/fog choices that obscure terrain legibility
- output should be deterministic enough for screenshot-based checks

The preview is not a beauty shot.
It is a trustworthy inspection view.

### 5. Reuse One SubViewport, Do Not Spawn A Grid Of Them

The compare tool must not create one live `SubViewport` per cell.

Instead, Phase 2 should use one reusable preview renderer instance that:

- mounts into the scene tree once
- renders one chunk preview at a time
- captures the texture
- reuses the same viewport/camera/preview scene for the next chunk

This avoids runaway scene-tree growth and makes lifecycle management clear.

Expected lifecycle:

1. create or mount preview renderer
2. request chunk preview
3. await render completion
4. copy the captured texture/image into the UI artifact
5. clear temporary preview chunk content
6. reuse renderer for the next chunk
7. free the renderer when the view closes if no cache is being retained

### 6. Compare Uses The Same Preview Artifact

The compare tool should show:

- **Macro**
  macro map crop
- **Runtime Preview**
  true top-down render of the `LOD0` chunk grid
- **Diff**
  semantic agreement overlay

Important:

The diff must not infer terrain truth from rendered preview colors.

The agreement computation should use chunk classification data derived from the
same runtime chunk source, while the visible preview panel uses the rendered
image.

Preserve the current good seam:

- runtime chunk data decides semantic agreement
- rendered preview is for human inspection

If the macro side needs classification, derive it from macro generation data or
an explicit semantic mask, not from image-color heuristics.

That keeps the preview honest and the diff meaningful.

### 7. Launcher Preview Replaces The Current Micro-Map Concept

In the map selector, when a chunk is hovered or selected, the side preview
should be the rendered runtime chunk preview.

The launcher should no longer present a biome-style micro-map as if it were the
real terrain.

The macro map remains the navigation surface for choosing chunks.
The runtime preview becomes the inspection surface for confirming the choice.

---

## UX Contract

### Launcher

The selector flow becomes:

1. open macro map
2. hover or select chunk
3. see true top-down runtime preview of that chunk
4. launch into the same chunk terrain the preview represented

The preview should answer:

- coastline shape
- terrain silhouette
- visible water coverage
- obvious terrain harshness / relief

### Compare Generation

The compare flow becomes:

1. choose macro region
2. show macro crop
3. show true top-down runtime chunk preview grid
4. show semantic agreement / disagreement overlay

This lets the user compare:

- what the macro world map promises
- what the actual traversable runtime chunk looks like

---

## Modifies

Expected primary files:

```text
scripts/ui/map_selector.gd
scripts/ui/compare_generation_view.gd
scripts/world/voxel_mesh_builder.gd
scripts/autoload/generation_manager.gd  (only for shared preview config helpers)
```

Expected new files:

```text
scripts/ui/runtime_chunk_preview_renderer.gd
scenes/ui/runtime_chunk_preview_renderer.tscn   (optional)
```

Possible Rust-side support additions if needed:

```text
gdextension/src/lib.rs
```

Only add Rust API surface if GDScript cannot cleanly render the preview from
existing chunk-generation and mesh-building seams.

---

## Implementation

### Phase 1 - Build A Reusable Runtime Chunk Preview Renderer

Add a dedicated renderer that:

- accepts explicit `seed` and `chunk coord`
- generates the real `LOD0` chunk source
- builds the terrain and water mesh for preview
- renders an orthographic top-down image to a `Texture2D`
- exposes an async completion path suitable for UI callers

Suggested implementation path:

- generate chunk via a seed-explicit preview path rather than relying on
  `GameState.world_seed`
- reuse `VoxelMeshBuilder` mesh assembly or a preview-safe extraction of it
- host the preview in a `SubViewport`
- render through a fixed orthographic camera
- return or emit a texture for UI consumers

The preview renderer should be shareable and not embedded directly inside map
selector logic.

**Verification:** a known chunk renders to a texture without loading the full
world scene.

---

### Phase 2 - Replace Compare Micro Panel

Update the compare generation view so the middle panel is the true top-down
runtime preview grid instead of a proxy biome image.

Rules:

- each cell uses the Phase 1 preview renderer
- each cell represents the actual `LOD0` runtime chunk
- the visible image is not derived from `export_layer_rgba("biome")`
- cells render asynchronously, one per deferred frame or similarly throttled
  cadence, so the compare view remains responsive
- the default rendered compare grid is `4 x 4`
- larger grids such as `8 x 8` are follow-up work unless performance proves
  acceptable with the serial renderer and caching path

The diff panel should still compute semantic agreement from real chunk data,
not from preview color heuristics.

**Verification:** compare view opens from the selector and displays a rendered
preview grid that clearly matches the chunk terrain shape without freezing the
UI.

---

### Phase 3 - Replace Launcher Micro Preview

Update the map selector so the per-chunk preview shown to the user is the true
runtime preview.

Requirements:

- hovering or selecting a chunk updates the preview
- the preview uses the same renderer as the compare tool
- preview loading states are clear and non-blocking
- repeated preview requests for the same chunk should reuse cached results when
  practical

**Verification:** selecting a chunk shows the same preview image later reused by
the compare flow for that chunk.

---

### Phase 4 - Verification And Capture Workflow

Add a verification path that proves the preview is representing the runtime
chunk rather than a proxy.

Preferred checks:

1. render a preview for a known chunk
2. launch the same chunk in a controlled runtime
3. capture a top-down or near-top-down evidence image from the live scene
4. manually confirm shape and water agreement

Automation does not need to solve full image matching in this spec, but the
capture workflow should make review straightforward.

---

## Risks

### Preview Performance

Rendering real `LOD0` chunks is heavier than painting a biome image.

Mitigations:

- use one reusable preview renderer for compare generation
- render compare cells serially with yields between cells
- keep rendered compare on a smaller default grid
- lazy-load only the selected / hovered chunk preview in launcher
- use chunk-preview caching
- cap preview texture resolution to a sane size such as `256` or `512`
- avoid loading unnecessary world runtime systems

### Seed Drift Between Macro Context And Preview Context

If the preview renderer reads `GameState.world_seed`, it can render the wrong
chunk when the launcher or compare flow is operating from macro artifact
context.

Mitigation:

- pass explicit seed into preview requests
- keep launcher and compare preview generation tied to the same seed source used
  for the selected macro map context

### Material Drift Between Preview And Runtime

If the preview uses different materials than runtime, it can reintroduce a
"looks similar but not the same" problem.

Mitigation:

- prefer reusing the same mesh and material configuration path
- if a simplified preview material is required, document exactly what is being
  simplified and keep geometry/water truth unchanged

### Camera Framing Drift

A badly framed top-down camera can make the preview feel cropped or misleading.

Mitigation:

- make framing deterministic
- cover the full chunk footprint consistently
- keep orthographic size tied to chunk bounds

### UI Stall During Preview Generation

If preview rendering happens synchronously on selection, the selector can feel
sticky.

Mitigation:

- render asynchronously where practical
- show loading state
- cache results by seed/chunk

---

## Acceptance Criteria

- the compare tool no longer uses a biome-style micro proxy for its user-facing
  middle panel
- the compare tool middle panel is rendered from the real `LOD0` runtime chunk
- the compare tool uses one reusable preview renderer, not a grid of concurrent
  live preview scenes
- the compare tool remains responsive while preview cells are being generated
- the launcher chunk preview is rendered from the real `LOD0` runtime chunk
- the same preview renderer is used by launcher and compare flows
- the preview is a top-down render of real terrain and water geometry, not a
  2D biome/debug raster
- the agreement overlay does not classify macro or micro truth from preview
  colors
- the preview for a chosen chunk is visually consistent with the terrain the
  player traverses after launch

---

## Out Of Scope Follow-Ups

These may become later specs:

- replacing the in-world debug `MapOverlay` local map with the same rendered
  preview pipeline
- generating multi-chunk stitched runtime preview atlases
- exact automated image comparison between preview and live world capture
- previewing time-of-day or atmosphere variants

---

## Codex Prompt

Read `specs/008-true-topdown-runtime-chunk-preview.md` in full before starting.

Implement **Phase 1 and Phase 2 only** in this pass.
Treat Phases 3 and 4 as follow-up work unless explicitly requested.

In this pass:

1. create a reusable runtime chunk preview renderer that renders a true
   top-down orthographic preview of the real `LOD0` chunk terrain
2. replace the compare view micro panel with that rendered preview
3. ensure the compare diff uses semantic terrain data, not preview colors
4. render compare cells asynchronously with one reusable preview renderer rather
   than many concurrent live `SubViewport`s
5. pass seed explicitly into preview generation; do not rely on
   `GameState.world_seed`

Do not replace the macro map.
Do not downgrade the preview to `LOD1` or `LOD2`.
Do not use biome PNG colors as a substitute for chunk truth.

Prefer reusing existing chunk-generation and mesh-building seams rather than
adding parallel terrain-generation ownership paths.
