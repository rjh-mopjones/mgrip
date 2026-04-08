# Spec 006 — Dynamic Sky and Atmospheric Lighting

**Status:** Draft
**Priority:** High
**Depends On:** Spec 005 Phase 1 stable (`RuntimeChunkPresentation` and `AtmosphereClass` available)

---

## World Axis Convention

```
south  = +Z  (dayside, substellar point, increasing chunk_coord.y, increasing position.z)
north  = -Z  (nightside)
up     = +Y
```

The sun rises from the southern horizon as the player walks south. At the terminus
(light_level ≈ 0) the sun sits on the southern horizon. At the substellar point
(light_level = 1.0) the sun is directly overhead.

---

## Problem

The sky is currently static and generic. On a tidally locked world, the sky is not
decoration — it is the primary communicator of where the player is on the planet.

Current state:

- the sun does not move as the player walks toward or away from the dayside
- the sky uses a simple hand-authored colour gradient, not any form of atmospheric scattering
- there is no distinction between light at a low angle (terminus) and light overhead (dayside)
- the nightside has no character — no stars, no aurora, no ambient planetary glow
- the `DirectionalLight3D` direction is never updated from player position

On Margin, position is time of day. Every step south is a step toward noon. Every step north
is a step toward night. The sky must express that at all times.

---

## Design Position

### The sun position is a function of where you are, not when it is

`average_light_level` from `RuntimeChunkPresentation` is the canonical source of sun
elevation. It already encodes how much solar energy reaches a given point on the planet.
Map it directly to a sun elevation angle. No planet geometry changes are needed.

```
# world axes: south = +Z, north = -Z, up = +Y
sun_elevation = average_light_level * PI / 2.0
sun_direction = vec3(0.0, sin(sun_elevation), cos(sun_elevation))
# at elevation=0 (terminus): sun sits on southern (+Z) horizon
# at elevation=PI/2 (substellar): sun is directly overhead
```

The noise warping in `light_level` means the sun elevation is not perfectly banded —
it has the same natural irregularity as the terrain generator. That is desirable.

### Rayleigh and Mie scattering, not gradients

Replace the current gradient sky with a physically-inspired single-scatter approximation.
Do not use full ray-marching in the first pass — a simplified phase-function approach
gives 90% of the visual quality at a fraction of the cost.

The atmosphere at low sun angles (the terminus) must read dramatically different from
the atmosphere at high sun angles (the dayside interior). This is the visual heart of
the world.

### Scattering parameters driven by AtmosphereClass

Do not hardcode scattering coefficients. Use `AtmosphereClass` from `RuntimeChunkPresentation`
(spec 005) to select a scattering profile. Humid terminus air should look different from
dry dayside radiance. The atmosphere class is the right abstraction for this.

### Nightside is alive

The nightside is not just the absence of day. It has:

- a faint planetary rim glow from dayside light wrapping the atmosphere
- stars (procedural, hash-based in the shader — no texture)
- aurora curtains animated in the sky
- a subtle geothermal or bioluminescent ground glow hint at the horizon

Aurora intensity scales with distance from the substellar point (i.e., `1.0 - light_level`).
The nightside is the most aurora-active region. The terminus is a transition.

### The DirectionalLight3D is the sun

The existing `DirectionalLight3D` reference in `WorldEnvironmentController` is set
correctly — the sky shader already reads `LIGHT0_DIRECTION` from it. Updating the
light's rotation to match `sun_direction` is all that is needed to make the sun move.

---

## Goals

- sun elevation in the sky is a function of the current chunk's `average_light_level`
- walking south raises the sun; walking north lowers it toward the horizon and beyond
- sky uses Rayleigh and Mie scattering approximation — not a hand-painted gradient
- atmosphere colour and scattering density varies by `AtmosphereClass`
- terminus reads as a permanent golden hour with thick low-angle light and redness
- dayside reads as harsh overhead radiation with thin, washed-out sky
- nightside has stars, aurora curtains, and ambient planetary glow
- `DirectionalLight3D` direction is updated on every chunk transition
- terrain receives dramatically different light by zone — not one flat ambient

---

## Non-Goals

- full volumetric ray-marching (defer to a later pass)
- aurora mechanic integration (magnetism mechanic not yet designed)
- clouds or volumetric fog
- time-of-day progression (there is no time of day on Margin)
- changing the world geometry to represent planet curvature

---

## Architecture

### Sun direction update

