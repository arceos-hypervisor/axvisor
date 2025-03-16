.PHONY: scp_bin

PORT ?= 2334

define scp_bin
	scp -P $(PORT) $(OUT_BIN) ubuntu@localhost:~/evm-intel.bin
endef