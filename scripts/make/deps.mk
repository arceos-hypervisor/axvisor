# linux kernel version
KVER ?= 6.8.0-78-generic
KDIR = /lib/modules/$(KVER)/build

BUILD_DIR = $(CURDIR)/build
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# shim
SHIM_DIR = $(CURDIR)/deps/shim
SHIM_BIN = $(SHIM_DIR)/shim.bin

$(SHIM_BIN):
	$(info Building shim)
	LOG=debug $(MAKE) -C $(SHIM_DIR)

shim: $(SHIM_BIN)

# axcli
AXCLI_DIR = $(CURDIR)/deps/axvisor-tools/axcli
AXCLI_BIN = $(AXCLI_DIR)/target/release/axcli

$(AXCLI_BIN):
	$(info Building axcli)
	cd $(AXCLI_DIR) && cargo build --release

axcli: $(AXCLI_BIN) $(BUILD_DIR)
	cp $(AXCLI_BIN) $(BUILD_DIR)/

# eqdriver
EQDRIVER_DIR = $(CURDIR)/deps/axvisor-tools/eqdriver
EQDRIVER_BIN = $(EQDRIVER_DIR)/eqdriver.ko

$(EQDRIVER_BIN):
	$(info --- Building eqdriver for kernel $(KVER))
	KDIR=$(KDIR) $(MAKE) -C $(EQDRIVER_DIR)

eqdriver: $(EQDRIVER_BIN) $(BUILD_DIR)
	cp $(EQDRIVER_BIN) $(BUILD_DIR)/

# jailhouse
JAILHOUSE_DIR = $(CURDIR)/deps/jailhouse-equation
jailhouse: $(BUILD_DIR)
	$(info --- Building jailhouse for kernel $(KVER))
	KDIR=$(KDIR) $(MAKE) -C $(JAILHOUSE_DIR)
	cp -r $(JAILHOUSE_DIR) $(BUILD_DIR)/

# guest scripts
guest_script: $(BUILD_DIR)
	cp $(CURDIR)/scripts/vmm/guest/* $(BUILD_DIR)/

# put them up
deps: shim axcli eqdriver jailhouse guest_script

.PHONY: shim axcli eqdriver jailhouse guest_script deps