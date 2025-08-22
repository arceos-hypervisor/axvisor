#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional
from .config import AxvisorConfig, create_config_from_args, format_make_command_base
from .setup import setup_arceos


def main(args) -> int:
    """创建磁盘镜像"""
    print("执行 disk_img 功能...")

    # 首先设置 arceos 依赖
    print("设置 arceos 依赖...")
    if not setup_arceos():
        print("设置 arceos 失败，无法继续执行 disk_img")
        return 1

    cmd = format_make_command_base()

    if args.image:
        # 如果指定了镜像路径和文件名，则添加到命令中
        cmd.append(f"DISK_IMG={args.image}")

    cmd.append("disk_img")

    # 构建 make 命令
    cmd = " ".join(cmd)

    print(f"执行命令: {cmd}")

    try:
        # 执行 make 命令
        subprocess.run(cmd, shell=True, check=True)
        print("磁盘镜像创建完成!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"磁盘镜像创建失败，退出码: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"磁盘镜像创建过程中发生错误: {e}")
        return 1
