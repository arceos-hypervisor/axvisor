#!/usr/bin/env bash
set -euo pipefail

# Simple helper to prepare ArceOS guest image for QEMU testing.
# It will:
# 1. Download the qemu_aarch64_arceos guest image (if not already present)
# 2. Patch the ArceOS QEMU VM config to point kernel_path to the downloaded image
# 3. Copy rootfs.img into the repository tmp/ directory so that QEMU config works out of the box

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_NAME="qemu_aarch64_arceos"
IMAGE_DIR="/tmp/axvisor/${IMAGE_NAME}"
VMCONFIG_PATH="${REPO_ROOT}/configs/vms/arceos-aarch64-qemu-smp1.toml"
ROOTFS_TARGET="${REPO_ROOT}/tmp/rootfs.img"

echo "[setup_qemu_arceos] AxVisor repo: ${REPO_ROOT}"

echo "[setup_qemu_arceos] Step 1: ensure guest image is downloaded..."
if [ ! -d "${IMAGE_DIR}" ]; then
  echo "  -> Image directory ${IMAGE_DIR} not found, downloading via cargo xtask image..."
  (cd "${REPO_ROOT}" && cargo xtask image download "${IMAGE_NAME}")
else
  echo "  -> Found existing image directory: ${IMAGE_DIR}"
fi

KERNEL_IMAGE="${IMAGE_DIR}/qemu-aarch64"
ROOTFS_IMAGE="${IMAGE_DIR}/rootfs.img"

if [ ! -f "${KERNEL_IMAGE}" ]; then
  echo "ERROR: kernel image not found at ${KERNEL_IMAGE}" >&2
  exit 1
fi

if [ ! -f "${ROOTFS_IMAGE}" ]; then
  echo "ERROR: rootfs image not found at ${ROOTFS_IMAGE}" >&2
  exit 1
fi

echo "[setup_qemu_arceos] Step 2: patch VM config kernel_path..."
if [ ! -f "${VMCONFIG_PATH}" ]; then
  echo "ERROR: VM config file not found at ${VMCONFIG_PATH}" >&2
  exit 1
fi

# The runtime loader expects an absolute path under /tmp/axvisor/...
ABS_KERNEL_PATH="/tmp/axvisor/${IMAGE_NAME}/qemu-aarch64"
sed -i 's|^kernel_path *=.*|kernel_path = "'"${ABS_KERNEL_PATH}"'"|' "${VMCONFIG_PATH}"
echo "  -> Updated kernel_path in ${VMCONFIG_PATH} to ${ABS_KERNEL_PATH}"

echo "[setup_qemu_arceos] Step 3: prepare rootfs for QEMU config..."
mkdir -p "${REPO_ROOT}/tmp"
cp "${ROOTFS_IMAGE}" "${ROOTFS_TARGET}"
echo "  -> Copied ${ROOTFS_IMAGE} -> ${ROOTFS_TARGET}"

cat <<EOF

[setup_qemu_arceos] Done.
You can now run the QEMU test with:

  cd ${REPO_ROOT}
  cargo xtask qemu \\
    --build-config configs/board/qemu-aarch64.toml \\
    --qemu-config .github/workflows/qemu-aarch64.toml \\
    --vmconfigs configs/vms/arceos-aarch64-qemu-smp1.toml

When you see 'Hello, world!' in the QEMU output and ostool reports a detected success pattern,
the local QEMU test environment is working correctly.

EOF

