#!/bin/bash
# -*- coding: utf-8 -*-

# NimbOS 镜像制作脚本
# 参照 .github/workflows/actions/setup-nimbos-guest-image/action.yml 实现

set -e

# 获取脚本所在目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 推算项目根目录（假设脚本在 scripts/ 目录下）
WORKDIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# 输出函数
info() { echo -e "${BLUE}ℹ️${NC} $*"; }
success() { echo -e "${GREEN}✅${NC} $*"; }
warning() { echo -e "${YELLOW}⚠️${NC} $*"; }
error() { echo -e "${RED}❌${NC} $*"; }
step() { echo -e "${CYAN}🚀${NC} $*"; }

# 错误处理
handle_error() {
    error "脚本失败: $1"
    exit 1
}

trap 'handle_error "脚本执行中断"' ERR

# 默认配置

DEFAULT_ARCH="aarch64"
DEFAULT_VERSION="latest"
DISK_PATH=""
ZIP_PATH=""
UNZIP_PATH=""
BIOS_PATH=""
DEFAULT_REPO="arceos-hypervisor/nimbos"
DEFAULT_BIOS_VERSION="latest"
DEFAULT_BIOS_REPO="arceos-hypervisor/axvm-bios-x86"


# 解析命令行参数
parse_args() {
    ARCH="$DEFAULT_ARCH"
    VERSION="$DEFAULT_VERSION"
    REPO="$DEFAULT_REPO"
    BIOS_VERSION="$DEFAULT_BIOS_VERSION"
    BIOS_REPO="$DEFAULT_BIOS_REPO"

    while [[ $# -gt 0 ]]; do
        case $1 in
            --arch)
                ARCH="$2"
                shift 2
                ;;
            --version)
                VERSION="$2"
                shift 2
                ;;
            --repo)
                REPO="$2"
                shift 2
                ;;
            --bios-version)
                BIOS_VERSION="$2"
                shift 2
                ;;
            --bios-repo)
                BIOS_REPO="$2"
                shift 2
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                error "未知参数: $1"
                show_help
                exit 1
                ;;
        esac
    done

    # 验证必需参数
    if [[ -z "$ARCH" ]]; then
        error "--arch 参数是必需的"
        show_help
        exit 1
    fi

    DISK_PATH="${WORKDIR}/tmp/nimbos-${ARCH}.img"
    ZIP_PATH="${WORKDIR}/tmp/${ARCH}_usertests.zip"
    UNZIP_PATH="${WORKDIR}/tmp/nimbos-${ARCH}"
    BIOS_PATH="${WORKDIR}/tmp/axvm-bios.bin"
}

# 显示帮助信息
show_help() {
    echo -e "${CYAN}🔧 NimbOS 镜像制作工具${NC}"
    echo
    echo -e "${YELLOW}📋 用法:${NC} $0 --arch <架构> [选项...]"
    echo
    echo -e "${YELLOW}⚙️ 必需参数:${NC}"
    echo "  --arch <架构>     - 目标架构 (例如: x86_64, aarch64)"
    echo
    echo -e "${YELLOW}🔧 可选参数:${NC}"
    echo "  --version <版本>  - NimbOS 版本 (默认: $DEFAULT_VERSION)"
    echo "  --disk-path <路径> - 磁盘镜像输出路径 (默认: $DEFAULT_DISK_PATH)"
    echo "  --repo <仓库>     - NimbOS GitHub 仓库 (默认: $DEFAULT_REPO)"
    echo "  --bios-version <版本> - BIOS 版本 (仅 x86_64, 默认: $DEFAULT_BIOS_VERSION)"
    echo "  --bios-repo <仓库> - BIOS GitHub 仓库 (仅 x86_64, 默认: $DEFAULT_BIOS_REPO)"
    echo "  --help, -h        - 显示此帮助信息"
    echo
    echo -e "${YELLOW}📚 示例:${NC}"
    echo "  $0 --arch x86_64"
    echo "  $0 --arch x86_64 --version v1.0.0 --disk-path custom.img"
    echo "  $0 --arch aarch64 --repo myorg/nimbos"
}

# 检查依赖
check_dependencies() {
    local missing_deps=()

    if ! command -v curl >/dev/null 2>&1; then
        missing_deps+=("curl")
    fi

    if ! command -v jq >/dev/null 2>&1; then
        missing_deps+=("jq")
    fi

    if ! command -v unzip >/dev/null 2>&1; then
        missing_deps+=("unzip")
    fi

    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        error "缺少必要依赖: ${missing_deps[*]}"
        info "请安装缺少的依赖后重试"
        exit 1
    fi
}

