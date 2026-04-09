# AGENTS.md

## Project Snapshot

- Margin's Grip is a Godot 4.3 + Rust GDExtension project for a 3D voxel
  open-world survival RPG set on the tidally locked planet Margin.
- The current repo is primarily a terrain/runtime project:
  - streamed terrain runtime
  - chunk streaming, LOD, collision ownership, and prewarm
  - player traversal over generated terrain
  - flythrough and screenshot automation
  - pre-launch map selector and direct spawn flow
- It is not yet a full gameplay runtime. Do not invent finished APIs for:
  - inventory
  - crafting
  - combat
  - quests
  - survival-state systems

## External Documentation

The main external design docs live in the Obsidian vault at:

- `/Users/roryhedderman/Documents/mop-jones-brain`

The main Margins Grip notes live under:

- `/Users/roryhedderman/Documents/mop-jones-brain/Notes`

Start with the smallest set of notes that fits the task.

High-value notes:

- `Margin's Grip Game World Primer.md`
  hub note for the setting and linked design docs
- `Margin's Grip - Godot Migration Prompt.md`
  technical migration spec, world invariants, and agent iteration loop
- `Margin's Grip - Iteration 0 Plan.md`
  current migration-plan context and headless verification workflow
- `Margin's Grip - World Generation.md`
  TerrainGen / LifeGen / SceneGen breakdown
- `Margin's Grip - Game Loop.md`
  current gameplay-loop state

For gameplay or lore questions, relevant notes are also available as:

- `Margin's Grip - Player & Progression.md`
- `Margin's Grip - Combat & Resolution.md`
- `Margin's Grip - Items & Crafting.md`
- `Margin's Grip - Survival & Movement.md`
- `Margin's Grip - Interface.md`
- `Margin's Grip - Geography.md`
- `Margin's Grip - History.md`
- `Margin's Grip - Factions.md`

Do not trawl the whole vault unless the task needs it. Read narrowly.

## Repo Layout

- `project.godot`
  project entry, input map, and autoload wiring
- `scenes/`
  runtime scenes
- `scripts/autoload/`
  singleton state and automation helpers
- `scripts/world/`
  terrain runtime, chunk streaming, and mesh building
- `scripts/player/`
  player controller
- `scripts/ui/`
  map overlay and selector UI
- `gdextension/`
  Rust workspace for terrain generation and mesh data
- `specs/`
  numbered markdown specs used by this repo

Current scene entry points:

- `scenes/main_menu.tscn`
  app start scene
- `scenes/world.tscn`
  main streamed-world runtime
- `scenes/map_selector.tscn`
  pre-launch map selector

Current autoloads in `project.godot`:

- `GameState`
- `GenerationManager`
- `Flythrough`

## Ownership Boundaries

Preserve these file responsibilities when making changes:

- `scripts/autoload/game_state.gd`
  shared world seed, current chunk, anchor chunk, and launch-request state
- `scripts/autoload/generation_manager.gd`
  chunk/world/block coordinate helpers and Rust-backed generation entry points
- `scripts/autoload/flythrough.gd`
  automated camera movement, settle timing, screenshot capture, and flythrough
  modes
- `scripts/world/world.gd`
  runtime world bootstrap, player placement, terrain sampling helpers, and map
  overlay integration
- `scripts/world/chunk_streamer.gd`
  chunk lifecycle, LOD selection, prewarm logic, horizon streaming, and
  collision focus
- `scripts/player/fps_controller.gd`
  player movement and the scripted-motion seam
- `gdextension/src/lib.rs`
  GDExtension entry point exposing Rust terrain generation and mesh data

Build on these seams. Do not create parallel ownership paths unless the user
explicitly wants a refactor.

## World and Visual Invariants

These are easy to accidentally break. Keep them in mind when touching terrain,
biomes, visuals, or worldgen logic:

- Margin is tidally locked.
- The day side and night side are permanent.
- The habitable band is the terminus ring between them.
- South/day side trends hot and harsh.
- North/night side trends frozen and dark.
- Temperature derives from light level and altitude, not generic Earth-like
  latitude noise.
