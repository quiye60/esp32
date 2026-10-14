# 23 · 抽象层：embedded-hal / embedded-io trait

> 代码：[src/bin/23_traits.rs](../src/bin/23_traits.rs)　运行：`cargo run --release --bin 23_traits`

## 这一课想讲清一件事

**写「不认芯片」的代码**。同一段逻辑换个芯片、换个外设还能用，靠的就是 trait。

## 涉及的东西

| trait | 来自 | 谁实现了它 |
| --- | --- | --- |
| `embedded_hal::digital::StatefulOutputPin` | embedded-hal 1.0 | `esp_hal::gpio::Output` |
| `embedded_hal::delay::DelayNs` | embedded-hal 1.0 | `esp_hal::delay::Delay` |
| `embedded_io::Write` | embedded-io 0.7 | `esp_hal::uart::Uart`、USB 串口等 |

## 代码结构

```rust
fn blink<P: StatefulOutputPin, D: DelayNs>(pin: &mut P, delay: &mut D, times: u32) { ... }
fn banner<W: Write>(w: &mut W) -> Result<(), W::Error> { ... }
```

两个函数体里**没有** esp32s3 / GPIO2 / esp-hal 的字样：`blink` 只要求「能设电平 + 能延时」，`banner` 只要求「能写字节」。然后把 esp-hal 的 `Output` / `Delay` / `Uart` 递进去就能跑。

## 预期

L1 上的 LED 闪 3 次（RTT 里有提示），程序进入空闲循环。

## 关键点

- **延时也要传进来**：trait 只描述能力，不描述「怎么等」。这样同一个 `blink` 既能配阻塞延时，也能配异步延时。
- 返回 `Result<(), W::Error>` 而不是具体错误类型，这样任何 writer 都能用。
- 这一层是**生态的接口层**：别人的传感器驱动 crate 只依赖这些 trait，所以同一个驱动在 ESP32、STM32、RP2040 上都能用。

## 延伸

- 想深挖：`esp-hal/src/gpio/embedded_hal_impls.rs`（谁实现了哪些 trait）、`esp-hal/src/uart/compat.rs`
- 异步版的对应物：`embedded-hal-async`、`embedded-io-async`（第 19 课的 `into_async()` 就是在用它们）
