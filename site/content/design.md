# Design

[Erosion sandbox](/sandbox/): land grown from uplift and cut by rivers on the real world, with the parameters on sliders (spec 013).

## Approach

| Principle | What it means |
|---|---|
| Light, not latitude | Temperature derives from light level and altitude. No equator, no seasons. |
| One generator | The world map and the streamed terrain come from the same Rust code, so they agree. |
| Rivers follow rules | Only where precipitation exceeds evaporation. Follow valleys, merge downstream, end at the sea. |
| Agent playtesting | A headless agent moves through the world and reports observations. Used for regression checks. |
| Deterministic history | World state at any time is computed from seed and time. No background simulation. Not built. |
| Event-log saves | A save is seed, clock and player actions. Not built. |
