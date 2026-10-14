# 17 · 时间 API 全景：Instant / Delay / Rate / Timer / Ticker / with_timeout

> 代码：[src/bin/17_time_delay.rs](../src/bin/17_time_delay.rs)　运行：`cargo run --release --bin 17_time_delay`

## 四套时间工具，分别用在哪

| 工具 | 类型 | 用在哪 | 会不会让出 CPU |
| --- | --- | --- | --- |
| `esp_hal::time::Instant` | 阻塞 | 测耗时、看开机多久 | —（只是读时间） |
| `Delay` | 阻塞 | 初始化时序、传感器"等它准备" | ❌ 忙等 |
| `esp_hal::time::{Rate, Duration}` | 配置值 | 外设配置里描述频率/超时（LEDC、SPI、TWAI…） | —（只是类型） |
| `embassy_time::{Timer, Ticker, with_timeout}` | 异步 | 主业务里的等待、定时节奏 | ✅ 让出 CPU |

## 预期输出

```
1) delay_millis(200) 实测 2000xx us
   开机至今 ... ms（Instant::duration_since_epoch）
2) Rate::from_khz(24) = 24000 Hz
   HalDuration::from_millis(1500) 在配置里表示 1500 ms 超时
3) Timer::after(500ms) 开始等…
   等到了，开机 ... ms
   Ticker 第 1 拍，开机 ... ms
   ...
4) with_timeout(300ms, 等1s) → 超时 Err（预期）
17 课演示结束，程序进入空闲循环
```

## 关键点

- **名字冲突**：`esp_hal::time::Duration` 和 `embassy_time::Duration` 是两个不同的类型（前者是 fugit 的 Duration，用于外设配置；后者是 embassy 的）。本课用 `as` 起别名区分，这是常见做法。
- **Ticker vs Timer 循环**：`loop { Timer::after(1s).await }` 的实际周期是"1 s + 处理耗时"，会累积漂移；`Ticker::every(1s).next().await` 按计划时间推进，不累积。
- **with_timeout 的语义**：超时会**取消**内部那个 Future（在这里就是取消 `Timer::after`），返回 `Err(TimeoutError)`。它和 `select` 是异步编程里最常用的两个"组合子"。
- 阻塞延时精度看 `as_micros()` 的值：比 200 ms 略大是正常的（调用开销 + 打印本身耗时）。

## 坑

- `Timer`/`Ticker` 必须在 `esp_rtos::start(...)` **之后**使用。顺序反了不会报错，但会永远等不到时间（现象是程序卡住不动）。
- `delay.delay_nanos(500)` 只是"睡至少 500 ns"：实际精度受 CPU 主频（S3：240 MHz，一个时钟约 4 ns）和指令开销限制。

## 延伸

- 本项目 [05-时间_延时_超时.md](05-时间_延时_超时.md)（概念版）
- 官方对照：`esp-hal/src/time.rs`、`esp-hal/src/delay.rs`；`reference/esp-hal-1.2.2/examples/async/embassy_hello_world`
