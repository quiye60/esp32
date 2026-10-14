# 13 · 最小可跑程序：RTT 打印

> 代码：[src/bin/13_hello_rtt.rs](../src/bin/13_hello_rtt.rs)　运行：`cargo run --release --bin 13_hello_rtt`

## 这一课要你确认的一件事

**"能看到打印"这条链路通了**。后面所有课都靠它验证，所以先把它跑通再往下走。

## 涉及的东西

| 东西 | 在哪 | 作用 |
| --- | --- | --- |
| `#[panic_handler]` + `esp_app_desc!()` | 本课文件（每个课都自带一份，照 [tasks.rs](../src/bin/tasks.rs) 抄） |
| `rtt_target::rtt_init_print!()` | 打开 RTT（RTT 是内存环形缓冲，probe-rs 负责取走） |
| `esp_hal::init(Config::default().with_cpu_clock(CpuClock::max()))` | 配 CPU 频率 + 初始化芯片（全程序只能调一次） |
| `#[esp_rtos::main]` + `async fn main` | 本课文件 | 建线程模式执行器，所以 main 能写 `await` |
| `Timer::after(...).await` | embassy-time | 等待期间让出 CPU（对比第 17 课的阻塞延时） |
| `esp_rtos::start(timer0, FROM_CPU_INTR0)` | 本课文件 | 给 embassy 装上时基驱动，`Timer` 才有效 |

## 运行与预期

```bash
cargo run --release --bin 13_hello_rtt
```

probe-rs 会先烧录再打开 RTT，你应该看到：

```
13 · hello, esp32s3!
第 1 次心跳，开机 0 ms
第 2 次心跳，开机 1000 ms
...
```

## 坑

- **只插 USB 串口、没接调试器 → 一行都看不到**。本项目用 probe-rs + RTT，不是串口打印（见 [11-排错清单](11-排错清单.md)）。
- **忘了 `esp_rtos::start(...)` → 打印几行后 panic**，报错长这样：

  ```
  panic: panicked at .../esp-rtos-0.4.0/src/fmt.rs:257:5:
  unwrap of `scheduler.time_driver.as_mut()` failed: NoneError
  ```

  原因：`Timer::after` 需要调度器里的"时基驱动"来安排唤醒，而那个驱动只有
  `esp_rtos::start(...)` 才会装上。
  有意思的是 `Instant::now()` 和第一行打印都正常（它们直接读硬件定时器），
  所以症状是"前面都好，一 await 就崩"——遇到这个报错，先查 `esp_rtos::start` 在不在。
- panic 处理器里也用 `rprintln!`，而 RTT 是 `rtt_init_print!()` 打开的。所以"还没 init 就 panic"可能看不到信息——这也是为什么它永远写在 main 第一句。
- 改完 `Cargo.toml`、`.cargo/esp-config.toml` 记得 `cargo clean`，否则编译期配置不会重新生成。

## 延伸

- 官方对照：`reference/esp-hal-1.2.2/examples/hello_world`（它用 `esp_println` 走串口，其余结构一样）
- 想看 RTT 的原始细节：`teach/06-日志与调试.md`
