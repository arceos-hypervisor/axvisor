#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional
from .config import AxvisorConfig, create_config_from_args
from .setup import setup_arceos
from . import build


def main(args) -> int:
    """Run the project"""
    print("Running run task...")

    # Create config object
    config: AxvisorConfig = create_config_from_args(args)
    # Build first
    print("Building project before run...")
    build_result = build.main(args)
    if build_result != 0:
        print("Build failed; aborting run")
        return build_result
    # Build make command
    cmd = config.format_make_command("run")

    print(f"Executing: {cmd}")

    try:
        # Run make run command
        result = subprocess.run(
            cmd, shell=True, check=True, env=config.get_subprocess_env()
        )
        print("Run completed!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"Run failed, exit code: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"Error during run: {e}")
        return 1
