#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path
from typing import Any

import agent_runtime_bridge_runner as bridge_runner


REPO_ROOT = Path(__file__).resolve().parents[1]
RUNTIME_PRESENTATION_BIN = REPO_ROOT / "gdextension/target/release/margins_grip"
VISUAL_AUDIT_ENV = "MG_AGENT_RUNTIME_WORLD_ORIGIN"
VISUAL_AUDIT_RUNTIME_ENV = "MG_RUNTIME_VISUAL_AUDIT"
DEFAULT_OUTPUT_DIR = Path("/tmp/mgrip_runtime_presentation_visual_audit")
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
WORLD_SCAN_STEP = 64
WORLD_SCAN_WIDTH = 1024
WORLD_SCAN_HEIGHT = 512
REFINE_SCAN_STEP = 32
REFINE_SCAN_RADIUS = 128
NEIGHBOR_SAMPLE_OFFSETS = (
    (-24, 0),
    (24, 0),
    (0, -24),
    (0, 24),
    (-18, -18),
    (18, 18),
)
ORBIT_CAMERA_OFFSETS = (
    (-144, -96),
    (-144, -32),
    (-144, 32),
    (-144, 96),
    (-96, -144),
    (-96, 144),
    (-48, -168),
    (-48, 168),
)
CAMERA_HEIGHT_OFFSETS_BY_ZONE = {
    "AbyssalNight": 20.0,
    "DeepNightIce": 22.0,
    "OuterTerminus": 10.0,
    "DryDaysideMargin": 11.0,
}

AUDIT_CASES = (
    {
        "name": "abyssal_night",
        "world_origin": (0.0, 0.0),
        "focus_probes": [
            (96, 24),
            (124, 48),
            (148, 0),
            (132, 72),
            (172, -20),
        ],
        "expected_zone": "AbyssalNight",
    },
    {
        "name": "deep_night_ice",
        "world_origin": (256.0, 0.0),
        "preferred_water_states": ["FrozenSea", "IceSheet"],
        "preferred_palettes": ["BlueIce"],
        "focus_probes": [
            (96, 20),
            (132, 40),
            (164, 0),
            (144, 64),
            (188, -12),
        ],
        "expected_zone": "DeepNightIce",
    },
    {
        "name": "outer_terminus",
        "world_origin": (256.0, 192.0),
        "preferred_landforms": ["RiverCutLowland", "CoastShelf"],
        "preferred_water_states": ["MeltwaterChannel", "LiquidCoast", "MarshWater"],
        "focus_probes": [
            (92, 36),
            (124, 54),
            (148, 18),
            (110, 84),
            (170, 0),
        ],
        "expected_zone": "OuterTerminus",
    },
    {
        "name": "dry_dayside_margin",
        "world_origin": (896.0, 352.0),
        "preferred_landforms": ["DuneWaste", "Basin", "Badlands", "FractureBelt"],
        "preferred_palettes": ["ScorchedStone", "SaltCrust", "IronOxideHighland"],
        "focus_probes": [
            (96, 0),
            (132, 18),
            (156, 52),
            (118, -24),
            (184, 12),
            (160, 64),
            (128, 96),
        ],
        "expected_zone": "DryDaysideMargin",
    },
    {
        "name": "salt_basin",
        "world_origin": (960.0, 384.0),
        "preferred_landforms": ["Basin"],
        "preferred_water_states": ["EvaporiteBasin"],
        "preferred_palettes": ["SaltCrust"],
        "focus_probes": [
            (108, -18),
            (136, 0),
            (124, 36),
            (152, 54),
            (176, -26),
        ],
        "expected_zone": "DryDaysideMargin",
    },
)

MAX_BRIGHT_GREEN_RATIO = 0.0035
MAX_GREEN_DRIFT_RATIO = 0.018
MIN_CAPTURE_SET_LUMINANCE_RANGE = 0.20
MIN_PEAK_LUMINANCE_STDDEV = 0.18
MIN_MEDIAN_PAIRWISE_MEAN_COLOR_DISTANCE = 0.20
MIN_WARMTH_DELTA = 0.080


class AuditError(RuntimeError):
    pass


_WORLD_SCAN_CACHE: dict[tuple[int, int], list[dict[str, Any]]] = {}


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir.expanduser()
    output_dir.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(output_dir / "screenshots", ignore_errors=True)
    report_path = output_dir / "visual_audit_report.json"
    if report_path.exists():
        report_path.unlink()

    try:
        captures = []
        for case in AUDIT_CASES:
            print(
                f"Capturing visual audit case {case['name']} at world origin {case['world_origin']}",
                flush=True,
            )
            captures.append(run_case(case, args, output_dir))
        report = build_report(captures)
        report_path.write_text(json.dumps(report, indent=2))
        print(json.dumps(report, indent=2), flush=True)
        print(f"\nVisual audit report: {report_path}", flush=True)
        return 0 if report["ok"] else 1
    except (AuditError, bridge_runner.BridgeError, subprocess.CalledProcessError) as exc:
        print(f"runtime presentation visual audit failed: {exc}", file=sys.stderr)
        return 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Capture representative runtime screenshots and audit their visual presentation."
    )
    parser.add_argument(
        "--godot-bin",
        default=str(bridge_runner.DEFAULT_GODOT_BIN),
        help="Path to the Godot executable.",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=60.0,
        help="Timeout for each launch and bridge request.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help="Directory where screenshots and the audit report will be written.",
    )
    return parser.parse_args()


