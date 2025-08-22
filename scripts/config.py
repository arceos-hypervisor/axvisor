#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import argparse
from typing import Dict, List, Optional, Tuple, Any
from dataclasses import dataclass


@dataclass
class AxvisorConfig:
    """Axvisor 配置类，存储所有合并后的配置参数"""

    # 基本配置
    plat: str = "aarch64-generic"
    arch: Optional[str] = None
    package: Optional[str] = None
    features: List[str] = None
    arceos_features: List[str] = None
    arceos_args: List[str] = None
    vmconfigs: List[str] = None

    def __post_init__(self):
        """初始化后处理，确保列表字段不为 None"""
        if self.features is None:
            self.features = []
        if self.arceos_features is None:
            self.arceos_features = []
        if self.arceos_args is None:
            self.arceos_args = []
        if self.vmconfigs is None:
            self.vmconfigs = []

    def _load_platform_config(self):
        """从平台文件夹中的 axconfig.toml 文件读取配置参数"""
        try:
            # 构建平台配置文件路径
            platform_dir = os.path.join("platform", self.plat)
            config_file = os.path.join(platform_dir, "axconfig.toml")

            if not os.path.exists(config_file):
                print(f"警告：平台配置文件 {config_file} 不存在")
                return

            # 读取配置文件
            try:
                import toml

                with open(config_file, "r", encoding="utf-8") as f:
                    config_data = toml.load(f)

                    # 读取 arch 参数
                    if self.arch is None:
                        arch = config_data.get("arch")
                        if arch:
                            print(f"从 {config_file} 读取到 arch: {arch}")
                            self.arch = arch
                        else:
                            print(f"警告：在 {config_file} 中未找到 arch 字段")

                    # 读取 package 参数
                    if self.package is None:
                        package = config_data.get("package")
                        if package:
                            print(f"从 {config_file} 读取到 package: {package}")
                            self.package = package
                        else:
                            print(f"警告：在 {config_file} 中未找到 package 字段")

            except ImportError:
                print("警告：需要安装 toml 库来读取平台配置文件")
            except Exception as e:
                print(f"警告：读取平台配置文件 {config_file} 失败: {e}")

        except Exception as e:
            print(f"警告：加载平台配置信息时发生错误: {e}")

    def _load_arch_from_platform(self) -> Optional[str]:
        """从平台文件夹中的 axconfig.toml 文件读取 arch 参数（保持向后兼容）"""
        try:
            # 构建平台配置文件路径
            platform_dir = os.path.join("platform", self.plat)
            config_file = os.path.join(platform_dir, "axconfig.toml")

            if not os.path.exists(config_file):
                print(f"警告：平台配置文件 {config_file} 不存在")
                return None

            # 读取配置文件
            try:
                import toml

                with open(config_file, "r", encoding="utf-8") as f:
                    config_data = toml.load(f)
                    arch = config_data.get("arch")
                    if arch:
                        print(f"从 {config_file} 读取到 arch: {arch}")
                        return arch
                    else:
                        print(f"警告：在 {config_file} 中未找到 arch 字段")
                        return None
            except ImportError:
                print("警告：需要安装 toml 库来读取平台配置文件")
                return None
            except Exception as e:
                print(f"警告：读取平台配置文件 {config_file} 失败: {e}")
                return None

        except Exception as e:
            print(f"警告：加载平台架构信息时发生错误: {e}")
            return None

    def get_arch_specific_qemu_args(self) -> str:
        """根据架构生成特定的 QEMU 参数"""
        arch_qemu_args = ""

        # 根据架构添加特定的 QEMU 参数
        if self.arch == "aarch64":
            arch_qemu_args = "-machine virtualization=on"
        elif self.arch == "x86_64":
            # x86_64 架构使用 Intel VT-x 虚拟化支持
            arch_qemu_args = "-accel kvm"
        elif self.arch == "riscv64":
            # RISC-V 架构的虚拟化参数
            arch_qemu_args = "-machine virt -cpu rv64"

        return arch_qemu_args

    def get_arch_specific_variables(
        self, existing_make_vars: Optional[Dict[str, str]] = None
    ) -> Dict[str, str]:
        """根据架构生成特定的 make 变量，考虑已存在的变量"""
        arch_vars = {}

        # 获取架构特定的 QEMU 参数
        arch_qemu_args = self.get_arch_specific_qemu_args()

        if arch_qemu_args:
            # 检查是否已经存在 QEMU_ARGS
            existing_qemu_args = ""
            if existing_make_vars and "QEMU_ARGS" in existing_make_vars:
                existing_qemu_args = existing_make_vars["QEMU_ARGS"].strip('"')

            # 合并参数：如果已存在参数，则追加；否则直接使用架构参数
            if existing_qemu_args:
                combined_args = f"{existing_qemu_args} {arch_qemu_args}"
            else:
                combined_args = arch_qemu_args

            arch_vars["QEMU_ARGS"] = f'"{combined_args}"'

        return arch_vars

    def get_make_variables(self) -> Dict[str, str]:
        """根据配置生成 make 变量"""
        make_vars = {}

        if self.arch == "aarch64":
            make_vars["LD_SCRIPT"] = "link.x"

        # 使用从平台配置文件读取的 package，如果没有则回退到旧的方式
        if self.package:
            make_vars["MYPLAT"] = self.package
        else:
            make_vars["MYPLAT"] = f"axplat-{self.plat}"

        # 构建 APP_FEATURES
        app_features = [f"plat-{self.plat}"]
        if self.features:
            app_features.extend(self.features)
        make_vars["APP_FEATURES"] = ",".join(app_features)

        # ArceOS 特性
        arceos_features = ["page-alloc-64g"]
        if self.arceos_features:
            arceos_features.extend(self.arceos_features)
            make_vars["FEATURES"] = ",".join(arceos_features)

        # ArceOS 参数
        if self.arceos_args:
            for arg in self.arceos_args:
                if "=" in arg:
                    key, value = arg.split("=", 1)
                    make_vars[key.strip()] = value.strip()
                else:
                    make_vars[arg.strip()] = "y"

        # 添加架构特定的变量（传递现有的 make_vars 以便合并 QEMU_ARGS）
        arch_vars = self.get_arch_specific_variables(make_vars)
        make_vars.update(arch_vars)

        return make_vars

    def get_env_variables(self) -> Dict[str, str]:
        """根据配置生成环境变量"""
        env_vars = {}

        # 处理 vmconfigs 作为环境变量
        if self.vmconfigs:
            env_vars["AXVISOR_VM_CONFIGS"] = ",".join(self.vmconfigs)

        return env_vars

    def format_make_command(self, target: str = "") -> str:
        """格式化 make 命令，包含环境变量"""
        make_vars = self.get_make_variables()
        env_vars = self.get_env_variables()

        cmd_parts = format_make_command_base()

        # 添加环境变量
        for key, value in env_vars.items():
            cmd_parts.append(f"{key}={value}")

        # 添加 make 变量
        for key, value in make_vars.items():
            cmd_parts.append(f"{key}={value}")

        if target:
            cmd_parts.append(target)

        return " ".join(cmd_parts)

    def get_subprocess_env(self) -> Dict[str, str]:
        """获取用于 subprocess 的环境变量字典"""
        env = os.environ.copy()
        env.update(self.get_env_variables())
        return env


