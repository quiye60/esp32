# 19 · UART：阻塞收发 + 转异步

> 代码：[src/bin/19_uart.rs](../src/bin/19_uart.rs)　运行：`cargo run --release --bin 19_uart`

## 涉及的东西

| API | 说明 |
| --- | --- |
| `Uart::new(UART1, Config::default().with_baudrate(115200))` | 建 UART，返回 `Result`（波特率算不出来才会失败） |
| `.with_tx(pin)` / `.with_rx(pin)` | S3 的 UART 引脚可任选（走 GPIO 交换矩阵） |
| `uart.write(&[u8])` / `uart.read(&mut [u8])` | 阻塞收发，返回实际字节数 |
| `uart.into_async()` | 换成中断驱动实现，类型变 `Uart<'_, Async>` |
| `embedded_io_async::Write/Read` | 异步读写 trait（esp-hal 已实现 0.7 版；写全路径可避免和阻塞同名方法混淆） |

## 接线（回环自测）

```
GPIO17 (UART1 TX) ──跳线── GPIO18 (UART1 RX)
```

这样发出去的数据会立刻被自己收到，不用任何外部器件就能验证读写。想看电脑上的输出，把 USB-TTL 的 RXD 接 GPIO17、GND 接板子 GND（115200 8N1）。

## 预期

```
1) 阻塞模式：发 8 字节，再把回环读回来
   已发送 8 字节
   收到 8 字节：BLOCKING
2) 异步模式：每隔 1 秒回环一次
   异步收到 7 字节：[41, 53, 59, 4E, 43, D, A]
...
```

## 关键点

- **UART0 别用来练手**：它被 ROM/bootloader 用于启动日志，本项目固定用 UART1。
- 阻塞版和异步版的**方法名一样**（都有 `write`/`read`），所以代码里写全路径 `embedded_io_async::Write::write_all(&mut uart, ...)` 最不容易混。
- 异步版必须已经 `esp_rtos::start(...)`，否则中断/时基没起来。

## 延伸

- 官方对照：`reference/esp-hal-1.2.2/examples/async/embassy_serial`（异步 + 任务版）、`hil-test/src/bin/uart.rs`（含 split / 流控 / 超时）
- 想在 UART 上跑日志：见课程 07（`ESP_LOG` / 日志前端）