- No green vegetation palette anywhere. If output looks Earth-green, it is
  wrong.
- Dayside liquid surface water should not behave like a normal Earth world.

If a terrain or biome change makes the world look generic-Earth, assume it is
wrong until proven otherwise.

## Macro Map and Compare Notes

When touching macro/runtime coherence, local maps, or Compare Generation:

- `biome.png` is loaded from the newest layers artifact under
  `~/.margins_grip/layers/<tag>/images/biome.png`. If a change affects macro
  biome semantics, regenerate the layers artifact before trusting compare or
  selector screenshots.
- Macro/runtime linkage is by world coordinate and generator semantics, not by
  trying to compare two presentation images directly. Keep the distinction
  between:
  - visible macro context (`biome.png`)
  - generated macro semantic truth (`generate_region(..., freq_scale=1.0)`)
  - runtime `LOD0` semantic truth
  - player-facing runtime local-map presentation
- Compare Generation keeps `biome.png` as world-context, but the scored macro
  truth should come from generated macro semantic data at `freq_scale=1.0`,
  not palette heuristics sampled from the visible PNG.
- The runtime local map, selector preview, and in-level `[M]` map should share
  the same LOD0 data-driven renderer. Do not let one path drift onto an older
  biome-export shortcut.
- That renderer is `scripts/ui/runtime_chunk_preview_renderer.gd`. It does not
  render the 3D scene. It rasterizes a top-down map from runtime `LOD0` chunk
  data using:
  - `block_heights(HEIGHT_SCALE)` for elevation
  - `is_ocean_grid()` for water occupancy
  - `export_layer_rgba("biome")` for biome identity colour
- It should expose and keep distinct:
  - the player-facing local-map `image`
  - the raw runtime `biome_image`
  - the runtime `ocean_mask_image`
- Do not collapse those into one concept. The visible local map is for players;
  the raw biome and mask images are for compare semantics.
- The runtime chunk source for those maps should come from
  `GenerationManager.generate_runtime_chunk_for_lod_with_seed(..., "LOD0")`,
  which currently means `512` resolution, `detail_level = 2`, `freq_scale = 8.0`.
- The coordinate path should stay explicit:
  - compare/selector picks a region or chunk
  - chunk coords convert to world origin via
    `GenerationManager.chunk_coord_to_world_origin(...)`
  - runtime `LOD0` generation happens from that world origin
  - macro compare generation uses the same world region at `freq_scale=1.0`
- Ocean in the local map should be derived from the runtime fluid mask and
  rendered as readable blue water first. Land should be height/slope shaded and
  only lightly tinted toward biome colour. Do not let the local map become a
  disguised biome export or a fake scene render.
- The runtime local map should read as terrain/ocean first. Water-biome drift
  belongs in compare diagnostics, not in the base ocean colour of the map.
- In Compare Generation, keep `biome.png` as the visible macro context, but do
  not score truth from its palette. Generate fresh macro semantic data at
  `freq_scale=1.0`, then compare:
  - macro biome via `export_layer_rgba("biome")`
  - macro ocean via `is_ocean_grid()`
  against the runtime `LOD0` biome/ocean data derived the same way.
- Treat the compare panels differently:
  - `Macro Visual` is user-facing context
  - `Runtime Local Map` is player-facing terrain preview
  - `Macro Colours over Runtime` is a bridge view only
  - `Delta` is the actual scored diagnostic surface
- Ocean/land agreement is the strongest signal for spec-007-style validation.
  Exact biome mismatch is still a useful diagnostic, but it is noisier and
  should not be interpreted as the same class of failure without further
  normalization.
- `CoralReef` was removed because it repeatedly created misleading
  underwater-biome comparison output. Do not reintroduce a special reef biome
  casually without reconsidering compare semantics and map readability.
- Windowed screenshot probes are the trustworthy receipt for compare and
  local-map UI changes. Headless runs are useful for parse/load checks but not
  for judging the final rendered UI.
- If you add temporary screenshot-probe plumbing to runtime scenes such as
  `world.gd`, keep it clearly developer-gated and be explicit when removing it
  afterwards so it does not look like unexplained feature churn.

