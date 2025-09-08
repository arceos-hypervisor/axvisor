#!/usr/bin/env bash
set -euo pipefail
# make.sh - 简易项目入口脚本
# 功能：可选择性运行 bootstrap，然后将参数透传给 scripts/task.py

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"

usage() {
    cat <<EOF
Usage: make.sh [--no-bootstrap] [--force-bootstrap] [--bootstrap-only] [--] <task.py args...>

Options:
  --no-bootstrap      Skip running scripts/bootstrap.sh
  --force-bootstrap   Force running bootstrap even if marker says done
  --bootstrap-only    Only run bootstrap and exit
  -h, --help          Show this help message

Any remaining arguments are forwarded to: python3 -m scripts.task
EOF
}

NO_BOOTSTRAP=0
FORCE_BOOTSTRAP=0
BOOTSTRAP_ONLY=0

POSITIONAL=()
known_cmds=("setup" "build" "run" "clippy" "clean" "disk_img")
is_known() {
    local v="$1"
    for k in "${known_cmds[@]}"; do
        [[ "$k" == "$v" ]] && return 0
    done
    return 1
}

while [[ $# -gt 0 ]]; do
    # If this is a non-option (doesn't start with '-') and matches a known
    # subcommand, stop parsing and forward the rest as task args.
    if [[ "${1:0:1}" != "-" ]]; then
        if is_known "$1"; then
            POSITIONAL+=("$@")
            break
        else
            # Not a known subcommand: treat as a positional and continue parsing
            POSITIONAL+=("$1")
            shift
            continue
        fi
    fi

    case "$1" in
        --no-bootstrap)
            NO_BOOTSTRAP=1; shift ;;
        --force-bootstrap)
            FORCE_BOOTSTRAP=1; shift ;;
        --bootstrap-only)
            BOOTSTRAP_ONLY=1; shift ;;
        -h|--help)
            usage; exit 0 ;;
        --)
            shift; POSITIONAL+=("$@"); break ;;
        --*)
            # unknown long option -> forward to task.py
            POSITIONAL+=("$1"); shift ;;
        *)
            POSITIONAL+=("$1"); shift ;;
    esac
done

set -- "${POSITIONAL[@]}"

cd "$PROJECT_ROOT"

# Decide whether to run bootstrap
if [[ "$NO_BOOTSTRAP" -eq 1 ]]; then
    echo "bootstrap: skipped by --no-bootstrap"
else
    if [[ "$FORCE_BOOTSTRAP" -eq 1 ]]; then
        echo "bootstrap: forcing bootstrap (--force-bootstrap)"
        bash scripts/bootstrap.sh || { echo "bootstrap failed"; exit 1; }
    else
        echo "bootstrap: running (will skip internally if up-to-date)"
        bash scripts/bootstrap.sh || { echo "bootstrap failed"; exit 1; }
    fi
fi

if [[ "$BOOTSTRAP_ONLY" -eq 1 ]]; then
    echo "--bootstrap-only specified: exiting after bootstrap"
    exit 0
fi

# If we reach here, forward all args to task.py as a module
echo "forwarding to: python3 -m scripts.task $*"
exec python3 -m scripts.task "$@"
