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
.PHONY: build
build: setup
	@echo "==> Building project using $(VENV_PYTHON) ./scripts/task.py build"
	@$(VENV_PYTHON) ./scripts/task.py build

.PHONY: run
run: setup
	@echo "==> Running project using $(VENV_PYTHON) ./scripts/task.py run"
	@$(VENV_PYTHON) ./scripts/task.py run

.PHONY: clean
clean:
	@$(VENV_PYTHON) ./scripts/task.py clean
	@cargo clean

ARCH ?= aarch64

.PHONY: clippy
clippy:
	@$(VENV_PYTHON) ./scripts/task.py clippy --arch $(ARCH)