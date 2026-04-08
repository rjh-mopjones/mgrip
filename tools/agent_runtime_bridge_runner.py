#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_GODOT_BIN = Path("/Applications/Godot.app/Contents/MacOS/Godot")
DEFAULT_APP_NAME = "Margin's Grip"
BRIDGE_DIR_NAME = "agent_runtime_bridge"
PASSIVE_WINDOW_ARG = "--agent-runtime-passive-window"
PASSIVE_WINDOW_ENV = "MG_AGENT_RUNTIME_PASSIVE_WINDOW"
LAUNCH_TOKEN_PREFIX = "--agent-runtime-launch-token="


class BridgeError(RuntimeError):
    pass


@dataclass
class LaunchedGodot:
    process: subprocess.Popen[str]
    cleanup_pattern: str | None = None


def main() -> int:
    args = parse_args()
    bridge_root = Path(args.bridge_root).expanduser()
    launched: LaunchedGodot | None = None

    try:
        if not args.attach:
            shutil.rmtree(bridge_root, ignore_errors=True)
            launched = launch_godot(args, bridge_root)

        state = wait_for_state(bridge_root, args.timeout_seconds, require_runtime=args.wait_for_runtime)
        print(f"Bridge ready: runtime_id={state['runtime_id']} runtime_available={state['runtime_available']}")

        summary = run_cross_chunk_smoke(bridge_root, args.timeout_seconds)
        print(json.dumps(summary, indent=2))
        return 0
    except BridgeError as exc:
        print(f"agent bridge runner failed: {exc}", file=sys.stderr)
        return 1
    finally:
        if launched is not None:
            terminate_process(launched)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Drive Margin's Grip through the developer agent-runtime file bridge.")
    parser.add_argument(
        "--godot-bin",
        default=os.environ.get("GODOT_BIN", str(DEFAULT_GODOT_BIN)),
        help="Path to the Godot executable.",
    )
    parser.add_argument(
        "--bridge-root",
        default=str(default_bridge_root()),
        help="Path to the agent-runtime bridge directory.",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=30.0,
        help="Timeout for bridge startup and individual request waits.",
    )
    parser.add_argument(
        "--attach",
        action="store_true",
        help="Attach to an already running Godot process instead of launching one.",
    )
    parser.add_argument(
        "--wait-for-runtime",
        action="store_true",
        default=True,
        help="Wait for the world runtime to register before sending actions.",
    )
    parser.add_argument(
        "--no-wait-for-runtime",
        dest="wait_for_runtime",
        action="store_false",
        help="Only wait for the bridge state file, not runtime availability.",
    )
    parser.add_argument(
        "--windowed",
        action="store_true",
        help="Run with the normal display driver so screenshot capture can produce real images.",
    )
    return parser.parse_args()


def default_bridge_root() -> Path:
    home = Path.home()
    system = platform.system()
    if system == "Darwin":
        app_userdata = home / "Library/Application Support/Godot/app_userdata"
    elif system == "Windows":
        app_userdata = home / "AppData/Roaming/Godot/app_userdata"
    else:
        app_userdata = home / ".local/share/godot/app_userdata"
    return app_userdata / DEFAULT_APP_NAME / BRIDGE_DIR_NAME


