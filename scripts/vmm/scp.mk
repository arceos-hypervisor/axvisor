.PHONY: scp_qemu scp_hw

BINARY_NAME ?= evm-intel.bin

PORT ?= 2334

HW_USER ?= os
HW_IP ?= 123.123.123.123

define scp_qemu
	scp -P $(PORT) $(OUT_BIN) ubuntu@localhost:~/$(BINARY_NAME)
endef

define scp_hw
	scp  $(OUT_BIN) $(HW_USER)@$(HW_IP):~/$(BINARY_NAME)
endef