def load_config_file(config_path: str = ".hvconfig.toml") -> Dict[str, Any]:
    """从配置文件加载配置"""
    if os.path.exists(config_path):
        try:
            import toml

            with open(config_path, "r", encoding="utf-8") as f:
                return toml.load(f)
        except ImportError:
            print("警告：需要安装 toml 库来读取 .hvconfig.toml 文件")
            return {}
        except Exception as e:
            print(f"警告：读取配置文件 {config_path} 失败: {e}")
            return {}
    return {}


def array_to_comma_separated(value: Any) -> str:
    """将数组转换为逗号分隔的字符串"""
    if isinstance(value, list):
        # 过滤掉空字符串
        filtered_values = [str(v) for v in value if v]
        return ",".join(filtered_values) if filtered_values else ""
    return str(value) if value else ""


def string_or_array_to_list(value: Any) -> List[str]:
    """将字符串或数组转换为字符串列表"""
    if value is None:
        return []
    elif isinstance(value, list):
        # 过滤掉空字符串
        return [str(v) for v in value if v]
    elif isinstance(value, str):
        # 按逗号分割字符串，过滤掉空字符串
        return [item.strip() for item in value.split(",") if item.strip()]
    else:
        return [str(value)] if value else []


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    """为解析器添加通用参数"""
    parser.add_argument(
        "-c",
        "--config",
        type=str,
        default=".hvconfig.toml",
        help="Config file path (default: .hvconfig.toml)",
    )
    parser.add_argument(
        "--plat",
        type=str,
        help="Platform (default: aarch64-generic)",
    )
    parser.add_argument(
        "--arch",
        type=str,
        help="Architecture (auto-detected from platform config if not specified)",
    )
    parser.add_argument(
        "--package",
        type=str,
        help="Platform package name (auto-detected from platform config if not specified)",
    )
    parser.add_argument(
        "--features", type=str, help="Hypervisor features (comma-separated)"
    )
    parser.add_argument(
        "--arceos-features", type=str, help="ArceOS features (comma-separated)"
    )
    parser.add_argument(
        "--arceos-args", type=str, help="ArceOS arguments (comma-separated)"
    )
    parser.add_argument(
        "--vmconfigs", type=str, help="VM configuration file path (comma-separated)"
    )