def launch_godot(args: argparse.Namespace, bridge_root: Path) -> LaunchedGodot:
    godot_bin = Path(args.godot_bin).expanduser()
    if not godot_bin.exists():
        raise BridgeError(f"Godot executable not found at {godot_bin}")

    launch_token = f"runner_{int(time.time() * 1000)}"
    godot_args = ["--path", str(REPO_ROOT), "--quiet"]
    if not args.windowed:
        godot_args[0:0] = ["--display-driver", "headless"]
    godot_user_args = ["--agent-runtime", "--agent-runtime-quick-launch"]
    if args.windowed:
        godot_user_args.extend([PASSIVE_WINDOW_ARG, f"{LAUNCH_TOKEN_PREFIX}{launch_token}"])
    godot_args.extend(["--", *godot_user_args])

    env = os.environ.copy()
    env["MG_AGENT_RUNTIME"] = "1"
    env["MG_AGENT_RUNTIME_BRIDGE_ROOT"] = str(bridge_root)
    if args.windowed:
        env[PASSIVE_WINDOW_ENV] = "1"

    if args.windowed and platform.system() == "Darwin":
        app_bundle = resolve_app_bundle(godot_bin)
        if app_bundle is not None and bridge_root == default_bridge_root():
            command = ["open", "-g", "-n", "-a", str(app_bundle), "--args", *godot_args]
            print("Launching:", " ".join(command))
            process = subprocess.Popen(
                command,
                cwd=REPO_ROOT,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                text=True,
                env=env,
            )
            return LaunchedGodot(process=process, cleanup_pattern=f"{LAUNCH_TOKEN_PREFIX}{launch_token}")

    command = [str(godot_bin), *godot_args]
    print("Launching:", " ".join(command))
    process = subprocess.Popen(
        command,
        cwd=REPO_ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
        env=env,
    )
    return LaunchedGodot(process=process)


def terminate_process(launched: LaunchedGodot) -> None:
    if launched.cleanup_pattern is not None:
        subprocess.run(["pkill", "-f", "--", launched.cleanup_pattern], check=False)
    process = launched.process
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5.0)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5.0)


def resolve_app_bundle(godot_bin: Path) -> Path | None:
    for parent in godot_bin.parents:
        if parent.suffix == ".app":
            return parent
    return None


def wait_for_state(bridge_root: Path, timeout_seconds: float, require_runtime: bool) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    state_path = bridge_root / "state.json"
    last_state: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        if state_path.exists():
            try:
                last_state = read_json(state_path)
            except json.JSONDecodeError:
                time.sleep(0.05)
                continue
            if last_state.get("enabled") and (not require_runtime or last_state.get("runtime_available")):
                return last_state
        time.sleep(0.1)
    raise BridgeError(f"Timed out waiting for bridge state at {state_path}. Last state: {last_state}")


def request(bridge_root: Path, command: str, args: dict[str, Any] | None = None, timeout_seconds: float = 30.0) -> dict[str, Any]:
    requests_dir = bridge_root / "requests"
    responses_dir = bridge_root / "responses"
    requests_dir.mkdir(parents=True, exist_ok=True)
    responses_dir.mkdir(parents=True, exist_ok=True)

    request_id = f"{int(time.time() * 1000)}_{command}"
    request_path = requests_dir / f"{request_id}.json"
    response_path = responses_dir / f"{request_id}.json"

    write_json(
        request_path,
        {
            "schema_version": 1,
            "request_id": request_id,
            "command": command,
            "args": args or {},
        },
    )

    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if response_path.exists():
            try:
                return read_json(response_path)
            except json.JSONDecodeError:
                time.sleep(0.05)
                continue
        time.sleep(0.1)
    raise BridgeError(f"Timed out waiting for response to {command} ({request_id})")


