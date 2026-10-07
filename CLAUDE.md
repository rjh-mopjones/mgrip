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
- Provinces fill drainage basins (`compute_basins`); settlements are placed
  by site appeal (river mouths, confluences, shores); roads are routed one
  at a time over shared ground and record their river crossings. See
  `specs/013`, "Stage 6 as built", before changing any of it
- Every stage takes an `mg_life::Grid` (the sphere, or a flat patch in
  tests). Measure distances, step to neighbours and weigh areas through it,
  never with raw `x`/`y` arithmetic, so the seam, the poles and the
  narrowing of cells towards them are handled in one place. Areas are in
  equatorial cells
- Orders and nomads are not represented
- Calibration is open; read the spec's open questions before relying on the
  numbers
- The game reads LifeGen through a civ pack (`data/civ/seed_<seed>.mgciv`,
  gitignored, written by `margins_grip export civ-pack`), loaded beside
  the macro pack. It gives the HUD its province, state and nearest
  settlement, and `scripts/world/civ_marks.gd` marks settlements, roads
  and bridges on the ground. Regenerate it after any LifeGen or terrain
  change; a pack for another seed is ignored. Marks, not buildings: there
  are still no settlement, road or bridge assets

## Quick reference

Build Rust extension:
```sh
cargo build --release --manifest-path gdextension/Cargo.toml
```

