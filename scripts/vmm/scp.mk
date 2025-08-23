.PHONY: scp_qemu scp_hw

PORT ?= 2334

HW_USER ?= os
HW_IP ?= 123.123.123.123

define scp_qemu
	scp -P $(PORT) -r $(BUILD_DIR)/* ubuntu@localhost:~/
endef

define scp_hw
	scp -r $(BUILD_DIR)/* $(HW_USER)@$(HW_IP):~/
endef