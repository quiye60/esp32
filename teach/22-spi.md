# 22 · SPI 主机：一根跳线做回环自测

> 代码：[src/bin/22_spi.rs](../src/bin/22_spi.rs)　运行：`cargo run --release --bin 22_spi`

## 涉及的东西

| API | 说明 |
| --- | --- |
| `Spi::new(SPI2, Config::default().with_frequency(...).with_mode(Mode::_0))` | 建 SPI 主机（模式 0 最常用） |
| `.with_sck(...)` / `.with_mosi(...)` / `.with_miso(...)` | 时钟 / 主机输出 / 主机输入 |
| `spi.transfer(&mut buf)` | **全双工**：一边把 buf 发出去，一边把 MISO 上的电平写回 buf |
| `spi.write(&[u8])` / `spi.read(&mut buf)` | 只发 / 只收 |

## 接线

```
GPIO12 = SCK
GPIO11 = MOSI ──跳线── GPIO13 = MISO
```

原理：发出去的电平被自己的 MISO 直接读回来，所以「发出 == 收到」。

## 预期

```
发出：[01, 02, 03, 04]
收到：[01, 02, 03, 04]  ← 接了 MOSI-MISO 跳线时应与上面完全相同
write(0xA5) 完成，下面这条是只收模式（没从机时读到 0x00/0xFF）
read 到：[00, 00]
```

拔掉跳线后再跑，收到的会变成 `00` 或 `FF` —— 这是判断「跳线有没有接好」的快捷方法。

## 关键点

- SPI 必须有**片选（CS）**才能和真正的从机通信：CS 拉低选中从机，拉高释放。回环自测不需要 CS，这也是本课不接器件的原因。
- `transfer` 的长度由缓冲区决定：发 4 字节就一定收 4 字节（全双工是同时发生的）。
- 3.3V 电平，别直接接 5V 器件。

## 延伸

- 官方对照：`examples/peripheral/spi/loopback`、`hil-test/src/bin/spi.rs`（含从机、DMA、PSRAM 版）
- 接真器件（如 SPI 屏幕）时：CS 用普通 `Output`，每次传输前 `set_low()`、传输后 `set_high()`
