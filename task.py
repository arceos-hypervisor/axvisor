#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import argparse
import sys
import importlib
from scripts.config import add_common_arguments


def main():
    parser = argparse.ArgumentParser(description="Axvisor 命令行工具")
    subparsers = parser.add_subparsers(dest="command", help="可用命令")

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
        help="Architecture",
    )

    # clean 命令
    subparsers.add_parser("clean", help="清理构建产物")

    # disk_img 命令
    disk_parser = subparsers.add_parser("disk_img", help="创建磁盘镜像")
    disk_parser.add_argument(
        "--image",
        type=str,
        default="",
        help="磁盘镜像路径和文件名",
    )

    args = parser.parse_args()

    if args.command == "setup":
        mod = importlib.import_module("scripts.setup")
        exit_code = mod.main(args)
        sys.exit(exit_code)
    elif args.command == "build":
        mod = importlib.import_module("scripts.build")
        exit_code = mod.main(args)
        sys.exit(exit_code)
    elif args.command == "run":
        mod = importlib.import_module("scripts.run")
        exit_code = mod.main(args)
        sys.exit(exit_code)
    elif args.command == "clippy":
        mod = importlib.import_module("scripts.clippy")
        exit_code = mod.main(args)
        sys.exit(exit_code)
    elif args.command == "clean":
        mod = importlib.import_module("scripts.clean")
        exit_code = mod.main(args)
        sys.exit(exit_code)
    elif args.command == "disk_img":
        mod = importlib.import_module("scripts.disk_img")
        exit_code = mod.main(args)
        sys.exit(exit_code)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