def run_case(case: dict[str, Any], args: argparse.Namespace, output_dir: Path) -> dict[str, Any]:
    case_name = str(case["name"])
    selected_origin = select_case_world_origin(case)
    world_x = float(selected_origin["world_x"])
    world_y = float(selected_origin["world_y"])
    case_bridge_root = output_dir / f"bridge_{case_name}"
    screenshot_dir = output_dir / "screenshots"
    screenshot_dir.mkdir(parents=True, exist_ok=True)
    launched: bridge_runner.LaunchedGodot | None = None
    previous_override = os.environ.get(VISUAL_AUDIT_ENV)
    previous_runtime_flag = os.environ.get(VISUAL_AUDIT_RUNTIME_ENV)
    os.environ[VISUAL_AUDIT_ENV] = f"{world_x},{world_y}"
    os.environ[VISUAL_AUDIT_RUNTIME_ENV] = "1"

    try:
        shutil.rmtree(case_bridge_root, ignore_errors=True)
        launch_args = argparse.Namespace(
            godot_bin=args.godot_bin,
            bridge_root=str(case_bridge_root),
            timeout_seconds=args.timeout_seconds,
            attach=False,
            wait_for_runtime=True,
            windowed=True,
        )
        launched = bridge_runner.launch_godot(launch_args, case_bridge_root)
        state = bridge_runner.wait_for_state(
            case_bridge_root,
            args.timeout_seconds,
            require_runtime=True,
        )
        print(
            f"Visual audit launch ready: case={case_name} runtime_id={state['runtime_id']}",
            flush=True,
        )

        start_payload = bridge_runner.response_payload(
            bridge_runner.request(
                case_bridge_root,
                "start_session",
                {
                    "goal_label": f"visual_audit_{case_name}",
                    "metadata": {
                        "scenario": "runtime_presentation_visual_audit",
                        "case": case_name,
                        "world_origin": {"x": world_x, "y": world_y},
                    },
                },
                args.timeout_seconds,
            )
        )
        bridge_runner.require_action_status(
            start_payload["result"], "completed", "start_session"
        )
        observation = start_payload["observation"]
        runtime_presentation = observation.get("runtime_presentation", {})
        runtime_zone = enum_name(runtime_presentation, "planet_zone")
        wait_for_player_settled(case_bridge_root, 3.0, args.timeout_seconds)

        anchor_block = current_scene_block(observation["player_position"])
        focus_block = resolve_land_block(
            case_bridge_root,
            anchor_block,
            case["focus_probes"],
            args.timeout_seconds,
            preferred_distance=140.0,
        )
        best_view = capture_best_view(
            case_bridge_root,
            case_name,
            runtime_zone,
            focus_block,
            args.timeout_seconds,
        )
        case_path = screenshot_dir / f"{case_name}.png"
        shutil.copy2(best_view["source_path"], case_path)

        end_payload = bridge_runner.response_payload(
            bridge_runner.request(
                case_bridge_root,
                "end_session",
                {"reason": "visual audit complete"},
                args.timeout_seconds,
            )
        )
        bridge_runner.require_action_status(
            end_payload["result"], "completed", "end_session"
        )

        metrics = analyze_png(case_path)
        return {
            "name": case_name,
            "world_origin": {"x": world_x, "y": world_y},
            "origin_selection": selected_origin,
            "expected_zone": case["expected_zone"],
            "runtime_zone": runtime_zone,
            "runtime_atmosphere": enum_name(runtime_presentation, "atmosphere_class"),
            "runtime_palette": enum_name(runtime_presentation, "surface_palette_class"),
            "runtime_landform": enum_name(runtime_presentation, "landform_class"),
            "camera_block": best_view["camera_block"],
            "focus_block": focus_block,
            "selected_view": best_view["selected_view"],
            "screenshot_path": str(case_path),
            "session_id": end_payload["result"]["data"]["session"]["session_id"],
            "metrics": best_view["metrics"],
        }
    finally:
        if previous_override is None:
            os.environ.pop(VISUAL_AUDIT_ENV, None)
        else:
            os.environ[VISUAL_AUDIT_ENV] = previous_override
        if previous_runtime_flag is None:
            os.environ.pop(VISUAL_AUDIT_RUNTIME_ENV, None)
        else:
            os.environ[VISUAL_AUDIT_RUNTIME_ENV] = previous_runtime_flag
        if launched is not None:
            time.sleep(0.5)
            bridge_runner.terminate_process(launched)


