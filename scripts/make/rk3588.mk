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

define upload_image
	@echo "Uploading image to RK3588..."
	cp $(OUT_IMG) /srv/tftp/axvisor
	@echo "Image uploaded to /srv/tftp/axvisor"
	@echo "You can now boot the image using the RK3588 board."
	@echo "Coping this command to uboot console:"
	@echo ""
	@echo 'setenv serverip 192.168.50.138;setenv ipaddr 192.168.50.8;tftp 0x00480000 192.168.50.138:axvisor;tftp 0x10000000 192.168.50.138:rk3588_dtb.bin;bootm 0x00480000 - 0x10000000;'
	@echo ""
endef