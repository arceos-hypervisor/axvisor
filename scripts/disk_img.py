#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional
from .config import AxvisorConfig, create_config_from_args, format_make_command_base
from .setup import setup_arceos


def main(args) -> int:
    """Create disk image"""
    print("Running disk_img task...")

    # Setup arceos dependency first
    print("Setting up arceos dependency...")
    if not setup_arceos():
        print("Failed to setup arceos, cannot create disk image")
        return 1

    cmd = format_make_command_base()

    if args.image:
        # Handle image path: convert relative path to absolute
        image_path = args.image
        if not os.path.isabs(image_path):
            # Compute absolute path relative to project root
            project_root = os.getcwd()
            image_path = os.path.abspath(os.path.join(project_root, image_path))
        # Add image path to command if specified
        cmd.append(f"DISK_IMG={image_path}")

    cmd.append("disk_img")

    # Build make command
    cmd = " ".join(cmd)

    print(f"Executing: {cmd}")

    try:
        # Run make command
        subprocess.run(cmd, shell=True, check=True)
        print("Disk image created successfully!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"Disk image creation failed, exit code: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"Error during disk image creation: {e}")
        return 1