## Coordinate and Chunk Terminology

Follow the naming rules from `specs/002-open-world-streaming-and-lod.md`:

- `chunk`
  the runtime streamed ownership unit keyed by integer chunk coords
- `chunk coord`
  the integer coordinate of that runtime chunk
- `world coord` or `world origin`
  generator-space coordinate
- `scene block`
  scene-space block coordinate relative to the current anchor chunk
- `block coord`
  local `0..511` coordinate inside one runtime chunk
- `region tile`
  larger meso-scale generator output, not a runtime chunk
- `macro map`
  full-world debug map, not a chunk
- `LOD0` / `LOD1` / `LOD2`
  chunk representations, not different chunk types

Do not use `chunk` to mean multiple scales interchangeably.
Do not return or document unlabeled coordinates.

## Spec Workflow

This repo uses numbered markdown specs under `specs/`.
It does not currently use a formal `openspec/changes/...` tree.

Important specs right now:

- `specs/002-open-world-streaming-and-lod.md`
  overview of completed streaming/LOD work
- `specs/002a-chunk-model-and-near-streaming-foundation.md`
  chunk model and near streaming foundation
- `specs/002b-horizon-and-lod-expansion.md`
  horizon and LOD expansion
- `specs/003-pre-launch-map-selector-and-direct-spawn.md`
  map selector and launch-flow work
- `specs/004-agent-playtest-runtime.md`
  proposed developer-only agent runtime

When implementing planned work:

- read the relevant numbered spec first
- preserve numbering continuity
- update specs in-place when the user wants spec edits
- do not invent a new spec format unless the user asks for a migration

## Agentic Playtesting

When working on runtime automation or AI playtesting, use
`specs/004-agent-playtest-runtime.md` as the contract.

Rules:

- prefer the structured observe -> act -> observe loop over raw keyboard or
  mouse emulation
- route movement through `scripts/player/fps_controller.gd`
- route terrain and traversal queries through `scripts/world/world.gd`
- keep chunk streaming ownership in `scripts/world/chunk_streamer.gd`
- treat `chunk coord`, `world origin`, `scene block`, and `player_position` as
  distinct and explicitly labeled
- keep the runtime developer-only and inert by default
- do not expose unfinished agent tooling as a player-facing flow
- do not invent inventory, crafting, combat, or quest actions until those
  systems actually exist in runtime code

Phase 1 agent-runtime contract:

- treat the "external agent" as developer automation calling a stable local API
  inside the running Godot process
- do not bind phase 1 to remote sockets, stdio bridges, or cloud transport
- if a transport adapter is added later, it must preserve the same action and
  observation schema

Implemented local bridge details:

- the developer transport is a file-based JSON bridge rooted at
  `user://agent_runtime_bridge`
- request files are written to `requests/` and response files appear in
  `responses/`
- the live bridge status file is `user://agent_runtime_bridge/state.json`
- bridge event records append to `user://agent_runtime_bridge/events.jsonl`
- enable the runtime with `--agent-runtime`
- use `--agent-runtime-quick-launch` to bypass the menu and enter the world
  scene for automation
- use `--agent-runtime-passive-window` for windowed automation runs that should
  avoid grabbing focus or capturing the mouse
- `--agent-runtime-smoke-test` still runs the in-engine smoke harness
- the external bridge runner lives at
  `tools/agent_runtime_bridge_runner.py`
- on macOS, the runner uses background app launch for `--windowed` runs and
  should be preferred over launching Godot manually for screenshot checks
- the bridge currently supports:
  - `ping`
  - `get_state`
  - `get_observation`
  - `current_session_summary`
  - `start_session`
  - `end_session`
  - `submit_action`
  - `await_current_action`
  - `run_step`
  - `interrupt_current_action`
- readiness actions now include:
  - `wait_for_chunk_loaded`
  - `wait_for_ring_ready`
  - `wait_for_player_settled`
- in headless mode, screenshot requests return
  `headless_screenshot_unavailable` instead of crashing; use a non-headless
  display driver when image evidence is required

If implementing the agent runtime, keep these details explicit:

