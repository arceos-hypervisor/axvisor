#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess


def setup_arceos():
    """Setup arceos dependency"""
    arceos_dir = ".arceos"

    if not os.path.exists(arceos_dir):
        print("Cloning arceos repository...")
        try:
            # Clone arceos repository
            result = subprocess.run(
                [
                    "git",
                    "clone",
                    "https://github.com/arceos-hypervisor/arceos",
                    "-b",
                    "hypervisor",
                    arceos_dir,
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            print("arceos repository cloned")
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to clone arceos repository: {e}")
            print(f"Stderr: {e.stderr}")
            return False
        except Exception as e:
            print(f"Error while setting up arceos: {e}")
            return False
    else:
        print(".arceos directory already exists")
        return True


def main(args=None):
    """Entry point when used as standalone command"""
    print("Running setup-arceos task...")
    return 0 if setup_arceos() else 1
