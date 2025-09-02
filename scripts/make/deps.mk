# linux kernel version
KVER ?= $(shell uname -r)
KDIR = /lib/modules/$(KVER)/build
NPROC ?= $(shell nproc)

BUILD_DIR = $(CURDIR)/build
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# shim
SHIM_DIR = $(CURDIR)/deps/shim
SHIM_BIN = $(SHIM_DIR)/shim.bin

shim:
	$(info Building shim)
	$(MAKE) PLATFORM=axvisor -C $(SHIM_DIR)
# axcli
AXCLI_DIR = $(CURDIR)/deps/axvisor-tools/axcli
AXCLI_BIN = $(AXCLI_DIR)/target/release/axcli

axcli: $(BUILD_DIR)
	$(info Building axcli)
	cd $(AXCLI_DIR) && unset RUSTFLAGS && cargo build --release
	cp $(AXCLI_BIN) $(BUILD_DIR)/

# eqdriver
EQDRIVER_DIR = $(CURDIR)/deps/axvisor-tools/eqdriver
EQDRIVER_BIN = $(EQDRIVER_DIR)/eqdriver.ko

eqdriver: $(BUILD_DIR)
	$(info --- Building eqdriver for kernel $(KVER))
	KDIR=$(KDIR) $(MAKE) -j $(NPROC) -C $(EQDRIVER_DIR)
	cp $(EQDRIVER_BIN) $(BUILD_DIR)/

# jailhouse
JAILHOUSE_DIR = $(CURDIR)/deps/jailhouse-equation
JAILHOUSE_DRIVER = $(JAILHOUSE_DIR)/driver/jailhouse.ko
JAILHOUSE_TOOL = $(JAILHOUSE_DIR)/tools/jailhouse
jailhouse: $(BUILD_DIR)
	$(info --- Building jailhouse for kernel $(KVER))
	KDIR=$(KDIR) $(MAKE) -j $(NPROC) -C $(JAILHOUSE_DIR)
	cp $(JAILHOUSE_DRIVER) $(JAILHOUSE_TOOL) $(BUILD_DIR)/

# guest scripts
guest_script: $(BUILD_DIR)
	cp $(CURDIR)/scripts/vmm/guest/* $(BUILD_DIR)/

# put them up
deps: shim axcli eqdriver jailhouse guest_script

.PHONY: shim axcli eqdriver jailhouse guest_script deps