def select_case_world_origin(case: dict[str, Any]) -> dict[str, Any]:
    fallback = {
        "world_x": float(case["world_origin"][0]),
        "world_y": float(case["world_origin"][1]),
        "reason": "fallback_origin",
    }
    if not RUNTIME_PRESENTATION_BIN.exists():
        return fallback

    rows = scan_world_presentations(seed=42, step=WORLD_SCAN_STEP)
    matching = [
        row for row in rows if row["planet_zone"] == case["expected_zone"]
    ]
    if not matching:
        return fallback

    matching.sort(
        key=lambda row: presentation_candidate_score(row, case),
        reverse=True,
    )
    coarse_candidates = matching[:4]
    refined_candidates: list[dict[str, Any]] = []
    for candidate in coarse_candidates:
        refined_candidates.extend(
            scan_window_presentations(
                seed=42,
                center_world_x=int(candidate["world_x"]),
                center_world_y=int(candidate["world_y"]),
                radius=REFINE_SCAN_RADIUS,
                step=REFINE_SCAN_STEP,
            )
        )
    refined_candidates.extend(
        scan_window_presentations(
            seed=42,
            center_world_x=int(fallback["world_x"]),
            center_world_y=int(fallback["world_y"]),
            radius=REFINE_SCAN_RADIUS,
            step=REFINE_SCAN_STEP,
        )
    )
    refined_matching = [
        row for row in refined_candidates if row["planet_zone"] == case["expected_zone"]
    ]
    if refined_matching:
        best = max(refined_matching, key=lambda row: presentation_candidate_score(row, case))
    else:
        best = coarse_candidates[0]
    return {
        "world_x": int(best["world_x"]),
        "world_y": int(best["world_y"]),
        "interestingness": round(float(best["interestingness"]), 6),
        "planet_zone": best["planet_zone"],
        "landform_class": best["landform_class"],
        "water_state": best["water_state"],
        "surface_palette_class": best["surface_palette_class"],
        "reason": "world_scan_best_match",
    }


def presentation_candidate_score(row: dict[str, Any], case: dict[str, Any]) -> float:
    preferred_landforms = set(case.get("preferred_landforms", []))
    preferred_water_states = set(case.get("preferred_water_states", []))
    preferred_palettes = set(case.get("preferred_palettes", []))
    high_relief_landforms = {
        "Ridge",
        "Escarpment",
        "BrokenHighland",
        "AlpineMassif",
        "Badlands",
        "FractureBelt",
        "VolcanicField",
        "CliffCoast",
    }
    mid_relief_landforms = {
        "Basin",
        "Plateau",
        "RiverCutLowland",
        "DuneWaste",
        "CoastShelf",
        "FrozenShelf",
    }

    score = float(row["interestingness"])
    landform = str(row["landform_class"])
    if landform in preferred_landforms:
        score += 0.24
    if row["water_state"] in preferred_water_states:
        score += 0.18
    if row["surface_palette_class"] in preferred_palettes:
        score += 0.14
    if landform in high_relief_landforms:
        score += 0.28
    elif landform in mid_relief_landforms:
        score += 0.12
    elif landform == "FlatPlain":
        score -= 0.32
    return score


def scan_world_presentations(seed: int, step: int) -> list[dict[str, Any]]:
    cache_key = (seed, step)
    cached = _WORLD_SCAN_CACHE.get(cache_key)
    if cached is not None:
        return cached

    rows: list[dict[str, Any]] = []
    for world_y in range(0, WORLD_SCAN_HEIGHT + 1, step):
        for world_x in range(0, WORLD_SCAN_WIDTH + 1, step):
            out = subprocess.check_output(
                [
                    str(RUNTIME_PRESENTATION_BIN),
                    "inspect",
                    "chunk-presentation",
                    "--format",
                    "json",
                    "--",
                    str(seed),
                    str(world_x),
                    str(world_y),
                ],
                text=True,
            )
            data = json.loads(out)
            rows.append(
                {
                    "world_x": world_x,
                    "world_y": world_y,
                    "planet_zone": data["planet_zone"],
                    "interestingness": data["interestingness_score"],
                    "landform_class": data["landform_class"],
                    "water_state": data["water_state"],
                    "surface_palette_class": data["surface_palette_class"],
                }
            )
    _WORLD_SCAN_CACHE[cache_key] = rows
    return rows


def scan_window_presentations(
    seed: int,
    center_world_x: int,
    center_world_y: int,
    radius: int,
    step: int,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    min_world_x = max(0, center_world_x - radius)
    max_world_x = min(WORLD_SCAN_WIDTH, center_world_x + radius)
    min_world_y = max(0, center_world_y - radius)
    max_world_y = min(WORLD_SCAN_HEIGHT, center_world_y + radius)
    for world_y in range(min_world_y, max_world_y + 1, step):
        for world_x in range(min_world_x, max_world_x + 1, step):
            out = subprocess.check_output(
                [
                    str(RUNTIME_PRESENTATION_BIN),
                    "inspect",
                    "chunk-presentation",
                    "--format",
                    "json",
                    "--",
                    str(seed),
                    str(world_x),
                    str(world_y),
                ],
                text=True,
            )
            data = json.loads(out)
            rows.append(
                {
                    "world_x": world_x,
                    "world_y": world_y,
                    "planet_zone": data["planet_zone"],
                    "interestingness": data["interestingness_score"],
                    "landform_class": data["landform_class"],
                    "water_state": data["water_state"],
                    "surface_palette_class": data["surface_palette_class"],
                }
            )
    return rows


def current_scene_block(player_position: dict[str, Any]) -> dict[str, int]:
    return {
        "x": int(round(float(player_position["x"]) - 0.5)),
        "z": int(round(float(player_position["z"]) - 0.5)),
    }


def resolve_land_block(
    bridge_root: Path,
    anchor_block: dict[str, int],
    probe_offsets: list[tuple[int, int]],
    timeout_seconds: float,
    avoid_block: dict[str, int] | None = None,
    preferred_distance: float | None = None,
) -> dict[str, int]:
    best_candidate: dict[str, Any] | None = None
    for offset_x, offset_z in probe_offsets:
        requested = {
            "x": anchor_block["x"] + int(offset_x),
            "z": anchor_block["z"] + int(offset_z),
        }
        nearest_payload = bridge_runner.run_step(
            bridge_root,
            "find_nearest_land",
            {"scene_block": requested},
            timeout_seconds,
        )
        nearest = nearest_payload["result"]["data"]["nearest_land_scene_block"]
        if avoid_block is not None and nearest == avoid_block:
            continue
        height = float(nearest_payload["result"]["data"]["height"])
        terrain_shape = describe_block_shape(bridge_root, nearest, timeout_seconds)
        distance = math.sqrt(
            float(nearest["x"] - anchor_block["x"]) ** 2
            + float(nearest["z"] - anchor_block["z"]) ** 2
        )
        # Favor probes that give us readable terrain breakup, not just arbitrary distance.
        score = (
            distance * 0.25
            + terrain_shape["relief"] * 2.2
            + terrain_shape["mean_neighbor_delta"] * 1.4
        )
        if preferred_distance is not None:
            score -= abs(distance - preferred_distance) * 0.35
        candidate = {
            "block": {"x": int(nearest["x"]), "z": int(nearest["z"])},
            "height": height,
            "score": score,
            "terrain_shape": terrain_shape,
        }
        if best_candidate is None or candidate["score"] > best_candidate["score"]:
            best_candidate = candidate
    if best_candidate is None:
        raise AuditError(
            f"failed to resolve a land block from probes {probe_offsets} around {anchor_block}"
        )
    return best_candidate["block"]


def capture_best_view(
    bridge_root: Path,
    case_name: str,
    runtime_zone: str,
    focus_block: dict[str, int],
    timeout_seconds: float,
) -> dict[str, Any]:
    focus_shape = describe_block_shape(bridge_root, focus_block, timeout_seconds)
    candidate_views: list[dict[str, Any]] = []
    camera_height_offset = CAMERA_HEIGHT_OFFSETS_BY_ZONE.get(runtime_zone, 10.0)
    focus_height_offset = max(1.5, min(6.0, focus_shape["relief"] * 0.12))

    for index, (offset_x, offset_z) in enumerate(ORBIT_CAMERA_OFFSETS):
        requested_camera = {
            "x": int(focus_block["x"]) + int(offset_x),
            "z": int(focus_block["z"]) + int(offset_z),
        }
        camera_block = resolve_land_block(
            bridge_root,
            focus_block,
            [(offset_x, offset_z)],
            timeout_seconds,
            avoid_block=focus_block,
            preferred_distance=160.0,
        )
        camera_shape = describe_block_shape(bridge_root, camera_block, timeout_seconds)
        distance = math.sqrt(
            float(camera_block["x"] - focus_block["x"]) ** 2
            + float(camera_block["z"] - focus_block["z"]) ** 2
        )
        camera_height = sample_height(bridge_root, camera_block, timeout_seconds)
        focus_height = focus_shape["center_height"]
        visibility = estimate_visibility(
            bridge_root,
            camera_block,
            focus_block,
            camera_height + camera_height_offset,
            focus_height + focus_height_offset,
            timeout_seconds,
        )
        if distance < 80.0:
            continue
        if camera_height < focus_height - 18.0:
            continue
        if visibility["hidden_fraction"] > 0.2 or visibility["min_clearance"] < -2.5:
            continue

        bridge_runner.run_step(
            bridge_root,
            "teleport_to_block",
            {"scene_block": camera_block, "height_offset": camera_height_offset},
            timeout_seconds,
        )
        wait_for_player_settled(bridge_root, 1.5, timeout_seconds)
        wait_for_ring_ready(bridge_root, timeout_seconds)
        bridge_runner.run_step(
            bridge_root,
            "look_at",
            {
                "scene_block": focus_block,
                "height_offset": focus_height_offset,
                "tolerance_degrees": 1.0,
            },
            timeout_seconds,
        )
        screenshot_step = bridge_runner.run_step(
            bridge_root,
            "capture_screenshot",
            {"file_name": f"{case_name}_candidate_{index}"},
            timeout_seconds,
        )
        screenshot_path = Path(str(screenshot_step["result"]["data"]["absolute_path"]))
        metrics = analyze_png(screenshot_path)
        composition_score = score_capture(metrics, visibility)
        candidate_views.append(
            {
                "index": index,
                "requested_camera_block": requested_camera,
                "camera_block": camera_block,
                "distance": round(distance, 3),
                "camera_height": round(camera_height, 3),
                "focus_height": round(focus_height, 3),
                "camera_shape": camera_shape,
                "visibility": visibility,
                "composition_score": round(composition_score, 6),
                "metrics": metrics,
                "source_path": screenshot_path,
            }
        )

    if not candidate_views:
        raise AuditError(f"failed to capture any candidate views for {case_name}")

    candidate_views.sort(key=lambda candidate: candidate["composition_score"], reverse=True)
    best_view = candidate_views[0]
    return {
        "camera_block": best_view["camera_block"],
        "selected_view": {
            "index": best_view["index"],
            "distance": best_view["distance"],
            "camera_height": best_view["camera_height"],
            "focus_height": best_view["focus_height"],
            "camera_shape": best_view["camera_shape"],
            "visibility": best_view["visibility"],
            "composition_score": best_view["composition_score"],
            "candidate_count": len(candidate_views),
        },
        "metrics": best_view["metrics"],
        "source_path": best_view["source_path"],
    }


def wait_for_player_settled(
    bridge_root: Path,
    settle_timeout_seconds: float,
    timeout_seconds: float,
) -> bool:
    try:
        bridge_runner.run_step(
            bridge_root,
            "wait_for_player_settled",
            {"timeout_seconds": settle_timeout_seconds},
            timeout_seconds,
        )
        return True
    except bridge_runner.BridgeError:
        return False


def wait_for_ring_ready(bridge_root: Path, timeout_seconds: float) -> bool:
    try:
        bridge_runner.run_step(
            bridge_root,
            "wait_for_ring_ready",
            {"timeout_seconds": 4.0},
            timeout_seconds,
        )
        return True
    except bridge_runner.BridgeError:
        return False


def describe_block_shape(
    bridge_root: Path,
    scene_block: dict[str, int],
    timeout_seconds: float,
) -> dict[str, float]:
    heights = [sample_height(bridge_root, scene_block, timeout_seconds)]
    center_height = heights[0]
    deltas = []
    for offset_x, offset_z in NEIGHBOR_SAMPLE_OFFSETS:
        neighbor = {
            "x": int(scene_block["x"]) + int(offset_x),
            "z": int(scene_block["z"]) + int(offset_z),
        }
        neighbor_height = sample_height(bridge_root, neighbor, timeout_seconds)
        heights.append(neighbor_height)
        deltas.append(abs(center_height - neighbor_height))
    return {
        "center_height": center_height,
        "relief": max(heights) - min(heights),
        "mean_neighbor_delta": sum(deltas) / max(1, len(deltas)),
    }


def sample_height(
    bridge_root: Path,
    scene_block: dict[str, int],
    timeout_seconds: float,
) -> float:
    payload = bridge_runner.run_step(
        bridge_root,
        "sample_height",
        {"scene_block": scene_block},
        timeout_seconds,
    )
    return float(payload["result"]["data"]["height"])


def enum_name(runtime_presentation: dict[str, Any], key: str) -> str:
    value = runtime_presentation.get(key, {})
    if isinstance(value, dict):
        return str(value.get("name", ""))
    return str(value)


def analyze_png(path: Path) -> dict[str, Any]:
    width, height, rgba = decode_png_rgba(path.read_bytes())
    pixel_count = width * height
    if pixel_count <= 0:
        raise AuditError(f"{path} decoded to an empty image")

    step = max(1, int(math.sqrt(pixel_count / 50000.0)))
    total = 0
    green_drift = 0
    bright_green = 0
    sum_r = 0.0
    sum_g = 0.0
    sum_b = 0.0
    sum_l = 0.0
    sum_l2 = 0.0
    sum_saturation = 0.0
    lower_total = 0
    lower_sum_l = 0.0
    lower_sum_l2 = 0.0
    middle_total = 0
    middle_sum_l = 0.0
    middle_sum_l2 = 0.0
    lower_edge_strength = 0.0
    middle_edge_strength = 0.0

    for y in range(0, height, step):
        row_start = y * width * 4
        for x in range(0, width, step):
            base = row_start + x * 4
            r = rgba[base]
            g = rgba[base + 1]
            b = rgba[base + 2]
            a = rgba[base + 3]
            if a < 8:
                continue
            total += 1
            rf = r / 255.0
            gf = g / 255.0
            bf = b / 255.0
            luminance = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf
            sum_r += rf
            sum_g += gf
            sum_b += bf
            sum_l += luminance
            sum_l2 += luminance * luminance
            sum_saturation += max(rf, gf, bf) - min(rf, gf, bf)

            if y >= int(height * 0.5):
                lower_total += 1
                lower_sum_l += luminance
                lower_sum_l2 += luminance * luminance
                lower_edge_strength += local_edge_strength(rgba, width, height, x, y, step)
            elif int(height * 0.3) <= y < int(height * 0.65):
                middle_total += 1
                middle_sum_l += luminance
                middle_sum_l2 += luminance * luminance
                middle_edge_strength += local_edge_strength(rgba, width, height, x, y, step)

            if gf > max(rf, bf) + 0.06 and gf > 0.14:
                green_drift += 1
            if gf > max(rf, bf) + 0.10 and gf > 0.22:
                bright_green += 1

    if total == 0:
        raise AuditError(f"{path} contained no visible pixels")

    mean_r = sum_r / total
    mean_g = sum_g / total
    mean_b = sum_b / total
    mean_l = sum_l / total
    variance_l = max(0.0, (sum_l2 / total) - mean_l * mean_l)
    lower_mean_l = lower_sum_l / max(1, lower_total)
    lower_variance_l = max(0.0, (lower_sum_l2 / max(1, lower_total)) - lower_mean_l * lower_mean_l)
    middle_mean_l = middle_sum_l / max(1, middle_total)
    middle_variance_l = max(0.0, (middle_sum_l2 / max(1, middle_total)) - middle_mean_l * middle_mean_l)
    return {
        "width": width,
        "height": height,
        "sample_step": step,
        "sampled_pixels": total,
        "mean_rgb": [round(mean_r, 6), round(mean_g, 6), round(mean_b, 6)],
        "mean_luminance": round(mean_l, 6),
        "luminance_stddev": round(math.sqrt(variance_l), 6),
        "lower_half_luminance_stddev": round(math.sqrt(lower_variance_l), 6),
        "middle_band_luminance_stddev": round(math.sqrt(middle_variance_l), 6),
        "lower_half_edge_strength": round(lower_edge_strength / max(1, lower_total), 6),
        "middle_band_edge_strength": round(middle_edge_strength / max(1, middle_total), 6),
        "mean_saturation": round(sum_saturation / total, 6),
        "warmth": round(mean_r - mean_b, 6),
        "green_drift_ratio": round(green_drift / total, 6),
        "bright_green_ratio": round(bright_green / total, 6),
    }


def local_edge_strength(
    rgba: bytes,
    width: int,
    height: int,
    x: int,
    y: int,
    step: int,
) -> float:
    center = sampled_luminance(rgba, width, x, y)
    total = 0.0
    count = 0
    if x + step < width:
        total += abs(center - sampled_luminance(rgba, width, x + step, y))
        count += 1
    if y + step < height:
        total += abs(center - sampled_luminance(rgba, width, x, y + step))
        count += 1
    return total / max(1, count)


def sampled_luminance(rgba: bytes, width: int, x: int, y: int) -> float:
    base = (y * width + x) * 4
    rf = rgba[base] / 255.0
    gf = rgba[base + 1] / 255.0
    bf = rgba[base + 2] / 255.0
    return 0.2126 * rf + 0.7152 * gf + 0.0722 * bf


def estimate_visibility(
    bridge_root: Path,
    camera_block: dict[str, int],
    focus_block: dict[str, int],
    camera_eye_height: float,
    focus_eye_height: float,
    timeout_seconds: float,
) -> dict[str, float]:
    min_clearance = float("inf")
    hidden_samples = 0
    sample_count = 0
    steps = 7
    for index in range(1, steps):
        t = float(index) / float(steps)
        sample_block = {
            "x": int(round(lerp(camera_block["x"], focus_block["x"], t))),
            "z": int(round(lerp(camera_block["z"], focus_block["z"], t))),
        }
        terrain_height = sample_height(bridge_root, sample_block, timeout_seconds)
        ray_height = lerp(camera_eye_height, focus_eye_height, t)
        clearance = ray_height - terrain_height
        min_clearance = min(min_clearance, clearance)
        if clearance < 0.75:
            hidden_samples += 1
        sample_count += 1
    return {
        "min_clearance": round(min_clearance, 6),
        "hidden_fraction": round(hidden_samples / max(1, sample_count), 6),
    }


def lerp(start: float, end: float, t: float) -> float:
    return start + (end - start) * t


def score_capture(metrics: dict[str, Any], visibility: dict[str, float]) -> float:
    return (
        metrics["lower_half_edge_strength"] * 6.0
        + metrics["middle_band_edge_strength"] * 4.0
        + metrics["lower_half_luminance_stddev"] * 2.8
        + metrics["middle_band_luminance_stddev"] * 1.8
        + metrics["mean_saturation"] * 0.6
        + max(0.0, visibility["min_clearance"]) * 0.08
        - visibility["hidden_fraction"] * 12.0
        - metrics["bright_green_ratio"] * 40.0
        - metrics["green_drift_ratio"] * 20.0
    )


def build_report(captures: list[dict[str, Any]]) -> dict[str, Any]:
    failures: list[str] = []

    for capture in captures:
        metrics = capture["metrics"]
        if capture["runtime_zone"] != capture["expected_zone"]:
            failures.append(
                f"{capture['name']} launched in unexpected zone: expected {capture['expected_zone']}, got {capture['runtime_zone']}"
            )
        if metrics["bright_green_ratio"] > MAX_BRIGHT_GREEN_RATIO:
            failures.append(
                f"{capture['name']} exceeded bright-green threshold: {metrics['bright_green_ratio']:.6f}"
            )
        if metrics["green_drift_ratio"] > MAX_GREEN_DRIFT_RATIO:
            failures.append(
                f"{capture['name']} exceeded green-drift threshold: {metrics['green_drift_ratio']:.6f}"
            )
    pairwise_distances = []
    for index, left in enumerate(captures):
        left_rgb = left["metrics"]["mean_rgb"]
        for right in captures[index + 1 :]:
            right_rgb = right["metrics"]["mean_rgb"]
            distance = math.sqrt(
                (left_rgb[0] - right_rgb[0]) ** 2
                + (left_rgb[1] - right_rgb[1]) ** 2
                + (left_rgb[2] - right_rgb[2]) ** 2
            )
            pairwise_distances.append(
                {
                    "left": left["name"],
                    "right": right["name"],
                    "mean_color_distance": round(distance, 6),
                }
            )

    luminance_values = [capture["metrics"]["mean_luminance"] for capture in captures]
    luminance_range = (
        max(luminance_values) - min(luminance_values) if luminance_values else 0.0
    )
    if luminance_range < MIN_CAPTURE_SET_LUMINANCE_RANGE:
        failures.append(
            f"capture set lacked luminance spread: range {luminance_range:.6f}"
        )

    peak_luminance_stddev = max(
        (capture["metrics"]["luminance_stddev"] for capture in captures),
        default=0.0,
    )
    if peak_luminance_stddev < MIN_PEAK_LUMINANCE_STDDEV:
        failures.append(
            f"capture set lacked local contrast: peak luminance stddev {peak_luminance_stddev:.6f}"
        )

    sorted_distances = sorted(
        entry["mean_color_distance"] for entry in pairwise_distances
    )
    median_distance = 0.0
    if sorted_distances:
        middle = len(sorted_distances) // 2
        if len(sorted_distances) % 2 == 0:
            median_distance = (
                sorted_distances[middle - 1] + sorted_distances[middle]
            ) * 0.5
        else:
            median_distance = sorted_distances[middle]
    if median_distance < MIN_MEDIAN_PAIRWISE_MEAN_COLOR_DISTANCE:
        failures.append(
            f"capture set lacked color separation: median mean-color distance {median_distance:.6f}"
        )

    warmth_by_name = {capture["name"]: capture["metrics"]["warmth"] for capture in captures}
    warmth_delta = warmth_by_name["dry_dayside_margin"] - warmth_by_name["deep_night_ice"]
    if warmth_delta < MIN_WARMTH_DELTA:
        failures.append(
            f"dayside capture did not read warmer than nightside strongly enough: delta {warmth_delta:.6f}"
        )

    return {
        "ok": not failures,
        "thresholds": {
            "max_bright_green_ratio": MAX_BRIGHT_GREEN_RATIO,
            "max_green_drift_ratio": MAX_GREEN_DRIFT_RATIO,
            "min_capture_set_luminance_range": MIN_CAPTURE_SET_LUMINANCE_RANGE,
            "min_peak_luminance_stddev": MIN_PEAK_LUMINANCE_STDDEV,
            "min_median_pairwise_mean_color_distance": MIN_MEDIAN_PAIRWISE_MEAN_COLOR_DISTANCE,
            "min_dayside_vs_nightside_warmth_delta": MIN_WARMTH_DELTA,
        },
        "captures": captures,
        "pairwise_mean_color_distances": pairwise_distances,
        "capture_set_metrics": {
            "luminance_range": round(luminance_range, 6),
            "peak_luminance_stddev": round(peak_luminance_stddev, 6),
            "median_pairwise_mean_color_distance": round(median_distance, 6),
            "dayside_vs_nightside_warmth_delta": round(warmth_delta, 6),
        },
        "failures": failures,
    }


def decode_png_rgba(data: bytes) -> tuple[int, int, bytes]:
    if not data.startswith(PNG_SIGNATURE):
        raise AuditError("file is not a PNG")

    width = 0
    height = 0
    bit_depth = 0
    color_type = 0
    interlace = 0
    idat_parts: list[bytes] = []
    offset = len(PNG_SIGNATURE)

    while offset < len(data):
        if offset + 8 > len(data):
            raise AuditError("truncated PNG chunk header")
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        chunk_type = data[offset + 4 : offset + 8]
        chunk_data_start = offset + 8
        chunk_data_end = chunk_data_start + length
        chunk_crc_end = chunk_data_end + 4
        if chunk_crc_end > len(data):
            raise AuditError("truncated PNG chunk body")
        chunk_data = data[chunk_data_start:chunk_data_end]
        offset = chunk_crc_end

        if chunk_type == b"IHDR":
            width, height, bit_depth, color_type, _, _, interlace = struct.unpack(
                ">IIBBBBB", chunk_data
            )
        elif chunk_type == b"IDAT":
            idat_parts.append(chunk_data)
        elif chunk_type == b"IEND":
            break

    if width <= 0 or height <= 0:
        raise AuditError("PNG missing valid IHDR")
    if bit_depth != 8:
        raise AuditError(f"unsupported PNG bit depth {bit_depth}")
    if interlace != 0:
        raise AuditError("interlaced PNGs are not supported")
    if color_type not in (2, 6):
        raise AuditError(f"unsupported PNG color type {color_type}")

    bytes_per_pixel = 3 if color_type == 2 else 4
    stride = width * bytes_per_pixel
    raw = zlib.decompress(b"".join(idat_parts))
    expected = height * (stride + 1)
    if len(raw) != expected:
        raise AuditError(
            f"unexpected decompressed PNG size {len(raw)} (expected {expected})"
        )

    previous = bytearray(stride)
    output = bytearray(width * height * 4)
    src = 0
    dst = 0
    for _ in range(height):
        filter_type = raw[src]
        src += 1
        line = bytearray(raw[src : src + stride])
        src += stride
        unfilter_scanline(line, previous, bytes_per_pixel, filter_type)
        if color_type == 6:
            output[dst : dst + width * 4] = line
            dst += width * 4
        else:
            for index in range(0, stride, 3):
                output[dst] = line[index]
                output[dst + 1] = line[index + 1]
                output[dst + 2] = line[index + 2]
                output[dst + 3] = 255
                dst += 4
        previous = line
    return width, height, bytes(output)


def unfilter_scanline(
    line: bytearray, previous: bytearray, bytes_per_pixel: int, filter_type: int
) -> None:
    if filter_type == 0:
        return
    if filter_type == 1:
        for index in range(len(line)):
            left = line[index - bytes_per_pixel] if index >= bytes_per_pixel else 0
            line[index] = (line[index] + left) & 0xFF
        return
    if filter_type == 2:
        for index in range(len(line)):
            line[index] = (line[index] + previous[index]) & 0xFF
        return
    if filter_type == 3:
        for index in range(len(line)):
            left = line[index - bytes_per_pixel] if index >= bytes_per_pixel else 0
            up = previous[index]
            line[index] = (line[index] + ((left + up) >> 1)) & 0xFF
        return
    if filter_type == 4:
        for index in range(len(line)):
            left = line[index - bytes_per_pixel] if index >= bytes_per_pixel else 0
            up = previous[index]
            up_left = previous[index - bytes_per_pixel] if index >= bytes_per_pixel else 0
            line[index] = (line[index] + paeth_predictor(left, up, up_left)) & 0xFF
        return
    raise AuditError(f"unsupported PNG filter type {filter_type}")


def paeth_predictor(left: int, up: int, up_left: int) -> int:
    predictor = left + up - up_left
    left_distance = abs(predictor - left)
    up_distance = abs(predictor - up)
    up_left_distance = abs(predictor - up_left)
    if left_distance <= up_distance and left_distance <= up_left_distance:
        return left
    if up_distance <= up_left_distance:
        return up
    return up_left


if __name__ == "__main__":
    raise SystemExit(main())
