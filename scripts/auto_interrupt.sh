#!/bin/bash

if [ $# -eq 0 ]; then
    echo "用法: $0 <命令> [参数...]"
    echo "示例: $0 ostool run uboot"
    exit 1
fi

COMMAND=("$@")
echo "执行命令: ${COMMAND[*]}"

"${COMMAND[@]}" 2>&1 | while IFS= read -r line; do
    echo "$line"

    if [[ "$line" == *"[OK] Default guest initialized"* ]]; then
        echo "检测到完成信号，退出中..."

        sleep 2

        pgrep -f "qemu" 2>/dev/null | while read pid; do
            echo "kill -9 $pid"
            kill -9 "$pid" 2>/dev/null || true
        done

        pkill -9 -f "${COMMAND[0]}" 2>/dev/null
        
        break
    fi
done

echo "完成"