# CLAUDE.md

## What is this project?

Margin's Grip — a Godot 4.3 + Rust GDExtension 3D voxel open-world survival
RPG set on the tidally locked planet Margin. Currently a terrain/runtime
project: streamed terrain, chunk streaming, LOD, player traversal, flythrough
verification, and a developer-only agent playtest runtime.

Not yet a full gameplay runtime. Do not invent APIs for inventory, crafting,
combat, quests, or survival systems — they don't exist yet.

## Quick reference

Build Rust extension:
```sh
cargo build --release --manifest-path gdextension/Cargo.toml
```

Run the project:
```sh
godot --path /Users/roryhedderman/Documents/GodotProjects/mgrip
```

Run fly-swim smoke test (headless):
```sh
godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --fly-swim-smoke-test
```

Run agent smoke test (headless):
```sh
godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --agent-runtime-smoke-test
```

Flythrough verification:
```sh
godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough
godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough-boundary
godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough-crossing
godot --display-driver headless --path /Users/roryhedderman/Documents/GodotProjects/mgrip -- --flythrough-flight
```

External bridge test:
```sh
python3 tools/test_fly_swim.py [--windowed]
```

## Repo layout

| Path | Purpose |
|------|---------|
| `project.godot` | Project entry, input map, autoload wiring |
| `scenes/` | Runtime scenes (`main_menu.tscn`, `world.tscn`, `map_selector.tscn`) |
| `scripts/autoload/` | Singletons: `game_state.gd`, `generation_manager.gd`, `flythrough.gd`, `agent_runtime.gd` |
| `scripts/world/` | Terrain runtime, chunk streaming (`world.gd`, `chunk_streamer.gd`) |
| `scripts/player/` | Player controller (`fps_controller.gd`) |
| `scripts/ui/` | Map overlay, chunk preview renderer |
| `gdextension/` | Rust workspace for terrain generation and mesh data |
| `specs/` | Numbered markdown specs |
| `tools/` | Python test harnesses for agent bridge |

## Key ownership boundaries

- `game_state.gd` — shared world seed, current/anchor chunk, launch state
- `generation_manager.gd` — coordinate helpers, Rust-backed generation entry points
- `flythrough.gd` — automated camera, settle timing, screenshot capture
- `world.gd` — world bootstrap, player placement, terrain sampling, map overlay
- `chunk_streamer.gd` — chunk lifecycle, LOD, prewarm, horizon streaming, collision
- `fps_controller.gd` — player movement, scripted motion seam, fly/swim states
- `agent_runtime.gd` — developer-only agent session, action dispatch, observation API

Build on these seams. Do not create parallel ownership paths.

## Conventions

- Follow specs under `specs/` — read the relevant one before implementing
- Coordinate terminology from `specs/002`: `chunk_coord`, `world_origin`,
  `scene_block`, `block_coord`, `player_position` — keep them distinct and labeled
- Agent runtime follows `specs/004` contract — structured observe/act/observe loop
- Movement goes through `fps_controller.gd`, not raw input emulation
- Terrain queries go through `world.gd`
- Chunk ownership stays in `chunk_streamer.gd`
- The agent runtime must stay developer-only and inert by default

## World invariants

- Margin is tidally locked — permanent day side and night side
- The habitable band is the terminus ring between them
- South/day = hot and harsh, North/night = frozen and dark
- Temperature derives from light level and altitude, not Earth-like latitude
- No green vegetation palette anywhere — if it looks Earth-green, it's wrong
- Dayside liquid water evaporates — not normal Earth rivers or oceans

## River invariants

- Rivers only form where precipitation exceeds evaporation — the terminus band
- No surface rivers on deep dayside (water evaporates) or deep nightside (frozen solid)
- No frozen rivers, no desert rivers — only liquid surface water in the habitable terminus
- Every river must flow into a body of water (the sea) — no rivers ending mid-land
- Rivers widen downstream as tributaries merge — headwaters thin, mouth wide
- Rivers cannot be wider than two chunks (2 world units)
- Rivers follow terrain — they sit in valleys, not painted on flat ground
- Rivers form dendritic drainage networks — tributaries branch and merge into trunk systems
- No rivers rendered in ocean cells — river stops at coastline

## Git

- Never add Co-Authored-By signatures
- Keep commits focused — separate tidying from behavior changes
- Don't push without being asked

## After making changes

- Rust changes: rebuild the extension
- Terrain/streaming changes: run appropriate flythrough mode
- Player controller changes: run `--fly-swim-smoke-test`
- Agent runtime changes: run `--agent-runtime-smoke-test`
- Map/UI changes: verify with windowed run, not headless

## Detailed context

See `AGENTS.md` for full ownership boundaries, coordinate conventions, agentic
playtesting contract, external documentation pointers, and world visual
invariants.