def save_config_to_file(
    config: AxvisorConfig, config_path: str = ".hvconfig.toml"
) -> bool:
    """将配置保存到文件"""
    try:
        import toml

        # 准备配置数据
        config_data = {}

        config_data["plat"] = config.plat

        if config.features:
            config_data["features"] = config.features

        if config.arceos_features:
            config_data["arceos_features"] = config.arceos_features

        if config.arceos_args:
            config_data["arceos_args"] = config.arceos_args

        if config.vmconfigs:
            config_data["vmconfigs"] = config.vmconfigs

        # 写入文件
        with open(config_path, "w", encoding="utf-8") as f:
            toml.dump(config_data, f)

        print(f"配置已保存到 {config_path}")
        return True

    except ImportError:
        print("警告：需要安装 toml 库来保存配置文件")
        return False
    except Exception as e:
        print(f"警告：保存配置文件 {config_path} 失败: {e}")
        return False


def create_config_from_args(args: argparse.Namespace) -> AxvisorConfig:
    """从命令行参数和配置文件创建配置对象"""
    # 从命令行参数获取配置文件路径，如果没有指定则使用默认值
    config_file_path = getattr(args, "config", ".hvconfig.toml")

    # 加载配置文件
    config_file = load_config_file(config_file_path)

    # 创建配置对象
    config = AxvisorConfig()

    # 合并配置文件参数（配置文件优先级较低）
    if "plat" in config_file:
        config.plat = config_file["plat"]

    if "features" in config_file:
        config.features = string_or_array_to_list(config_file["features"])

    if "arceos_features" in config_file:
        config.arceos_features = string_or_array_to_list(config_file["arceos_features"])

    if "arceos_args" in config_file:
        config.arceos_args = string_or_array_to_list(config_file["arceos_args"])

    if "vmconfigs" in config_file:
        config.vmconfigs = string_or_array_to_list(config_file["vmconfigs"])

    # 合并命令行参数（命令行参数优先级较高）
    if args.plat:
        config.plat = args.plat

    config._load_platform_config()

    if args.features:
        config.features = string_or_array_to_list(args.features)

    if hasattr(args, "arceos_features") and args.arceos_features:
        config.arceos_features = string_or_array_to_list(args.arceos_features)

    if hasattr(args, "arceos_args") and args.arceos_args:
        config.arceos_args = string_or_array_to_list(args.arceos_args)

    if args.vmconfigs:
        config.vmconfigs = string_or_array_to_list(args.vmconfigs)

    return config


# 保持向后兼容的函数
def merge_config(args: argparse.Namespace) -> AxvisorConfig:
    """合并命令行参数和配置文件，返回配置对象"""
    return create_config_from_args(args)


def get_make_variables(
    args: argparse.Namespace,
) -> Tuple[Dict[str, str], Dict[str, str]]:
    """保持向后兼容的函数"""
    config = create_config_from_args(args)
    return config.get_make_variables(), config.get_env_variables()


def format_make_command_base() -> List[str]:
    cmd_parts = []
    cmd_parts.extend(["make", "-C", ".arceos", f"A={os.getcwd()}"])
    return cmd_parts
