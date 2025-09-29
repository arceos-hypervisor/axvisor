#!/bin/bash
# -*- coding: utf-8 -*-

# Axvisor unified management script
# Replaces the Makefile, providing complete project management functionality

set -e

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"
HVCONFIG="$PROJECT_ROOT/.hvconfig.toml"
DEFAULT_HVCONFIG="$PROJECT_ROOT/configs/def_hvconfig.toml"
VENV_DIR="$PROJECT_ROOT/venv"
VENV_MARKER="$VENV_DIR/.bootstrapped"
REQUIREMENTS="$PROJECT_ROOT/scripts/requirements.txt"

# Switch to project root
cd "$PROJECT_ROOT"

# Output helper functions - unified emoji style
info() { echo -e "${BLUE}ℹ️${NC} $*"; }
success() { echo -e "${GREEN}✅${NC} $*"; }
warning() { echo -e "${YELLOW}⚠️${NC} $*"; }
error() { echo -e "${RED}❌${NC} $*"; }
step() { echo -e "${CYAN}🚀${NC} $*"; }
debug() { echo -e "${CYAN}🔍${NC} $*"; }

# Error handling
handle_error() {
    error "命令失败: $1"
    exit 1
}

trap 'handle_error "Script interrupted"' ERR

# Check system dependencies
check_system_deps() {
    local missing_deps=()
    
    # Check Python 3
    if ! command -v python3 >/dev/null 2>&1; then
        missing_deps+=("python3")
    fi
    
    # Check Cargo
    if ! command -v cargo >/dev/null 2>&1; then
        missing_deps+=("cargo")
    fi
    
    if [[ ${#missing_deps[@]} -gt 0 ]]; then
    error "Missing required dependencies: ${missing_deps[*]}"
    info "Install the missing dependencies and retry"
        exit 1
    fi
}

# Determine whether venv setup is needed
needs_venv_setup() {
    # Virtual environment directory does not exist
    if [[ ! -d "$VENV_DIR" ]]; then
        return 0
    fi
    
    # Python executable missing inside venv
    if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
        return 0
    fi
    
    # requirements.txt is newer than the bootstrap marker
    if [[ "$REQUIREMENTS" -nt "$VENV_MARKER" ]]; then
        return 0
    fi
    
    return 1
}

# Setup virtual environment
setup_venv() {
    if ! needs_venv_setup; then
        return 0
    fi
    
    step "Setting up Python virtual environment..."
    
    # 运行 bootstrap 脚本
    ./scripts/bootstrap.sh
    
    success "Virtual environment ready"
}

# Config file management
setup_defconfig() {
    step "Setting default config..."
    
    if [[ ! -f "$DEFAULT_HVCONFIG" ]]; then
    error "Default config file $DEFAULT_HVCONFIG not found"
        exit 1
    fi
    
    if [[ -f "$HVCONFIG" ]]; then
    warning "Config file $HVCONFIG already exists"
    read -p "Overwrite existing config? (y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            info "Operation cancelled"
            return 0
        fi
    fi
    
    cp "$DEFAULT_HVCONFIG" "$HVCONFIG"
    success "Copied $DEFAULT_HVCONFIG -> $HVCONFIG"
    
    info "Config file setup completed"
    info "Edit $HVCONFIG to customize settings"
}

# Ensure config file exists (silent)
ensure_config() {
    if [[ ! -f "$HVCONFIG" ]]; then
        if [[ -f "$DEFAULT_HVCONFIG" ]]; then
            info "Auto copying default config file..."
            cp "$DEFAULT_HVCONFIG" "$HVCONFIG"
            success "Copied $DEFAULT_HVCONFIG -> $HVCONFIG"
        else
            warning "Default config file $DEFAULT_HVCONFIG not found"
            warning "Run './axvisor.sh defconfig' first to create it"
        fi
    fi
}

# Run Python task (unified entry point)
run_python_task() {
    local cmd="$1"
    shift
    
    # Check if help flag requested
    for arg in "$@"; do
        if [[ "$arg" == "--help" || "$arg" == "-h" ]]; then
            step "Showing help for $cmd..."
            setup_venv
            source "$VENV_DIR/bin/activate"
            python3 scripts/task.py "$cmd" --help
            return $?
        fi
    done
    
    # Smart argument parsing based on command
    case "$cmd" in
        "clippy")
            parse_clippy_args "$@"
            ;;
        "disk_img")
            parse_disk_img_args "$@"
            ;;
        "build")
            parse_build_args "$@"
            ;;
        "run")
            parse_run_args "$@"
            ;;
        *)
            # Other commands: pass all args
            step "Executing command $cmd..."
            if [[ $# -gt 0 ]]; then
                debug "Args: $*"
            fi
            setup_venv
            source "$VENV_DIR/bin/activate"
            python3 scripts/task.py "$cmd" "$@"
            ;;
    esac
}

# Parse clippy command arguments
parse_clippy_args() {
    local arch="aarch64"  # default arch
    local extra_args=()
    
    # Parse args
    while [[ $# -gt 0 ]]; do
        case $1 in
            --arch)
                arch="$2"
                shift 2
                ;;
            *)
                # First positional arg (without --arch) is treated as architecture
                if [[ ${#extra_args[@]} -eq 0 && "$1" != -* ]]; then
                    arch="$1"
                    shift
                else
                    extra_args+=("$1")
                    shift
                fi
                ;;
        esac
    done
    
    step "Running clippy (arch: $arch)..."
    if [[ ${#extra_args[@]} -gt 0 ]]; then
    debug "Extra args: ${extra_args[*]}"
    fi
    
    setup_venv
    source "$VENV_DIR/bin/activate"
    python3 scripts/task.py clippy --arch "$arch" "${extra_args[@]}"
}

# Parse disk_img command arguments
parse_disk_img_args() {
    local image="disk.img"  # default image name
    local extra_args=()
    
    # Parse args
    while [[ $# -gt 0 ]]; do
        case $1 in
            --image)
                image="$2"
                shift 2
                ;;
            *)
                # First positional arg (without --image) is image name
                if [[ ${#extra_args[@]} -eq 0 && "$1" != -* ]]; then
                    image="$1"
                    shift
                else
                    extra_args+=("$1")
                    shift
                fi
                ;;
        esac
    done
    
    step "Creating disk image: $image"
    if [[ ${#extra_args[@]} -gt 0 ]]; then
    debug "Extra args: ${extra_args[*]}"
    fi
    
    setup_venv
    source "$VENV_DIR/bin/activate"
    python3 scripts/task.py disk_img --image "$image" "${extra_args[@]}"
}

# Parse build command arguments
parse_build_args() {
    step "Building project..."
    if [[ $# -gt 0 ]]; then
    debug "Build args: $*"
    fi
    
    setup_venv
    source "$VENV_DIR/bin/activate"
    python3 scripts/task.py build "$@"
}

# Parse run command arguments
parse_run_args() {
    step "Running project..."
    if [[ $# -gt 0 ]]; then
    debug "Run args: $*"
    fi
    
    setup_venv
    source "$VENV_DIR/bin/activate"
    python3 scripts/task.py run "$@"
}

# Show help information
show_help() {
    echo -e "${CYAN}🔧 Axvisor Project Management Tool${NC}"
    echo
    echo -e "${YELLOW}📋 Usage:${NC} $0 <command> [args...]"
    echo
    echo -e "${YELLOW}🛠️ Environment:${NC}"
    echo "  setup           - 🚀 Setup development environment"
    echo "  defconfig       - ⚙️ Copy default config file"
    echo "  check-deps      - ✅ Check system dependencies"
    echo "  rebuild-venv    - 🔄 Force rebuild virtual environment"
    echo "  dev-env         - 🔧 Development environment helper"
    echo
    echo -e "${YELLOW}🔨 Build:${NC}"
    echo "  build [args]    - 🏗️ Build project (args passthrough)"
    echo "  clean [args]    - 🧹 Clean build artifacts"
    echo "  clippy [args]   - 🔍 Run clippy lint (supports --arch)"
    echo
    echo -e "${YELLOW}▶️ Run:${NC}"
    echo "  run [args]      - 🚀 Run project (args passthrough)"
    echo "  disk_img [args] - 💾 Create disk image (supports --image)"
    echo
    echo -e "${YELLOW}ℹ️ Info:${NC}"
    echo "  status          - 📊 Show project status"
    echo "  version         - 📦 Show version information"
    echo "  help            - ❓ Show this help"
    echo
    echo -e "${YELLOW}🎯 Advanced:${NC}"
    echo "  • All commands support --help"
    echo "  • Arguments passed directly to task.py"
    echo "  • Smart argument parsing (legacy/new)"
    echo
    echo -e "${YELLOW}📚 Build examples:${NC}"
    echo "  $0 build --plat aarch64-qemu-virt-hv"
    echo "  $0 build --plat aarch64-generic --features fs"
    echo "  $0 clippy --arch aarch64"
    echo
    echo -e "${YELLOW}🎮 Run examples:${NC}"
    echo "  $0 run --plat aarch64-qemu-virt-hv"
    echo "  $0 run --vmconfigs configs/vms/linux-qemu-aarch64.toml"
}

# Show project status
show_status() {
    step "Project status"
    
    echo "Project root: $PROJECT_ROOT"
    echo "Config file: $([ -f "$HVCONFIG" ] && echo "✓ Present" || echo "✗ Missing")"
    echo "Virtual env: $([ -d "$VENV_DIR" ] && echo "✓ Present" || echo "✗ Missing")"
    
    if [[ -f "$VENV_MARKER" ]]; then
    echo "Env status: ✓ Initialized"
    local timestamp=$(grep "timestamp:" "$VENV_MARKER" 2>/dev/null | cut -d' ' -f2- || echo "unknown")
    echo "Initialized time: $timestamp"
    else
    echo "Env status: ✗ Not initialized"
    fi
    
    # Check system dependencies
    echo "System deps:"
    command -v python3 >/dev/null 2>&1 && echo "  Python3: ✓" || echo "  Python3: ✗"
    command -v cargo >/dev/null 2>&1 && echo "  Cargo: ✓" || echo "  Cargo: ✗"
    
    # Show latest build artifact timestamp
    if [[ -f "axvisor-dev_aarch64-generic.bin" ]]; then
        local build_time=$(stat -c %y "axvisor-dev_aarch64-generic.bin" 2>/dev/null | cut -d' ' -f1,2)
    echo "Latest build: $build_time"
    fi
}

# Show version information
show_version() {
    echo "Axvisor management script v2.0"
    echo "Project: axvisor-dev"
    echo "Branch: $(git branch --show-current 2>/dev/null || echo "unknown")"
    echo "Commit: $(git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
}

# Force rebuild virtual environment
rebuild_venv() {
    step "Force rebuilding virtual environment..."
    
    if [[ -d "$VENV_DIR" ]]; then
    warning "Removing existing virtual environment..."
        rm -rf "$VENV_DIR"
    fi
    
    setup_venv
    success "Virtual environment rebuilt"
}

# Setup full development environment
setup_environment() {
    step "Setting up development environment..."
    check_system_deps
    setup_venv
    success "Development environment setup completed"
}

# Main command dispatcher
main() {
    local cmd="${1:-help}"
    shift || true  # 移除第一个参数，剩余参数传递给子命令
    
    case "$cmd" in
    # Help & info
        "help"|"-h"|"--help")
            show_help
            ;;
        "version"|"-v"|"--version")
            show_version
            ;;
        "status")
            show_status
            ;;
            
    # Environment management
        "setup")
            setup_environment
            ;;
        "defconfig")
            setup_defconfig
            ;;
        "check-deps")
            check_system_deps
            success "所有系统依赖已满足"
            ;;
        "rebuild-venv")
            rebuild_venv
            ;;
            
    # Build & development commands
        "build")
            run_python_task build "$@"
            ;;
        "clean")
            run_python_task clean "$@"
            # Additionally clean cargo artifacts
            if command -v cargo >/dev/null 2>&1; then
                step "Cleaning Cargo build artifacts..."
                cargo clean
            fi
            success "Clean completed"
            ;;
        "clippy")
            run_python_task clippy "$@"
            ;;
        "run")
            run_python_task run "$@"
            ;;
        "disk_img")
            run_python_task disk_img "$@"
            ;;
        "dev-env")
            step "Setting up development environment..."
            setup_venv
            source "$VENV_DIR/bin/activate"
            python3 scripts/dev_env.py "$@"
            ;;
            
    # Unknown command
        *)
            error "Unknown command: $cmd"
            info "Use '$0 help' to list available commands"
            exit 1
            ;;
    esac
}

# Script entry point
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # Handle interrupt signal
    trap 'echo -e "\n${YELLOW}User cancelled operation${NC}"; exit 130' INT
    
    # Execute main function
    main "$@"
fi
