.PHONY: r flash

target_dir=$(shell cargo metadata --format-version=1 | python3 -c "import sys, json; print(json.load(sys.stdin)['target_directory'])")
# target_dir=$(shell cargo metadata --format-version=1 | grep -o '"target_directory":"[^"]*"' | sed 's/"target_directory":"//;s/"//')

target_name=echokit

# 构建 release 版本
r: 
	cargo build --release
	cp $(target_dir)/xtensa-esp32s3-espidf/release/$(target_name) ./target/

# 烧录固件到设备
flash:
	espflash flash --monitor --partition-table partitions.csv --flash-size 16mb ./target/$(target_name)