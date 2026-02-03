#!/usr/bin/env bash
set -euo pipefail

# Simple helper to prepare NimbOS guest image for QEMU x86_64 testing.
# It will:
# 1. Download the qemu_x86_64_nimbos guest image (if not already present)
# 2. Patch the NimbOS QEMU VM config to point kernel_path to the downloaded image
# 3. Copy rootfs.img into the repository tmp/ directory so that QEMU config works out of the box
#
# IMPORTANT: x86_64 AxVisor requires VT-x/VMX support. This test will FAIL on:
#   - WSL2 (no nested virtualization / no /dev/kvm)
#   - Environments without KVM acceleration
# Use a physical Linux machine or a VM with nested virtualization enabled.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_NAME="qemu_x86_64_nimbos"
IMAGE_DIR="/tmp/axvisor/${IMAGE_NAME}"
VMCONFIG_PATH="${REPO_ROOT}/configs/vms/nimbos-x86_64-qemu-smp1.toml"
ROOTFS_TARGET="${REPO_ROOT}/tmp/rootfs.img"

echo "[setup_qemu_nimbos] AxVisor repo: ${REPO_ROOT}"

echo "[setup_qemu_nimbos] Step 1: ensure guest image is downloaded..."
if [ ! -d "${IMAGE_DIR}" ]; then
  echo "  -> Image directory ${IMAGE_DIR} not found, downloading via cargo xtask image..."
  (cd "${REPO_ROOT}" && cargo xtask image download "${IMAGE_NAME}")
else
  echo "  -> Found existing image directory: ${IMAGE_DIR}"
fi

KERNEL_IMAGE="${IMAGE_DIR}/qemu-x86_64"
ROOTFS_IMAGE="${IMAGE_DIR}/rootfs.img"

if [ ! -f "${KERNEL_IMAGE}" ]; then
  echo "ERROR: kernel image not found at ${KERNEL_IMAGE}" >&2
  exit 1
fi

if [ ! -f "${ROOTFS_IMAGE}" ]; then
  echo "ERROR: rootfs image not found at ${ROOTFS_IMAGE}" >&2
  exit 1
fi

echo "[setup_qemu_nimbos] Step 2: patch VM config kernel_path..."
if [ ! -f "${VMCONFIG_PATH}" ]; then
  echo "ERROR: VM config file not found at ${VMCONFIG_PATH}" >&2
  exit 1
fi

ABS_KERNEL_PATH="/tmp/axvisor/${IMAGE_NAME}/qemu-x86_64"
sed -i 's|^kernel_path *=.*|kernel_path = "'"${ABS_KERNEL_PATH}"'"|' "${VMCONFIG_PATH}"
echo "  -> Updated kernel_path in ${VMCONFIG_PATH} to ${ABS_KERNEL_PATH}"

echo "[setup_qemu_nimbos] Step 3: inject kernel into rootfs (image_location=fs requires kernel inside disk)..."
mkdir -p "${REPO_ROOT}/tmp"
# Copy rootfs and inject kernel at /tmp/axvisor/qemu_x86_64_nimbos/qemu-x86_64 (path inside the disk)
cp "${ROOTFS_IMAGE}" "${ROOTFS_TARGET}"
MOUNT_POINT="${REPO_ROOT}/tmp/nimbos_mount"
mkdir -p "${MOUNT_POINT}"
INJECTED=0
# Try mount with offset=0 (whole-disk FAT)
if sudo mount -o loop,offset=0 "${ROOTFS_TARGET}" "${MOUNT_POINT}" 2>/dev/null; then
  sudo mkdir -p "${MOUNT_POINT}/tmp/axvisor/${IMAGE_NAME}"
  sudo cp "${KERNEL_IMAGE}" "${MOUNT_POINT}/tmp/axvisor/${IMAGE_NAME}/qemu-x86_64"
  sudo umount "${MOUNT_POINT}"
  INJECTED=1
  echo "  -> Injected kernel into rootfs at /tmp/axvisor/${IMAGE_NAME}/qemu-x86_64"
fi
# Try partition offset if whole-disk mount failed
if [ "$INJECTED" -eq 0 ] && sudo mount -o loop,offset=$((2048*512)) "${ROOTFS_TARGET}" "${MOUNT_POINT}" 2>/dev/null; then
  sudo mkdir -p "${MOUNT_POINT}/tmp/axvisor/${IMAGE_NAME}"
  sudo cp "${KERNEL_IMAGE}" "${MOUNT_POINT}/tmp/axvisor/${IMAGE_NAME}/qemu-x86_64"
  sudo umount "${MOUNT_POINT}"
  INJECTED=1
  echo "  -> Injected kernel into rootfs (partition offset)"
fi
# Fallback: guestmount
if [ "$INJECTED" -eq 0 ] && command -v guestmount &>/dev/null; then
  if guestmount -a "${ROOTFS_TARGET}" -m /dev/sda1 "${MOUNT_POINT}" 2>/dev/null; then
    mkdir -p "${MOUNT_POINT}/tmp/axvisor/${IMAGE_NAME}"
    cp "${KERNEL_IMAGE}" "${MOUNT_POINT}/tmp/axvisor/${IMAGE_NAME}/qemu-x86_64"
    guestunmount "${MOUNT_POINT}"
    INJECTED=1
    echo "  -> Injected kernel via guestmount"
  fi
fi
rmdir "${MOUNT_POINT}" 2>/dev/null || true
if [ "$INJECTED" -eq 0 ]; then
  echo "  -> ERROR: Could not inject kernel into rootfs. Need sudo mount or guestmount."
  echo "     Install: sudo apt install libguestfs-tools"
  echo "     AxVisor (image_location=fs) expects kernel at /tmp/axvisor/${IMAGE_NAME}/qemu-x86_64 inside rootfs."
  exit 1
fi

cat <<EOF

[setup_qemu_nimbos] Done.
You can now run the QEMU test with:

  cd ${REPO_ROOT}
  cargo xtask qemu \\
    --build-config configs/board/qemu-x86_64.toml \\
    --qemu-config .github/workflows/qemu-x86_64.toml \\
    --vmconfigs configs/vms/nimbos-x86_64-qemu-smp1.toml

When you see 'usertests passed!' in the QEMU output and ostool reports a detected success pattern,
the NimbOS guest QEMU test is working correctly.

*** REQUIREMENT: This test requires VT-x/VMX support and KVM. It will FAIL on WSL2.
    On VMX-capable hosts, use:
      --qemu-config .github/workflows/qemu-x86_64-kvm.toml
    See draft/docs/x86_64-NimbOS验证说明.md for details.

EOF
