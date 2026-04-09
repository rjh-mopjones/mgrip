# Spec 009 — Humidity Terminator Ring Fix

**Status:** Proposed
**Priority:** High
**Depends On:** Spec 007 (world-anchored noise tiers)

## Problem

The humidity layer is visually flat — nearly uniform dark blue across the entire
macro map. On a tidally locked planet, humidity is the primary climate driver:
saturated at the terminator convergence zone, bone dry on the scorched dayside,
frozen out on the nightside. The current output does not express this.

Downstream consequences of broken humidity:

- biome classification collapses (everything trends toward the same moisture
  class)
- aridity layer is too uniform (derived from humidity)
- river density is uncontrolled (rivers flow through deserts)
- vegetation density has no clear zone differentiation
- erosion lacks moisture-driven shaping
- temperature moderation is flat (humidity buffers temperature by up to +5°C)

Fixing humidity is the single highest-leverage terrain generation change because
every downstream derived layer depends on it.

## Current Reality

### What exists

The `generate_terminator_model()` function in `humidity.rs` implements a
physically motivated atmospheric model with four components:

1. **Terminator peak** — Gaussian centered at `light_level = 0.2` with σ = 0.22
2. **Day-side drying** — quadratic reduction for `light > 0.4`
3. **Night-side cold trap** — linear ramp for `light < 0.15`
4. **Continental moisture decay** — piecewise linear from ocean (1.0) to deep
   inland (0.1)

Final combination: `0.2 × base_noise + 0.3 × scaled_moisture + 0.5 × atmospheric`

### Why the output is flat

**Root cause 1 — GPU replaces the terminator model with a weaker version.**

When GPU is available (the default on macOS), `biome_map.rs:300` directly
assigns GPU humidity values. The CPU `generate_terminator_model()` is never
called.

The GPU humidity shader (`pipelines.rs:359-400`) computes its own light_level
inline with critical simplifications:

- **No domain warping** — CPU uses two-pass Perlin warp (±12% and ±6% in
  normalized coords) to create irregular climate boundaries. GPU uses clean
  radial distance. This removes the spatial texture that makes the terminator
  feel real.
- **No scatter noise** — CPU adds ±5% fBm variation to light. GPU omits this
  entirely.

The resulting GPU light field is smoother and more symmetric, producing a weaker,
broader Gaussian peak.

**Root cause 2 — The Gaussian is too wide.**

σ = 0.22 on a light range of [0, 1] means the half-max width extends from
light ≈ 0.0 to light ≈ 0.4. The "peak" is really a broad plateau covering most
of the map. A humidity ring should be a visible ring, not a vague bias.

**Root cause 3 — Night-side suppression is too weak.**

At `light = 0.05`, `night_trap = 0.433`. Combined with a still-high
`terminator_peak ≈ 0.79`, the deep nightside retains atmospheric humidity of
~0.34 before moisture source weighting. The nightside should be approaching zero
atmospheric moisture — it's all frozen out.

**Root cause 4 — Day-side drying starts too late.**

The quadratic drying only triggers at `light > 0.4`, which is well past the
terminator. In reality, evaporation on a tidally locked planet is extreme even
at moderate illumination. The transition from humid terminator to dry dayside
should be much sharper.

**Root cause 5 — Base noise dilutes the signal.**

20% weight to raw fBm adds uniform 0.0–1.0 noise everywhere, washing out the
structured atmospheric signal. On a planet where humidity is dominated by
stellar physics, geological noise should be a minor perturbation.

## Goals

1. Make the humidity map show a clear, visible ring at the terminator.
2. Drop humidity toward zero on the deep nightside and deep dayside.
3. Ensure continental interiors are drier than coasts within the habitable band.
4. Bring GPU and CPU humidity paths into parity.
5. Preserve the terminator model's physical motivation while tuning its
   parameters.
6. Validate by regenerating the macro map and visually inspecting humidity.png,
   biome.png, and aridity.png.

## Non-Goals

- Reworking the biome classification splines (separate concern).
- Adding new derived layers.
- Modifying the river generation algorithm (though rivers should naturally
  improve when humidity is correct).
- Changing the macro/runtime tier split from Spec 007.

## Design

### Section 1 — Tighten the Gaussian

Narrow σ from 0.22 to 0.12. This makes the half-max band approximately
`light ∈ [0.08, 0.32]` instead of `[0.0, 0.4]` — a visible ring instead of a
broad plateau.

The peak center stays at `light = 0.2`. This corresponds to the atmospheric
convergence zone where updraft maximizes condensation, which is correct for a
tidally locked planet.

### Section 2 — Strengthen night-side suppression

Replace the linear night trap with a steeper curve:

```
if light < 0.1:
    night_trap = light / 0.1  // linear 0→1 over a narrower band
```

At `light = 0.05` this gives `night_trap = 0.5` (down from 0.433). At
`light = 0.01` this gives `night_trap = 0.1` (down from 0.21). Deep night is
now truly dry.

The threshold drops from 0.15 to 0.1 — the cold trap kicks in closer to the
terminator, matching the expectation that atmospheric moisture freezes out fast
in darkness.

