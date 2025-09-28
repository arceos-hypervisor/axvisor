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

        echo "安全查找并杀死QEMU进程..."
        
        # 获取当前脚本及其父进程的PID，避免误杀
        SCRIPT_PID=$$
        PARENT_PID=$PPID
        
        echo "当前脚本PID: $SCRIPT_PID"
        echo "父进程PID: $PARENT_PID"
        
        # 查找QEMU进程，但排除脚本相关进程
        pgrep -f "qemu" 2>/dev/null | while read pid; do
            # 检查是否是脚本相关进程
            if [ "$pid" != "$SCRIPT_PID" ] && [ "$pid" != "$PARENT_PID" ]; then
                # 进一步检查进程命令行
                CMD=$(ps -p "$pid" -o cmd --no-headers 2>/dev/null)
                if [[ "$CMD" == *"qemu-system"* ]]; then
                    echo "kill -9 $pid (QEMU系统进程)"
                    kill -9 "$pid" 2>/dev/null || true
                else
                    echo "跳过进程 $pid (非QEMU系统进程): $CMD"
                fi
            else
                echo "跳过脚本相关进程: $pid"
            fi
        done

        exit 0
    fi
done

echo "完成"