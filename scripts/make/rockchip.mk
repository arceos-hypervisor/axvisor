GITHUB_URL = https://github.com/arceos-hypervisor/platform_tools/releases/download/latest/rockchip.zip

MKIMG_FILE = tools/rockchip/mk_boot_img.py
ZIP_FILE = tools/rockchip.zip

check-download-rockchip:
ifeq ("$(wildcard $(MKIMG_FILE))","")
	@if [ ! -f "$(ZIP_FILE)" ]; then \
		echo "Downloading from $(GITHUB_URL)..."; \
		wget -O $(ZIP_FILE) $(GITHUB_URL); \
	fi
	@echo "Unzipping..."
	@unzip -o $(ZIP_FILE) -d tools
endif

rockchip: check-download-rockchip build
	# todo: add bsp select
	$(MKIMG_FILE) --bsp rk356x/rk3568-firefly-roc-pc-se --kernel $(OUT_BIN) --dst  $(OUT_DIR)/tmp
	@echo 'Built the FIT-uImage boot.img'
	sudo upgrade_tool di -boot  tmp/boot.img
