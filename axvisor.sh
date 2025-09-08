#!/bin/bash
# -*- coding: utf-8 -*-

# Axvisor 统一管理脚本
# 替代 Makefile，提供完整的项目管理功能

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# 项目配置
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"
HVCONFIG="$PROJECT_ROOT/.hvconfig.toml"
DEFAULT_HVCONFIG="$PROJECT_ROOT/configs/def_hvconfig.toml"
VENV_DIR="$PROJECT_ROOT/venv"
VENV_MARKER="$VENV_DIR/.bootstrapped"
REQUIREMENTS="$PROJECT_ROOT/scripts/requirements.txt"

# 切换到项目根目录
cd "$PROJECT_ROOT"

# 输出函数
info() { echo -e "${BLUE}💡${NC} $*"; }
success() { echo -e "${GREEN}✅${NC} $*"; }
warning() { echo -e "${YELLOW}ℹ️${NC} $*"; }
error() { echo -e "${RED}❌${NC} $*"; }
step() { echo -e "${CYAN}==>${NC} $*"; }

# 错误处理
handle_error() {
    error "命令失败: $1"
    exit 1
}

trap 'handle_error "脚本执行中断"' ERR

# 检查系统依赖
check_system_deps() {
    local missing_deps=()
    
    # 检查 Python 3
    if ! command -v python3 >/dev/null 2>&1; then
        missing_deps+=("python3")
    fi
    
    # 检查 Cargo
    if ! command -v cargo >/dev/null 2>&1; then
        missing_deps+=("cargo")
    fi
    
    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        error "缺少必要依赖: ${missing_deps[*]}"
        info "请安装缺少的依赖后重试"
        exit 1
    fi
}

# 检查虚拟环境是否需要设置
needs_venv_setup() {
    # 虚拟环境不存在
    if [[ ! -d "$VENV_DIR" ]]; then
        return 0
    fi
    
    # Python 可执行文件不存在
    if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
        return 0
    fi
    
    # requirements.txt 更新了
    if [[ "$REQUIREMENTS" -nt "$VENV_MARKER" ]]; then
        return 0
    fi
    
    return 1
}

# 设置虚拟环境
setup_venv() {
    if ! needs_venv_setup; then
        return 0
    fi
    
    step "设置 Python 虚拟环境..."
    
    # 运行 bootstrap 脚本
    ./scripts/bootstrap.sh
    
    success "虚拟环境设置完成"
}

# 配置文件管理
setup_defconfig() {
    step "设置默认配置..."
    
    if [[ ! -f "$DEFAULT_HVCONFIG" ]]; then
        error "默认配置文件 $DEFAULT_HVCONFIG 不存在"
        exit 1
    fi
    
    if [[ -f "$HVCONFIG" ]]; then
        warning "配置文件 $HVCONFIG 已存在"
        read -p "是否覆盖现有配置？(y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            info "已取消操作"
            return 0
        fi
    fi
    
    cp "$DEFAULT_HVCONFIG" "$HVCONFIG"
    success "已复制 $DEFAULT_HVCONFIG -> $HVCONFIG"
    
    info "配置文件设置完成"
    info "可以编辑 $HVCONFIG 来自定义配置"
}

# 确保配置文件存在（静默方式）
ensure_config() {
    if [[ ! -f "$HVCONFIG" ]]; then
        if [[ -f "$DEFAULT_HVCONFIG" ]]; then
            info "自动复制默认配置文件..."
            cp "$DEFAULT_HVCONFIG" "$HVCONFIG"
            success "已复制 $DEFAULT_HVCONFIG -> $HVCONFIG"
        else
            warning "默认配置文件 $DEFAULT_HVCONFIG 不存在"
            warning "请先运行 './axvisor.sh defconfig' 设置配置文件"
        fi
    fi
}

# 运行 Python 任务
run_python_task() {
    setup_venv
    source "$VENV_DIR/bin/activate"
    python3 scripts/task.py "$@"
}

# 显示帮助信息
show_help() {
    echo -e "${CYAN}Axvisor 项目管理工具${NC}"
    echo
    echo -e "${YELLOW}用法:${NC} $0 <命令> [参数...]"
    echo
    echo -e "${YELLOW}环境管理:${NC}"
    echo "  setup           - 设置开发环境"
    echo "  defconfig       - 设置默认配置文件"
    echo "  check-deps      - 检查系统依赖"
    echo "  rebuild-venv    - 强制重建虚拟环境"
    echo
    echo -e "${YELLOW}构建命令:${NC}"
    echo "  build [args]    - 构建项目"
    echo "  clean           - 清理构建产物"
    echo "  clippy [arch]   - 运行代码检查"
    echo
    echo -e "${YELLOW}运行命令:${NC}"
    echo "  run [args]      - 运行项目"
    echo "  disk_img [img]  - 创建磁盘镜像"
    echo
    echo -e "${YELLOW}快捷方式:${NC}"
    echo "  quick-build     - 快速构建 (默认平台)"
    echo "  quick-run       - 快速运行 (默认配置)"
    echo "  dev-build       - 开发构建 (setup + build)"
    echo "  dev-run         - 开发运行 (setup + run)"
    echo
    echo -e "${YELLOW}信息命令:${NC}"
    echo "  status          - 显示项目状态"
    echo "  version         - 显示版本信息"
    echo "  help            - 显示此帮助信息"
    echo
    echo -e "${YELLOW}构建示例:${NC}"
    echo "  $0 build --plat aarch64-qemu-virt-hv"
    echo "  $0 build --plat aarch64-generic --features irq,mem"
    echo "  $0 quick-build"
    echo
    echo -e "${YELLOW}运行示例:${NC}"
    echo "  $0 run --plat aarch64-qemu-virt-hv"
    echo "  $0 run --vmconfigs configs/vms/linux-qemu-aarch64.toml"
    echo "  $0 quick-run"
    echo
    echo -e "${YELLOW}其他示例:${NC}"
    echo "  $0 defconfig"
    echo "  $0 clippy aarch64"
    echo "  $0 disk_img custom-disk.img"
    echo "  $0 dev-build"
}

