# 18 · embassy 任务与同步原语：spawn / Signal / Channel / Mutex

> 代码：[src/bin/18_embassy_async.rs](../src/bin/18_embassy_async.rs)　运行：`cargo run --release --bin 18_embassy_async`

## 四个"执行体"的分工

| 谁 | 干什么 | 怎么和别的协作 |
| --- | --- | --- |
| `button_task` | 等按键按下 | `Signal` 通知 `led_task` |
| `led_task` | 收通知 → 翻转 LED | `Signal::wait()` |
| `sensor_task` | 每 500 ms 造一个读数 | 累加进 `Mutex`，塞进 `Channel` |
| `main` | 收数据打印（消费者） | `Channel::receive()` |

## 三种同步原语怎么选

| 原语 | 语义 | 什么时候用 |
| --- | --- | --- |
| `Signal<_, T>` | 只保存**最新一次**的值，没人等也不丢 | "通知一下"、状态变化、中断→任务 |
| `Channel<_, T, N>` | 队列，容量 N；满了 `send().await` 等消费者 | 数据流、生产者/消费者、不想丢数据 |
| `Mutex<_, T>` | 异步锁，`lock().await` 拿到 `&mut T` | 多个任务改同一份状态 |

另外两个常见的（后面无线课会用到）：`embassy_sync::channel::Watch`（只关心最新值 + 支持多消费者）、`embassy_sync::pubsub`（发布订阅）。

## 预期

```
[main] 收到读数 1，累计 = 1
[main] 收到读数 2，累计 = 3
[main] 收到读数 3，累计 = 6
[button_task] 检测到按下 → 通知 led_task
[led_task] LED = 亮
...
```

（`累计 = 1, 3, 6, 10…` 是 1+2+3+… 的和，正好用来检查共享状态没被写坏。）

## 关键点

- **任务函数是"生产 token"**：`sensor_task()` 本身不执行任何代码，它返回 `Result<SpawnToken, SpawnError>`；`spawner.spawn(...)` 才把任务交给执行器。embassy-executor 0.10 里 `spawn` 返回 `()`，所以写法是 `spawner.spawn(sensor_task().unwrap());`。
- **外设可以交给任务**：`Input` / `Output` 的句柄是 `'static` 的，直接当参数传给任务，主程序就不用再管它。
- **`*TOTAL.lock().await`**：`lock()` 拿到的是 guard，`*` 解引用才能取出里面的 `u32`；锁在表达式结束后立刻释放（不要把它跨 `await` 长期持有）。
- 三个通信对象都是 `static`，且 `CriticalSectionRawMutex` 是"关中断"级别的锁——够用来保护这些小数据，也不会引入优先级反转问题。

## 坑

- 任务里**不要写阻塞代码**（`delay.delay_millis()`、`while` 死等）。那会把整个执行器卡住，别的任务一个都跑不了 —— 阻塞代码应该单独起一个"专用任务"或用 `embassy_futures::block_on` 包起来。
- 任务需要的资源必须 `'static`（要么是 `static`、要么是外设句柄、要么是 `mk_static!` 出来的），否则编译器会报 lifetime 错误。
- `Signal` 只留最新值：如果信号发得太快，中间的会被覆盖（要保住每一次就用 `Channel`）。

## 附录：第二个核（选做）

ESP32-S3 是双核。把任务放到第二个核上跑需要：

1. `static APP_CORE_STACK: StaticCell<Stack<8192>>` 准备栈；
2. `esp_rtos::start_second_core(peripherals.CPU_CTRL, peripherals.FROM_CPU_INTR1, stack, || { /* 第二个核上的 Executor */ })`；
3. 在闭包里建一个 `esp_rtos::embassy::Executor` 并 `run()`。

完整可跑代码见官方例程 `reference/esp-hal-1.2.2/examples/async/embassy_multicore`（本课没做成 bin，是因为它和"任务/同步"是两个主题）。**要的话告诉我，我加一课 `19b_multicore.rs`。**

## 延伸

- 本项目 [04-embassy异步与任务.md](04-embassy异步与任务.md)、[09-实战2-多任务与共享状态.rs](09-实战2-多任务与共享状态.rs)
