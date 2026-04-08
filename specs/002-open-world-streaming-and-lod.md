# Spec 002 - Open World Expansion Overview

**Status:** Open
**Priority:** Critical

This spec has been split into two smaller specs because the original scope
combined two different jobs:

1. define a coherent runtime chunk model and get near streaming working safely
2. extend that model outward into horizon rendering and visual LOD

We want to do those in order, not all at once.

## Why This Was Split

The original `002` mixed foundational architecture with later expansion work:

- chunk naming and coordinate ownership were still ambiguous
- metrics did not exist yet
- the runtime still loaded a single chunk synchronously
- horizon, LOD, prioritization, and seam work were all stacked on top

That is too much change to reason about at once.

## Required Naming Rule

Chunk terminology must become coherent before the system expands:

- `chunk` = the runtime streamed ownership unit keyed by integer chunk coords
- `chunk coord` = the integer coordinate of that runtime chunk
- `world coord` = the continuous generator-space coordinate
- `block coord` = the local 0..511 coordinate inside one runtime chunk
- `region tile` = the larger meso-scale generator output; not a chunk
- `macro map` = the full-world debug map; not a chunk
- `LOD0` / `LOD1` / `LOD2` = representations of a chunk, not new chunk types

Do not use the word `chunk` to refer to multiple scales interchangeably.

## Split Specs

- [002a-chunk-model-and-near-streaming-foundation.md](/Users/roryhedderman/Documents/GodotProjects/mgrip/specs/002a-chunk-model-and-near-streaming-foundation.md)
  defines naming, metrics, lifecycle ownership, coordinate helpers, and the
  first near-streaming pass
- [002b-horizon-and-lod-expansion.md](/Users/roryhedderman/Documents/GodotProjects/mgrip/specs/002b-horizon-and-lod-expansion.md)
  builds on 002a to add visual LOD, horizon terrain, prioritization, seam
  checks, and map-debug integration

## Execution Order

1. Complete `002a`
2. Verify `002a` success criteria
3. Only then begin `002b`

## Gate Between 002a and 002b

Do not start the horizon/LOD work until all of the following are true:

- chunk naming is coherent in code comments, logs, and runtime helpers
- the player can move across chunk boundaries without exposing large voids
- chunk metrics identify the dominant activation cost
- the streamed runtime has explicit chunk lifecycle ownership

