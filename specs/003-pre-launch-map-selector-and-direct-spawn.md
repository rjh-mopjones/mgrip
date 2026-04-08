# Spec 003 - Pre-Launch Map Selector and Direct Spawn

**Status:** Open
**Priority:** High
**Depends On:** Spec 002 complete

## Problem

The project currently has only one launch path:

- [scripts/main_menu.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/main_menu.gd)
  immediately enters the world scene
- [scripts/world/world.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/world.gd)
  chooses the starting chunk from exported `world_x` / `world_y`

That is good for development speed, but it does not support a player-facing
flow where we:

1. launch the game
2. open a world map
3. scroll and inspect the macro map
4. highlight a chunk to spawn into
5. click `Launch Level`
6. spawn into that selected chunk

We want the new selector flow without losing the current direct-coordinate path.

## Goals

1. Add a pre-launch map selector flow for choosing a spawn chunk from the macro
   map.
2. Preserve the current direct launch flow for fast iteration, testing, and
   editor use.
3. Keep the world runtime as the owner of final spawn placement.
4. Make chunk selection visually clear before launch.
5. Fail safely when the macro map asset is unavailable.

## Non-Goals

- exact click-to-pixel spawn placement inside a chunk
- replacing the in-game debug overlay with final player UI
- changing chunk streaming architecture
- changing world generation truth in Rust
- adding save/load or persistent discovered-map progression
- final menu art or final UX polish

## Core Decision

The selector should choose a `chunk coord`, not a precise block coordinate.

That keeps the launch contract small and stable:

- the selector decides which runtime chunk to enter
- the world runtime decides the exact landing point inside that chunk
- existing land-safe spawn logic remains the source of truth

This spec must not replace direct coordinate launch.
It adds a second launch path that resolves into the same world scene.

## Launch Model

Introduce one shared launch request contract in game state.

Suggested shape:

```gdscript
enum LaunchMode {
	DIRECT_COORD,
	SELECTED_CHUNK,
}

var launch_mode: LaunchMode = LaunchMode.DIRECT_COORD
var launch_world_origin: Vector2 = Vector2(440.0, 220.0)
var launch_chunk: Vector2i = Vector2i.ZERO
var has_pending_launch: bool = false
```

Behavior:

- `DIRECT_COORD`
  Launch the world using an explicit generator-space world origin.
- `SELECTED_CHUNK`
  Launch the world using an explicit runtime chunk coord chosen from the map.
- no pending launch request
  Fall back to the current world scene defaults so running `world.tscn`
  directly in the editor still works.

## World Rules

- Preserve the current direct launch behavior as a first-class path.
- Do not force every launch through the selector screen.
- Keep `world.gd` compatible with editor-driven startup through exported
  `world_x` / `world_y`.
- A selected chunk must resolve to world coordinates through
  `GenerationManager.chunk_coord_to_world_origin()`.
- Final player placement should continue using the existing land-safe spawn
  logic in the chosen chunk.
- If the macro map image is missing, the user must still be able to quick
  launch directly into the world.

## Current Reality

- [project.godot](/Users/roryhedderman/Documents/GodotProjects/mgrip/project.godot)
  starts at `res://scenes/main_menu.tscn`
- [scripts/main_menu.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/main_menu.gd)
  immediately changes to `world.tscn`
- [scripts/world/world.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/world.gd)
  derives `_anchor_chunk` from exported `world_x` / `world_y`
- [scripts/ui/map_overlay.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/ui/map_overlay.gd)
  already knows how to load and display the latest macro map image, but it is
  an in-game debug overlay, not a pre-launch selector
- [scripts/autoload/game_state.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/autoload/game_state.gd)
  currently tracks world seed and current/anchor chunk only

## Architecture Direction

### 1. Two Entry Paths, One World Scene

Keep both launch paths:

- `Quick Launch`
  Enter the world immediately using direct world coordinates.
- `Open Map`
  Enter a selector scene, choose a chunk, then launch the same world scene.

The world scene should not care whether the chosen start came from:

- exported defaults in the scene
- a quick-launch button
- a selected map chunk

It should only consume a normalized launch request.

### 2. New Selector Scene, Not a Reused Debug Overlay

Do not turn the current in-game overlay into the selector screen.

Instead:

- add a dedicated selector scene and script for pre-launch UX
- extract shared macro-map loading or coordinate math only if useful
- keep [scripts/ui/map_overlay.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/ui/map_overlay.gd)
  focused on streamed-world debugging

### 3. Chunk Selection First, Spawn Refinement Later

The first version should support:

- pan or scroll around the macro map
- zoom in enough to read chunk intent clearly
- hover or move a cursor over chunks
- visibly highlight the current target chunk
- click `Launch Level` to commit that chunk

Do not add exact intra-chunk click spawning in this spec.

### 4. Safe Fallbacks

If macro map content is unavailable:

- the selector should show a clear unavailable state
- `Quick Launch` must remain available
- the game must still be playable without the selector path

## Modifies

Expected primary files:

