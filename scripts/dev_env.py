#!/usr/bin/env python3
import os
import subprocess


def main():
    subprocess.run("cargo install cargo-lpatch", shell=True, check=True)

    # 克隆其他仓库到 crates 目录
    repos = [
        "axvm",
        "axvcpu",
        "axaddrspace",
        "arm_vcpu",
        "axdevice",
        "arm_vgic",
        "axhvc",
    ]

    for one in repos:
        subprocess.run(f"cargo lpatch -n {one}", shell=True, check=True)

    # 创建 .vscode 目录并生成 settings.json
    os.makedirs(".vscode", exist_ok=True)
    with open(".vscode/settings.json", "w") as settings_json:
        settings_json.write(
            """
{
    "rust-analyzer.cargo.target": "aarch64-unknown-none-softfloat",
    "rust-analyzer.check.allTargets": false,
    "rust-analyzer.cargo.features": ["fs"],
}
    """
        )

    print("patch success")


if __name__ == "__main__":
    main()