`WorldEnvironmentController` already holds a reference to `_sun: DirectionalLight3D`.

Add a method `apply_sun_direction(light_level: float)`:

```gdscript
func apply_sun_direction(light_level: float) -> void:
    # south = +Z, north = -Z, up = +Y
    # sun rises from the southern (+Z) horizon as light_level increases
    # DirectionalLight3D shines in its local -Z direction
    # rotating by -elevation around X lifts the light source from horizon to zenith
    var elevation := light_level * PI / 2.0
    _sun.rotation = Vector3(-elevation, 0.0, 0.0)
    # resulting LIGHT0_DIRECTION in sky shader = (0, -sin(elevation), -cos(elevation))
    # resulting sun_dir (toward sun) = (0, sin(elevation), cos(elevation))
```

Call this from `apply_runtime_presentation()` using `average_light_level` from
the presentation dictionary.

The sky shader already reads `LIGHT0_DIRECTION` from the DirectionalLight3D automatically.
No shader parameter changes needed for sun direction.

### Sky shader rewrite

Replace the current `sky.gdshader` gradient approach with a scattering-based sky.

**LIGHT0_DIRECTION in Godot sky shaders**

In a Godot sky shader, `LIGHT0_DIRECTION` is the direction the light *travels*
(from source toward the scene). To get the direction toward the sun:

```glsl
vec3 sun_dir = -LIGHT0_DIRECTION; // direction from scene toward the sun
float sun_elev = dot(sun_dir, vec3(0.0, 1.0, 0.0)); // 0.0 = horizon, 1.0 = overhead
```

`TIME` is available for animation. `LIGHT0_SIZE` gives the sun's angular diameter
in radians and can be used for the sun disc instead of a hardcoded uniform.

**Inputs the shader needs:**

```glsl
// scattering profile (set by AtmosphereClass)
// scatter_coeffs must be a vec3 with B > G > R so blue light is stripped
// from the sun path faster than red at low sun angles — this is what produces
// horizon reddening. Default approximates an alien non-Earth atmosphere.
uniform vec3 scatter_coeffs = vec3(0.5, 1.2, 2.8);   // R, G, B relative strengths
uniform float rayleigh_strength = 1.0;
uniform float mie_strength = 1.0;
uniform float atmosphere_density = 1.0;
uniform vec3 mie_colour : source_color = vec3(0.98, 0.62, 0.28);
uniform float mie_g = 0.76; // Henyey-Greenstein anisotropy: 0=isotropic, 0.9=tight halo
                             // use ~0.76 for clear dayside, ~0.60 for hazy terminus

// nightside
uniform float night_blend = 0.0;         // 0 = day, 1 = full night
uniform float aurora_intensity = 0.0;    // 0 = none, 1 = full
uniform vec3 aurora_colour_a : source_color = vec3(0.10, 0.90, 0.30);  // oxygen green
uniform vec3 aurora_colour_b : source_color = vec3(0.72, 0.14, 0.86);  // high-altitude purple

// ground glow
uniform vec3 ground_glow_colour : source_color = vec3(0.08, 0.04, 0.06);
uniform float ground_glow_intensity = 0.4;
```

**Day sky computation (simplified single-scatter):**