```text
scripts/main_menu.gd
scripts/autoload/game_state.gd
scripts/world/world.gd
scripts/ui/map_overlay.gd        (only if shared map helpers are extracted)
project.godot                    (only if new input actions or scene wiring are needed)
```

Expected new files:

```text
scenes/map_selector.tscn
scripts/ui/map_selector.gd
```

## Implementation

### Section 1 - Add a Shared Launch Request

**Files:**

- `scripts/autoload/game_state.gd`
- `scripts/world/world.gd`

Add launch-request state to `GameState` so the main menu and selector can pass
start intent into the world scene.

`world.gd` should:

- read a pending launch request if one exists
- derive the anchor chunk from that request
- clear or consume the request after startup
- fall back to exported `world_x` / `world_y` when no request exists

**Verification:** Launching `world.tscn` directly in the editor still spawns at
the current default coordinate. A pending launch request overrides it.

---

### Section 2 - Replace Auto-Enter With Explicit Menu Choices

**Files:**

- `scenes/main_menu.tscn`
- `scripts/main_menu.gd`

Replace the current immediate scene swap with at least two explicit actions:

- `Quick Launch`
- `Open Map`

Suggested behavior:

- `Quick Launch`
  builds a direct-coordinate launch request using the current default world
  origin, then loads `world.tscn`
- `Open Map`
  changes to `map_selector.tscn`

Optional later polish may add a visible summary of the current default
coordinate or selected chunk, but that is not required for this spec.

**Verification:** The player can choose either path from the main menu instead
of being forced directly into the world.

---

### Section 3 - Build the Pre-Launch Map Selector

**Files:**

- `scenes/map_selector.tscn`
- `scripts/ui/map_selector.gd`

The selector must:

- load the latest available macro map image
- display it large enough for navigation
- support scrolling or zooming for map inspection
- map cursor position to runtime `chunk coord`
- show the currently highlighted chunk clearly
- display the selected chunk textually
- provide `Launch Level` and `Back` actions

Recommended display details:

- a visible chunk-outline rectangle
- a small coordinate readout such as `Chunk (440, 220)`
- a disabled `Launch Level` button until a chunk is selected

If the macro map asset is missing:

- show a clear message
- disable chunk launch
- keep `Back` available

**Verification:** The user can move around the macro map, highlight a chunk,
and see exactly which chunk will be launched.

---

### Section 4 - Resolve Selected Chunk Into Spawn

**Files:**

- `scripts/autoload/game_state.gd`
- `scripts/world/world.gd`

When launching from the selector:

- store the chosen `chunk coord`
- derive its world origin in `world.gd`
- bootstrap the selected runtime chunk
- place the player with the existing safe land-finding logic

This keeps chunk choice and actual spawn placement separate.

**Verification:** Selecting a chunk from the map launches into that chunk and
spawns the player on valid terrain or the existing water fallback path.

---

### Section 5 - Keep Direct Coordinate Launch First-Class

**Files:**

- `scripts/main_menu.gd`
- `scripts/world/world.gd`
- `scripts/autoload/game_state.gd`

The direct path must remain easy to use for development.

Minimum support:

- a single quick-launch action from the main menu
- fallback behavior when entering `world.tscn` directly
- no requirement to open the map selector before testing spawn changes

This is a product and workflow requirement, not optional polish.

**Verification:** Developers can still change one coordinate and launch
straight into the world without going through the selector flow.

## UI Notes

The selector is not the debug overlay.

It should feel like a simple pre-launch tool:

- large map viewport
- clear highlighted chunk
- obvious `Launch Level` action
- obvious way back out

Avoid adding runtime debug metrics, active LOD counts, or streamer details to
this screen unless they directly help chunk selection.

## Risks

- macro-map pixels may not map 1:1 to runtime chunk coords without explicit
  conversion rules
- using the debug overlay directly could tangle pre-launch UI with in-world HUD
  responsibilities
- exact click-to-world spawn could expand scope and blur ownership between the
  selector and world runtime
- selector flow can become a blocker if direct launch is not preserved

## Final Verification

1. Start the game and confirm the main menu offers `Quick Launch` and `Open Map`.
2. Use `Quick Launch` and confirm the world still starts at the direct
   coordinate path.
3. Use `Open Map`, move around the macro map, and highlight a chunk.
4. Click `Launch Level` and confirm the world loads into that selected chunk.
5. Confirm final player placement still uses the world runtime's safe spawn
   rules.
6. Confirm launching `world.tscn` directly in the editor still works without
   the selector flow.
7. Confirm the game still has a usable path when no macro map image is present.

## Tomorrow Start Prompt

```text
Read specs/003-pre-launch-map-selector-and-direct-spawn.md.
Implement sections 1 and 2 first:
- add a shared launch request in GameState
- make world.gd consume it with a safe fallback to exported world_x/world_y
- replace the main menu auto-enter with explicit Quick Launch and Open Map actions

Then implement the new map selector scene for chunk selection, but keep direct
coordinate launch fully intact.
```
