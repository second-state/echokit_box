# Setup the EchoKit device

Firmware for the EchoKit device.

<br>
<div align="center">
    <a href="https://echokit.dev/">Website</a> |
    <a href="https://discord.gg/Fwe3zsT5g3">Discord</a> |
    <a href="https://youtu.be/Zy-rLT4EgZQ">Live Demo</a> |
    <a href="https://echokit.dev/docs/quick-start/">Documentation</a>
</div>
</br>

## Buttons on the device

The `RST` button is to restart the system. On the EchoKit devkit, it is labeled as `rst` on the main ESP32 board.

The `K0` button is the main action button for the application. On the EchoKit devkit, it is the single button to the left of the LCD screen on the extension board.

> The `boot` button on the ESP32 board is the SAME as the `K0` button.

## Quick start

Flash the `echokit.bin` device image using the web-based [ESP Launchpad](https://espressif.github.io/esp-launchpad/?flashConfigURL=https://echokit.dev/firmware/echokit.toml) tool.

## Install espflash

Assume that you [installed the Rust compiler](https://www.rust-lang.org/tools/install) on your computer.

```
cargo install cargo-espflash espflash ldproxy
```

## Build the firmware

Get a pre-compiled binary version of the firmware. The firmware binary file is `echokit`.

```
curl -L -o echokit https://echokit.dev/firmware/echokit-boards
```

To build the `echokit` firmware file from source, you need to make sure that you install the [OS-specific dependencies](https://docs.espressif.com/projects/rust/book/installation/std-requirements.html) and then [ESP toolchain for Rust](https://docs.espressif.com/projects/rust/book/installation/riscv-and-xtensa.html). You can then build from the source and find the binary firmware in `target/xtensa-esp32s3-espidf/release/`.

```
cargo build --release
```

Optional: Build the device image.

```
espflash save-image --chip esp32s3 --merge --flash-size 16mb target/xtensa-esp32s3-espidf/release/echokit echokit.bin
```

<details>
<summary> Alternative firmware </summary>

If you have the fully integrared box device, you can use the following command to download a pre-built binary.

```
curl -L -o echokit https://echokit.dev/firmware/echokit-box
```

To build it from the Rust source code. 

```
cargo build  --no-default-features --features box
```

</details>

## Flash the firmware

Connect to your computer to the EchoKit device USB port labeled `TTL`. Allow the computer to accept connection from the device when prompted. 

> On many devices, there are two USB ports, but only the `SLAVE` port can take commands from another computer. You must connect to that `SLAVE` USB port. The detected USB serial port should be `JTAG`. IT CANNOT be `USB Single`.

```
espflash flash --monitor --flash-size 16mb echokit
```

The response is as follows.

```
[2025-04-28T16:51:43Z INFO ] Detected 2 serial ports
[2025-04-28T16:51:43Z INFO ] Ports which match a known common dev board are highlighted
[2025-04-28T16:51:43Z INFO ] Please select a port
✔ Remember this serial port for future use? · no
[2025-04-28T16:52:00Z INFO ] Serial port: '/dev/cu.usbmodem2101'
[2025-04-28T16:52:00Z INFO ] Connecting...
[2025-04-28T16:52:00Z INFO ] Using flash stub
Chip type:         esp32s3 (revision v0.2)
Crystal frequency: 40 MHz
Flash size:        8MB
Features:          WiFi, BLE
... ...
I (705) boot: Loaded app from partition at offset 0x10000
I (705) boot: Disabling RNG early entropy source...
I (716) cpu_start: Multicore app
```

> If you have problem with flashing, try press down the `RST` button and, at the same time, press and release the `boot` (or `K0`) button. The device should enter into a special mode and be ready for flashing. 

## Reset the device

Reset the device (simulate the RST button or power up).

```
espflash reset
```

Delete the existing firmware if needed.

```
espflash erase-flash
```

## Next steps

You will need to configure and start up an [EchoKit server](https://github.com/second-state/echokit_server), and then configure your device to connect to the server in order for the EchoKit device to be fully functional.



