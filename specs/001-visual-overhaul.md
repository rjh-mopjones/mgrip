# Spec 001 — Visual Overhaul: From Prototype to Cozy

**Status:** Open
**Priority:** Critical — terrain pipeline works but visual output is unusable

## Problem

The 3D terrain renderer produces correct terrain shapes (coastlines, bays, elevation) but looks grim:

- ~~All-white terrain because the default spawn is in The Black~~ **(FIXED — spawn moved to Terminus)**
- Flat unshaded vertex colors with harsh stripe artifacts on cliff faces
- No ambient occlusion — blocks look flat instead of solid
- No greedy meshing on side faces — millions of tiny 1-block-wide quads create visual noise
- Micro detail octaves are disabled (`detail_level=0`) making terrain smooth at block scale
- ~~HEIGHT_SCALE=80 too low~~ **(PARTIALLY FIXED — now 150, may need 200)**
- Fog dissolves distant terrain into dark maroon void
- Water is a flat blue rectangle

## World Rules (must not violate)

- **Green is not dominant.** Red supergiant → photosynthesizers lean toward black, dark purple, burgundy, maroon. Sparse muted green vegetation is acceptable; lush Earth-green is not.
- **Sub-stellar point at normalized (0.5, 1.0)** = world coords (512, 512). South = day, North = night.
- **Three zones:** The Wash (dayside, scorching), Terminus (habitable crescent), The Black (frozen nightside).
- **Temperature derived from light_level.** Never independent.
- **45°C hard vegetation gate.** Above 45°C, force Arid — no vegetation.
- **No liquid surface water on dayside.** All evaporated.

## Modifies

```
scripts/world/world.gd
scripts/world/voxel_mesh_builder.gd
scenes/world.tscn
gdextension/src/lib.rs
assets/shaders/water.gdshader  (NEW)
```

## Implementation

### Section 1 — Move Default Spawn to Terminus

**File:** `scripts/world/world.gd`

**ALREADY DONE** — spawn is now at `world_x = 400.0, world_y = 200.0`. This is in the Terminus zone. If the terrain is still showing mostly one biome type, try `world_y = 250.0` which is closer to the center of the habitable crescent (normalized y ≈ 0.49, moderate light level). The key biome variety lives in the `light_level` range 0.28–0.62 which maps to temperatures 0–45°C (Temperate through Warm climate classes).

**Verification:** Run the game. Terrain should show a mix of dark purples, maroons, browns, and sandy tones — not all-white. If still mostly one color, check the biome map overlay (M key) to see what biomes are actually being generated at this location.

---

### Section 2 — Enable Micro Heightmap Detail

**File:** `gdextension/src/lib.rs`

The API now uses two separate freq_scale parameters: `height_freq_scale` (8.0 — controls continentalness, tectonic, rock_hardness, peaks_valleys) and `biome_freq_scale` (30.0 — controls humidity). But `detail_level` is still 0, which means `derive_micro_heightmap` (biome_map.rs line 302–311) is never called.

```rust
// BEFORE (current state, line ~227-238)
pub fn generate_chunk(&self, seed: i64, world_x: f64, world_y: f64) -> Gd<MgBiomeMap> {
    let map = BiomeMap::generate(
        seed as u32,
        world_x, world_y,
        1.0, 1.0,
        512, 512,
        0,              // detail_level=0, micro detail disabled
        false,
        false,
        8.0,            // height_freq_scale
        30.0,           // biome_freq_scale
    );

// AFTER
pub fn generate_chunk(&self, seed: i64, world_x: f64, world_y: f64) -> Gd<MgBiomeMap> {
    let map = BiomeMap::generate(
        seed as u32,
        world_x, world_y,
        1.0, 1.0,
        512, 512,
        2,              // detail_level=2 enables derive_micro_heightmap
        false,
        false,
        8.0,            // height_freq_scale — smooth hills, ~64px wavelength
        30.0,           // biome_freq_scale  — rapid biome variation within hills
    );
```

