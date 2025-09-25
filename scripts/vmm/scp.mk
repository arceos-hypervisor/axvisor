.PHONY: scp_qemu scp_hw

PORT ?= 2334

HW_USER ?= os
HW_IP ?= 123.123.123.123

define scp_qemu
	scp -P $(PORT) -r $(BUILD_DIR)/axcli $(BUILD_DIR)/evm-intel.bin deps/jailhouse-equation ubuntu@localhost:~/
endef

define scp_hw
	scp $(BUILD_DIR)/axcli $(BUILD_DIR)/evm-intel.bin $(HW_USER)@$(HW_IP):~/
endef