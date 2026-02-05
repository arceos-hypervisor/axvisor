#!/usr/bin/env bash
# Wrapper: delegates to unified setup_qemu.sh
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${SCRIPT_DIR}/setup_qemu.sh" arceos

