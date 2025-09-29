#!/bin/bash

if [ $# -eq 0 ]; then
    echo "Usage: $0 <command> [arguments...]"
    echo "Example: $0 ostool run uboot"
    exit 1
fi

COMMAND=("$@")
echo "Executing command: ${COMMAND[*]}"

"${COMMAND[@]}" 2>&1 | while IFS= read -r line; do
    echo "$line"

    if [[ "$line" == *"[OK] Default guest initialized"* ]]; then
        echo "Completion signal detected, exiting..."

        sleep 2

        echo "Safely finding and killing QEMU processes..."
        
        # Get current script and parent process PIDs to avoid killing them
        SCRIPT_PID=$$
        PARENT_PID=$PPID
        
        echo "Current script PID: $SCRIPT_PID"
        echo "Parent process PID: $PARENT_PID"
        
        # Find QEMU processes, but exclude script-related processes
        pgrep -f "qemu" 2>/dev/null | while read pid; do
            # Check if it's a script-related process
            if [ "$pid" != "$SCRIPT_PID" ] && [ "$pid" != "$PARENT_PID" ]; then
                # Further check process command line
                CMD=$(ps -p "$pid" -o cmd --no-headers 2>/dev/null)
                if [[ "$CMD" == *"qemu-system"* ]]; then
                    echo "kill -9 $pid (QEMU system process)"
                    kill -9 "$pid" 2>/dev/null || true
                else
                    echo "Skipping process $pid (not a QEMU system process): $CMD"
                fi
            else
                echo "Skipping script-related process: $pid"
            fi
        done

        exit 0
    fi
done

echo "Done"