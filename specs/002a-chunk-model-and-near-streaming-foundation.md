# Spec 002a - Chunk Model and Near-Streaming Foundation

**Status:** Open
**Priority:** Critical
**Depends On:** Spec 001 complete

## Problem

The game still has a single-chunk runtime path, and the word `chunk` is doing
too much work:

- the playable runtime unit is a 1.0 x 1.0 world-unit terrain chunk
- Rust also exposes a larger meso-scale generator output
- current comments and naming blur those together
- the runtime has no explicit chunk lifecycle yet
- we still do not know whether the main bottleneck is generation, meshing,
  collision, or scene attachment

Before adding horizon and LOD systems, we need one clear runtime model.

## Goals

1. Establish a coherent naming model for chunk-related concepts.
2. Add metrics around chunk activation before optimizing.
3. Introduce explicit chunk coordinate helpers and runtime ownership.
4. Replace the single-chunk assumption with a small near-streaming ring.
5. Keep the current playable terrain path intact while the architecture grows.

## Non-Goals

- far horizon rendering
- final LOD system
- final collision policy for every radius
- background threading unless metrics prove it is needed now
- final debug UI polish

## Naming and Coordinate Model

These names are mandatory in code comments, logs, helper names, and new docs:

- `world coord`
  Continuous generator-space coordinate used by Rust generation.
- `chunk coord`
  Integer runtime coordinate of one streamed chunk.
- `block coord`
  Local 0..511 coordinate inside one streamed chunk.
- `chunk`
  The runtime streamed ownership unit. A chunk is what gets requested,
  generated, meshed, attached, activated, and unloaded.
- `region tile`
  The larger meso-scale generator output. It is not a chunk.
- `macro map`
  The full-world debug output. It is not a chunk.
- `LOD0`, `LOD1`, `LOD2`
  Representations of a chunk, not separate coordinate systems.

Rules:

- Do not call a region tile a chunk.
- `GameState.current_chunk` must always mean the current runtime chunk coord.
- If public API renames are cheap, prefer names like
  `generate_micro_chunk()` and `generate_region_tile()`.
- If public renames are deferred, add wrappers or comments now so the naming is
  still coherent from GDScript outward.

## Constraints

- Keep the world-generation truth in Rust unless measurement gives a reason not
  to.
- Do not break the current single-chunk playable path while introducing the new
  runtime model.
- Favor explicit ownership over clever abstraction.
- Keep initial streaming conservative.
- No green in terrain, atmosphere, clutter, or materials.

## Current Reality

- [world.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/world.gd)
  still generates and activates one chunk synchronously at startup.
- [generation_manager.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/autoload/generation_manager.gd)
  does not yet expose a coherent chunk naming model.
- [game_state.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/autoload/game_state.gd)
  has `current_chunk`, but the wider runtime does not yet use it.
- [map_overlay.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/ui/map_overlay.gd)
  still assumes a single loaded local chunk.

## Modifies

Expected primary files:

```text
scripts/world/world.gd
scripts/autoload/generation_manager.gd
scripts/autoload/game_state.gd
scripts/ui/map_overlay.gd
scenes/world.tscn
gdextension/src/lib.rs            (only if naming cleanup requires small API wrappers)
```

Expected new files:

```text
scripts/world/chunk_streamer.gd
scripts/world/world_chunk.gd
scripts/world/chunk_metrics.gd
```

## Architecture Direction

### 1. Chunk Ownership First

Introduce one runtime unit of ownership:

- a chunk is keyed by `chunk coord`
- a chunk has explicit state
- a chunk owns visual representation, collision ownership, and cleanup

Minimum states:

- requested
- generating
- meshing
- active
- unloading

### 2. Metrics Before Optimization

Instrument the current path before changing too much.

Track at minimum:

- chunk coord
- requested representation or LOD
- Rust generation time
- mesh build time
- collision build time
- scene attachment time
- total activation time
- active chunk counts
- pending chunk counts

Print concise aggregate logs every few seconds before building fancy UI.

### 3. Explicit Coordinate Helpers

