#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess


def setup_arceos():
    """设置 arceos 依赖"""
    arceos_dir = ".arceos"

    if not os.path.exists(arceos_dir):
        print("正在克隆 arceos 仓库...")
        try:
            # 克隆 arceos 仓库
            result = subprocess.run(
                [
                    "git",
                    "clone",
                    "https://github.com/arceos-hypervisor/arceos",
                    "-b",
                    "vmm-dev",
                    arceos_dir,
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            print("arceos 仓库克隆完成")
            return True
        except subprocess.CalledProcessError as e:
            print(f"克隆 arceos 仓库失败: {e}")
            print(f"错误输出: {e.stderr}")
            return False
        except Exception as e:
            print(f"设置 arceos 过程中发生错误: {e}")
            return False
    else:
        print(".arceos 文件夹已存在")
        return True


def main(args=None):
    """作为独立命令使用时的入口"""
    print("执行 setup-arceos 功能...")
    return 0 if setup_arceos() else 1
