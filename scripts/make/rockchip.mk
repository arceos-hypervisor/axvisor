rockchip: build
    # Check if tmp/extboot.img exists
	@if [ ! -f tmp/extboot.img ]; then \
		echo "Error: tmp/extboot.img does not exist." >&2; \
		exit 1; \
	fi
	
	@echo 'Making extboot.img'
	./tool/extboot.sh --kernel $(OUT_BIN) --img tmp/extboot.img
	sudo upgrade_tool di -boot  tmp/extboot.img
	sudo upgrade_tool rd
