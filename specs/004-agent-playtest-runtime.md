# Spec 004 - Agent Playtest Runtime

**Status:** Proposed
**Priority:** High
**Depends On:** Spec 002a / 002b foundations remaining stable

## Problem

The runtime is now strong enough to stream terrain, attach chunk LODs, prewarm
ahead of player motion, and run scripted flythrough verification. But there is
still no stable machine-facing control surface for an external coding agent to
iteratively play the runtime.

Right now, the closest thing to an agent harness is spread across existing
seams:

- [scripts/player/fps_controller.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/player/fps_controller.gd)
  already supports scripted movement through `set_scripted_motion()` and
  `clear_scripted_motion()`
- [scripts/world/world.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/world.gd)
  already exposes terrain sampling helpers such as `sample_surface_height()`
  and `nearest_land_block()`
- [scripts/autoload/flythrough.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/autoload/flythrough.gd)
  already drives scripted movement, settle timing, and screenshot capture for
  automated verification
- [scripts/autoload/generation_manager.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/autoload/generation_manager.gd)
  and [gdextension/src/lib.rs](/Users/roryhedderman/Documents/GodotProjects/mgrip/gdextension/src/lib.rs)
  already expose runtime chunk and mesh generation
- [scripts/world/chunk_streamer.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/chunk_streamer.gd)
  already owns streaming, prewarm, LOD selection, and chunk lifecycle decisions

That is enough to support a first agent runtime, but not enough in its current
ad hoc form.

If we want Codex or another coding agent to iteratively play the game, inspect
outcomes, and continue, we need one explicit developer-facing runtime contract
instead of one-off flythrough logic.

## Current Reality

This repo is not yet at a full survival-RPG gameplay runtime.

What exists now:

- streamed terrain runtime
- player movement over generated terrain
- chunk streaming, LOD, collision ownership, and prewarm behavior
- map overlay and flythrough debug harnesses
- Rust-backed terrain generation and chunk mesh preparation

What does not exist yet in runtime code:

- survival systems exposed through gameplay loops
- inventory, crafting, combat, or quest control surfaces
- a unified action/result API for external automation

So phase 1 of this spec is not "LLM plays the full RPG."
It is:

**an external agent can iteratively observe the world runtime, navigate it,
request debug observations, and validate streaming and traversal behavior.**

## Goals

1. Add a developer-only agent runtime that can drive the existing world
   iteratively.
2. Reuse current seams instead of bypassing runtime ownership.
3. Support an explicit observe -> act -> observe loop.
4. Make runs inspectable and reproducible.
5. Keep the first version narrowly focused on terrain/runtime traversal and
   validation.
6. Leave room for later survival, combat, and inventory actions once those
   systems actually exist.

## Non-Goals

- full keyboard or mouse emulation
- screen-scraping as the primary interface
- shipping cloud inference in the player build
- pretending the current repo already has gameplay systems it does not yet
  expose
- replacing flythrough mode immediately
- NPC autonomy or companion planning

## Core Decision

Do not make the agent "press WASD" as the primary control path.

The agent runtime should wrap the seams the repo already has:

- movement still goes through
  [scripts/player/fps_controller.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/player/fps_controller.gd)
- terrain and traversal queries still go through
  [scripts/world/world.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/world.gd)
- chunk ownership stays in
  [scripts/world/chunk_streamer.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/world/chunk_streamer.gd)
- chunk generation stays behind
  [scripts/autoload/generation_manager.gd](/Users/roryhedderman/Documents/GodotProjects/mgrip/scripts/autoload/generation_manager.gd)
  and [gdextension/src/lib.rs](/Users/roryhedderman/Documents/GodotProjects/mgrip/gdextension/src/lib.rs)

This agent runtime is an adapter layer, not a parallel engine.

## Transport / Entry Contract

Phase 1 should define one clear control boundary.

For this spec, "external agent" means developer automation that can invoke a
stable local API inside a running local debug session.

Phase 1 contract:

- `agent_runtime.gd` exposes the first-class action and observation API
- the API is local to the Godot runtime process
- the API is intended for developer tooling, debug harnesses, or future local
  adapters
- network transport, remote sockets, stdio bridges, or cloud-connected control
  layers are out of scope for this spec

This keeps the first version focused on the stable runtime contract instead of
binding the design too early to one transport choice.

A later spec may add a transport adapter, but that adapter must preserve the
same action and observation schema.

## Schema Version and Coordinate Conventions

