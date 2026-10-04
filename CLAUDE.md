# CLAUDE.md

## What is this project?

Margin's Grip — a Godot 4.3 + Rust GDExtension 3D voxel open-world survival
RPG set on the tidally locked planet Margin. Currently a terrain/runtime
project: streamed terrain, chunk streaming, LOD, player traversal, flythrough
verification, and a developer-only agent playtest runtime.

Not yet a full gameplay runtime. Do not invent APIs for inventory, crafting,
combat, quests, or survival systems — they don't exist yet.

LifeGen (provinces, factions, settlements, roads, trade) is ported from the
Bevy prototype under `specs/011`, plus a seventh stage that names everything.
All stages exist in `gdextension/crates/mg_life` at macro resolution (one
cell per chunk).

- The lore's named states are authored in `gdextension/data/lifegen_states.ron`
  and placed first; generated minor states fill the rest
- Generated names are built from the word lists in
  `gdextension/data/lifegen_names.ron`. Change names there, not in code
- A "major river" is read from the macro river layer, whose value is the
  river's relative size (`TerrainQuery::river_at`)
- Every stage takes an `mg_life::Grid` (resolution + whether the grid is a
  ring). Measure distances and step between columns through it, never with
  raw `x` arithmetic, so the east-west seam is handled in one place
- Orders and nomads are not represented
- Calibration is open; read the spec's open questions before relying on the
  numbers
- Nothing in the Godot runtime reads LifeGen data yet

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
| `site/` | Static website generator (`build.ts`), content and assets |

## Key ownership boundaries

- `game_state.gd` — shared world seed, current/anchor chunk, launch state
- `generation_manager.gd` — coordinate helpers, Rust-backed generation entry points
- `flythrough.gd` — automated camera, settle timing, screenshot capture
- `world.gd` — world bootstrap, player placement, terrain sampling, map overlay
- `chunk_streamer.gd` — chunk lifecycle, LOD, prewarm, horizon streaming, collision
- `fps_controller.gd` — player movement, scripted motion seam, fly/swim states
- `agent_runtime.gd` — developer-only agent session, action dispatch, observation API
- `mg_life` (Rust crate) — LifeGen; reads terrain only through `mg_core::TerrainQuery`

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
- The map wraps east to west, not north to south. Its left and right edges
  are neighbours
- Light is distance from the sub-stellar point (bottom centre of the map), so
  the terminus is an arc across the map, not a straight band. This is
  deliberate: it is a round world. Do not flatten it into a ring
- Neighbouring chunks share their border samples: a chunk's last column and
  its neighbour's first column are the same world positions and must have the
  same heights. Check with `margins_grip inspect chunk-seam <seed> <x> <y>`
- The habitable band is the terminus ring between them
- South/day = hot and harsh, North/night = frozen and dark
- Temperature derives from light level and altitude, not Earth-like latitude
- No green vegetation palette anywhere — if it looks Earth-green, it's wrong
- Dayside liquid water evaporates — not normal Earth rivers or oceans
- Most of the world is land and most of it is hostile, by design. On seed 42:
  89% of chunks are land, 11% liquid sea; by zone, 55% dayside, 22% terminus,
  23% nightside. Do not "fix" this towards Earth-like proportions

## River invariants

Rivers are drawn from one shared geometry (`RiverCourse`, see `specs/012`):
the macro map, map tiles and game chunks all sample the same courses. Do not
add a second way of rasterising rivers.

- Rivers only form where precipitation exceeds evaporation — the terminus band
- No surface rivers on deep dayside (water evaporates) or deep nightside (frozen solid)
- No frozen rivers, no desert rivers — only liquid surface water in the habitable terminus
- Every river must flow into a body of water (the sea) — no rivers ending mid-land
- Rivers widen downstream as tributaries merge — headwaters thin, mouth wide
- Rivers cannot be wider than two chunks (2 world units)
- Rivers follow terrain — they sit in valleys, not painted on flat ground
- Rivers form dendritic drainage networks — tributaries branch and merge into trunk systems
- No rivers rendered in ocean cells — river stops at coastline

## Website

A static site lives in `site/`: overview, world map with in-browser play, lore,
design notes and specs, devlog. Built locally; not deployed yet.

```sh
cd site && bun install && bun run build   # pages -> site/dist
bun run typecheck && bun run lint         # after editing build.ts or assets
```

- `site/build.ts` — the whole generator. Sources: `site/content/` (overview,
  design approach, map page), the Obsidian vault (lore + design notes),
  `specs/` (spec pages), `git log` (devlog)
- `site/assets/` — `site.css`, `map.js`
- `site/dist/` — all output, gitignored. Also holds the web build
  (`dist/play/`) and map data (`dist/map/`), which have their own commands
  below and are not touched by the site build