Civ pack (LifeGen for the game; needs a layers artifact):
```sh
./gdextension/target/release/margins_grip export civ-pack data/civ/seed_42.mgciv
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
| `scripts/world/` | Terrain runtime, chunk streaming (`world.gd`, `chunk_streamer.gd`), LifeGen marks (`civ_marks.gd`) |
| `scripts/player/` | Player controller (`fps_controller.gd`) |
| `scripts/ui/` | Map overlay, chunk preview renderer |
| `gdextension/` | Rust workspace for terrain generation and mesh data |
| `specs/` | Numbered markdown specs |
| `tools/` | Python test harnesses for agent bridge |
| `site/` | Static website generator (`build.ts`), content and assets |
| `gdextension/crates/mg_web` | Terrain tiles for the site's map, built as WebAssembly |

## Key ownership boundaries

- `game_state.gd` — shared world seed, current/anchor chunk, launch state
- `generation_manager.gd` — coordinate helpers, Rust-backed generation entry points
- `flythrough.gd` — automated camera, settle timing, screenshot capture
- `world.gd` — world bootstrap, player placement, terrain sampling, map overlay
- `chunk_streamer.gd` — chunk lifecycle, LOD, prewarm, horizon streaming, collision
- `fps_controller.gd` — player movement, scripted motion seam, fly/swim states
- `agent_runtime.gd` — developer-only agent session, action dispatch, observation API
- `civ_marks.gd` — marks on the ground for settlements, roads and bridges, from the civ pack; reads terrain only through `world.gd`
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
- The map wraps east to west: its left and right edges are neighbours. A
  step past its top or bottom edge comes down the far side of that pole,
  half a world away. The generator does that; the game never streams past
  a pole
- Margin is a sphere (`specs/014`, `mg_core::sphere`). The sub-stellar
  point is 45° up from its south pole, at the bottom centre of the map
  (`specs/015`, `SUN_LATITUDE_DEGREES`); the anti-stellar point is 45° down
  from the north pole at the top edges. Light is the angle from the
  sub-stellar point, warped by noise, so the terminator is a great circle
  and the terminus arcs across the map. The flat map is an equirectangular
  projection: columns are longitude, rows latitude, the top and bottom
  edges the poles. Everything that measures a distance, an area or a step
  goes through the sphere module; nothing else knows the projection
- The world is generated on a cubed sphere (`specs/015`, `mg_core::cube`):
  six square faces of 296 cells, no cell a sliver and no pole in any solve.
  `MacroMap` (`mg_noise/src/macro_map.rs`) holds every layer on the cube
  and reads them off onto the 1024 by 512 flat map (`BiomeMap`) at the end;
  everything downstream reads that map. Solvers on macro data take a
  `&CubeGrid` and never index `y * width + x`. A step at a face edge is a
  bug: check with `margins_grip inspect cube-seam <seed>`
- The land is seeded, not regenerated (`specs/017`): the flat map's
  continents the author chose are the `wind-4` layers artifact, exported
  once as `~/.margins_grip/seed_42_wind4.mgseed` (`export seed-land`), and
  `generate layers 42 <tag> --seed-land <file>` grows the cube from it.
  Generating without the seed makes a different world. The seed carries the
  flat map's light too, which is what put the terminus where it is
- LifeGen runs on the cube's cells (`mg_life::Grid::cube`, the faces
  stacked into one raster) from the macro cube stored beside a layers
  artifact (`macro_cube.bin`); `export site-map` and `export civ-pack`
  carry its provinces, settlements and roads onto the chunk raster. A
  province radiating from a pole is a bug: check `provinces-poles.png` in
  the site map export
- Neighbouring chunks share their border samples: a chunk's last column and
  its neighbour's first column are the same world positions and must have the
  same heights. Check with `margins_grip inspect chunk-seam <seed> <x> <y>`
- The habitable band is the terminus ring between them
- South/day = hot and harsh, North/night = frozen and dark
- Temperature derives from light level and altitude, not Earth-like latitude
- No green vegetation palette anywhere — if it looks Earth-green, it's wrong
- Dayside liquid water evaporates — not normal Earth rivers or oceans
- Liquid sea is one unbroken ring round the world: the whole rim can be
  sailed. `rim_sea.rs` cuts straits to make it so, and
  `margins_grip export site-map` reports if it is ever broken
- Where the sea freezes and where it dries out are ragged lines, not arcs
  (`sea_margin_drift`). Anything that asks whether sea is liquid must pass
  that drift, or it will disagree with the map
- Most of the world is land and most of it is hostile, by design. On seed 42,
  by area: 89% land, 11% liquid water; 50% dayside, 28% terminus, 22%
  nightside (`export site-map` prints these). Do not "fix" this towards
  Earth-like proportions

## River invariants

Rivers are drawn from one shared geometry (`RiverCourse`, see `specs/012`):
the macro map, map tiles and game chunks all sample the same courses. Do not
add a second way of rasterising rivers. The invariants below are enforced in
`mark_surface_rivers` (`mg_noise/src/rivers.rs`): a river is drawn only where
its water stays liquid all the way to a body of water.
Rivers and valleys come from one drainage solve (`mg_noise/src/drainage.rs`):
erosion carves the terrain with it and the river network is read from it. Do
not solve flow a second time for rivers.

- Rivers only form where precipitation exceeds evaporation — the terminus band
- No surface rivers on deep dayside (water evaporates) or deep nightside (frozen solid)
- No frozen rivers, no desert rivers — only liquid surface water in the habitable terminus
- Every river ends in a body of water: the sea, or a terminal lake where
  the ground beyond is too dry or too cold for it to run on (decided
  2026-10-05, `specs/013`). No river just stops on dry ground
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

The macro heightmap is grown, not drawn from noise (`specs/013`,
`mg_noise/src/landscape.rs`): land starts flat, is lifted where the crust is
pushed up, and is cut by rivers until the two balance. Biomes read the real
height and slope of that land. Do not add height from an independent noise
layer at macro scale.

What cuts the land depends on the zone (`erosion_sim.rs`): rivers in the
terminus; ice on the night side, which flows down its own surface, widens
valleys and can dig below the sea; flood-cut canyons in the desert. Lakes
are hollows that hold water, kept as a water level beside the heightmap.
Wind (`wind.rs`) carries desert sand to where it slackens: sand seas are
placed by that, not by a noise layer, and the ground there stands in dunes.
The same wind carries moisture off the liquid sea and rains it on windward
slopes (`climate.rs`): humidity is that rain, not noise. The only noise
layers biomes still read are rock hardness and the tectonic layer.
Ground below sea level is sea, wherever it came from. All of it has sliders
in the sandbox; change a default there first, then copy it into the code.

The macro map also carries its land on a finer cube, four cells per macro
cell each way (`FineHeights`, a cube field; `BiomeMap::fine_heights`, in the
macro pack too). Rivers are read from the drainage of that cube, and
anchored tiles, chunks and the site's images take their ground from it by
sampling it at a world position. Generating the macro map takes about a
minute and a half and 2 GB. `margins_grip inspect relief <seed> <png>`
writes a hillshade of it, with the poles seen from above beside it.

`/sandbox/` is the erosion sandbox for `specs/013`: the landscape step
(uplift, drainage, stream power erosion) running on the real coastline in the
browser, with its parameters on sliders (`mg_web/src/sandbox.rs`,
`site/assets/sandbox.js`). It runs the generator's own `erosion_step`. The
values chosen there are copied by hand into `ErosionParams::default`,
`UpliftMix::default` and the run-off constants in `drainage.rs`.

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
./gdextension/target/release/margins_grip export macro-pack 42 data/macro/seed_42.mgmacro --seed-land ~/.margins_grip/seed_42_wind4.mgseed
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
./gdextension/target/release/margins_grip generate layers 42 <tag> --seed-land ~/.margins_grip/seed_42_wind4.mgseed
```

