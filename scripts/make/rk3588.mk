RK3588_GITHUB_URL = https://github.com/arceos-hypervisor/platform_tools/releases/download/latest/rk3588.zip
RK3588_MKIMAGE = ./tools/rk3588/mkimage

OUT_IMG := $(OUT_DIR)/$(APP_NAME)_$(PLAT_NAME).img

.PHONY: build_image

build_image: build
ifeq ($(wildcard $(RK3588_MKIMAGE)),)
		@echo "file not found, downloading from $(RK3588_GITHUB_URL)..."; 
		wget $(RK3588_GITHUB_URL); 
		unzip -o rk3588.zip -d tools; 
		rm rk3588.zip;
endif
	$(RK3588_MKIMAGE) -n axvisor -A arm64 -O linux -T kernel -C none -a 0x00480000 -e 0x00480000 -d $(OUT_BIN) $(OUT_IMG)
	@echo 'Built the uboot image ${OUT_IMG} successfully!'