The key to terminus reddening is computing two separate optical depths:
`view_optical` (how thick the atmosphere is along the view ray) and
`sun_optical` (how thick it is along the sun's path to the viewer). When the sun
is near the horizon `sun_optical` is large, and `exp(-sun_optical * scatter_coeffs)`
strips blue far more than red, reddening the sunlight before it scatters into the sky.

```glsl
float rayleigh_phase(float cos_theta) {
    return 0.75 * (1.0 + cos_theta * cos_theta);
}

float mie_phase(float cos_theta, float g) {
    float g2 = g * g;
    // clamp denominator to avoid div-by-zero at exact g=1
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 0.0001), 1.5);
}

void sky() {
    vec3 sun_dir = -LIGHT0_DIRECTION;
    float sun_elev = dot(sun_dir, vec3(0.0, 1.0, 0.0));
    float cos_theta = dot(EYEDIR, sun_dir);

    // Two optical depth paths — the mechanism that produces horizon reddening
    float view_optical = atmosphere_density / max(EYEDIR.y + 0.12, 0.04);
    float sun_optical  = atmosphere_density / max(sun_elev  + 0.12, 0.04);

    vec3 beta_r = scatter_coeffs * rayleigh_strength;

    // Sun light reddened by its path through atmosphere — more at low angles
    vec3 sun_transmit = exp(-sun_optical * beta_r * 0.01);

    float r_phase = rayleigh_phase(cos_theta);
    float m_phase = mie_phase(cos_theta, mie_g);

    // Scattered sky dome colour
    vec3 sky_col = r_phase * beta_r * sun_transmit * view_optical * 0.02;

    // Mie haze (glow around sun direction)
    sky_col += m_phase * mie_strength * mie_colour * sun_transmit * view_optical * 0.008;

    // Sun disc — only when sun is above horizon
    if (LIGHT0_ENABLED && sun_elev > -0.04) {
        // Use explicit 1.0 - smoothstep to avoid relying on inverted-edge GLSL behavior
        float angle_to_sun = acos(clamp(cos_theta, -1.0, 1.0));
        float disc = 1.0 - smoothstep(LIGHT0_SIZE * 0.35, LIGHT0_SIZE, angle_to_sun);
        sky_col += LIGHT0_COLOR * disc * 1.4 * sun_transmit;
    }

    // Night blend — applied above horizon only, before ground, so ground glow is preserved
    if (EYEDIR.y > 0.0) {
        vec3 night_sky = vec3(0.004, 0.002, 0.010);
        sky_col = mix(sky_col, night_sky, night_blend);
    }

    // Ground (below horizon) — applied last so it always shows regardless of night_blend
    float below = 1.0 - smoothstep(-0.25, 0.0, EYEDIR.y);
    sky_col = mix(sky_col, ground_glow_colour * ground_glow_intensity, below * 0.9);

    COLOR = sky_col;
}
```

Note: the scale factors (0.02, 0.008, 0.01) are tuning knobs that compensate for the
unitless `scatter_coeffs`. Codex should keep them as named constants so they can be
adjusted visually.

**Star field (nightside, in shader):**

Stars only appear above the horizon and fade in with `night_blend`.

```glsl
float star_field(vec3 dir) {
    if (dir.y < 0.0) return 0.0;
    vec3 d = floor(dir * 300.0);
    float h = fract(sin(dot(d, vec3(127.1, 311.7, 74.3))) * 43758.5);
    return step(0.997, h) * smoothstep(0.1, 0.5, night_blend);
}
```

**Aurora (nightside, in shader):**

Curtains are biased toward the north (-Z direction on Margin) since the nightside
is where aurora is strongest. `abs()` on the wave avoids hard cutoff edges.
`TIME` is available in Godot sky shaders.

```glsl
float aurora(vec3 dir, float t) {
    if (dir.y < 0.05) return 0.0;
    // northward bias: aurora fades toward dayside (+Z)
    float north_factor = smoothstep(0.2, -0.4, dir.z);
    float lat = dir.y;
    float lon = atan(dir.x, dir.z);
    float wave = abs(sin(lon * 3.0 + t * 0.28) * sin(lon * 7.1 - t * 0.13));
    float curtain = smoothstep(0.05, 0.45, lat) * smoothstep(0.85, 0.45, lat);
    return wave * curtain * north_factor;
}
```

Use a second slower wave to blend between `aurora_colour_a` and `aurora_colour_b`
for colour variation across the curtain.

**Ground glow:**

A soft upward-facing tinted light applied below the horizon, representing heat,
geothermal activity, or reflected dayside from the atmosphere. Set by profile.

### AtmosphereClass scattering profiles

**Extend** the existing profiles in `WorldEnvironmentController` — do not discard the
current fog, ambient, or sun colour values. Add scattering parameters alongside them.

`mie_g` is tunable per zone: higher g (≈0.85) = tighter sun corona on clear dayside;
lower g (≈0.55) = more diffuse haze on humid or frosty terminus.

| AtmosphereClass    | rayleigh_str | mie_str | atmos_density | mie_g | aurora_intensity_max |
|--------------------|--------------|---------|--------------|-------|----------------------|
| BlastedRadiance    | 0.6          | 1.4     | 1.4          | 0.85  | 0.0                  |
| HarshAmberHaze     | 0.8          | 1.2     | 1.2          | 0.80  | 0.0                  |
| DryTwilight        | 1.0          | 0.9     | 0.9          | 0.72  | 0.0                  |
| TemperateTwilight  | 1.1          | 1.0     | 1.0          | 0.70  | 0.05                 |
| WetTwilight        | 1.3          | 1.1     | 1.2          | 0.62  | 0.1                  |
| FrostTwilight      | 1.2          | 0.8     | 1.1          | 0.60  | 0.3                  |
| PolarGlow          | 1.0          | 0.6     | 0.9          | 0.58  | 0.6                  |
| BlackIceDark       | 0.7          | 0.5     | 0.8          | 0.55  | 0.85                 |
| GeothermalNight    | 0.6          | 0.9     | 1.0          | 0.65  | 0.5                  |

`aurora_intensity_max` is the class ceiling. Actual `aurora_intensity` is computed from
`light_level` at runtime (see WorldEnvironmentController changes). `night_blend` is not
in this table — it is always derived from `light_level`, never from the profile class.

Per-profile sun colour and energy for the DirectionalLight3D are retained from the
existing implementation. `scatter_coeffs` uses the same base `vec3(0.5, 1.2, 2.8)`
across all profiles — only the strength multipliers vary per zone.

### WorldEnvironmentController changes

**Extend** `_profile_for()` — do not remove anything from it. Add scattering uniforms
alongside the existing fog, ambient, and sun colour values.

Add two new responsibilities:

1. **`apply_sun_direction(light_level: float)`** — called every chunk transition, rotates
   the DirectionalLight3D
2. **Sky shader scattering uniforms** — set from the extended profile inside
   `apply_runtime_presentation()`

`night_blend` is always derived from `average_light_level` — it is not a fixed profile
value. The `aurora_intensity` column in the profile table is a per-class *maximum*; it
is scaled down toward zero as light_level increases:

```gdscript
# night_blend: 1.0 at light_level=0, 0.0 at light_level=0.4+
var night_blend := 1.0 - clamp(light_level * 2.5, 0.0, 1.0)
# aurora: profile_aurora_base is the maximum for this atmosphere class
var aurora_intensity := profile_aurora_base * (1.0 - clamp(light_level * 3.0, 0.0, 1.0))
_sky_material.set_shader_parameter("night_blend", night_blend)
_sky_material.set_shader_parameter("aurora_intensity", aurora_intensity)
```

---

## Terrain lighting changes

Once the DirectionalLight3D direction is dynamic, terrain will automatically receive
correctly angled direct light. No terrain shader changes are required in Phase 1.

However, the terrain shader should read shadow tint and ambient tint from the
`AtmosphereClass` profile to match. Ambient light colour and energy from
`Environment.ambient_light_color` and `Environment.ambient_light_energy` should
be set to match the scattering profile's expected ambient. This is already wired
in the environment controller — it just needs updating to the new profile structure.

---

## Implementation Phases

### Phase 1 — Sun direction + sky shader rewrite

**WorldEnvironmentController:**

- add `apply_sun_direction(light_level: float)`
- call it from `apply_runtime_presentation()` using `average_light_level`
- retain existing profile dispatch but update it to set scattering uniforms

**sky.gdshader:**

- replace gradient approach with Rayleigh + Mie phase function sky
- retain sun disc and glow, tracking DirectionalLight3D via `LIGHT0_DIRECTION`
- add `night_blend` uniform — sky fades to dark when nightside; stars and aurora deferred to Phase 2

**Outcome:** walking south raises the sun. The sky responds with real scattering.
Terminus reads as thick low-angle red-orange light. Dayside reads as harsh and washed.

### Phase 2 — Nightside sky

**sky.gdshader:**

- add procedural star field
- add aurora curtain animation using `aurora_intensity` and `aurora_colour_a/b`
- add ground glow (soft horizon uplight) for geothermal and nightside planetary rim

**WorldEnvironmentController:**

- compute `night_blend` and `aurora_intensity` from `light_level`
- set aurora colour pair from `AtmosphereClass` profile (PolarGlow and BlackIceDark
  get the most active aurora colours)

**Outcome:** nightside is alive. Stars appear. Aurora is visible in polar and nightside
zones. Terminus has a faint aurora hint.

### Phase 3 — Terrain lighting correctness

- ambient light colour and energy match the scattering profile
- shadow colour is driven by the same profile (cool blue terminus shadows, harsh
  orange dayside ambient)
- fog colour matches the scattering output near the horizon

**Outcome:** terrain lighting is consistent with the sky. The same physical logic
drives both.

---

## Acceptance Criteria

### Sun and sky

- walking south visibly raises the sun from the horizon toward overhead
- walking north lowers the sun toward the horizon and into night
- at `light_level ≈ 0.0` the sun is below the horizon; nightside sky is fully active
- at `light_level ≈ 1.0` the sun is near-overhead; dayside sky is fully active
- the terminus (light_level ≈ 0.3–0.5) reads as a permanent red-orange low-angle sky

### Scattering

- the sky does not use hard-authored colour gradients for the primary dome colour
- low sun angle produces visible redness and horizon glow from Rayleigh scattering
- high sun angle produces a brighter, harsher, less saturated sky
- the sun disc and glow track the actual DirectionalLight3D direction

### Nightside

- stars are visible when `night_blend > 0.4`
- aurora curtains are visible in PolarGlow, BlackIceDark, and GeothermalNight zones
- the nightside is not just black — a faint planetary rim glow and ground glow exist
- aurora may use green — classic oxygen-band aurora green is encouraged

### Architecture

- sun direction is never hardcoded — always derived from `average_light_level`
- sky shader receives scattering coefficients, not final colours
- `DirectionalLight3D` direction is updated at every chunk transition
- existing `AtmosphereClass` profile dispatch is preserved and extended, not replaced

---

## Risks

- simplified single-scatter may not produce convincing enough redness at very low angles
- aurora animation may flicker or alias at low `aurora_intensity` values
- scattering coefficients may need visual tuning per AtmosphereClass profile

## Mitigations

- test at the terminus first — that is the most demanding angle case
- tune aurora with a dedicated low `night_blend` test location at the polar boundary
- keep the profile table exposed so coefficients can be tuned without shader changes

---

## Modifies

```
assets/shaders/sky.gdshader                          (rewrite)
scripts/world/world_environment_controller.gd        (extend)
```

No Rust changes. No new files required.

---

## Codex Phase 1 Prompt

Using this spec as full context, implement **Phase 1 only**.

The goal: walking south raises the sun, walking north lowers it. The sky uses Rayleigh and
Mie scattering instead of a gradient. The terminus reads as thick red-orange low-angle light.

---

**sky.gdshader** (`assets/shaders/sky.gdshader`) — full rewrite.

Keep `shader_type sky;` and `render_mode disable_fog;` — both are required.

Remove all existing uniforms (zenith_color, upper_color, horizon_color, haze_color,
horizon_height, horizon_softness, sun_disc_size, sun_glow_size) and replace with the
uniforms defined in the Architecture section of this spec.

`scatter_coeffs` must be declared as `uniform vec3 scatter_coeffs` with NO `: source_color`
hint — it is a coefficient vec3, not a colour. Adding source_color would apply gamma
correction and produce wrong values.

Key implementation rules:
- `LIGHT0_DIRECTION` is the direction light travels (source→scene); direction toward sun
  is `vec3 sun_dir = -LIGHT0_DIRECTION`
- Compute two optical depths: `view_optical` and `sun_optical` separately — see Architecture
- `sun_transmit = exp(-sun_optical * beta_r * RAYLEIGH_SCALE)` is the mechanism that reddens
  the sun at low angles; without it there is no terminus redness
- Sun disc: use `1.0 - smoothstep(LIGHT0_SIZE * 0.35, LIGHT0_SIZE, angle_to_sun)` — do not
  use inverted smoothstep edges as that is undefined GLSL behaviour
- Sun disc only renders when `LIGHT0_ENABLED && sun_elev > -0.04`
- Night blend applies above horizon only — ground glow is applied last and must not be
  overridden by night_blend (see Architecture for correct ordering)
- `TIME` is available in sky shaders — not needed in Phase 1 but do not use it
- Stars and aurora are Phase 2 — add the `night_blend`, `aurora_intensity`,
  `aurora_colour_a`, `aurora_colour_b`, `ground_glow_colour`, `ground_glow_intensity`
  uniforms with correct defaults so Phase 2 can populate them, but do not implement the
  star or aurora logic yet

---

**WorldEnvironmentController** (`scripts/world/world_environment_controller.gd`):

Add `apply_sun_direction(light_level: float)`:
```gdscript
func apply_sun_direction(light_level: float) -> void:
    if _sun == null:
        return
    # rotation in radians; south = +Z; -X rotation lifts sun from southern horizon to zenith
    _sun.rotation = Vector3(-clamp(light_level, 0.0, 1.0) * PI / 2.0, 0.0, 0.0)
```

Call `apply_sun_direction` from `apply_runtime_presentation()` using:
```gdscript
var light_level := float(runtime_presentation.get("average_light_level", 0.0))
apply_sun_direction(light_level)
```

The key name `"average_light_level"` is confirmed from the Rust GDExtension source.

**Remove** these `set_shader_parameter` calls from `apply_runtime_presentation` — these
uniforms no longer exist in the rewritten shader and will cause runtime errors if called:
`"zenith_color"`, `"upper_color"`, `"horizon_color"`, `"haze_color"`, `"horizon_height"`,
`"horizon_softness"`, `"sun_disc_size"`, `"sun_glow_size"`

Do NOT remove the fog, ambient light, or `_sun` colour/energy assignments — those drive
`_environment` and `_sun` directly and are unrelated to the sky shader uniforms.

**Extend** `_profile_for()` by adding scattering keys to each existing branch dictionary —
do not create a separate match statement or lookup function. Each branch dict should gain:
`"rayleigh_strength"`, `"mie_strength"`, `"atmosphere_density"`, `"mie_g"` from the
profile table in the Architecture section.

After the profile is selected, set ALL new shader uniforms explicitly via
`set_shader_parameter`. Every uniform in the rewritten shader that is profile-driven must
have a corresponding call — the controller creates a fresh ShaderMaterial and shader
defaults are not guaranteed to propagate. Required calls to add:

```gdscript
_sky_material.set_shader_parameter("scatter_coeffs", Vector3(0.5, 1.2, 2.8))
_sky_material.set_shader_parameter("rayleigh_strength", float(profile["rayleigh_strength"]))
_sky_material.set_shader_parameter("mie_strength", float(profile["mie_strength"]))
_sky_material.set_shader_parameter("atmosphere_density", float(profile["atmosphere_density"]))
_sky_material.set_shader_parameter("mie_g", float(profile["mie_g"]))
_sky_material.set_shader_parameter("night_blend", night_blend)
_sky_material.set_shader_parameter("aurora_intensity", aurora_intensity)
```

Where `night_blend` and `aurora_intensity` are derived from `light_level` as:
```gdscript
var night_blend := 1.0 - clamp(light_level * 2.5, 0.0, 1.0)
var aurora_intensity := float(profile.get("aurora_intensity_max", 0.0)) * (1.0 - clamp(light_level * 3.0, 0.0, 1.0))
```

Do not modify `world.gd` or any Rust code. Do not implement stars or aurora logic.

The terminus is the most demanding test case. At `average_light_level ≈ 0.1` the sun
should sit just above the southern (+Z) horizon and the sky should read red-orange.
At `average_light_level ≈ 1.0` the sun is near-overhead and the sky is harsher and
less saturated.

## Codex Phase 2 Prompt

Using this spec as full context, implement **Phase 2 only**.

Phase 1 is complete. The sun moves with player position. The sky uses Rayleigh and Mie
scattering. `night_blend` is already wired and working.

**sky.gdshader** additions:

- procedural star field using a hash on `floor(EYEDIR * 300.0)` — stars only above horizon
  (`EYEDIR.y > 0.0`), fade in with `smoothstep(0.1, 0.5, night_blend)`
- animated aurora curtains: use `abs()` on layered sinusoidal waves to avoid hard cutoff
  edges; bias curtains toward the north (-Z direction on Margin) using
  `smoothstep(0.2, -0.4, EYEDIR.z)` as a north_factor multiplier; curtains appear
  between `lat = 0.05` and `lat = 0.85` (lat = EYEDIR.y); scaled by `aurora_intensity`
- colour the aurora by blending between `aurora_colour_a` (oxygen green) and
  `aurora_colour_b` (high-altitude purple) using a second slower wave on longitude
- ground glow below the horizon using `ground_glow_colour` and `ground_glow_intensity`
  — applied last so it is not overridden by night_blend
- `TIME` is available in Godot sky shaders for animation

**New uniforms to add to sky.gdshader:**
`aurora_intensity`, `aurora_colour_a`, `aurora_colour_b`, `ground_glow_colour`,
`ground_glow_intensity`

**WorldEnvironmentController** additions:

- compute `aurora_intensity` from `aurora_intensity_max` (from profile) scaled by
  `1.0 - clamp(light_level * 3.0, 0.0, 1.0)` — already described in spec
- set `aurora_colour_a` and `aurora_colour_b` from profile; PolarGlow and BlackIceDark
  get the most saturated aurora colours
- set `ground_glow_colour` from profile — nightside and geothermal zones get warmer glows

aurora green (oxygen band) is the primary aurora colour — lean into it.
