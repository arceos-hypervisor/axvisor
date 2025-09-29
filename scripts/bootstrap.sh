#!/bin/bash
# -*- coding: utf-8 -*-

# Axvisor Bootstrap Script
# This script creates a Python virtual environment and installs task.py dependencies

set -e  # Exit on error

# Colored output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Output helpers
info() { echo -e "${BLUE}ℹ${NC} $1"; }
success() { echo -e "${GREEN}✓${NC} $1"; }
warning() { echo -e "${YELLOW}⚠${NC} $1"; }
error() { echo -e "${RED}✗${NC} $1"; }

# Get project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# If already inside a virtual environment, just update deps
if [[ -n "$VIRTUAL_ENV" ]]; then
    info "Detected active virtual environment: $VIRTUAL_ENV"
    
    # Ensure requirements.txt exists
    if [[ ! -f "scripts/requirements.txt" ]]; then
    error "scripts/requirements.txt not found"
        exit 1
    fi
    
    # Install / update dependencies
    info "Updating Python dependencies..."
    pip install -q -r scripts/requirements.txt -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple
    success "Dependencies updated"
    exit 0
fi

# Virtual environment and marker file
VENV_DIR="venv"
MARKER_FILE="$VENV_DIR/.bootstrapped"
REQUIREMENTS_FILE="scripts/requirements.txt"

# Compute dependency hash
compute_dep_hash() {
    local pyver
    pyver=$(python3 --version 2>/dev/null || echo "unknown")
    if [[ -f "$REQUIREMENTS_FILE" ]]; then
        {
            echo "$pyver"
            grep -v '^#' "$REQUIREMENTS_FILE" | grep -v '^$' | sort
        } | sha256sum | awk '{print $1}'
    else
        echo "$pyver-no-requirements" | sha256sum | awk '{print $1}'
    fi
}

# Decide if bootstrap is needed
check_bootstrap_needed() {
    # Need bootstrap if venv directory missing
    if [[ ! -d "$VENV_DIR" ]]; then
    return 0  # need bootstrap
    fi
    
    # Need bootstrap if marker file missing
    if [[ ! -f "$MARKER_FILE" ]]; then
    return 0  # need bootstrap
    fi
    
    # Check dependency hash
    local existing_hash current_hash
    existing_hash=$(awk -F":" '/^hash:/ {print $2}' "$MARKER_FILE" 2>/dev/null | tr -d '[:space:]') || existing_hash=""
    current_hash=$(compute_dep_hash)
    
    if [[ "$existing_hash" != "$current_hash" ]]; then
    info "Dependency changes detected, re-bootstrap required"
    return 0  # need bootstrap
    fi
    
    # Ensure python executable exists in venv
    if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
    warning "Python in virtual env not available, re-bootstrap required"
    return 0  # need bootstrap
    fi
    
    return 1  # bootstrap not needed
}
# Fast path: already bootstrapped
if ! check_bootstrap_needed; then
    success "Bootstrap already done and dependencies unchanged, skipping"
    exit 0
fi

info "Starting Python virtual environment setup..."

# Check system dependencies
check_system_deps() {
    info "Checking system dependencies..."
    
    # Check python3 exists
    if ! command -v python3 >/dev/null 2>&1; then
    error "python3 not found. Please install Python 3"
        exit 1
    fi
    
    # Report Python version
    local pyver
    pyver=$(python3 --version 2>&1 | awk '{print $2}' | cut -d. -f1,2)
    info "Detected Python version: $pyver"
    
    # Check venv module
    if ! python3 -c "import venv" 2>/dev/null; then
    error "python3-venv module not found"
    echo "Install python3-venv via your package manager:"
    echo "  Ubuntu/Debian: sudo apt install python3-venv"
    echo "  CentOS/RHEL:   sudo yum install python3-venv"
    echo "  Fedora:        sudo dnf install python3-venv"
        exit 1
    fi
    
    # Check requirements.txt exists
    if [[ ! -f "$REQUIREMENTS_FILE" ]]; then
    error "$REQUIREMENTS_FILE not found"
        exit 1
    fi
    
    success "System dependency check passed"
}

# Create virtual environment
setup_venv() {
    info "Preparing virtual environment..."
    
    # Remove broken venv
    if [[ -d "$VENV_DIR" ]] && [[ ! -x "$VENV_DIR/bin/python3" ]]; then
    warning "Corrupted virtual environment detected, removing..."
        rm -rf "$VENV_DIR"
    fi
    
    # Create venv if missing
    if [[ ! -d "$VENV_DIR" ]]; then
    info "Creating new virtual environment..."
        python3 -m venv "$VENV_DIR"
    success "Virtual environment created"
    else
    info "Using existing virtual environment"
    fi
}

# Install dependencies
install_deps() {
    info "Installing Python dependencies..."
    
    # 激活虚拟环境
    source "$VENV_DIR/bin/activate"
    
    # Upgrade pip (quiet)
    python -m pip install -q --upgrade pip -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple
    
    # Install requirements
    pip install -q -r "$REQUIREMENTS_FILE" -i https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple
    
    success "Dependencies installed"
}

# Verify installation
verify_installation() {
    info "Verifying installation..."
    
    # 测试 task.py
    if source "$VENV_DIR/bin/activate" && python3 ./scripts/task.py --help >/dev/null 2>&1; then
    success "task.py runs correctly"
    else
    error "task.py execution failed"
        exit 1
    fi
}

# Write completion marker
write_marker() {
    local dep_hash
    dep_hash=$(compute_dep_hash)
    
    cat > "$MARKER_FILE" <<EOF
hash: $dep_hash
timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)
python_version: $(python3 --version)
EOF
    
    success "Bootstrap marker written (hash: ${dep_hash:0:8}...)"
}

# Main execution flow
main() {
    check_system_deps
    setup_venv
    install_deps
    verify_installation
    write_marker
    
    success "Virtual environment setup complete!"
    info "Activate with: source venv/bin/activate"
    info "Use 'make help' to see available commands"
}

# Execute main function
main "$@"