Takes about two minutes.

World map for the site (generated, gitignored, needs a layers artifact):

```sh
./gdextension/target/release/margins_grip export site-map site/dist/map
```

Terrain generator for the map's zoomed-in tiles (plain WebAssembly, stable
Rust, no emscripten; the site build copies it to `dist/assets/terrain.wasm`):

```sh
(cd gdextension && cargo build -p mg_web --release --target wasm32-unknown-unknown)
cd site && bun run build
```

Writes one PNG per layer in the artifact, `relief.png` (hillshade of the
fine-grid land), `sea.png` (liquid sea on the same grid; the map's coast),
`network.json` (road paths, trade flows, river courses),
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

The map opens as a globe (`view.globe`; `?view=flat` opens the sheet): the same
shader with a sphere in `chunkAtPixel`, and every overlay placed through
`toScreen`, which says when a point is round the back. Keep both projections
in that one place. The world is generated on the cube (`specs/015`, `017`),
so the globe is the planet. The sheet is drawn to 50° of latitude and the
cube's polar faces beyond (`cap-north.png`, `cap-south.png`, with
`cap-<pole>-provinces.bin`), so the poles are never the sheet's stretched
rows; flat, the map is the whole sheet (`band_latitude` in `map.json`).

Zoomed in, the map lays sharper terrain tiles over the whole-world image.
They are rendered in the browser, on demand, by `gdextension/crates/mg_web`
(the game's generator compiled to WebAssembly) from `world.mgmacro`, the
macro pack the site map export writes. `mg_noise::generate_map_tile` is the
one definition of a map tile: `macromap.png` and the browser tiles both use
it. Rebuild `mg_web` after any terrain generation change, or the map's
close-up terrain will disagree with its overview.

Lore content:

- Source of truth is the Obsidian vault at
  `~/Documents/mop-jones-brain/Notes/` (`Margin's Grip - <Topic>.md`, index in
  `Margin's Grip Game World Primer.md`)
- Vault technical notes predate the Godot migration (Bevy, 2D top-down) — the
  repo wins on any technical disagreement
- All lore in the Primer is public, including History and Main Quest

## Known issues

- Headless runs print `Parameter "m" is null` from `mesh_get_surface_count`
  for every road strip `civ_marks.gd` builds: the dummy render server does
  not track SurfaceTool meshes. Harmless; it does not happen windowed
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