# 显示项目状态
show_status() {
    step "项目状态"
    
    echo "项目根目录: $PROJECT_ROOT"
    echo "配置文件: $([ -f "$HVCONFIG" ] && echo "✓ 存在" || echo "✗ 不存在")"
    echo "虚拟环境: $([ -d "$VENV_DIR" ] && echo "✓ 已设置" || echo "✗ 未设置")"
    
    if [[ -f "$VENV_MARKER" ]]; then
        echo "环境状态: ✓ 已初始化"
        local timestamp=$(grep "timestamp:" "$VENV_MARKER" 2>/dev/null | cut -d' ' -f2- || echo "未知")
        echo "初始化时间: $timestamp"
    else
        echo "环境状态: ✗ 未初始化"
    fi
    
    # 检查系统依赖
    echo "系统依赖:"
    command -v python3 >/dev/null 2>&1 && echo "  Python3: ✓" || echo "  Python3: ✗"
    command -v cargo >/dev/null 2>&1 && echo "  Cargo: ✓" || echo "  Cargo: ✗"
    
    # 显示最近的构建产物
    if [[ -f "axvisor-dev_aarch64-generic.bin" ]]; then
        local build_time=$(stat -c %y "axvisor-dev_aarch64-generic.bin" 2>/dev/null | cut -d' ' -f1,2)
        echo "最近构建: $build_time"
    fi
}

# 显示版本信息
show_version() {
    echo "Axvisor 管理脚本 v2.0"
    echo "项目: axvisor-dev"
    echo "分支: $(git branch --show-current 2>/dev/null || echo "未知")"
    echo "提交: $(git rev-parse --short HEAD 2>/dev/null || echo "未知")"
}

# 强制重建虚拟环境
rebuild_venv() {
    step "强制重建虚拟环境..."
    
    if [[ -d "$VENV_DIR" ]]; then
        warning "删除现有虚拟环境..."
        rm -rf "$VENV_DIR"
    fi
    
    setup_venv
    success "虚拟环境重建完成"
}

# 开发者快捷方式
dev_build() {
    step "开发构建 (setup + build)..."
    setup_environment
    run_python_task build "$@"
}

dev_run() {
    step "开发运行 (setup + run)..."
    setup_environment
    run_python_task run "$@"
}

# 设置完整的开发环境
setup_environment() {
    step "设置开发环境..."
    check_system_deps
    setup_venv
    success "开发环境设置完成"
}

# 主命令处理
main() {
    local cmd="${1:-help}"
    shift || true  # 移除第一个参数，剩余参数传递给子命令
    
    case "$cmd" in
        # 帮助和信息
        "help"|"-h"|"--help")
            show_help
            ;;
        "version"|"-v"|"--version")
            show_version
            ;;
        "status")
            show_status
            ;;
            
        # 环境管理
        "setup")
            setup_environment
            ;;
        "defconfig")
            setup_defconfig
            ;;
        "check-deps")
            check_system_deps
            success "所有系统依赖已满足"
            ;;
        "rebuild-venv")
            rebuild_venv
            ;;
            
        # 构建命令
        "build")
            ensure_config
            step "构建项目..."
            run_python_task build "$@"
            ;;
        "clean")
            step "清理构建产物..."
            run_python_task clean "$@"
            # 额外清理 cargo 产物
            if command -v cargo >/dev/null 2>&1; then
                cargo clean
            fi
            success "清理完成"
            ;;
        "clippy")
            local arch="${1:-aarch64}"
            step "运行代码检查 (架构: $arch)..."
            run_python_task clippy --arch "$arch"
            ;;
            
        # 运行命令
        "run")
            ensure_config
            step "运行项目..."
            run_python_task run "$@"
            ;;
        "disk_img")
            local image="${1:-disk.img}"
            step "创建磁盘镜像: $image"
            run_python_task disk_img --image "$image"
            ;;
            
        # 快捷方式
        "quick-build")
            ensure_config
            step "快速构建 (默认平台)..."
            run_python_task build --plat aarch64-generic
            ;;
        "quick-run")
            ensure_config
            step "快速运行 (默认配置)..."
            run_python_task run --plat aarch64-generic
            ;;
        "dev-build")
            ensure_config
            dev_build "$@"
            ;;
        "dev-run")
            ensure_config
            dev_run "$@"
            ;;
            
        # 未知命令
        *)
            error "未知命令: $cmd"
            info "使用 '$0 help' 查看可用命令"
            exit 1
            ;;
    esac
}

# 脚本入口点
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # 处理中断信号
    trap 'echo -e "\n${YELLOW}用户中断操作${NC}"; exit 130' INT
    
    # 执行主函数
    main "$@"
fi
