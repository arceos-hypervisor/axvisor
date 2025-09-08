
# Prefer venv python if available, otherwise fall back to system python3
VENV_PY := venv/bin/python
PYTHON := $(if $(wildcard $(VENV_PY)),$(VENV_PY),$(or $(PYTHON),python3))
TASK := $(PYTHON) -m scripts.task
# Run bootstrap.sh unconditionally; script will quickly return if up-to-date
BOOTSTRAP_CHECK := ./scripts/bootstrap.sh

# PASS_ARGS collects extra words after the target and the ARGS variable.
# Use the `--` separator to stop make option parsing and pass raw flags,
# e.g.:
#   make clippy -- --arch x86_64
# or
#   make clippy ARGS="--arch x86_64"
PASS_ARGS := $(MAKECMDGOALS) $(ARGS)

.PHONY: help task setup build run clippy clean disk_img

help:
	@echo "Usage: make <target> -- Targets: setup build run clippy clean disk_img help"
	@echo "You can also run: make task ARGS=\"build --release\""

task:
	@$(BOOTSTRAP_CHECK); $(TASK) $(filter-out $@,$(PASS_ARGS))

setup:
	@$(BOOTSTRAP_CHECK); $(TASK) setup $(filter-out $@,$(PASS_ARGS))

build:
	@$(BOOTSTRAP_CHECK); $(TASK) build $(filter-out $@,$(PASS_ARGS))

run:
	@$(BOOTSTRAP_CHECK); $(TASK) run $(filter-out $@,$(PASS_ARGS))

clippy:
	@$(BOOTSTRAP_CHECK); $(TASK) clippy $(filter-out $@,$(PASS_ARGS))

clean:
	@$(BOOTSTRAP_CHECK); $(TASK) clean $(filter-out $@,$(PASS_ARGS)); cargo clean

disk_img:
	@$(BOOTSTRAP_CHECK); $(TASK) disk_img $(filter-out $@,$(PASS_ARGS)) $(IMGARGS)

# Allow passing extra command-line words after the target, e.g.
#   make clippy --arch x86_64
# MAKECMDGOALS will contain the extra words; provide a no-op pattern
# rule so make doesn't error on unknown goals. The target recipes
# forward those extra words to the underlying Python task.
%:
	@:
