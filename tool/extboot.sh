#!/bin/bash

# 解析参数
while [[ "$#" -gt 0 ]]; do
    case $1 in
        --img) EXTBOOT_IMG="$2"; shift ;;
        --kernel) NEW_IMAGE_PATH="$2"; shift ;;
        *) echo "Unknown parameter passed: $1"; exit 1 ;;
    esac
    shift
done

# 检查必要参数是否已设置
if [ -z "${EXTBOOT_IMG}" ] || [ -z "${NEW_IMAGE_PATH}" ]; then
    echo "Usage: $0 --img <extboot.img> --kernel <new Image path>"
    exit 1
fi

MOUNT_POINT="./tmp/mnt/extboot"

# 创建挂载点
mkdir -p "$MOUNT_POINT"

# 挂载 extboot.img
sudo mount -o loop "$EXTBOOT_IMG" "$MOUNT_POINT"

# 查找所有以 Image- 开头的文件
for old_image in "$MOUNT_POINT"/Image-*; do
    if [ -f "$old_image" ]; then
        # 获取原始文件名（如 Image-1234）
        filename=$(basename "$old_image")
        sudo rm "$old_image"
        sudo cp "$NEW_IMAGE_PATH" "$old_image"
        echo "Replaced: $old_image with new Image"
    fi
done

# 卸载 extboot.img
sudo umount "$MOUNT_POINT"