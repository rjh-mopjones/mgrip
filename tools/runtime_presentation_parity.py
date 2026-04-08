#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import agent_runtime_bridge_runner as bridge_runner


REPO_ROOT = Path(__file__).resolve().parents[1]
CLI_MANIFEST_PATH = REPO_ROOT / "gdextension/Cargo.toml"
CLI_BINARY_PATH = REPO_ROOT / "gdextension/target/release/margins_grip"
FLOAT_KEYS = (
    "interestingness_score",
    "average_light_level",
    "average_temperature",
    "average_humidity",
    "average_aridity",
    "average_snowpack",
    "average_water_table",
)
ENUM_KEYS = (
    "planet_zone",
    "atmosphere_class",
    "water_state",
    "landform_class",
    "surface_palette_class",
)
GRID_KEYS = (
    "water_state_grid",
    "landform_grid",
    "surface_palette_grid",
)


def main() -> int:
    args = parse_args()
    bridge_root = Path(args.bridge_root).expanduser()
    launched: bridge_runner.LaunchedGodot | None = None

    try:
        ensure_cli_binary()
        if not args.attach:
            shutil.rmtree(bridge_root, ignore_errors=True)
            launched = bridge_runner.launch_godot(args, bridge_root)

        state = bridge_runner.wait_for_state(
            bridge_root, args.timeout_seconds, require_runtime=True
        )
        print(
            f"Bridge ready: runtime_id={state['runtime_id']} runtime_available={state['runtime_available']}"
        )

        observation = fetch_observation(bridge_root, args.timeout_seconds)
        seed = int(observation["world_seed"])
        current_chunk = observation["current_chunk"]
        world_units_per_chunk = float(
            observation["runtime_constants"]["world_units_per_chunk"]
        )
        world_x = float(current_chunk["x"]) * world_units_per_chunk
        world_y = float(current_chunk["y"]) * world_units_per_chunk

        runtime_summary = normalize_runtime_presentation(
            observation.get("runtime_presentation", {})
        )
        offline_summary = compute_offline_chunk_presentation(seed, world_x, world_y)
        mismatches = diff_presentations(
            runtime_summary, offline_summary, tolerance=args.float_tolerance
        )

        print(
            json.dumps(
                {
                    "ok": not mismatches,
                    "seed": seed,
                    "chunk_coord": current_chunk,
                    "world_origin": {"x": world_x, "y": world_y},
                    "runtime_summary": runtime_summary,
                    "offline_summary": offline_summary,
                    "mismatches": mismatches,
                },
                indent=2,
            )
        )
        return 0 if not mismatches else 1
    except (bridge_runner.BridgeError, subprocess.CalledProcessError, json.JSONDecodeError) as exc:
        print(f"runtime presentation parity check failed: {exc}", file=sys.stderr)
        return 1
    finally:
        if launched is not None:
            bridge_runner.terminate_process(launched)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare the current Godot runtime chunk presentation with the offline Rust-generated summary."
    )
    parser.add_argument(
        "--godot-bin",
        default=str(bridge_runner.DEFAULT_GODOT_BIN),
        help="Path to the Godot executable.",
    )
    parser.add_argument(
        "--bridge-root",
        default=str(bridge_runner.default_bridge_root()),
        help="Path to the agent-runtime bridge directory.",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=45.0,
        help="Timeout for building, launching, and bridge requests.",
    )
    parser.add_argument(
        "--float-tolerance",
        type=float,
        default=1e-4,
        help="Tolerance used when comparing floating-point averages.",
    )
    parser.add_argument(
        "--attach",
        action="store_true",
        help="Attach to an already running Godot process instead of launching one.",
    )
    parser.add_argument(
        "--windowed",
        action="store_true",
        help="Run with the normal display driver instead of headless mode.",
    )
    return parser.parse_args()


def ensure_cli_binary() -> None:
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--manifest-path",
            str(CLI_MANIFEST_PATH),
        ],
        cwd=REPO_ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def fetch_observation(bridge_root: Path, timeout_seconds: float) -> dict[str, Any]:
    response = bridge_runner.request(
        bridge_root,
        "get_observation",
        {"options": {"debug": True}},
        timeout_seconds,
    )
    payload = bridge_runner.response_payload(response)
    return payload["observation"]


def compute_offline_chunk_presentation(seed: int, world_x: float, world_y: float) -> dict[str, Any]:
    result = subprocess.run(
        [
            str(CLI_BINARY_PATH),
            "inspect",
            "chunk-presentation",
            str(seed),
            f"{world_x}",
            f"{world_y}",
            "--format",
            "json",
        ],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def normalize_runtime_presentation(runtime_presentation: dict[str, Any]) -> dict[str, Any]:
    normalized = {
        "planet_zone": enum_name(runtime_presentation, "planet_zone"),
        "atmosphere_class": enum_name(runtime_presentation, "atmosphere_class"),
        "water_state": enum_name(runtime_presentation, "water_state"),
        "landform_class": enum_name(runtime_presentation, "landform_class"),
        "surface_palette_class": enum_name(runtime_presentation, "surface_palette_class"),
        "reduced_grids": normalize_reduced_grids(runtime_presentation.get("reduced_grids", {})),
    }
    for key in FLOAT_KEYS:
        normalized[key] = float(runtime_presentation.get(key, 0.0))
    return normalized


def normalize_reduced_grids(reduced_grids: dict[str, Any]) -> dict[str, Any]:
    normalized: dict[str, Any] = {}
    for key in GRID_KEYS:
        grid = reduced_grids.get(key, {})
        normalized[key] = {
            "width": int(grid.get("width", 0)),
            "height": int(grid.get("height", 0)),
            "digest": str(grid.get("digest", "")),
        }
    return normalized


def enum_name(runtime_presentation: dict[str, Any], key: str) -> str:
    value = runtime_presentation.get(key, {})
    if isinstance(value, dict):
        return str(value.get("name", ""))
    return str(value)


def diff_presentations(
    runtime_summary: dict[str, Any],
    offline_summary: dict[str, Any],
    tolerance: float,
) -> list[str]:
    mismatches: list[str] = []
    for key in ENUM_KEYS:
        if str(runtime_summary.get(key, "")) != str(offline_summary.get(key, "")):
            mismatches.append(
                f"{key} mismatch: runtime={runtime_summary.get(key)!r} offline={offline_summary.get(key)!r}"
            )
    for key in FLOAT_KEYS:
        runtime_value = float(runtime_summary.get(key, 0.0))
        offline_value = float(offline_summary.get(key, 0.0))
        if abs(runtime_value - offline_value) > tolerance:
            mismatches.append(
                f"{key} mismatch: runtime={runtime_value:.6f} offline={offline_value:.6f}"
            )
    runtime_grids = runtime_summary.get("reduced_grids", {})
    offline_grids = offline_summary.get("reduced_grids", {})
    for key in GRID_KEYS:
        runtime_grid = runtime_grids.get(key, {})
        offline_grid = offline_grids.get(key, {})
        if runtime_grid != offline_grid:
            mismatches.append(
                f"{key} mismatch: runtime={runtime_grid!r} offline={offline_grid!r}"
            )
    return mismatches


if __name__ == "__main__":
    raise SystemExit(main())