**Why:** With `detail_level=0`, height variation comes only from the base fBm layers at 8× frequency — this gives ~64-pixel-wavelength terrain features but smooth surfaces between them. Enabling detail_level=2 calls `derive_micro_heightmap`, which adds independently-normalized octaves 8–15 (start freq 2.56) on top of the base heightmap. This creates block-level hills and valleys within the larger terrain shapes.

The detail noise uses true world coords via `px_to_wx`/`py_to_wy` (e.g., 400.0–401.0 for a chunk at world_x=400). At start freq 2.56, that's `400 * 2.56 = 1024` — plenty of variation across the 512 pixels.

**Verification:** After rebuilding the GDExtension (`cd gdextension && cargo build --release`), run the game. Terrain should show small-scale undulation and block-level height variation, not just smooth large-scale shapes.

**Risk:** The detail octaves operate on true wx/wy while base layers see scaled coords (8× / 30×). If micro detail looks too uniform or disconnected from the terrain shape, try passing `wx * height_freq_scale` to `derive_micro_heightmap` instead of plain `wx` — this would make the detail noise vary at the same spatial rate as the terrain features.

---

### Section 3 — Increase HEIGHT_SCALE

**File:** `scripts/world/voxel_mesh_builder.gd`

HEIGHT_SCALE is currently 150.0 (already increased from the original 80.0). This is a reasonable starting point. After enabling micro detail (Section 2), evaluate whether 150 gives enough relief:

- At scale 150, typical land (heightmap 0.0–0.3) = 0–45 blocks. Mountains (0.5) = 75 blocks. With micro detail budgets (0.40 for mountains), peaks could reach ~120 blocks.
- If terrain still feels flat after Section 2, increase to 200.0:

```gdscript
# CURRENT
const HEIGHT_SCALE := 150.0
const SEA_LEVEL_Y  := -1  # floor(-0.01 * 150) = -1

# IF NEEDED
const HEIGHT_SCALE := 200.0
const SEA_LEVEL_Y  := -2  # floor(-0.01 * 200) = -2
```

**Why:** The relationship between HEIGHT_SCALE and freq_scale determines how dramatic terrain looks. With `height_freq_scale=8.0`, terrain features have ~64-pixel wavelengths. At scale 150, a hill peak of heightmap=0.15 is 22 blocks tall over a 64-block-wide hill — that's a 19° slope, which feels natural. At scale 200 the same hill is 30 blocks / 64 wide = 25° — steeper but still fine.

**Verification:** Mountains should be visually imposing. From ground level in a mountainous chunk, peaks should fill a significant portion of the vertical view. Plains should still have gentle rolling hills, not be flat.

---

### Section 4 — Switch to Per-Pixel Shading + Enable SSAO

**File:** `scripts/world/voxel_mesh_builder.gd`

Replace the land material setup:

```gdscript
# BEFORE (lines 78-80)
var land_mat := StandardMaterial3D.new()
land_mat.vertex_color_use_as_albedo = true
land_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED

# AFTER
var land_mat := StandardMaterial3D.new()
land_mat.vertex_color_use_as_albedo = true
land_mat.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL
land_mat.roughness = 0.95
land_mat.metallic = 0.0
```

Remove the manual brightness multipliers in `_side_faces`:

```gdscript
# BEFORE (lines 197-199)
var ew := Color(col.r * 0.65, col.g * 0.65, col.b * 0.65)
var ns := Color(col.r * 0.80, col.g * 0.80, col.b * 0.80)

# AFTER — use biome color directly, let Godot lighting handle shading
var ew := col
var ns := col
```

**File:** `scenes/world.tscn`

In the Environment sub-resource (id="4"), add SSAO:

```
ssao_enabled = true
ssao_radius = 2.0
ssao_intensity = 1.5
ssao_power = 1.5
```

Also increase ambient light to prevent shadow faces going pitch black:

```
# BEFORE
ambient_light_energy = 1.4

# AFTER
ambient_light_energy = 2.2
```

**Why:** `SHADING_MODE_UNSHADED` means the DirectionalLight, shadows, and ambient light in the scene are completely ignored. The terrain is just flat vertex colors with crude per-face brightness hacks (0.65 for east/west, 0.80 for north/south). This creates the harsh dark-stripe look on every cliff face.