The agent contract needs explicit versioning and one primary coordinate
language.

Every observation and action result should include at minimum:

- `schema_version`
- `session_id`
- `step_index`
- `timestamp_ms`

Coordinate naming must stay explicit:

- `chunk_coord`
  runtime streamed chunk coordinate as `Vector2i`
- `world_origin`
  generator-space chunk origin as `Vector2`
- `scene_block`
  scene-space X/Z block coordinate relative to the current `anchor_chunk`
- `player_position`
  scene-space `Vector3`

Action coordinate rules for phase 1:

- `move_to_block`
  accepts a `scene_block` target
- `teleport_to_block`
  accepts a `scene_block` target
- `sample_height`
  accepts a `scene_block` target
- `find_nearest_land`
  accepts a `scene_block` target

Observations should report both runtime chunk identity and the player's
scene-space transform so callers do not have to infer or convert unnamed
coordinate spaces.

Do not return unlabeled coordinates whose meaning depends on the caller
remembering whether a field is chunk-local, world-origin, or scene-relative.

## Runtime Discovery Rules

The agent runtime should not rely on ad hoc tree search as its primary runtime
discovery path.

Preferred ownership:

- `world.gd` registers itself with `agent_runtime.gd` when ready
- the world runtime provides access to the current player, head, and camera
- `agent_runtime.gd` clears those references when the world exits

Fallback behavior:

- tree search may be used only as a developer-friendly recovery path
- if runtime ownership cannot be resolved, the action must return a structured
  rejection such as `no_world_runtime` instead of failing silently

This keeps runtime ownership explicit and avoids duplicating the looser
discovery style used in one-off automation.

## Developer Gating

The agent runtime must be explicitly disabled by default.

Phase 1 should require a concrete developer-only activation path such as:

- a CLI flag like `--agent-runtime`
- a debug build check
- an internal `GameState` or project setting gate enabled only for development

Minimum gating rules:

- the autoload may exist, but it must remain inert unless the developer gate is
  enabled
- no player-facing menu or UI should expose it by default
- exported player builds should not enable the runtime accidentally
- local-only developer use is the default assumption

## Agent Runtime Model

Introduce one explicit developer-only session object for automation.

Minimum session fields:

- session id
- seed
- anchor chunk
- current chunk
- current player position
- current run goal or scenario label
- step count
- last action
- last result
- run status

Observation and result envelopes should also include:

- schema version
- session id
- step index
- timestamp
- action name where relevant

The first observation payload should stay compact and runtime-grounded.

Minimum observation fields:

- world seed
- anchor chunk
- current chunk
- player position
- player velocity
- player facing direction
- current loaded chunk counts by LOD
- pending chunk/job count
- prewarm target chunk
- nearby sampled terrain heights
- nearest-land result for requested probe points
- flythrough or debug flags where relevant

Optional debug-only observation fields:

- horizon runtime state
- ring readiness
- collision-enabled chunk set near the player

Initial action set:

- `teleport_to_block`
- `look_at`
- `move_in_direction`
- `move_to_block`
- `stop`
- `sample_height`
- `find_nearest_land`
- `wait_seconds`
- `capture_screenshot`
- `get_chunk_state`
- `end_session`

Every action should return a structured result:

- `accepted`
- `rejected`
- `completed`
- `interrupted`
- `timed_out`

Rejected actions should also include:

- error code
- human-readable reason
- optional hint

Examples:

- target chunk not loaded yet
- target block out of supported range
- no world runtime available
- motion request invalid because direction is zero
- screenshot path unavailable

## Action Semantics

Action behavior should be explicit enough that two different implementations
would still agree on outcomes.

Phase 1 semantics:

- `look_at`
  completes when yaw and pitch are within a small tolerance of the requested
  target, or rejects when the target vector is invalid
- `move_in_direction`
  accepts a non-zero horizontal direction plus speed and timeout, completes when
  its requested duration or settle condition is met, and times out if the
  bounded run cannot complete in time
- `move_to_block`
  completes when the player reaches an arrival radius around the requested
  `scene_block`, rejects if the target is invalid or unsupported, and times out
  if traversal does not settle
- `stop`
  completes when scripted motion is cleared and horizontal velocity falls within
  a small stop tolerance
- `wait_seconds`
  completes after the requested bounded delay while still allowing runtime
  streaming and observation updates
- `sample_height`, `find_nearest_land`, and `get_chunk_state`
  are immediate query actions and should complete without mutating player state
- `capture_screenshot`
  completes only after the image is successfully written and the output path is
  recorded
