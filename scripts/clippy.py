#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional
from .config import AxvisorConfig, create_config_from_args, format_make_command_base
from .setup import setup_arceos


def main(args) -> int:
    """运行 clippy 代码检查"""
    print("执行 clippy 功能...")

    # 首先设置 arceos 依赖
    print("设置 arceos 依赖...")
    if not setup_arceos():
        print("设置 arceos 失败，无法继续执行 clippy")
        return 1

    cmd = format_make_command_base()

    cmd.append(f"ARCH={args.arch}")

    cmd.append("clippy")

    # 构建 make 命令
    cmd = " ".join(cmd)

    print(f"执行命令: {cmd}")

    try:
        # 执行 make 命令
        subprocess.run(cmd, shell=True, check=True)
        print("clippy 检查完成!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"clippy 检查失败，退出码: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"clippy 检查过程中发生错误: {e}")
        return 1
