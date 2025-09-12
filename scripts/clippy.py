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
    """运行 clippy 代码检查"""
    print("执行 clippy 功能...")

    # 首先设置 arceos 依赖
    print("设置 arceos 依赖...")
    if not setup_arceos():
        print("设置 arceos 失败，无法继续执行 clippy")
        return 1
    # 读取根目录的 Cargo.toml 并解析 [features]
    cargo_toml_path = os.path.join(os.getcwd(), "Cargo.toml")
    if not os.path.exists(cargo_toml_path):
        print(f"未找到 {cargo_toml_path}，无法继续")
        return 1

    # 解析 Cargo.toml，使用第三方 toml 包（文本模式）
    parsed = None
    if _toml_impl is None:
        print("需要安装 python-toml 包（pip install toml）来解析 Cargo.toml")
        return 1
    try:
        with open(cargo_toml_path, "r", encoding="utf-8") as f:
            parsed = _toml_impl.load(f)
    except Exception as e:
        print(f"解析 Cargo.toml 失败: {e}")
        return 1

    features_dict = parsed.get("features", {}) if isinstance(parsed, dict) else {}
    all_features: List[str] = list(features_dict.keys())

    # 找出以 plat- 开头的 feature
    plat_features = [f for f in all_features if f.startswith("plat-")]
    # 其他非 plat 的 feature
    non_plat_features = [f for f in all_features if not f.startswith("plat-")]

    if not plat_features:
        print(
            "在 Cargo.toml 的 [features] 中未找到以 'plat-' 开头的 feature，将以所有 feature 运行一次 clippy"
        )
        features_arg = ",".join(all_features) if all_features else ""
        cmd_parts = ["cargo", "clippy"]
        if features_arg:
            cmd_parts.extend(["--features", f'"{features_arg}"'])
        cmd = " ".join(cmd_parts)
        print(f"执行命令: {cmd}")
        try:
            subprocess.run(cmd, shell=True, check=True)
            print("clippy 检查完成!")
            return 0
        except subprocess.CalledProcessError as e:
            print(f"clippy 检查失败，退出码: {e.returncode}")
            return e.returncode
        except Exception as e:
            print(f"clippy 检查过程中发生错误: {e}")
            return 1

    # 简单的 arch -> target 三元组映射（可按需扩展）
    arch_target_map = {
        "aarch64": "aarch64-unknown-none-softfloat",
        "x86": "x86_64-unknown-none",
        "x86_64": "x86_64-unknown-none",
        "riscv64": "riscv64gc-unknown-none-elf",
        "riscv": "riscv64gc-unknown-none-elf",
    }

    any_failure = False
    for plat in plat_features:
        # 从 plat 名称尝试提取 arch token（plat-<arch>-...）
        parts = plat.split("-")
        arch_token = parts[1] if len(parts) > 1 else None
        target = arch_target_map.get(arch_token) if arch_token else None

        # 构建 features: 选中当前 plat + 所有非 plat features（避免同时启用多个 plat）
        features_to_use = [plat] + non_plat_features
        features_arg = ",".join(features_to_use) if features_to_use else ""

        cmd_parts = ["cargo", "clippy"]
        if target:
            cmd_parts.extend(["--target", target])
        if features_arg:
            cmd_parts.extend(["--features", f'"{features_arg}"'])

        cmd = " ".join(cmd_parts)
        print(f"执行命令: {cmd}")

        try:
            subprocess.run(cmd, shell=True, check=True)
            print(f"{plat}: clippy 检查完成")
        except subprocess.CalledProcessError as e:
            print(f"{plat}: clippy 检查失败，退出码: {e.returncode}")
            any_failure = True
        except Exception as e:
            print(f"{plat}: clippy 检查过程中发生错误: {e}")
            any_failure = True

    return 1 if any_failure else 0