def run_cross_chunk_smoke(bridge_root: Path, timeout_seconds: float) -> dict[str, Any]:
    start_payload = response_payload(
        request(
            bridge_root,
            "start_session",
            {
                "goal_label": "bridge_chunk_crossing_smoke",
                "metadata": {
                    "scenario": "bridge_chunk_crossing_smoke",
                    "runner": "tools/agent_runtime_bridge_runner.py",
                },
            },
            timeout_seconds,
        )
    )
    start_result = start_payload["result"]
    require_action_status(start_result, "completed", "start_session")

    observation = start_payload["observation"]
    blocks_per_chunk = int(observation["runtime_constants"]["blocks_per_chunk"])
    anchor_chunk = observation["anchor_chunk"]
    current_chunk = observation["current_chunk"]
    origin_x = (int(current_chunk["x"]) - int(anchor_chunk["x"])) * blocks_per_chunk
    origin_z = (int(current_chunk["y"]) - int(anchor_chunk["y"])) * blocks_per_chunk
    seam_z = int(round(origin_z + blocks_per_chunk * 0.5))
    teleport_probe = {"x": int(round(origin_x + blocks_per_chunk - 16.0)), "z": seam_z}
    target_probe = {"x": int(round(origin_x + blocks_per_chunk + 24.0)), "z": seam_z}

    teleport_land_step = run_step(bridge_root, "find_nearest_land", {"scene_block": teleport_probe}, timeout_seconds)
    teleport_land = teleport_land_step["result"]["data"]["nearest_land_scene_block"]
    run_step(bridge_root, "teleport_to_block", {"scene_block": teleport_land}, timeout_seconds)
    run_step(bridge_root, "wait_for_player_settled", {"timeout_seconds": 3.0}, timeout_seconds)
    run_step(bridge_root, "wait_for_ring_ready", {"timeout_seconds": 10.0}, timeout_seconds)
    run_step(bridge_root, "wait_for_chunk_loaded", {"scene_block": target_probe, "timeout_seconds": 10.0}, timeout_seconds)

    move_land_step = run_step(bridge_root, "find_nearest_land", {"scene_block": target_probe}, timeout_seconds)
    move_target = move_land_step["result"]["data"]["nearest_land_scene_block"]
    move_step = run_step(
        bridge_root,
        "move_to_block",
        {
            "scene_block": move_target,
            "speed": 12.0,
            "arrival_radius": 1.5,
            "timeout_seconds": 8.0,
        },
        timeout_seconds,
    )
    run_step(bridge_root, "wait_for_player_settled", {"timeout_seconds": 3.0}, timeout_seconds)
    run_step(bridge_root, "wait_for_ring_ready", {"timeout_seconds": 10.0}, timeout_seconds)
    sample_step = run_step(bridge_root, "sample_height", {"scene_block": move_target}, timeout_seconds)
    screenshot_step = run_step(bridge_root, "capture_screenshot", {"file_name": "bridge_smoke"}, timeout_seconds)

    screenshot_result = screenshot_step["result"]
    screenshot_skipped = False
    if screenshot_result["status"] != "completed":
        if screenshot_result.get("error_code") != "headless_screenshot_unavailable":
            raise BridgeError(f"capture_screenshot failed: {json.dumps(screenshot_result, indent=2)}")
        screenshot_skipped = True

    end_payload = response_payload(
        request(
            bridge_root,
            "end_session",
            {"reason": "bridge runner complete"},
            timeout_seconds,
        )
    )

    return {
        "ok": True,
        "bridge_root": str(bridge_root),
        "initial_chunk": current_chunk,
        "teleport_target": teleport_land,
        "move_target": move_target,
        "moved_chunk": move_step["observation"]["current_chunk"],
        "sample_height": sample_step["result"]["data"]["height"],
        "screenshot": screenshot_result.get("data", {}),
        "screenshot_skipped": screenshot_skipped,
        "screenshot_error_code": screenshot_result.get("error_code", ""),
        "session": end_payload["result"]["data"]["session"],
    }


def run_step(bridge_root: Path, action: str, params: dict[str, Any], timeout_seconds: float) -> dict[str, Any]:
    payload = response_payload(
        request(
            bridge_root,
            "run_step",
            {
                "action": action,
                "params": params,
            },
            timeout_seconds,
        )
    )
    require_action_status(payload["result"], "completed", action, allow_headless_screenshot=True)
    return payload


def require_action_status(result: dict[str, Any], expected: str, action: str, allow_headless_screenshot: bool = False) -> None:
    if result.get("status") == expected:
        return
    if allow_headless_screenshot and result.get("error_code") == "headless_screenshot_unavailable":
        return
    raise BridgeError(f"{action} returned {result.get('status')}: {json.dumps(result, indent=2)}")


def response_payload(response: dict[str, Any]) -> dict[str, Any]:
    if not response.get("ok", False):
        raise BridgeError(f"Bridge transport error: {json.dumps(response, indent=2)}")
    return response["payload"]


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = path.with_suffix(path.suffix + ".tmp")
    temp_path.write_text(json.dumps(payload, indent=2))
    temp_path.replace(path)


if __name__ == "__main__":
    raise SystemExit(main())