### Section 3 — Bring day-side drying inward

Lower the drying onset from `light > 0.4` to `light > 0.3`:

```
if light > 0.3:
    let t = (light - 0.3) / 0.7;
    day_drying = 1.0 - t * t * 0.9  // stronger: 90% reduction at sub-stellar
```

This tightens the humid band and makes the transition from terminator to desert
more dramatic.

### Section 4 — Reduce base noise weight

Change the mix from `0.2/0.3/0.5` to `0.08/0.32/0.60`:

```
humidity = base_noise * 0.08 + scaled_moisture * 0.32 + atmospheric * 0.60
```

The atmospheric signal now dominates at 60%. Base noise becomes texture, not
structure.

### Section 5 — GPU parity

Add domain warping to the GPU humidity shader's light_level computation. Port
the CPU's two-pass warp logic:

```wgsl
// Pass 1: large-scale zone boundary warp
let warp1_x = fbm(wx * 0.0015, wy * 0.0015 + 50.0, ...) * 0.12;
let warp1_y = fbm(wx * 0.0015 + 150.0, wy * 0.0015, ...) * 0.12;
// Pass 2: smaller detail warp
let warp2_x = fbm(wx * 0.005, wy * 0.005 + 100.0, ...) * 0.06;
let warp2_y = fbm(wx * 0.005 + 200.0, wy * 0.005, ...) * 0.06;
```

This is the critical fix. Without domain warping, the GPU produces a perfectly
symmetric radial light field which the Gaussian smooths into mush. The warp
creates irregular climate zone boundaries that make the terminator ring feel
geographically real.

Also add scatter noise (±5% fBm variation on the final light value) to match
the CPU path.

### Section 6 — Visualization improvement

The current `humidity_to_rgba` ramp goes from dark navy (humidity=0) to medium
blue (humidity=1). This is hard to read. Add a wider color ramp:

```
humidity = 0.0  → dark brown    (arid)
humidity = 0.3  → tan/yellow    (dry)
humidity = 0.5  → green         (moderate)
humidity = 0.7  → teal          (humid)
humidity = 1.0  → bright blue   (saturated)
```

This makes the humidity map immediately readable as a climate map.

## Modifies

### Rust files

```
gdextension/crates/mg_noise/src/strategy/humidity.rs
gdextension/crates/mg_noise/src/gpu/pipelines.rs
gdextension/crates/mg_noise/src/visualization.rs
```

### No changes to

```
biome_map.rs          (pipeline stays the same, just parameters change)
biome_splines.rs      (biome classification unchanged)
derived/mod.rs        (derived layer formulas unchanged)
generation_manager.gd (Godot side unchanged)
```

## Verification

1. **Rebuild the Rust extension:**
   ```sh
   cargo build --release --manifest-path gdextension/Cargo.toml
   ```

2. **Regenerate the macro map:**
   ```sh
   margins_grip generate layers <seed> v7
   ```

3. **Visual inspection of humidity.png:**
   - There should be a clear bright ring at the terminator
   - Dayside should be dark/brown (dry)
   - Nightside should be dark/brown (frozen out)
   - Coastal areas within the ring should be brighter than continental interiors
   - The ring should have irregular edges (from domain warping), not a clean arc

4. **Visual inspection of biome.png:**
   - The habitable band should show more biome diversity
   - Desert/scorched biomes should dominate the dayside more cleanly
   - Frozen biomes should dominate the nightside more cleanly
   - The terminator band should show forests, meadows, and wet biomes

5. **Visual inspection of aridity.png:**
   - Should show inverse of humidity — dry everywhere except the terminator ring

6. **Compare generation check:**
   ```sh
   margins_grip compare-scale <seed> 440 220 4 /tmp/compare_v7
   ```
   - Macro and runtime biomes should still agree on ocean/land
   - Biome diversity within the habitable band should increase

7. **Runtime spot-check:**
   ```sh
   godot --path . -- --flythrough
   ```
   - Terrain in the habitable band should feel different from terrain on the
     dayside or nightside

## Risks

- Tightening the Gaussian may create a humidity "cliff" that produces sharp
  biome boundaries. If this happens, add a small amount of domain warping to
  the Gaussian center itself (perturb `light_level - 0.2` by ±0.03).
- GPU domain warping adds compute cost to the humidity shader. Monitor
  generation time — the two fBm evaluations for warp are low-octave (2-3) so
  impact should be small.
- Downstream layers that were tuned to the old flat humidity may need threshold
  adjustments. Check temperature moderation, erosion, and water table after the
  fix.
- The biome spline moisture thresholds (Arid < 0.2 < Dry < 0.4 < Moderate <
  0.6 < Humid < 0.8 < Saturated) were calibrated against the old range. If the
  new humidity has a different distribution, some thresholds may need shifting.

## Success Criteria

- Humidity.png shows a visible, irregular ring at the terminator
- The ring is narrow enough that most of the planet is clearly dry or frozen
- Biome.png shows increased diversity in the habitable band
- GPU and CPU paths produce visually similar humidity output
- All existing smoke tests still pass
- No macro/runtime ocean-land agreement regression