Per-pixel shading lets Godot's PBR lighting naturally darken faces away from the sun and lighten faces toward it. SSAO adds soft shadows in crevices and at block edges — this is the single biggest factor in making blocks feel solid and "cozy" (Minecraft uses a similar technique).

**Verification:** Cliff faces should show smooth light-to-shadow gradients instead of uniform dark stripes. Block edges where terrain steps down should show subtle dark shadows. The terrain should feel three-dimensional.

---

### Section 5 — Side Face Substrate Color Blending

**File:** `scripts/world/voxel_mesh_builder.gd`

Replace the `_side_faces` function to blend biome color toward a geological substrate:

```gdscript
func _side_faces(
        st: SurfaceTool,
        heights: PackedInt32Array,
        ocean_mask: PackedByteArray,
        x: int, y: int, z: int,
        col: Color, biome_idx: int) -> void:
    # Determine substrate color by biome group
    var substrate: Color
    if biome_idx >= 12 and biome_idx <= 18:
        # Frozen biomes: blue-gray ice substrate
        substrate = Color(0.55, 0.58, 0.65)
    elif biome_idx >= 38 and biome_idx <= 48:
        # Desert/volcanic: reddish-brown rock
        substrate = Color(0.52, 0.38, 0.28)
    else:
        # Default: dark brown earth
        substrate = Color(0.42, 0.32, 0.24)

    _side(st, heights, ocean_mask, x, y, z,  1,  0, col, substrate)
    _side(st, heights, ocean_mask, x, y, z, -1,  0, col, substrate)
    _side(st, heights, ocean_mask, x, y, z,  0,  1, col, substrate)
    _side(st, heights, ocean_mask, x, y, z,  0, -1, col, substrate)
```

Update `_side` to accept substrate and blend by depth:

```gdscript
func _side(
        st: SurfaceTool,
        heights: PackedInt32Array,
        ocean_mask: PackedByteArray,
        x: int, y: int, z: int,
        dx: int, dz: int,
        surface_col: Color, substrate: Color) -> void:
    var nx := x + dx
    var nz := z + dz
    var ny: int
    if nx < 0 or nx >= CHUNK_SIZE or nz < 0 or nz >= CHUNK_SIZE:
        ny = y - 1
    else:
        var nidx := nz * CHUNK_SIZE + nx
        ny = SEA_LEVEL_Y - 1 if ocean_mask[nidx] else heights[nidx]
    if ny >= y:
        return

    var x0 := float(x);  var x1 := x0 + 1.0
    var z0 := float(z);  var z1 := z0 + 1.0
    var y_top := float(y) + 1.0
    var y_bot := float(ny) + 1.0

    # Blend from surface to substrate over top 4 blocks of exposure
    var exposure := y_top - y_bot
    var t := clampf((exposure - 2.0) / 4.0, 0.0, 1.0)
    var col := surface_col.lerp(substrate, t * 0.7)

    st.set_color(col)
    # ... (rest of face emission unchanged — normals + vertices per direction)
```

**Note:** The `_side_faces` signature now takes `biome_idx` — the caller in `_build_land_sub` needs to pass it. Extract the biome index from `biome_rgba` at the pixel: `var biome_i := biome_rgba[bi]` won't work since biome_rgba is the color, not the index. Instead, use the biome index lookup from the BIOME_COLORS array, or better: pass the biome map's biome index layer separately.

**Alternative simpler approach:** Skip biome-group detection. Just always blend toward `Color(0.45, 0.35, 0.28)` (generic dark earth). This is 90% of the visual benefit with zero complexity. The biome-specific substrate is a polish pass.

**Verification:** Cliff faces should show the biome color at the top transitioning to earthy brown/gray as the cliff gets deeper. Looks like geological layers.

---

### Section 6 — Fix Fog Color

**File:** `scenes/world.tscn`

In the Environment sub-resource:

```
# BEFORE
fog_light_color = Color(0.20, 0.08, 0.14, 1)
fog_density = 0.0004

# AFTER
fog_light_color = Color(0.50, 0.22, 0.12, 1)
fog_density = 0.00025
```

**Why:** The current fog is dark maroon — distant terrain dissolves into darkness, making the world feel oppressive and enclosed. The sky horizon is warm amber `(0.72, 0.32, 0.12)`. Matching fog to the horizon creates atmospheric perspective — terrain fades into the warm glow of the red supergiant's light scattering through the atmosphere. Reduced density extends visibility.

**Verification:** Distant terrain should fade into a warm amber haze matching the sky horizon, not dark maroon.

---

### Section 7 — Greedy Meshing for Side Faces

**File:** `scripts/world/voxel_mesh_builder.gd`

This is a performance and visual quality improvement. Currently each block column in `_build_land_sub` individually calls `_side_faces`, emitting per-block quads. With 512×512 blocks, worst case is millions of tiny side quads.

Replace the per-block side emission with a post-pass greedy merge:

**Approach:** After computing all heights for the sub-chunk (64×64), for each of the 4 cardinal directions:

1. Build a 2D grid of "side face needed" flags: `needs_face[lz][lx] = true` if `height[x,z] > neighbor_height[x+dx, z+dz]`.
2. For cells that need a face, compute the face parameters: top_y = height, bottom_y = max(neighbor_height, SEA_LEVEL_Y - 1), color.
3. Greedy merge: scan rows, merge adjacent cells with **same top_y, same bottom_y, and same color** into wide quads.
4. Optionally extend merged runs in the perpendicular direction (same technique as top face greedy meshing).

This reduces side face count by 60–80% in typical terrain and eliminates the visual noise of per-block side edges.

**Implementation sketch:**

```gdscript
# After building top faces, do a second pass for each direction
for direction in [Vector2i(1,0), Vector2i(-1,0), Vector2i(0,1), Vector2i(0,-1)]:
    var dx := direction.x
    var dz := direction.y
    var side_visited := PackedByteArray()
    side_visited.resize(SUB_SIZE * SUB_SIZE)
    side_visited.fill(0)

    for lz in SUB_SIZE:
        for lx in SUB_SIZE:
            if side_visited[lz * SUB_SIZE + lx]:
                continue
            var x := ox + lx
            var z := oz + lz
            var idx := z * CHUNK_SIZE + x
            if ocean_mask[idx]:
                continue
            var y := heights[idx]
            var nx := x + dx
            var nz := z + dz
            var ny: int = _get_neighbor_height(nx, nz, heights, ocean_mask)
            if ny >= y:
                continue

            var face_col := _get_face_color(...)  # biome color + substrate blend

            # Extend run along the merge axis
            var run := 1
            # ... (same greedy logic as top faces, but checking same y, same ny, same color)
            # Emit one wide quad for the entire run
```

**Note:** This is the most complex section. If time-constrained, skip this and revisit after sections 1–6 are working. The visual improvement from shading + SSAO is larger.

