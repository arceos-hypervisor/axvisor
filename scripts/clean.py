#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import subprocess
from .config import format_make_command_base
from .setup import setup_arceos


def main(args) -> int:
    """Clean build artifacts"""
    print("Running clean task...")

    # Setup arceos dependency first
    print("Setting up arceos dependency...")
    if not setup_arceos():
        print("Failed to setup arceos, cannot clean")
        return 1

    cmd = format_make_command_base()

    cmd.append("clean")

    # Build make command string
    cmd = " ".join(cmd)

    print(f"Executing: {cmd}")

    try:
        # Run make command
        subprocess.run(cmd, shell=True, check=True)
        print("Clean succeeded!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"Clean failed, exit code: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"Error during clean: {e}")
        return 1
