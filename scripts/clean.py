#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import subprocess
from .config import format_make_command_base
from .setup import setup_arceos


def main(args) -> int:
    """清理构建产物"""
    print("执行 clean 功能...")

    # 首先设置 arceos 依赖
    print("设置 arceos 依赖...")
    if not setup_arceos():
        print("设置 arceos 失败，无法继续执行 clean")
        return 1

    cmd = format_make_command_base()

    cmd.append("clean")

    # 构建 make 命令
    cmd = " ".join(cmd)

    print(f"执行命令: {cmd}")

    try:
        # 执行 make 命令
        subprocess.run(cmd, shell=True, check=True)
        print("清理完成!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"清理失败，退出码: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"清理过程中发生错误: {e}")
        return 1
