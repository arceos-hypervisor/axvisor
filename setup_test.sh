#!/bin/bash

# 确保 tmp 目录存在
mkdir -p tmp

# 下载并创建镜像的完整函数
setup_nimbos_image() {
    local arch=$1
    local zip_file="tmp/${arch}_usertests.zip"
    local bin_name="nimbos-${arch}.bin"
    local img_name="nimbos-${arch}.img"
    local download_url="https://github.com/arceos-hypervisor/nimbos/releases/download/v0.7/${arch}_usertests.zip"
    
    echo "=== Setting up ${img_name} ==="
    
    # 检查并下载对应架构的zip文件
    if [ ! -f "${zip_file}" ]; then
        echo "Downloading ${arch}_usertests.zip..."
        wget -P tmp "${download_url}"
        if [ $? -ne 0 ]; then
            echo "Error: Failed to download ${arch}_usertests.zip"
            return 1
        fi
    else
        echo "${arch}_usertests.zip already exists, skipping download."
    fi
    
    echo "Creating ${img_name}..."
    
    # 清理并创建临时目录
    sudo rm -rf tmp/tmp_disk
    mkdir -p tmp/tmp_disk
    
    # 解压对应架构的文件
    unzip "${zip_file}" -d tmp/tmp_disk
    if [ $? -ne 0 ]; then
        echo "Error: Failed to extract ${zip_file}"
        return 1
    fi
    
    # 重命名二进制文件
    mv tmp/tmp_disk/nimbos.bin "tmp/tmp_disk/${bin_name}"
    
    # 如果是 x86_64 架构，下载并添加 axvm-bios.bin
    if [ "${arch}" == "x86_64" ]; then
        echo "Downloading axvm-bios.bin for x86_64..."
        local bios_file="tmp/axvm-bios.bin"
        local bios_url="https://github.com/arceos-hypervisor/axvm-bios-x86/releases/download/v0.1/axvm-bios.bin"
        
        # 下载 axvm-bios.bin
        if [ ! -f "${bios_file}" ]; then
            wget -O "${bios_file}" "${bios_url}"
            if [ $? -ne 0 ]; then
                echo "Error: Failed to download axvm-bios.bin"
                return 1
            fi
        else
            echo "axvm-bios.bin already exists, skipping download."
        fi
        
        # 复制到临时目录
        cp "${bios_file}" tmp/tmp_disk/
        if [ $? -ne 0 ]; then
            echo "Error: Failed to copy axvm-bios.bin"
            return 1
        fi
        
        echo "axvm-bios.bin added to x86_64 image."
    fi
    
    # 删除旧的镜像文件
    sudo rm -f ".arceos/${img_name}"
    
    # 创建新的镜像
    ./task.py disk_img --image "${img_name}"
    if [ $? -ne 0 ]; then
        echo "Error: Failed to create disk image ${img_name}"
        return 1
    fi
    
    # 挂载并复制文件
    sudo mkdir -p tmp/img
    sudo chown root:root tmp/tmp_disk/*
    sudo mount ".arceos/${img_name}" tmp/img
    if [ $? -ne 0 ]; then
        echo "Error: Failed to mount ${img_name}"
        return 1
    fi
    
    sudo mv tmp/tmp_disk/* tmp/img
    sudo umount tmp/img
    
    echo "${img_name} created successfully!"
    echo "=== ${arch} setup completed ==="
    echo
}

# 创建 aarch64 和 x86_64 镜像
setup_nimbos_image "aarch64"
setup_nimbos_image "x86_64"

echo "All nimbos images setup completed!"