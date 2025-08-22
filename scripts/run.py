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
    """运行项目"""
    print("执行 run 功能...")

    # 创建配置对象
    config: AxvisorConfig = create_config_from_args(args)

    # 首先执行 build
    print("运行前先构建项目...")
    build_result = build.main(args)
    if build_result != 0:
        print("构建失败，无法运行")
        return build_result

    # 构建 make 命令
    cmd = config.format_make_command("run")

    print(f"执行命令: {cmd}")

    try:
        # 执行 make run 命令
        result = subprocess.run(
            cmd, shell=True, check=True, env=config.get_subprocess_env()
        )
        print("运行完成!")
        return 0
    except subprocess.CalledProcessError as e:
        print(f"运行失败，退出码: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"运行过程中发生错误: {e}")
        return 1
