#!/usr/bin/env python3
"""Smoke test for flying and swimming state machine in fps_controller.gd.

Uses the agent-runtime file bridge to:
  1. Run the existing cross-chunk smoke test (verifies walking + scripted motion).
  2. Teleport the player to a water block and verify swimming behaviour
     (buoyancy prevents free-fall with full gravity).

Usage:
  python3 tools/test_fly_swim.py [--windowed] [--timeout-seconds 45]
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

# Re-use the bridge runner helpers.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_runtime_bridge_runner import (
    BridgeError,
    default_bridge_root,
    launch_godot,
    request,
    response_payload,
    run_step,
    terminate_process,
    wait_for_state,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
SEA_LEVEL_Y = -2  # Must match VoxelMeshBuilder.SEA_LEVEL_Y


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Smoke-test fly/swim state machine via agent bridge."
    )
    parser.add_argument(
        "--godot-bin",
        default=os.environ.get(
            "GODOT_BIN", "/Applications/Godot.app/Contents/MacOS/Godot"
        ),
    )
    parser.add_argument("--bridge-root", default=str(default_bridge_root()))
    parser.add_argument("--timeout-seconds", type=float, default=45.0)
    parser.add_argument("--windowed", action="store_true")
    args = parser.parse_args()

    bridge_root = Path(args.bridge_root).expanduser()
    launched = None
    passed = 0
    failed = 0

    try:
        import shutil

        shutil.rmtree(bridge_root, ignore_errors=True)
        launched = launch_godot(args, bridge_root)
        state = wait_for_state(bridge_root, args.timeout_seconds, require_runtime=True)
        print(f"Bridge ready: runtime_id={state['runtime_id']}")

        # ── Test 1: Basic walking + scripted motion ─────────────────────
        print("\n=== TEST 1: Walking + cross-chunk movement ===")
        try:
            _test_walking(bridge_root, args.timeout_seconds)
            print("PASS: Walking and scripted motion work correctly.")
            passed += 1
        except (BridgeError, AssertionError) as exc:
            print(f"FAIL: Walking test — {exc}")
            failed += 1

        # ── Test 2: Swimming (teleport to water) ───────────────────────
        print("\n=== TEST 2: Swimming (teleport into water) ===")
        try:
            result = _test_swimming(bridge_root, args.timeout_seconds)
            if result == "skip":
                print("SKIP: No water block found near spawn.")
            else:
                print("PASS: Swimming behaviour verified.")
                passed += 1
        except (BridgeError, AssertionError) as exc:
            print(f"FAIL: Swimming test — {exc}")
            failed += 1

        print(f"\n{'=' * 50}")
        print(f"Results: {passed} passed, {failed} failed")
        return 0 if failed == 0 else 1

    except BridgeError as exc:
        print(f"Bridge setup failed: {exc}", file=sys.stderr)
        return 1
    finally:
        if launched is not None:
            terminate_process(launched)


def _test_walking(bridge_root: Path, timeout: float) -> None:
    """Start a session, teleport, move across chunk, verify arrival."""
    start_payload = response_payload(
        request(
            bridge_root,
            "start_session",
            {
                "goal_label": "fly_swim_walk_smoke",
                "metadata": {
                    "scenario": "fly_swim_walk_smoke",
                    "runner": "tools/test_fly_swim.py",
                },
            },
            timeout,
        )
    )
    obs = start_payload["observation"]
    assert obs["runtime_available"], "Runtime not available"

    blocks_per_chunk = int(obs["runtime_constants"]["blocks_per_chunk"])
    anchor = obs["anchor_chunk"]
    current = obs["current_chunk"]
    origin_x = (int(current["x"]) - int(anchor["x"])) * blocks_per_chunk
    origin_z = (int(current["y"]) - int(anchor["y"])) * blocks_per_chunk

    # Find land, teleport, move a short distance (8 blocks) to verify movement works.
    center = {
        "x": origin_x + blocks_per_chunk // 2,
        "z": origin_z + blocks_per_chunk // 2,
    }
    land_step = run_step(
        bridge_root, "find_nearest_land", {"scene_block": center}, timeout
    )
    land = land_step["result"]["data"]["nearest_land_scene_block"]

    run_step(bridge_root, "teleport_to_block", {"scene_block": land}, timeout)
    run_step(bridge_root, "wait_for_player_settled", {"timeout_seconds": 5.0}, timeout)

    target = {"x": land["x"] + 8, "z": land["z"]}
    target_land = run_step(
        bridge_root, "find_nearest_land", {"scene_block": target}, timeout
    )
    move_target = target_land["result"]["data"]["nearest_land_scene_block"]

    move_step = run_step(
        bridge_root,
        "move_to_block",
        {
            "scene_block": move_target,
            "speed": 12.0,
            "arrival_radius": 3.0,
            "timeout_seconds": 15.0,
        },
        timeout,
    )

    move_status = move_step["result"]["status"]
    assert move_status == "completed", f"move_to_block returned {move_status}"

    run_step(bridge_root, "wait_for_player_settled", {"timeout_seconds": 3.0}, timeout)

    end_payload = response_payload(
        request(bridge_root, "end_session", {"reason": "walk smoke done"}, timeout)
    )
    assert end_payload["result"]["status"] == "completed"


def _test_swimming(bridge_root: Path, timeout: float) -> str:
    """Start a session, find a water block, teleport there, verify player doesn't free-fall.

    Returns "pass" or "skip".
    """
    start_payload = response_payload(
        request(
            bridge_root,
            "start_session",
            {
                "goal_label": "fly_swim_water_smoke",
                "metadata": {
                    "scenario": "fly_swim_water_smoke",
                    "runner": "tools/test_fly_swim.py",
                },
            },
            timeout,
        )
    )
    obs = start_payload["observation"]
    assert obs["runtime_available"], "Runtime not available"

    blocks_per_chunk = int(obs["runtime_constants"]["blocks_per_chunk"])
    anchor = obs["anchor_chunk"]
    current = obs["current_chunk"]
    origin_x = (int(current["x"]) - int(anchor["x"])) * blocks_per_chunk
    origin_z = (int(current["y"]) - int(anchor["y"])) * blocks_per_chunk
    cx = origin_x + blocks_per_chunk // 2
    cz = origin_z + blocks_per_chunk // 2

    # Grid search for a water block (terrain height below sea level).
    water_block = None
    for ring in range(0, 384, 32):
        if water_block is not None:
            break
        for dx in range(-ring, ring + 1, 32):
            for dz in [-ring, ring] if ring > 0 else [0]:
                step = run_step(
                    bridge_root,
                    "sample_height",
                    {"scene_block": {"x": cx + dx, "z": cz + dz}},
                    timeout,
                )
                height = float(step["result"]["data"]["height"])
                if height < SEA_LEVEL_Y:
                    water_block = {"x": cx + dx, "z": cz + dz}
                    print(
                        f"  Found water at ({cx + dx}, {cz + dz}) height={height:.1f}"
                    )
                    break

    if water_block is None:
        end_payload = response_payload(
            request(bridge_root, "end_session", {"reason": "no water found"}, timeout)
        )
        return "skip"

    # Teleport player to the water block.
    run_step(bridge_root, "teleport_to_block", {"scene_block": water_block}, timeout)

    # Wait a moment for physics to settle.
    run_step(bridge_root, "wait_seconds", {"duration_seconds": 1.5}, timeout)

    # Sample player position — if swimming works, player should NOT have fallen
    # far below sea level (buoyancy + reduced gravity should keep them near surface).
    # With normal gravity (25.0) over 1.5s the player would fall ~28 units.
    # With swim physics (gravity 5.0 - buoyancy 3.0 = net 2.0) they'd fall ~2.25 units.
    obs_after = run_step(
        bridge_root, "wait_for_player_settled", {"timeout_seconds": 2.0}, timeout
    )
    player_y = obs_after["observation"]["player_position"]["y"]
    fall_distance = SEA_LEVEL_Y - player_y

    print(
        f"  Player Y after 1.5s in water: {player_y:.2f}  (fall from sea level: {fall_distance:.2f})"
    )

    # With swimming buoyancy the player should be within ~5 units of sea level.
    # Without swimming (full gravity), they'd be ~28+ units below.
    assert fall_distance < 10.0, (
        f"Player fell {fall_distance:.1f} units below sea level — "
        f"swimming buoyancy may not be working (expected < 10)"
    )

    end_payload = response_payload(
        request(bridge_root, "end_session", {"reason": "swim smoke done"}, timeout)
    )
    assert end_payload["result"]["status"] == "completed"
    return "pass"


if __name__ == "__main__":
    raise SystemExit(main())
