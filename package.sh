#!/bin/bash
mkdir -p package

build_boards(){
    echo "build echokit(boards/DIY)"
    cargo build --release
    cp target/xtensa-esp32s3-espidf/release/echokit package/echokit_boards
    espflash save-image --chip esp32s3 --merge --flash-size 16mb --partition-table partitions.csv target/xtensa-esp32s3-espidf/release/echokit package/echokit_boards.bin
}

build_cube2(){
    echo "build echokit(cube2)"
    cargo build --release --features=cube2
    cp target/xtensa-esp32s3-espidf/release/echokit package/echokit_cube2
    espflash save-image --chip esp32s3 --merge --flash-size 16mb --partition-table partitions.csv target/xtensa-esp32s3-espidf/release/echokit package/echokit_cube2.bin
}

build_box(){
    echo "build echokit(box)"
    cargo build --release --features=box
    cp target/xtensa-esp32s3-espidf/release/echokit package/echokit_box
    espflash save-image --chip esp32s3 --merge --flash-size 16mb --partition-table partitions.csv target/xtensa-esp32s3-espidf/release/echokit package/echokit_box.bin
}

# 如果没有参数，默认构建全部
if [ $# -eq 0 ]; then
    build_boards
    build_cube2
    build_box
else
    # 遍历所有参数
    for target in "$@"; do
        case "$target" in
            boards)
                build_boards
                ;;
            cube2)
                build_cube2
                ;;
            box)
                build_box
                ;;
            *)
                echo "Unknown target: $target"
                echo "Usage: $0 [boards] [cube2] [box]"
                exit 1
                ;;
        esac
    done
fi

zip -r package package

if [ "$SAVE_IMAGE" != "true" ]; then
    rm -rf package
fi