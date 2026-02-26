#!/usr/bin/env bash
set -euo pipefail

# Unified QEMU guest setup script for AxVisor testing.
# Usage:
#   ./scripts/setup_qemu.sh [--guest] <guest>
#   ./scripts/setup_qemu.sh arceos
#   ./scripts/setup_qemu.sh --guest linux
#   ./scripts/setup_qemu.sh nimbos
#
# Supported guests: arceos, linux, nimbos

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  echo "Usage: $0 [--guest] <arceos|linux|nimbos>"
  echo ""
  echo "  arceos  - aarch64 ArceOS guest"
  echo "  linux   - aarch64 Linux guest"
  echo "  nimbos  - x86_64 NimbOS guest (requires VT-x/KVM)"
  echo ""
  echo "Examples:"
  echo "  $0 arceos"
  echo "  $0 --guest linux"
  exit 1
}

GUEST=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --guest)
      shift
      [[ $# -gt 0 ]] || usage
      GUEST="$1"
      shift
      break
      ;;
    arceos|linux|nimbos)
      GUEST="$1"
      shift
      break
      ;;
    -h|--help)
      usage
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      ;;
  esac
done

[[ -n "${GUEST}" ]] || usage

# Guest configuration: image_name|vmconfig|build_config|qemu_config|kernel_file|success_msg
case "$GUEST" in
  arceos)  CFG="qemu_aarch64_arceos|arceos-aarch64-qemu-smp1.toml|qemu-aarch64.toml|qemu-aarch64.toml|qemu-aarch64|Hello, world!" ;;
  linux)   CFG="qemu_aarch64_linux|linux-aarch64-qemu-smp1.toml|qemu-aarch64.toml|qemu-aarch64.toml|qemu-aarch64|test pass!" ;;
  nimbos)  CFG="qemu_x86_64_nimbos|nimbos-x86_64-qemu-smp1.toml|qemu-x86_64.toml|qemu-x86_64-kvm.toml|qemu-x86_64|usertests passed!" ;;
  *)       echo "Unknown guest: $GUEST" >&2; usage ;;
esac

IFS='|' read -r IMAGE_NAME VMCONFIG BUILD_CONFIG QEMU_CONFIG KERNEL_FILE SUCCESS_MSG <<< "$CFG"
# NOTE:
#  - `cargo xtask image download` 默认把镜像解压到 `/tmp/.axvisor-images/<IMAGE_NAME>`
#  - 这里直接使用该目录作为镜像来源，避免路径不一致
IMAGE_DIR="/tmp/.axvisor-images/${IMAGE_NAME}"
VMCONFIG_PATH="${REPO_ROOT}/configs/vms/${VMCONFIG}"
ROOTFS_TARGET="${REPO_ROOT}/tmp/rootfs.img"
KERNEL_IMAGE="${IMAGE_DIR}/${KERNEL_FILE}"
ROOTFS_IMAGE="${IMAGE_DIR}/rootfs.img"
ABS_KERNEL_PATH="${IMAGE_DIR}/${KERNEL_FILE}"

echo "[setup_qemu] Guest: ${GUEST} | Repo: ${REPO_ROOT}"

echo "[setup_qemu] Step 1: ensure guest image is downloaded..."
if [ ! -d "${IMAGE_DIR}" ]; then
  echo "  -> Image directory ${IMAGE_DIR} not found, downloading via cargo xtask image..."
  (cd "${REPO_ROOT}" && cargo xtask image download "${IMAGE_NAME}")
else
  echo "  -> Found existing image directory: ${IMAGE_DIR}"
fi

if [ ! -f "${KERNEL_IMAGE}" ]; then
  echo "ERROR: kernel image not found at ${KERNEL_IMAGE}" >&2
  exit 1
fi

if [ ! -f "${ROOTFS_IMAGE}" ]; then
  echo "ERROR: rootfs image not found at ${ROOTFS_IMAGE}" >&2
  exit 1
fi

# NimbOS x86_64 requires axvm-bios for bootstrapping
if [[ "$GUEST" == "nimbos" ]]; then
  BIOS_IMAGE="${IMAGE_DIR}/axvm-bios.bin"
  if [ ! -f "${BIOS_IMAGE}" ]; then
    echo "ERROR: axvm-bios.bin not found at ${BIOS_IMAGE}" >&2
    echo "  -> Please re-download the NimbOS image via 'cargo xtask image download qemu_x86_64_nimbos'." >&2
    exit 1
  fi
fi

echo "[setup_qemu] Step 2: patch VM config kernel_path..."
if [ ! -f "${VMCONFIG_PATH}" ]; then
  echo "ERROR: VM config file not found at ${VMCONFIG_PATH}" >&2
  exit 1
fi

sed -i 's|^kernel_path *=.*|kernel_path = "'"${ABS_KERNEL_PATH}"'"|' "${VMCONFIG_PATH}"
echo "  -> Updated kernel_path in ${VMCONFIG_PATH} to ${ABS_KERNEL_PATH}"

if [[ "$GUEST" == "nimbos" ]]; then
  ABS_BIOS_PATH="${IMAGE_DIR}/axvm-bios.bin"
  sed -i 's|^bios_path *=.*|bios_path = "'"${ABS_BIOS_PATH}"'"|' "${VMCONFIG_PATH}"
  echo "  -> Updated bios_path in ${VMCONFIG_PATH} to ${ABS_BIOS_PATH}"
fi

echo "[setup_qemu] Step 3: prepare rootfs..."
mkdir -p "${REPO_ROOT}/tmp"
cp "${ROOTFS_IMAGE}" "${ROOTFS_TARGET}"

echo "  -> Copied ${ROOTFS_IMAGE} -> ${ROOTFS_TARGET}"

cat <<EOF

[setup_qemu] Done. Guest: ${GUEST}
You can now run the QEMU test with:

  cd ${REPO_ROOT}
  cargo xtask qemu \\
    --build-config configs/board/${BUILD_CONFIG} \\
    --qemu-config .github/workflows/${QEMU_CONFIG} \\
    --vmconfigs configs/vms/${VMCONFIG}

Success indicator: '${SUCCESS_MSG}'

EOF

if [[ "$GUEST" == "nimbos" ]]; then
  echo "*** NimbOS requires VT-x/VMX and KVM. It will FAIL on WSL2."
  echo ""
fi
