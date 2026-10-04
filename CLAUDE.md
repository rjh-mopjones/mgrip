# CLAUDE.md

## What is this project?

Margin's Grip — a Godot 4.3 + Rust GDExtension 3D voxel open-world survival
RPG set on the tidally locked planet Margin. Currently a terrain/runtime
project: streamed terrain, chunk streaming, LOD, player traversal, flythrough
verification, and a developer-only agent playtest runtime.

Not yet a full gameplay runtime. Do not invent APIs for inventory, crafting,
combat, quests, or survival systems — they don't exist yet.

LifeGen (provinces, factions, settlements, roads, trade) is being ported from
the Bevy prototype in stages under `specs/011`. Stages 1 to 3 exist (analysis
grids, provinces, factions) in `gdextension/crates/mg_life`; settlements,
roads and trade do not. Generated factions are numbered, not the named
factions from the lore. Nothing in the Godot runtime reads
LifeGen data yet.

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
- The `wgpu` compute path in `mg_noise/src/gpu` cannot run in the web build —
  keep a CPU path working
- `rayon` threads on web need SharedArrayBuffer, so the host must send
  COOP/COEP headers (rules out GitHub Pages)
- Keep `mg_core` and `mg_noise` free of Godot dependencies so they can compile
  to wasm for the map
- GPU noise is compiled out on web (`cfg(target_os = "emscripten")`), so web
  terrain always comes from the CPU path while native may use the GPU path

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

World map for the site (generated, gitignored, needs a layers artifact):

```sh
./gdextension/target/release/margins_grip export site-map site/dist/map
```

Writes one PNG per layer in the artifact (macromap, heightmap, temperature, …,
shown as switchable layers on the map page), `chunks.bin` (per chunk: light
level, zone, biome) and `map.json`. The map's seed must match `GameState.world_seed`, or the map and
the spawned terrain will disagree.

Lore content:

- Source of truth is the Obsidian vault at
  `~/Documents/mop-jones-brain/Notes/` (`Margin's Grip - <Topic>.md`, index in
  `Margin's Grip Game World Primer.md`)
- Vault technical notes predate the Godot migration (Bevy, 2D top-down) — the
  repo wins on any technical disagreement
- All lore in the Primer is public, including History and Main Quest

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
