#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Axvisor 命令行工具
统一的项目管理入口点
"""

import argparse
import sys
import os
import importlib
import time
from pathlib import Path

# 添加当前脚本所在目录的上级目录到 Python 路径
SCRIPT_DIR = Path(__file__).parent.absolute()
PROJECT_ROOT = SCRIPT_DIR.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from scripts.config import add_common_arguments


def create_parser():
    """创建命令行参数解析器"""
    parser = argparse.ArgumentParser(
        description="Axvisor 命令行工具",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  %(prog)s build --plat aarch64-qemu-virt-hv
  %(prog)s run --vmconfigs configs/vms/linux-qemu-aarch64.toml
  %(prog)s clippy --arch aarch64
  %(prog)s disk_img --image custom-disk.img
        """,
    )

    # 添加全局选项
    parser.add_argument("--verbose", "-v", action="store_true", help="启用详细输出")

    parser.add_argument(
        "--quiet", "-q", action="store_true", help="静默输出（仅显示错误）"
    )

    subparsers = parser.add_subparsers(
        dest="command", help="可用命令", metavar="COMMAND"
    )

    # setup 命令
    setup_parser = subparsers.add_parser("setup", help="设置 arceos 依赖")

    # build 命令
    build_parser = subparsers.add_parser("build", help="构建项目")
    add_common_arguments(build_parser)

    # run 命令
    run_parser = subparsers.add_parser("run", help="运行项目")
    add_common_arguments(run_parser)

    # clippy 命令
    clippy_parser = subparsers.add_parser("clippy", help="运行 clippy 代码检查")
    clippy_parser.add_argument(
        "--arch",
        type=str,
        default="aarch64",
        help="目标架构 (默认: aarch64)",
    )

    # clean 命令
    clean_parser = subparsers.add_parser("clean", help="清理构建产物")

    # disk_img 命令
    disk_parser = subparsers.add_parser("disk_img", help="创建磁盘镜像")
    disk_parser.add_argument(
        "--image",
        type=str,
        default="disk.img",
        help="磁盘镜像路径和文件名 (默认: disk.img)",
    )

    return parser


def setup_logging(args):
    """设置日志级别"""
    if args.quiet:
        import logging

        logging.basicConfig(level=logging.ERROR)
    elif args.verbose:
        import logging

        logging.basicConfig(level=logging.DEBUG, format="%(levelname)s: %(message)s")


def run_command(cmd_name, args):
    """运行指定的命令模块"""
    command_map = {
        "setup": "scripts.setup",
        "build": "scripts.build",
        "run": "scripts.run",
        "clippy": "scripts.clippy",
        "clean": "scripts.clean",
        "disk_img": "scripts.disk_img",
    }

    module_name = command_map.get(cmd_name)
    if not module_name:
        print(f"错误: 未知命令 '{cmd_name}'", file=sys.stderr)
        return 1

    try:
        start_time = time.time()
        if args.verbose:
            print(f"执行命令: {cmd_name}")

        mod = importlib.import_module(module_name)
        result = mod.main(args)

        if args.verbose:
            elapsed = time.time() - start_time
            print(f"命令 '{cmd_name}' 完成，耗时 {elapsed:.2f}s")

        return result if result is not None else 0

    except ImportError as e:
        print(f"错误: 无法加载命令模块 '{module_name}': {e}", file=sys.stderr)
        return 1
    except Exception as e:
        print(f"错误: 命令 '{cmd_name}' 执行失败: {e}", file=sys.stderr)
        if args.verbose:
            import traceback

            traceback.print_exc()
        return 1


def main():
    """主入口函数"""
    parser = create_parser()

    # 如果没有参数，显示帮助
    if len(sys.argv) == 1:
        parser.print_help()
        return 0

    args = parser.parse_args()

    # 设置日志
    setup_logging(args)

    # 验证工作目录
    if not (PROJECT_ROOT / "Cargo.toml").exists():
        print("错误: 当前目录不是有效的 Axvisor 项目目录", file=sys.stderr)
        return 1

    # 执行命令
    if args.command:
        return run_command(args.command, args)
    else:
        parser.print_help()
        return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n用户中断操作", file=sys.stderr)
        sys.exit(130)
    except Exception as e:
        print(f"意外错误: {e}", file=sys.stderr)
        sys.exit(1)
