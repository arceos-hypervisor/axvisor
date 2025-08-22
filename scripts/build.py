#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import sys
from typing import Optional
from .config import AxvisorConfig, create_config_from_args, save_config_to_file
from .setup import setup_arceos


def main(args) -> int:
    """构建项目"""
    print("执行 build 功能...")

    # 获取配置文件路径
    config_file_path = getattr(args, "config", ".hvconfig.toml")

    # 检查配置文件是否存在
    config_exists = os.path.exists(config_file_path)

    # 首先设置 arceos 依赖
    print("设置 arceos 依赖...")
    if not setup_arceos():
        print("设置 arceos 失败，无法继续构建")
        return 1

    # 创建配置对象
    config: AxvisorConfig = create_config_from_args(args)

    # 构建 make 命令
    cmd = config.format_make_command("")

    print(f"执行命令: {cmd}")

    try:
        # 执行 make 命令
        result = subprocess.run(
            cmd, shell=True, check=True, env=config.get_subprocess_env()
        )
        print("构建成功!")

        # 如果配置文件不存在且有有意义的命令行参数，则创建配置文件
        if not config_exists:
            print(f"检测到 {config_file_path} 不存在，根据命令行参数创建配置文件...")
            if save_config_to_file(config, config_file_path):
                print(
                    f"配置文件创建成功，下次可以直接运行 './task.py build -c {config_file_path}' 而无需指定参数"
                )
            else:
                print("配置文件创建失败，下次仍需手动指定参数")

        return 0
    except subprocess.CalledProcessError as e:
        print(f"构建失败，退出码: {e.returncode}")
        return e.returncode
    except Exception as e:
        print(f"构建过程中发生错误: {e}")
        return 1