- schema versioning for observations and action results
- structured rejection/completion statuses
- runtime discovery and ownership
- developer gating such as `--agent-runtime` or equivalent
- session artifacts and logs

Required envelope fields:

- `schema_version`
- `session_id`
- `step_index`
- `timestamp_ms`

Required coordinate language:

- `chunk_coord`
- `world_origin`
- `scene_block`
- `player_position`

Do not accept or return unlabeled coordinates.

Preferred runtime discovery:

- `world.gd` registers itself with the agent runtime when ready
- the world runtime provides the current player, head, and camera
- the agent runtime clears those references when the world exits
- if runtime ownership is unavailable, return a structured rejection such as
  `no_world_runtime`

Required action/result semantics:

- results should distinguish `accepted`, `rejected`, `completed`,
  `interrupted`, and `timed_out`
- query actions should complete without mutating player state
- movement actions should use explicit arrival, stop, and timeout tolerances
- screenshot actions should only complete after the file is written
- teleport actions are developer-only and should reject invalid placements

Required step loop:

1. get observation
2. submit action
3. advance until settle, timeout, arrival, or interruption
4. return result plus the next observation

Required developer gating:

- the autoload may exist but must stay inert unless the developer gate is on
- no default player-facing UI should expose it
- exported builds should not enable it accidentally
- local-only developer use is the default assumption

Minimal end-to-end acceptance flow:

1. start an agent session in a developer-enabled runtime
2. request an observation
3. issue `move_to_block` or `move_in_direction`
4. cross a chunk boundary while normal streaming continues
5. issue `sample_height` or `find_nearest_land`
6. issue `capture_screenshot`
7. end the session
8. inspect the session artifacts

Recommended session artifact layout from `specs/004`:

```text
user://agent_sessions/<session_id>/
  session.json
  steps.jsonl
  screenshots/
```

## Working Style

- Propose a phased plan before edits.
- Keep changes small and reversible when possible.
- Prefer simple ownership-preserving changes over big abstractions.
- Reuse existing seams instead of bypassing them.
- Separate tidy-ups from behavior changes when practical.
- Keep responses concise and actionable.

Use the shared local guidance:

- `~/.ai-tools/MEMORY.md`
- `~/.ai-tools/best-practices.md`
- `~/.ai-tools/git-guidelines.md`

## Search, Tools, and Safety

- Prefer the `fff` MCP tools for file search if they are available.
- If `fff` is not available in the current session, fall back to `rg`.
- Prefer Bun for scripts when possible; otherwise use `tsx` for TypeScript.
- Never run destructive git commands.
- Prefer non-interactive git commands.
- If the repo contains unrelated local changes, do not revert them.

For JS/TS file changes:

- run typecheck
- run lint
- run Biome

For Rust or GDScript work, run the relevant native checks that actually apply to
the files you changed.

## Verification Commands

Common commands and flows:

- Rust build:
  `cargo build --release --manifest-path gdextension/Cargo.toml`
- Run the project:
  `godot --path /Users/roryhedderman/Documents/GodotProjects/mgrip`
- Headless scenic flythrough:
  `godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough`
- Headless boundary flythrough:
  `godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough-boundary`
- Headless seam-crossing flythrough:
  `godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough-crossing`
- Headless flight flythrough:
  `godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough-flight`

Current flythrough screenshots are written under:

- `/tmp/mgrip_flythrough/scene/`
- `/tmp/mgrip_flythrough/boundary/`
- `/tmp/mgrip_flythrough/crossing/`
- `/tmp/mgrip_flythrough/flight/`

After terrain-generation or runtime-streaming changes, prefer verification that
matches the seam you touched:

- Rust/GDExtension changes:
  rebuild the extension
- terrain, chunk, or camera/runtime changes:
  run an appropriate flythrough mode
- map-selector or launch-flow changes:
  verify the main menu, selector, and direct-launch fallback manually

## Knowledge Capture

- Specs are the source of truth for planned work.
- Obsidian notes are the source of truth for wider design and worldbuilding.
- qmd/project memory should capture concrete learnings, gotchas, and decisions
  after they are discovered.
- Do not record speculative future design as memory just because it was
  discussed once.
