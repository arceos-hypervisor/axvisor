#!/bin/bash
# -*- coding: utf-8 -*-

# Axvisor Bootstrap Script
# 此脚本用于创建 Python 虚拟环境并安装 task.py 所需的依赖

set -e  # 遇到错误时退出

echo "=== Axvisor Bootstrap Script ==="
echo "正在设置 Python 虚拟环境..."

# 检查 Python 版本
python_version=$(python3 --version 2>&1 | awk '{print $2}' | cut -d. -f1,2)
echo "检测到 Python 版本: $python_version"

# 检查是否有 python3-venv
if ! python3 -c "import venv" 2>/dev/null; then
    echo "错误: python3-venv 模块未找到，请安装 python3-venv"
    echo "在 Ubuntu/Debian 上运行: sudo apt install python3-venv"
    echo "在 CentOS/RHEL 上运行: sudo yum install python3-venv"
    exit 1
fi

# 检查 requirements.txt 文件是否存在
if [[ ! -f "requirements.txt" ]]; then
    echo "错误: requirements.txt 文件未找到"
    exit 1
fi

# 虚拟环境目录
VENV_DIR="venv"

# 创建虚拟环境（如果不存在）
if [[ ! -d "$VENV_DIR" ]]; then
    echo "创建 Python 虚拟环境..."
    python3 -m venv "$VENV_DIR"
    echo "✓ 虚拟环境已创建在 $VENV_DIR/"
else
    echo "✓ 检测到已存在的虚拟环境: $VENV_DIR/"
fi

# 激活虚拟环境
echo "激活虚拟环境..."
source "$VENV_DIR/bin/activate"

# 升级 pip
echo "升级 pip..."
python -m pip install -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple --upgrade pip

# 安装依赖
echo "正在安装 Python 依赖..."
pip install -r requirements.txt -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple

echo "依赖安装完成!"

# 测试 task.py 是否可以正常运行
echo "测试 task.py..."
if python3 ./task.py --help > /dev/null 2>&1; then
    echo "✓ task.py 运行正常"
else
    echo "✗ task.py 运行失败，请检查安装"
    exit 1
fi

echo "=== Bootstrap 完成 ==="
echo "虚拟环境已设置完成！"
echo ""
echo "要使用项目，请先激活虚拟环境："
echo "  source venv/bin/activate"
echo ""
echo "然后您可以使用以下命令:"
echo "  ./task.py build          # 构建项目"
echo "  ./task.py run            # 运行项目"
echo "  ./task.py build --help   # 查看构建选项"
echo "  ./task.py run --help     # 查看运行选项"
echo ""
echo "要退出虚拟环境，请运行:"
echo "  deactivate"