# 创建临时目录
setup_temp_dir() {
    step "创建临时目录..."
    mkdir -p "$WORKDIR/tmp"
    success "临时目录创建完成"
}

# 下载 NimbOS
download_nimbos() {
    step "下载 NimbOS ($ARCH, 版本: $VERSION)..."

    # 构建 GitHub API URL
    if [[ "$VERSION" == "latest" ]]; then
        RELEASE_URL="https://api.github.com/repos/$REPO/releases/latest"
    else
        RELEASE_URL="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
    fi

    # 获取 asset 下载 URL
    ASSET_NAME="${ARCH}_usertests.zip"
    # 先尝试获取 asset 的信息（包括 browser_download_url, size, label, name）
    ASSET_JSON=$(curl -s "$RELEASE_URL" | jq -r ".assets[] | select(.name == \"$ASSET_NAME\") | {url:.browser_download_url, size:.size, name:.name, sha256:(.label // null)}")

    if [[ -z "$ASSET_JSON" || "$ASSET_JSON" == "null" ]]; then
        error "在版本 $VERSION 中未找到资源 $ASSET_NAME"
        exit 1
    fi

    ASSET_URL=$(echo "$ASSET_JSON" | jq -r '.url')
    ASSET_SIZE=$(echo "$ASSET_JSON" | jq -r '.size')
    ASSET_LABEL=$(echo "$ASSET_JSON" | jq -r '.sha256')

    # 判断是否需要下载：文件不存在，或 size/sha256 与远端不一致
    need_download=0
    if [[ ! -f "$ZIP_PATH" ]]; then
        need_download=1
        info "$ASSET_NAME 不存在，准备下载"
    else
        # 如果 release 的 label 中包含 sha256（我们尝试使用 label 字段存放 checksum），则优先比对 sha256
        if [[ "$ASSET_LABEL" != "null" && "$ASSET_LABEL" != "" ]]; then
            # 期望 label 中为 sha256:abcdef... 或 直接 sha256
            expected_sha="$ASSET_LABEL"
            # 如果 label 以 "sha256:" 开头，去掉前缀
            expected_sha=${expected_sha#sha256:}
            actual_sha=$(sha256sum "$ZIP_PATH" | awk '{print $1}' 2>/dev/null || true)
            if [[ "$actual_sha" != "$expected_sha" ]]; then
                info "$ASSET_NAME 本地 sha256 与发布不一致，准备重新下载"
                need_download=1
            else
                success "$ASSET_NAME 本地 sha256 校验通过，跳过下载"
            fi
        else
            # 回退为按文件大小比对（不可靠但可用）
            actual_size=$(stat -c%s "$ZIP_PATH" 2>/dev/null || true)
            if [[ "$actual_size" != "$ASSET_SIZE" ]]; then
                info "$ASSET_NAME 本地大小 ($actual_size) 与发布大小 ($ASSET_SIZE) 不一致，准备重新下载"
                need_download=1
            else
                success "$ASSET_NAME 本地大小匹配，跳过下载"
            fi
        fi
    fi

    if [[ $need_download -eq 1 ]]; then
        info "下载 $ASSET_NAME..."
        curl -L -o "$ZIP_PATH" "$ASSET_URL"
        success "NimbOS 下载完成"
    fi
}

# 解压 NimbOS
extract_nimbos() {
    step "解压 NimbOS..."

    rm -rf "$UNZIP_PATH"
    mkdir -p "$UNZIP_PATH"
    unzip "$ZIP_PATH" -d "$UNZIP_PATH"

    success "NimbOS 解压完成"
}

# 下载 BIOS (仅 x86_64)
download_bios() {
    # 只在 x86_64 架构下载 BIOS
    if [[ "$ARCH" != "x86_64" ]]; then
        info "非 x86_64 架构，跳过 BIOS 下载"
        return 0
    fi

    step "下载 BIOS (版本: $BIOS_VERSION)..."

    if [[ "$BIOS_VERSION" == "latest" ]]; then
        BIOS_RELEASE_URL="https://api.github.com/repos/$BIOS_REPO/releases/latest"
    else
        BIOS_RELEASE_URL="https://api.github.com/repos/$BIOS_REPO/releases/tags/$BIOS_VERSION"
    fi

    # 尝试从 release 里读取 asset 的信息（可能包含 size 或 label 用于 checksum）
    BIOS_ASSET_JSON=$(curl -s "$BIOS_RELEASE_URL" | jq -r ".assets[] | select(.name == \"axvm-bios.bin\") | {url:.browser_download_url, size:.size, sha256:(.label // null)}")

    if [[ -z "$BIOS_ASSET_JSON" || "$BIOS_ASSET_JSON" == "null" ]]; then
        error "未找到 BIOS 资源"
        exit 1
    fi

    BIOS_ASSET_URL=$(echo "$BIOS_ASSET_JSON" | jq -r '.url')
    BIOS_ASSET_SIZE=$(echo "$BIOS_ASSET_JSON" | jq -r '.size')
    BIOS_ASSET_LABEL=$(echo "$BIOS_ASSET_JSON" | jq -r '.sha256')

    need_download=0
    if [[ ! -f "$BIOS_PATH" ]]; then
        need_download=1
        info "BIOS 文件不存在，准备下载"
    else
        if [[ "$BIOS_ASSET_LABEL" != "null" && "$BIOS_ASSET_LABEL" != "" ]]; then
            expected_sha=${BIOS_ASSET_LABEL#sha256:}
            actual_sha=$(sha256sum "$BIOS_PATH" | awk '{print $1}' 2>/dev/null || true)
            if [[ "$actual_sha" != "$expected_sha" ]]; then
                info "BIOS 本地 sha256 与发布不一致，准备重新下载"
                need_download=1
            else
                success "BIOS 本地 sha256 校验通过，跳过下载"
            fi
        else
            actual_size=$(stat -c%s "$BIOS_PATH" 2>/dev/null || true)
            if [[ "$actual_size" != "$BIOS_ASSET_SIZE" ]]; then
                info "BIOS 本地大小 ($actual_size) 与发布大小 ($BIOS_ASSET_SIZE) 不一致，准备重新下载"
                need_download=1
            else
                success "BIOS 本地大小匹配，跳过下载"
            fi
        fi
    fi

    if [[ $need_download -eq 1 ]]; then
        info "下载 axvm-bios.bin..."
        curl -L -o "$BIOS_PATH" "$BIOS_ASSET_URL"
        success "BIOS 下载完成"
    fi
}

# 创建磁盘镜像
create_disk_image() {
    step "创建磁盘镜像: $DISK_PATH"

    if [[ ! -f "$WORKDIR/axvisor.sh" ]]; then
        error "axvisor.sh 脚本不存在，请确保脚本在正确位置"
        exit 1
    fi

    "$WORKDIR/axvisor.sh" disk_img --image "$WORKDIR/tmp/nimbos-${ARCH}.img"

    success "磁盘镜像创建完成"
}

# 挂载镜像并复制文件
mount_and_copy() {
    step "挂载镜像并复制文件..."

    sudo rm -rf "$WORKDIR/tmp/img"
    sudo mkdir -p "$WORKDIR/tmp/img"
    sudo chown -R root:root "$WORKDIR/tmp/img"
    sudo mount "$WORKDIR/tmp/nimbos-${ARCH}.img" "$WORKDIR/tmp/img"
    sudo cp "${UNZIP_PATH}/nimbos.bin" "$WORKDIR/tmp/img/nimbos-${ARCH}.bin"
    sudo chown -R root:root "$WORKDIR/tmp/img"
    sudo umount "$WORKDIR/tmp/img"

    success "文件复制完成"
}

# 清理临时文件
cleanup() {
    step "清理临时文件..."
    rm -rf  "$WORKDIR/tmp/img"
    success "清理完成"
}

# 主函数
main() {
    parse_args "$@"

    info "开始制作 NimbOS 镜像"
    info "架构: $ARCH"
    info "版本: $VERSION"
    info "输出路径: $DISK_PATH"
    info "工作目录: $WORKDIR"

    setup_temp_dir
    download_nimbos
    extract_nimbos
    download_bios
    create_disk_image
    mount_and_copy
    cleanup

    success "NimbOS 镜像制作完成: $DISK_PATH"
}

# 脚本入口点
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # 处理中断信号
    trap 'echo -e "\n${YELLOW}用户中断操作${NC}"; exit 130' INT

    # 执行主函数
    main "$@"
fi
