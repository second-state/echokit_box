.PHONY: r flash

target_dir=$(shell cargo metadata --format-version=1 | python3 -c "import sys, json; print(json.load(sys.stdin)['target_directory'])")
# target_dir=$(shell cargo metadata --format-version=1 | grep -o '"target_directory":"[^"]*"' | sed 's/"target_directory":"//;s/"//')

target_name=echokit

# 构建 release 版本
r: 
	cargo build --release
	cp $(target_dir)/xtensa-esp32s3-espidf/release/$(target_name) ./target/

run:
	cargo run --release

# 烧录固件到设备
flash:
	espflash flash --monitor --baud 460800 --partition-table partitions.csv --flash-size 16mb target/$(target_name)

# 分区,显示偏移
flash-pt:
	espflash partition-table partitions.csv

monitor:
	espflash monitor

build-model:
	./buildmodel.sh

# 烧录模型
flash-model:
	espflash erase-region 0x710000 5000000
	espflash write-bin 0x710000 --baud 460800 srmodels/srmodels.bin

flash-erase:
	espflash erase-flash

flash-erase-pt:
	espflash erase-region 0x710000 5000000