Add shared helpers for:

- world position -> chunk coord
- chunk coord -> world origin
- chunk-local block position
- chunk distance in chunk space

These helpers should become the source of truth used by world runtime,
debugging, and map overlay logic.

### 4. Near Streaming Only

The first streaming pass should only solve the nearby world:

- active radius: `1`
- preload radius: `2`
- unload radius: `3`
- max activations per frame: `1` initially

Success means:

`the player can move across chunk boundaries and keep seeing nearby terrain`

It does not yet require a full horizon.

### 5. Conservative Collision Policy

Do not assume the full 3x3 near ring can immediately afford collision.

First pass policy:

- keep collision simple
- measure collision cost explicitly
- allow the first implementation to keep collision narrower than visual loading
  if needed
- only widen collision coverage after measurement and boundary testing

## Implementation

### Section 1 - Codify Naming and Chunk Helpers

Update naming in:

- comments
- logs
- helper functions
- autoload APIs
- any new runtime classes

At minimum:

- reserve `chunk` for the streamed runtime unit
- stop describing meso-scale generation as a chunk unless it truly maps 1:1 to
  runtime chunk ownership
- centralize chunk/world conversion helpers

**Verification:** Logs, comments, and helper names consistently distinguish
`chunk`, `region tile`, `world coord`, and `block coord`.

---

### Section 2 - Add Runtime Metrics

Instrument the existing single-chunk path first, then keep the instrumentation
as the streamer is introduced.

Also print aggregate state every few seconds:

- active chunk count
- chunk count by representation or LOD
- pending requests
- recent average activation times
- worst spike in the last window

**Verification:** Moving around produces readable timing output and chunk counts.

---

### Section 3 - Introduce Explicit Chunk Runtime Objects

Add runtime classes that make ownership obvious:

- `world_chunk.gd`
  Holds per-chunk state and references.
- `chunk_streamer.gd`
  Owns request, activation, and unload decisions.
- `chunk_metrics.gd`
  Owns timing capture and aggregate reporting.

Ownership should be easy to answer from the code:

- who requested a chunk
- who owns its mesh
- who owns its collision
- who unloads it

**Verification:** The runtime can enumerate loaded chunks and their states.

---

### Section 4 - Track Current Player Chunk Explicitly

Make the runtime update `GameState.current_chunk` from the player's world
position using the new coordinate helpers.

Logs should clearly report chunk changes.

**Verification:** Crossing a chunk boundary updates the current chunk coord once
and only once.

---

### Section 5 - Stream the Near Ring

Replace the hardcoded single-chunk startup path with a streamer that can keep a
small nearby set loaded.

Initial behavior:

- load the current chunk first
- then immediate neighbors
- then preload ring
- unload beyond the unload radius
- keep activation capped per frame

Do not begin far LOD or horizon logic here.

**Verification:** Standing near a boundary does not reveal a large void, and
moving across boundaries loads and unloads chunks predictably.

## Success Criteria

`002a` is complete when all of the following are true:

1. Chunk naming is coherent across runtime code, comments, logs, and helpers.
2. The player chunk coord is tracked explicitly and updates correctly.
3. The runtime can report chunk lifecycle state.
4. Nearby chunks stream in and out without large visible gaps.
5. Metrics show the dominant activation cost instead of leaving it to guesswork.

## Bottleneck Questions To Answer Before 002b

1. Is Rust generation slower than mesh creation?
2. Is collision creation too expensive for a full near ring?
3. Is scene attachment or node count a bigger cost than generation?
4. Do chunk activations need background work yet, or is capped synchronous work
   good enough for the next step?

## Tomorrow Start Prompt

```text
Read specs/002a-chunk-model-and-near-streaming-foundation.md.
Implement sections 1 through 3 first:
- naming cleanup and chunk/world helper functions
- runtime chunk metrics
- explicit world_chunk / chunk_streamer ownership structure

Do not jump to far horizon or full LOD yet.
Use the current playable chunk path as the base case and keep measurement in
place while the runtime grows.
```

