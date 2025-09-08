#!/bin/bash
# -*- coding: utf-8 -*-

# Axvisor Bootstrap Script
# 此脚本用于创建 Python 虚拟环境并安装 task.py 所需的依赖

set -e  # 遇到错误时退出

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 输出函数
info() { echo -e "${BLUE}ℹ${NC} $1"; }
success() { echo -e "${GREEN}✓${NC} $1"; }
warning() { echo -e "${YELLOW}⚠${NC} $1"; }
error() { echo -e "${RED}✗${NC} $1"; }

# 获取项目根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# 检查是否已经在虚拟环境中
if [[ -n "$VIRTUAL_ENV" ]]; then
    info "检测到已在虚拟环境中: $VIRTUAL_ENV"
    
    # 检查 requirements.txt 文件是否存在
    if [[ ! -f "scripts/requirements.txt" ]]; then
        error "scripts/requirements.txt 文件未找到"
        exit 1
    fi
    
    # 安装/更新依赖
    info "更新 Python 依赖..."
    pip install -q -r scripts/requirements.txt -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple
    success "依赖更新完成"
    exit 0
fi

# 虚拟环境和标记文件
VENV_DIR="venv"
MARKER_FILE="$VENV_DIR/.bootstrapped"
REQUIREMENTS_FILE="scripts/requirements.txt"

# 计算依赖哈希值
compute_dep_hash() {
    local pyver
    pyver=$(python3 --version 2>/dev/null || echo "unknown")
    if [[ -f "$REQUIREMENTS_FILE" ]]; then
        {
            echo "$pyver"
            grep -v '^#' "$REQUIREMENTS_FILE" | grep -v '^$' | sort
        } | sha256sum | awk '{print $1}'
    else
        echo "$pyver-no-requirements" | sha256sum | awk '{print $1}'
    fi
}

# 检查是否需要重新引导
check_bootstrap_needed() {
    # 如果虚拟环境不存在，需要引导
    if [[ ! -d "$VENV_DIR" ]]; then
        return 0  # 需要引导
    fi
    
    # 如果标记文件不存在，需要引导
    if [[ ! -f "$MARKER_FILE" ]]; then
        return 0  # 需要引导
    fi
    
    # 检查哈希值是否匹配
    local existing_hash current_hash
    existing_hash=$(awk -F":" '/^hash:/ {print $2}' "$MARKER_FILE" 2>/dev/null | tr -d '[:space:]') || existing_hash=""
    current_hash=$(compute_dep_hash)
    
    if [[ "$existing_hash" != "$current_hash" ]]; then
        info "检测到依赖变更，需要重新引导"
        return 0  # 需要引导
    fi
    
    # 检查虚拟环境的Python是否可用
    if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
        warning "虚拟环境的 Python 不可用，需要重新引导"
        return 0  # 需要引导
    fi
    
    return 1  # 不需要引导
}
# 快速检查并退出
if ! check_bootstrap_needed; then
    success "引导已完成且依赖未更改，跳过引导"
    exit 0
fi

info "开始设置 Python 虚拟环境..."

# 检查系统依赖
check_system_deps() {
    info "检查系统依赖..."
    
    # 检查 Python 3
    if ! command -v python3 >/dev/null 2>&1; then
        error "python3 未找到，请先安装 Python 3"
        exit 1
    fi
    
    # 检查 Python 版本
    local pyver
    pyver=$(python3 --version 2>&1 | awk '{print $2}' | cut -d. -f1,2)
    info "检测到 Python 版本: $pyver"
    
    # 检查 venv 模块
    if ! python3 -c "import venv" 2>/dev/null; then
        error "python3-venv 模块未找到"
        echo "请安装 python3-venv:"
        echo "  Ubuntu/Debian: sudo apt install python3-venv"
        echo "  CentOS/RHEL:   sudo yum install python3-venv"
        echo "  Fedora:        sudo dnf install python3-venv"
        exit 1
    fi
    
    # 检查 requirements.txt
    if [[ ! -f "$REQUIREMENTS_FILE" ]]; then
        error "$REQUIREMENTS_FILE 文件未找到"
        exit 1
    fi
    
    success "系统依赖检查完成"
}

# 创建虚拟环境
setup_venv() {
    info "设置虚拟环境..."
    
    # 如果虚拟环境已存在但损坏，删除它
    if [[ -d "$VENV_DIR" ]] && [[ ! -x "$VENV_DIR/bin/python3" ]]; then
        warning "检测到损坏的虚拟环境，正在删除..."
        rm -rf "$VENV_DIR"
    fi
    
    # 创建虚拟环境
    if [[ ! -d "$VENV_DIR" ]]; then
        info "创建新的虚拟环境..."
        python3 -m venv "$VENV_DIR"
        success "虚拟环境已创建"
    else
        info "使用现有虚拟环境"
    fi
}

# 安装依赖
install_deps() {
    info "安装 Python 依赖..."
    
    # 激活虚拟环境
    source "$VENV_DIR/bin/activate"
    
    # 升级 pip（静默）
    python -m pip install -q --upgrade pip -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple
    
    # 安装依赖
    pip install -q -r "$REQUIREMENTS_FILE" -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple
    
    success "依赖安装完成"
}

# 验证安装
verify_installation() {
    info "验证安装..."
    
    # 测试 task.py
    if source "$VENV_DIR/bin/activate" && python3 ./scripts/task.py --help >/dev/null 2>&1; then
        success "task.py 运行正常"
    else
        error "task.py 运行失败"
        exit 1
    fi
}

# 写入完成标记
write_marker() {
    local dep_hash
    dep_hash=$(compute_dep_hash)
    
    cat > "$MARKER_FILE" <<EOF
hash: $dep_hash
timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)
python_version: $(python3 --version)
EOF
    
    success "引导完成标记已写入 (hash: ${dep_hash:0:8}...)"
}

# 主要执行流程
main() {
    check_system_deps
    setup_venv
    install_deps
    verify_installation
    write_marker
    
    success "虚拟环境设置完成！"
    info "使用 'source venv/bin/activate' 激活环境"
    info "使用 'make help' 查看可用命令"
}

# 执行主函数
main "$@"
