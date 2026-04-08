# Spec 002b - Horizon and LOD Expansion

**Status:** Open
**Priority:** High
**Depends On:** Spec 002a complete

## Problem

Once near streaming works, the world can still feel locally correct but
visually finite:

- the visible world may still stop too close to the player
- mid and far terrain may still be too expensive if rendered like near terrain
- chunk activation may stall if loads are not prioritized
- seams may appear between representations at different distances

This spec extends the foundation from `002a` outward.

## Preconditions

Do not start this spec until:

- `002a` success criteria are met
- chunk naming is already coherent
- activation metrics are being recorded
- the runtime already has explicit chunk ownership and state

## Goals

1. Extend visible terrain meaningfully beyond the near ring.
2. Add chunk LOD representations without breaking the chunk naming model.
3. Prioritize chunk activation so movement stays stable.
4. Verify continuity between nearby chunks, far terrain, and the macro map.
5. Improve map overlay debugging for streamed-world work.

## Non-Goals

- final shipping optimization pass
- full gameplay systems
- final biome/material polish for every distance tier
- perfect visual parity between all LOD representations

## Chunk Naming Carry-Forward

`002a` naming rules remain in force.

Additional rules for this spec:

- a chunk is still the ownership unit
- `LOD0`, `LOD1`, and `LOD2` describe how a chunk is represented, not what it
  is
- if a far representation is built from larger region data, name that source
  clearly as a `region tile` or `region proxy`, not a chunk
- do not reintroduce ambiguous phrases like `meso chunk` unless the system
  genuinely uses that term as a runtime ownership unit

## Constraints

- Keep far terrain geographically coherent with the same world coordinate model.
- Do not add collision to mid or far representations by default.
- Prefer real terrain silhouettes over fake horizon rings.
- Do not rewrite Rust generation without measured evidence.
- Keep activation conservative until metrics justify widening it.

## Modifies

Expected primary files:

```text
scripts/world/world.gd
scripts/world/chunk_streamer.gd
scripts/world/world_chunk.gd
scripts/world/terrain_lod_builder.gd
scripts/world/chunk_metrics.gd
scripts/ui/map_overlay.gd
scenes/world.tscn
gdextension/src/lib.rs            (only if measurement proves generator-side support is needed)
```

## Architecture Direction

### 1. LOD As Representation, Not Identity

Keep chunk identity stable and vary only representation by distance:

- `LOD0`
  playable nearby representation
- `LOD1`
  cheaper mid-distance visual representation
- `LOD2`
  very cheap far-distance silhouette representation

This helps avoid naming drift and makes lifecycle ownership easier to follow.

### 2. Horizon From Real Terrain

The horizon should come from the real world model, not a decorative fake ring.

That means:

- far terrain must be placed in the same coordinate space
- silhouettes should agree with the macro map
- the visible world should feel geographically continuous

### 3. Prioritized Activation

Chunk activations must be ordered intentionally:

1. current player chunk
2. immediate neighbors
3. forward-facing nearby chunks
4. side and rear chunks
5. far horizon representations

Keep activation caps until profiling proves they can safely widen.

### 4. Seam Auditing

Continuity must be checked across:

- adjacent chunks at the same representation level
- near-to-mid transitions
- mid-to-far transitions
- macro map alignment versus visible terrain

## Suggested Initial Parameters

Start conservatively:

```gdscript
LOD0_ACTIVE_RADIUS := 1
LOD1_RADIUS := 3
LOD2_RADIUS := 6
MAX_CHUNK_ACTIVATIONS_PER_FRAME := 1
```

Adjust only after measurement.

## Implementation

### Section 1 - Add Mid-Distance Representation

Introduce a cheaper representation for terrain beyond the playable ring.

First-pass simplifications:

- lower mesh density
- no collision
- no clutter
- simpler material path allowed

Prefer a small number of clearly owned representations over many clever tiers.

**Verification:** The player can see beyond the near ring without immediate
frame collapse.

---

### Section 2 - Extend To a Real Terrain Horizon

Push visible terrain far enough out to create a believable horizon.

The first success condition is:

- coherent silhouettes
- no obvious edge-of-world cutoff
- no giant visible holes during movement

It does not require final material polish.

**Verification:** From elevated terrain, the player can see meaningful terrain
masses beyond the near ring.

---

### Section 3 - Prioritize Streaming Work

Prevent large request bursts from causing chaotic activation.

Use distance and view direction to prioritize:

- current chunk first
- near visible chunks next
- far horizon work last

**Verification:** Crossing chunk boundaries feels stable rather than collapsing
into long stalls.

---

### Section 4 - Audit Seams and World Coherence

Check for:

- cracks between neighboring chunks
- height mismatches at shared borders
- harsh visual jumps between LODs
- disagreement between macro map direction and visible terrain placement

Fix the common cases before polishing materials.

**Verification:** Normal movement does not reveal obvious cracks or giant
discontinuities.

---

### Section 5 - Upgrade Map Overlay For Streaming Debugging

Add streamed-world debugging information to the overlay:

- current chunk coord
- loaded chunk radius or active set if useful
- current representation counts
- local and macro views that still match runtime behavior

This remains a debugging tool, not final player UI.

**Verification:** The overlay helps explain what the streamer is doing while the
player moves.

## Provisional Success Budgets

Use these as initial guardrails on the current dev machine:

- steady-state movement should remain visually stable between activations
- chunk activation spikes over `50 ms` should be called out in logs
- multi-frame stalls over `150 ms` count as a failure to investigate
- any increase in visible range that breaks near-playable stability is too
  aggressive for this pass

These budgets can be adjusted once real measurements are collected.

## Final Verification

1. Move across several chunk boundaries in one direction.
2. Confirm nearby terrain remains playable.
3. Confirm distant terrain extends meaningfully beyond the near ring.
4. Confirm far terrain uses cheap representations without collision.
5. Confirm logs identify the dominant cost during chunk activation.
6. Confirm map overlay and macro map still match visible world direction.

## Tomorrow Start Prompt

```text
Read specs/002b-horizon-and-lod-expansion.md.
Assume 002a is already complete.
Implement sections 1 through 3 first:
- mid-distance terrain representation
- real terrain horizon extension
- prioritized chunk activation

Keep the chunk naming model from 002a intact.
LOD is a representation of a chunk, not a new chunk identity.
Do not add far collision unless profiling proves it is necessary.
```
