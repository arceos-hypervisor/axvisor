#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional, List
from .config import AxvisorConfig, create_config_from_args, format_make_command_base
from .setup import setup_arceos

try:
    # Use third-party `toml` package for parsing (keep behaviour simple)
    import toml as _toml_impl  # type: ignore
except Exception:
    _toml_impl = None


def main(args) -> int:
    """Run clippy lint checks"""
    print("Running clippy task...")

    # First setup arceos dependency
    print("Setting up arceos dependency...")
    if not setup_arceos():
        print("Failed to setup arceos, cannot continue clippy")
        return 1
    # Read root Cargo.toml and parse [features]
    cargo_toml_path = os.path.join(os.getcwd(), "Cargo.toml")
    if not os.path.exists(cargo_toml_path):
        print(f"Not found {cargo_toml_path}, abort")
        return 1
    # Parse Cargo.toml using third-party toml package
    parsed = None
    if _toml_impl is None:
        print("Need python 'toml' package (pip install toml) to parse Cargo.toml")
        return 1
    try:
        with open(cargo_toml_path, "r", encoding="utf-8") as f:
            parsed = _toml_impl.load(f)
    except Exception as e:
        print(f"Failed to parse Cargo.toml: {e}")
        return 1

    features_dict = parsed.get("features", {}) if isinstance(parsed, dict) else {}
    all_features: List[str] = list(features_dict.keys())

    # Collect features starting with plat-
    plat_features = [f for f in all_features if f.startswith("plat-")]
    # Non-plat features
    non_plat_features = [f for f in all_features if not f.startswith("plat-")]

    if not plat_features:
        print(
            "No 'plat-' features found in Cargo.toml [features]; running single clippy pass with all features"
        )
        features_arg = ",".join(all_features) if all_features else ""
        cmd_parts = ["cargo", "clippy"]
        if features_arg:
            cmd_parts.extend(["--features", f'"{features_arg}"'])
        cmd = " ".join(cmd_parts)
        print(f"Executing: {cmd}")
        try:
            subprocess.run(cmd, shell=True, check=True)
            print("Clippy finished successfully")
            return 0
        except subprocess.CalledProcessError as e:
            print(f"Clippy failed with exit code: {e.returncode}")
            return e.returncode
        except Exception as e:
            print(f"Error while running clippy: {e}")
            return 1

    # Simple arch -> target triple map (extend as needed)
    arch_target_map = {
        "aarch64": "aarch64-unknown-none-softfloat",
        "x86": "x86_64-unknown-none",
        "x86_64": "x86_64-unknown-none",
        "riscv64": "riscv64gc-unknown-none-elf",
        "riscv": "riscv64gc-unknown-none-elf",
    }

    any_failure = False
    for plat in plat_features:
        # Extract arch token from plat name (plat-<arch>-...)
        parts = plat.split("-")
        arch_token = parts[1] if len(parts) > 1 else None
        target = arch_target_map.get(arch_token) if arch_token else None

        # Build features: current plat + all non-plat ones (avoid enabling multiple plat features)
        features_to_use = [plat] + non_plat_features
        features_arg = ",".join(features_to_use) if features_to_use else ""

        cmd_parts = ["cargo", "clippy"]
        if target:
            cmd_parts.extend(["--target", target])
        if features_arg:
            cmd_parts.extend(["--features", f'"{features_arg}"'])

        cmd_parts.extend(
            [
                "--",
                "-D",
                "warnings",
            ]
        )

        cmd = " ".join(cmd_parts)
        print(f"Executing: {cmd}")

        try:
            subprocess.run(cmd, shell=True, check=True)
            print(f"{plat}: clippy finished successfully")
        except subprocess.CalledProcessError as e:
            print(f"{plat}: clippy failed, exit code: {e.returncode}")
            any_failure = True
        except Exception as e:
            print(f"{plat}: error while running clippy: {e}")
            any_failure = True

    return 1 if any_failure else 0
