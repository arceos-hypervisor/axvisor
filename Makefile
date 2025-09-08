## Makefile - convenience targets for developer setup

.PHONY: setup

HVCONFIG := .hvconfig.toml
DEFAULT_HVCONFIG := configs/def_hvconfig.toml

# Python executable inside venv if available, otherwise fallback
VENV_PYTHON := $(if $(wildcard venv/bin/python3),venv/bin/python3,python3)

# setup: prepare development environment
# - copy default hvconfig if .hvconfig.toml missing
# - run scripts/bootstrap.sh to create venv and install python deps
setup:
	@echo "==> Running project setup: copy hvconfig (if needed) and bootstrap"
	@if [ ! -f $(HVCONFIG) ]; then \
		if [ -f $(DEFAULT_HVCONFIG) ]; then \
			cp $(DEFAULT_HVCONFIG) $(HVCONFIG) && echo "Copied $(DEFAULT_HVCONFIG) -> $(HVCONFIG)"; \
		else \
			echo "Warning: $(DEFAULT_HVCONFIG) not found; skipping hvconfig copy"; \
		fi; \
	else \
		echo "$(HVCONFIG) already exists; skipping copy"; \
	fi
	@./scripts/bootstrap.sh

# build: build project using scripts/task.py via venv's python when available
# Usage: make build [ARGS="--plat aarch64-qemu-virt-hv --features feature1,feature2"]
# Examples:
#   make build
#   make build ARGS="--plat aarch64-qemu-virt-hv"
#   make build ARGS="--features irq,mem --arceos-features smp"
.PHONY: build
build: 
	@./scripts/bootstrap.sh
	@echo "==> Building project using $(VENV_PYTHON) ./scripts/task.py build $(ARGS)"
	@$(VENV_PYTHON) ./scripts/task.py build $(ARGS)

# run: run project using scripts/task.py via venv's python when available
# Usage: make run [ARGS="--plat aarch64-qemu-virt-hv --vmconfigs configs/vms/linux-qemu-aarch64.toml"]
# Examples:
#   make run
#   make run ARGS="--plat aarch64-qemu-virt-hv"
#   make run ARGS="--vmconfigs configs/vms/linux-qemu-aarch64.toml"
#   make run ARGS="--arceos-args DISK_IMG=disk.img,LOG=debug"
.PHONY: run
run: 
	@./scripts/bootstrap.sh
	@echo "==> Running project using $(VENV_PYTHON) ./scripts/task.py run $(ARGS)"
	@$(VENV_PYTHON) ./scripts/task.py run $(ARGS)

.PHONY: clean
clean:
	@./scripts/bootstrap.sh
	@$(VENV_PYTHON) ./scripts/task.py clean
	@cargo clean

ARCH ?= aarch64

.PHONY: clippy
clippy:
	@./scripts/bootstrap.sh
	@$(VENV_PYTHON) ./scripts/task.py clippy --arch $(ARCH)

DISK_IMG ?= disk.img

.PHONY: disk_img
disk_img:
	@./scripts/bootstrap.sh
	@$(VENV_PYTHON) ./scripts/task.py disk_img --image $(DISK_IMG)