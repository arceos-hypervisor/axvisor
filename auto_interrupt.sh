#!/bin/bash

if [ $# -eq 0 ]; then
    echo "用法: $0 <命令> [参数...]"
    echo "示例: $0 ostool run uboot"
    exit 1
fi

COMMAND=("$@")
echo "执行命令: ${COMMAND[*]}"

"${COMMAND[@]}" 2>&1 | while IFS= read -r line; do
    echo "输出: $line"

    if [[ "$line" == *"[OK] Default guest initialized"* ]]; then
        echo "检测到Shell就绪信号！发送中断信号..."
        
        PROCESS_NAME=$(basename "${COMMAND[0]}")
        PIDS=$(pgrep -f "${COMMAND[0]}")
        
        for PID in $PIDS; do
            echo "发送 SIGINT 到进程 $PID"
            kill -INT "$PID" 2>/dev/null
        done
        
        break
    fi
done

echo "完成"