**Verification:** Cliff faces should appear as smooth walls instead of rows of individual block edges. Frame time should improve (measure with Godot's built-in profiler — `Performance.get_monitor(Performance.TIME_FPS)`).

---

### Section 8 — Water Shader

**File:** `assets/shaders/water.gdshader` (NEW)

```glsl
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_disabled, diffuse_burley, specular_schlick_ggx;

uniform vec3 shallow_color : source_color = vec3(0.1, 0.35, 0.55);
uniform vec3 deep_color    : source_color = vec3(0.02, 0.08, 0.18);
uniform float wave_speed    = 0.8;
uniform float wave_height   = 0.15;

void vertex() {
    float wave1 = sin(VERTEX.x * 0.5 + TIME * wave_speed) * wave_height;
    float wave2 = sin(VERTEX.z * 0.7 + TIME * wave_speed * 0.8) * wave_height * 0.6;
    VERTEX.y += wave1 + wave2;
    // Recompute normal from wave derivatives
    float dx = cos(VERTEX.x * 0.5 + TIME * wave_speed) * 0.5 * wave_height;
    float dz = cos(VERTEX.z * 0.7 + TIME * wave_speed * 0.8) * 0.7 * wave_height * 0.6;
    NORMAL = normalize(vec3(-dx, 1.0, -dz));
}

void fragment() {
    float fresnel = pow(1.0 - dot(NORMAL, VIEW), 2.0);
    ALBEDO = mix(shallow_color, deep_color, fresnel * 0.5);
    ALPHA = mix(0.45, 0.88, fresnel);
    ROUGHNESS = 0.15;
    METALLIC = 0.1;
    SPECULAR = 0.5;
}
```

**File:** `scripts/world/voxel_mesh_builder.gd`

Replace the water material setup:

```gdscript
# BEFORE
var water_mat := StandardMaterial3D.new()
water_mat.albedo_color = Color(0.08, 0.42, 0.78, 0.72)
water_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
water_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
water_mat.cull_mode    = BaseMaterial3D.CULL_DISABLED

# AFTER
var water_mat := ShaderMaterial.new()
water_mat.shader = preload("res://assets/shaders/water.gdshader")
```

**Verification:** Water should have gentle surface ripples, transparency that increases at glancing angles (fresnel), and catch specular highlights from the sun.

---

## Implementation Order

Do these in sequence — each builds on the previous:

1. ~~**Section 1** — Move spawn to Terminus~~ **(DONE — world_x=400, world_y=200)**
2. **Section 2** — Enable detail_level=2 (requires `cd gdextension && cargo build --release`)
3. **Section 3** — Evaluate HEIGHT_SCALE (currently 150; increase to 200 if terrain still flat after Section 2)
4. **Section 4** — Per-pixel shading + SSAO (biggest visual jump)
5. **Section 6** — Fix fog color (atmosphere)
6. **Section 5** — Substrate blending (geological depth)
7. **Section 8** — Water shader (polish)
8. **Section 7** — Greedy side meshing (performance + visual cleanup)

## Verification

After all sections:

1. Run the game at default (400, 250). Terrain should show mixed biomes: dark maroon forests, burgundy woodlands, sandy beaches, gray mountains.
2. Blocks should have soft ambient occlusion shadows at edges.
3. Cliff faces should transition from biome color to earthy substrate.
4. Distant terrain should fade into warm amber fog matching the sky.
5. Water should ripple and catch specular highlights.
6. HEIGHT_SCALE=200 should produce mountains 60-100 blocks tall, plains with gentle 10-30 block hills.
7. Run at (220, 60) to verify The Black still works — should be white/ice terrain with blue-gray substrate, fog fading to dark purple.
8. Run at (500, 450) to verify The Wash — scorched rock, salt flats, desert, no vegetation.

## Constraints

- **Green should not dominate the palette.** Sparse, muted green is fine. If Godot's lighting tints large surfaces green unexpectedly, check the sun and ambient color.
- **Do not modify any Rust code in `gdextension/crates/mg_noise/`** except `src/lib.rs` (Section 2). The noise pipeline is correct.
- **Rebuild GDExtension after Section 2:** `cd gdextension && cargo build --release`
- **Test biome palette with shading:** PBR lighting can shift perceived color. The sun is warm amber `(0.95, 0.78, 0.60)` — this will warm up cool biome colors. If dark purple Forest starts looking brown under sunlight, that's physically correct. If anything looks green, the ambient light color `(0.20, 0.18, 0.28)` may need adjusting — shift it more toward purple `(0.22, 0.15, 0.30)`.

## Claude Code Prompt

```
Read the spec at specs/001-visual-overhaul.md. Read all files listed in the
"Modifies" section to understand current code. Section 1 is already done
(spawn moved to Terminus). Implement sections 2 through 8 in the order
specified under "Implementation Order". After section 2, run
`cd gdextension && cargo build --release` to rebuild the Rust GDExtension.
After section 3, evaluate whether HEIGHT_SCALE needs increasing based on
section 2's micro detail results. After all sections, verify the constraints
are met. Do NOT modify any Rust files under gdextension/crates/ — only
gdextension/src/lib.rs is allowed (section 2).
```