- `teleport_to_block`
  is developer-only, must report whether the destination is currently supported,
  and should reject rather than silently place the player into an invalid state
- `end_session`
  completes when logs are flushed and the runtime returns a final observation or
  final session summary

## Tick-Bounded Iteration

The agent runtime should support one explicit bounded step loop:

1. get observation
2. submit action
3. advance runtime until settle, timeout, arrival, or interruption
4. return action result plus the next observation

This keeps Codex-style iterative playtesting practical and debuggable.

## Session Artifacts

Each agent session should leave behind one inspectable artifact directory.

Recommended layout:

```text
user://agent_sessions/<session_id>/
  session.json
  steps.jsonl
  screenshots/
```

Artifact expectations:

- `session.json`
  contains run metadata and static session context
- `steps.jsonl`
  contains one append-only structured record per step
- `screenshots/`
  contains evidence images referenced by step results

Paths returned to the agent should be stable and machine-readable so later
tools can inspect or summarize them.

## Modifies

Expected primary files:

```text
project.godot
scripts/player/fps_controller.gd
scripts/world/world.gd
scripts/world/chunk_streamer.gd
scripts/autoload/game_state.gd
scripts/autoload/flythrough.gd
scripts/autoload/generation_manager.gd
gdextension/src/lib.rs
```

Expected new files:

```text
scripts/autoload/agent_runtime.gd
scripts/autoload/agent_session.gd
scripts/autoload/agent_observation_builder.gd
scripts/autoload/agent_action_validator.gd
```

Optional later file:

```text
gdextension/src/agent_exports.rs
```

if Rust-side observation helpers become necessary.

## Implementation

### Section 1 - Establish Agent Runtime Ownership

**Files:**

- `project.godot`
- `scripts/autoload/game_state.gd`
- `scripts/autoload/agent_runtime.gd`
- `scripts/autoload/agent_session.gd`

Add a small developer-only autoload that owns agent session state and action
dispatch.

Responsibilities:

- start or stop a session
- hold current session data
- dispatch actions to existing runtime seams
- collect structured observations
- expose one stable local developer API
- remain inert unless the developer gate is enabled

The runtime should be easy to disable and should not be part of the normal
player-facing surface by default.

**Verification:** A developer can start a session and request a basic
observation without modifying world state, and the runtime stays inactive when
the developer gate is not enabled.

---

### Section 2 - Expose Structured World Observation

**Files:**

- `scripts/autoload/agent_runtime.gd`
- `scripts/autoload/agent_observation_builder.gd`
- `scripts/autoload/game_state.gd`
- `scripts/world/world.gd`
- `scripts/world/chunk_streamer.gd`

Create a dedicated observation builder rather than assembling dictionaries ad
hoc inside `world.gd` or `flythrough.gd`.

Observation sources should include:

- `GameState`
- `world.gd`
- `chunk_streamer.gd`
- player transform and velocity

The observation contract should stay stable across repeated calls in the same
frame and should carry the schema version and explicit coordinate labels.

**Verification:** The observation payload is stable across repeated calls in
the same frame and includes current chunk, player transform, and streaming
state.

---

### Section 3 - Promote Scripted Motion Into an Agent Action Surface

**Files:**

- `scripts/player/fps_controller.gd`
- `scripts/autoload/agent_runtime.gd`
- `scripts/autoload/agent_action_validator.gd`

`fps_controller.gd` already supports scripted motion. Formalize that into the
agent action layer.

Add agent-callable helpers for:

- set movement vector
- clear movement
- orient yaw or pitch toward a target
- detect arrival, interruption, or timeout

Action helpers should formalize:

- arrival tolerance
- stop tolerance
- timeout handling
- result status mapping

Keep motion requests high-level and bounded instead of exposing raw input
events.

**Verification:** An external caller can move the player across at least one
chunk boundary without direct keyboard input.

---

### Section 4 - Promote Terrain Queries Into Agent Tools

**Files:**

- `scripts/world/world.gd`
- `scripts/autoload/agent_runtime.gd`
- `scripts/autoload/agent_observation_builder.gd`

`world.gd` already exposes terrain height and nearest-land helpers. Formalize
them as explicit agent-facing query helpers.

Add agent-callable helpers for:

- sample surface height at a block coordinate
- find nearest land from a block coordinate
- inspect current chunk runtime state

These queries should be usable both as standalone debug actions and as inputs
to later movement decisions.