- Which vault notes are published, and under which heading, is decided by the
  Primer note's groups: World and Quests go under Lore, the rest under Design

In-browser play is the primary target. Native download comes later.

Not done yet: deployment (the host must send COOP/COEP headers), release
(non-debug) web export, download size reduction.

Tone: matter-of-fact, portfolio style, mainly for the author's own reference.
No marketing copy, no decorative fonts, no scroll-driven layouts. State what
exists and what doesn't.

Constraints this puts on runtime work:

- Web export only supports the Compatibility renderer (WebGL2); the project is
  currently Forward+. Flag any new Forward+-only feature — it needs a
  Compatibility fallback
- `rayon` threads on web need SharedArrayBuffer, so the host must send
  COOP/COEP headers (rules out GitHub Pages)
- Keep `mg_core` and `mg_noise` free of Godot dependencies so they can compile
  to wasm for the map
- Terrain generation is CPU only and both builds anchor chunks to the same
  macro pack, so the native game, the web build and the layers artifacts
  describe the same world for a given seed
- `mg_noise::generate_macro_map(seed)` is the one definition of the macro map.
  Layers artifacts, macro packs and the runtime all call it

Web build (spike passed 2026-10-04: extension loads, chunks stream, terrain
renders in Chrome):

```sh
# Toolchain: Rust nightly + rust-src, emscripten 4.0.0 in ~/emsdk,
# Godot 4.3 web export templates (web_dlink_*)
source ~/emsdk/emsdk_env.sh
(cd gdextension && cargo +nightly build --release --lib -Zbuild-std --target wasm32-unknown-emscripten)
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . --export-debug "Web" site/dist/play/index.html
python3 tools/serve_web.py   # serves site/dist at http://localhost:8060/
```

- wasm rustflags live in `gdextension/.cargo/config.toml`
- Emscripten 3.1.64 (Godot 4.3's own version) cannot link current-nightly
  output; 4.0.0 can and loads fine
- `tools/web_shell.html` is the export's HTML shell: `?origin=x,y` in the URL
  becomes the spawn `world_origin`; without it the game opens on its menu
- Spawn point on web: pass `--agent-runtime-quick-launch` and
  `--agent-runtime-world-origin=x,y` through `GODOT_CONFIG.args` in the HTML
- Shaders: in the Compatibility renderer `TIME` only exists inside the shader
  entry function — pass it into helpers as a parameter

Macro pack (the macro map the game anchors chunks to; gitignored, both native
and web builds load it from `res://data/macro/`):

```sh
./gdextension/target/release/margins_grip export macro-pack 42 data/macro/seed_42.mgmacro
```

- Regenerate it after any change to terrain generation, and before a web
  export. The game checks a pack against the current generator and ignores a
  stale one
- Without a valid pack the game generates the macro map itself on first use:
  correct, but about 7 seconds natively and longer in a browser
- The game never reads `~/.margins_grip/`. Only the CLI does

Layers artifact (full macro layers and images for the CLI and the site map,
stored under `~/.margins_grip/layers/`):

```sh
./gdextension/target/release/margins_grip generate layers 42 <tag>
```

Takes about two minutes.

World map for the site (generated, gitignored, needs a layers artifact):

```sh
./gdextension/target/release/margins_grip export site-map site/dist/map
```

Writes one PNG per layer in the artifact, `relief.png` (hillshade of the macro
heightmap), `network.json` (road paths, trade flows, river courses),
`chunks.bin` (per chunk: light level, zone, biome, province
id) and `map.json` (province, faction and settlement tables). The map's seed
must match `GameState.world_seed`, or the map and the spawned terrain will
disagree.

The map page (`site/assets/map.js`) is a WebGL 2 campaign-style map: a
province-id texture plus a per-province colour table. Map modes, hover and
selection rewrite the table, not the images. Add a map mode by adding an
entry to `MAP_MODES`; do not bake a new image for it. Rivers, roads, trade
and settlements are drawn as vectors from `network.json`, with what shows
depending on zoom (`MAP_FEATURES`, `SETTLEMENT_STYLES`, `ROAD_STYLES`).

Lore content:

- Source of truth is the Obsidian vault at
  `~/Documents/mop-jones-brain/Notes/` (`Margin's Grip - <Topic>.md`, index in
  `Margin's Grip Game World Primer.md`)
- Vault technical notes predate the Godot migration (Bevy, 2D top-down) — the
  repo wins on any technical disagreement
- All lore in the Primer is public, including History and Main Quest

## Known issues

- The agent runtime smoke test failed with `action_timed_out` on
  `move_to_block`: the agent stopped at a chunk boundary. Terrain steps along
  chunk borders were the likely cause and are fixed, but the smoke test has
  not been re-run since. Tracked in GitHub issue #3
- `res://assets/icon.svg` is missing; an error is logged on every launch

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
