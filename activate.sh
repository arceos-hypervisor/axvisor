#!/bin/bash
# -*- coding: utf-8 -*-

# Axvisor 虚拟环境激活脚本
# 用法: source activate.sh

# 检查虚拟环境是否存在
if [[ ! -d "venv" ]]; then
    ./bootstrap.sh
fi

# 检查是否已经在虚拟环境中
if [[ "$VIRTUAL_ENV" != "" ]]; then
    echo "已经在虚拟环境中: $VIRTUAL_ENV"
    return 0 2>/dev/null || exit 0
fi

# 激活虚拟环境
echo "激活 Axvisor 虚拟环境..."
source venv/bin/activate

echo "✓ 虚拟环境已激活"
echo "您现在可以使用以下命令:"
echo "  ./task.py build          # 构建项目"
echo "  ./task.py run            # 运行项目"
echo "  ./task.py build --help   # 查看构建选项"
echo "  ./task.py run --help     # 查看运行选项"
echo ""
echo "要退出虚拟环境，请运行: deactivate"
