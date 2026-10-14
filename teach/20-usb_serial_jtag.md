# 20 · USB-Serial-JTAG：用板子自己的 USB 口当串口

> 代码：[src/bin/20_usb_serial_jtag.rs](../src/bin/20_usb_serial_jtag.rs)　运行：`cargo run --release --bin 20_usb_serial_jtag`

## 涉及的东西

| API | 说明 |
| --- | --- |
| `UsbSerialJtag::new(peripherals.USB_DEVICE)` | 建 USB 串口（路径是 `esp_hal::usb::usb_serial_jtag`） |
| `usb.write_all(&[u8])` / `usb.flush()` | 用 `embedded_io::Write` trait 发数据 |
| `usb.split()` | 拆成 `(rx, tx)` 两半，可以分别交给不同任务 |
| `rx.read(&mut buf)` | 用 `embedded_io::Read` 阻塞读（没人发数据就一直等） |

## 硬件

ESP32-S3 的那个 USB 口同时提供两条逻辑通道：

- **JTAG**：probe-rs 烧录、调试、RTT 走它
- **USB-Serial-JTAG 串口**：本课用的「串口」，在电脑上表现为 `/dev/ttyACM*`（Windows 是 COM 口）

两条通道互不干扰，所以 RTT 打印和这个串口可以同时用。引脚不用配（固定 GPIO19/20）。

## 运行与预期

```bash
cargo run --release --bin 20_usb_serial_jtag
```

1. RTT 里出现「已通过 USB 串口发出问候语」；
2. 另开一个终端 `screen /dev/ttyACM0 115200`（或任意串口工具）→ 能看到 `hello from USB-Serial-JTAG!`；
3. 在串口工具里打字 → 板子原样回显，同时 RTT 里出现「收到 N 字节」。

## 坑

- 串口工具要独占该端口；probe-rs 占着的时候可能打不开（先关掉 `cargo run` 之外的占用进程）。
- 阻塞 `read` 没人发数据时会一直等——这是设计如此，不是死机。
- 别把 RTT 和这个串口搞混：RTT 是内存环形缓冲（probe-rs 读），USB 串口是真正的 CDC 数据流。
