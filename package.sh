#!/bin/bash
mkdir package
cargo build --release
cp target/xtensa-esp32s3-espidf/release/echokit package/echokit_boards
espflash save-image --chip esp32s3 --merge --flash-size 16mb target/xtensa-esp32s3-espidf/release/echokit package/echokit_boards.bin

cargo build --release --features=cube
cp target/xtensa-esp32s3-espidf/release/echokit package/echokit_cube
espflash save-image --chip esp32s3 --merge --flash-size 16mb target/xtensa-esp32s3-espidf/release/echokit package/echokit_cube.bin

zip -r package package

rm -rf package