**Verification:** An agent can probe terrain ahead of movement and use the
result to avoid water or invalid landing points.

---

### Section 5 - Reuse the Flythrough Capture Path

**Files:**

- `scripts/autoload/flythrough.gd`
- `scripts/autoload/agent_runtime.gd`

Do not replace flythrough immediately. Instead, extract the reusable screenshot
and settle logic needed by the agent runtime.

The goal is to let an agent:

- move
- pause
- capture evidence
- continue

**Verification:** A session can capture a screenshot after a movement step and
record the output path in the action result.

---

### Section 6 - Add Session Logs

**Files:**

- `scripts/autoload/agent_runtime.gd`
- `scripts/autoload/agent_session.gd`

Each session should record:

- session metadata
- per-step observation summary
- action
- result
- timestamps
- screenshot paths when present
- schema version
- activation mode and relevant CLI flags
- world seed
- launch mode
- launch world origin or launch chunk
- anchor chunk at session start

This is the minimum needed to make Codex iteration useful.

**Verification:** A failed traversal or timeout leaves behind enough evidence
to understand what happened.

---

### Section 7 - Keep Full Gameplay Out of Scope Until It Exists

Do not invent action families for crafting, combat, quests, or survival state
until those systems have real runtime ownership points in code.

Instead, define the extension path now:

- future survival observation block
- future inventory block
- future combat block
- future objective block

**Verification:** The spec stays honest about current runtime scope and does
not force fake abstractions.

## Risks

- letting the agent bypass `fps_controller.gd` would create a second movement
  ownership path
- building observation dictionaries ad hoc in multiple files would make the
  contract unstable
- leaving the transport boundary implicit would make future adapters diverge
- mixing chunk, world, and scene coordinates in one unlabeled schema would make
  actions error-prone
- over-expanding the first action surface into inventory, crafting, or combat
  would hard-code fake gameplay abstractions
- replacing flythrough outright would throw away an already useful verification
  harness
- always-on agent wiring could leak developer-only behavior into normal runtime
  startup

## Success Criteria

`004` is complete when all of the following are true:

1. A developer can start an agent session against the current world runtime.
2. The agent can request a structured observation at any step.
3. The agent can issue bounded traversal and debug actions without using raw
   keyboard control.
4. The agent can move across chunk boundaries while the existing streaming
   system continues to own loading and unloading.
5. Observations and results include explicit schema versioning and coordinate
   conventions.
6. The agent can capture evidence and logs for iterative debugging.
7. The design clearly leaves room for later survival, combat, and inventory
   extensions without pretending they already exist.

## Follow-On Specs

Once real gameplay systems exist in runtime code, follow with:

- `005-agent-survival-observation-and-goals.md`
- `006-agent-inventory-crafting-actions.md`
- `007-agent-combat-and-resolution-hooks.md`

## Minimal End-to-End Flow

The first end-to-end acceptance scenario should stay small and concrete:

1. start an agent session in a developer-enabled runtime
2. request an observation
3. issue `move_to_block` or `move_in_direction` toward the next chunk
4. confirm the player crosses a chunk boundary while streaming continues
5. issue `sample_height` or `find_nearest_land` ahead of the player
6. issue `capture_screenshot`
7. end the session
8. inspect the session artifacts for logs and evidence

## Final Verification

1. Start the runtime with the developer gate enabled and begin an agent
   session.
2. Request a structured observation and confirm it includes schema version,
   player transform, chunk state, and streamer state with explicit coordinate
   naming.
3. Issue a bounded movement action and confirm the player moves without direct
   keyboard control.
4. Cross at least one chunk boundary and confirm chunk streaming remains owned
   by the existing runtime.
5. Request terrain probes ahead of movement and confirm the results are usable
   for traversal decisions.
6. Capture a screenshot during the session and confirm the action result records
   the output path.
7. End the session and confirm `session.json`, `steps.jsonl`, and any screenshot
   outputs are written to the session artifact directory.
8. Confirm a failed or timed-out step leaves behind enough structured logging
   to debug what happened.

## Tomorrow Start Prompt

```text
Read specs/004-agent-playtest-runtime.md.
Implement sections 1 through 3 first:
- add a developer-only agent runtime autoload
- add structured world observation
- formalize scripted player motion as agent actions
- define the schema version and coordinate contract up front

Do not invent inventory, combat, or quest APIs yet.
Build on existing seams in fps_controller.gd, world.gd, flythrough.gd,
chunk_streamer.gd, and generation_manager.gd.
```
