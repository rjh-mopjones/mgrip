#!/usr/bin/env python3

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CLI_MANIFEST_PATH = REPO_ROOT / "gdextension/Cargo.toml"
DEFAULT_GOLDEN_PATH = (
    REPO_ROOT / "gdextension/testdata/runtime_presentation/seed42_v1_step256.ron"
)
PARITY_SCRIPT_PATH = REPO_ROOT / "tools/runtime_presentation_parity.py"
VISUAL_AUDIT_SCRIPT_PATH = REPO_ROOT / "tools/runtime_presentation_visual_audit.py"


def main() -> int:
    args = parse_args()

    try:
        run_command(
            "Rust runtime-presentation classification test",
            [
                "cargo",
                "test",
                "--manifest-path",
                str(CLI_MANIFEST_PATH),
                "-p",
                "mg_noise",
                "runtime_presentation::tests::classifies_reference_chunks_for_dayside_terminus_and_nightside",
                "--",
                "--exact",
            ],
        )
        run_command(
            "Grid default-audit regression test",
            [
                "cargo",
                "test",
                "--manifest-path",
                str(CLI_MANIFEST_PATH),
                "--bin",
                "margins_grip",
                "tests::default_grid_audit_passes_for_seed_42_step_256",
                "--",
                "--exact",
            ],
        )
        run_command(
            "Grid golden regression test",
            [
                "cargo",
                "test",
                "--manifest-path",
                str(CLI_MANIFEST_PATH),
                "--bin",
                "margins_grip",
                "tests::presentation_grid_matches_seed_42_golden_fixture",
                "--",
                "--exact",
            ],
        )

        audit_command = [
            "cargo",
            "run",
            "--release",
            "--manifest-path",
            str(CLI_MANIFEST_PATH),
            "--bin",
            "margins_grip",
            "--",
            "inspect",
            "layer-presentation-grid",
            args.layers_tag,
            str(args.step),
            "--audit-defaults",
        ]
        if not args.skip_golden and args.golden is not None:
            audit_command.extend(["--golden", str(args.golden)])
        run_command("Offline grid audit", audit_command)

        if not args.skip_parity:
            parity_command = [
                sys.executable,
                str(PARITY_SCRIPT_PATH),
                "--timeout-seconds",
                str(args.timeout_seconds),
                "--float-tolerance",
                str(args.float_tolerance),
            ]
            if args.attach:
                parity_command.append("--attach")
            if args.windowed:
                parity_command.append("--windowed")
            if args.godot_bin is not None:
                parity_command.extend(["--godot-bin", args.godot_bin])
            if args.bridge_root is not None:
                parity_command.extend(["--bridge-root", args.bridge_root])
            run_command("Offline/runtime parity", parity_command)

        if args.visual_audit:
            visual_audit_command = [
                sys.executable,
                str(VISUAL_AUDIT_SCRIPT_PATH),
                "--timeout-seconds",
                str(args.timeout_seconds),
            ]
            if args.godot_bin is not None:
                visual_audit_command.extend(["--godot-bin", args.godot_bin])
            if args.visual_audit_output_dir is not None:
                visual_audit_command.extend(
                    ["--output-dir", str(args.visual_audit_output_dir)]
                )
            run_command("Windowed visual audit", visual_audit_command)

        print("\nRuntime presentation verification passed.")
        return 0
    except subprocess.CalledProcessError as exc:
        print(
            f"\nRuntime presentation verification failed during: {exc.cmd}",
            file=sys.stderr,
        )
        return exc.returncode or 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the full runtime-presentation verification workflow."
    )
    parser.add_argument(
        "--layers-tag",
        default="v1",
        help="Layers artifact tag to audit.",
    )
    parser.add_argument(
        "--step",
        type=int,
        default=256,
        help="World-space step size for the layer presentation grid audit.",
    )
    parser.add_argument(
        "--golden",
        type=Path,
        default=DEFAULT_GOLDEN_PATH,
        help="Path to the golden layer-presentation summary fixture.",
    )
    parser.add_argument(
        "--skip-golden",
        action="store_true",
        help="Skip golden comparison during the layer presentation grid audit.",
    )
    parser.add_argument(
        "--skip-parity",
        action="store_true",
        help="Skip launching or attaching to Godot for the offline/runtime parity check.",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=60.0,
        help="Timeout passed through to the parity checker.",
    )
    parser.add_argument(
        "--float-tolerance",
        type=float,
        default=1e-4,
        help="Float comparison tolerance passed through to the parity checker.",
    )
    parser.add_argument(
        "--attach",
        action="store_true",
        help="Attach the parity check to an already running Godot instance.",
    )
    parser.add_argument(
        "--windowed",
        action="store_true",
        help="Run parity in windowed mode instead of headless mode.",
    )
    parser.add_argument(
        "--godot-bin",
        help="Override the Godot binary path used by the parity check.",
    )
    parser.add_argument(
        "--bridge-root",
        help="Override the agent runtime bridge root used by the parity check.",
    )
    parser.add_argument(
        "--visual-audit",
        action="store_true",
        help="Run the heavier windowed screenshot-based visual audit after parity.",
    )
    parser.add_argument(
        "--visual-audit-output-dir",
        type=Path,
        help="Override the output directory used by the visual audit.",
    )
    return parser.parse_args()


def run_command(label: str, command: list[str]) -> None:
    print(f"\n== {label} ==", flush=True)
    print(" ".join(command), flush=True)
    subprocess.run(command, cwd=REPO_ROOT, check=True)


if __name__ == "__main__":
    raise SystemExit(main())
