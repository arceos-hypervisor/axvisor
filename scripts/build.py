#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional
from .config import AxvisorConfig, create_config_from_args, save_config_to_file
from .setup import setup_arceos


def main(args) -> int:
    """Build the project"""
    print("Running build task...")

    # Get config file path
    config_file_path = getattr(args, "config", ".hvconfig.toml")
    # Check if config file exists
    config_exists = os.path.exists(config_file_path)

    # Setup arceos dependency first
    print("Setting up arceos dependency...")
    if not setup_arceos():
        print("Failed to setup arceos, cannot build")
        return 1

    # Create config object
    config: AxvisorConfig = create_config_from_args(args)
    # Build make command
    cmd = config.format_make_command("")

    print(f"Executing: {cmd}")

    try:
        # Run make command
        result = subprocess.run(
            cmd, shell=True, check=True, env=config.get_subprocess_env()
        )
        print("Build succeeded!")

        # If config file missing and CLI args meaningful, create the config file
        if not config_exists:
            print(
                f"Detected missing {config_file_path}, creating config file from arguments..."
            )
            if save_config_to_file(config, config_file_path):
                print(
                    f"Config file created. Next time just run './task.py build -c {config_file_path}'"
                )
            else:
                print(
                    "Failed to create config file; you'll need to pass arguments again next run"
                )

        return 0
    except subprocess.CalledProcessError as e:
        print(f"Build failed, exit code: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"Error during build: {e}")
        return 1
