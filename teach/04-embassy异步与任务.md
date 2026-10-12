# 04 · embassy 异步与任务

## 4.1 `async/await` 的心智模型

```rust
loop {
    rprintln!("Hello world!");
    Timer::after(Duration::from_secs(1)).await;   // ← 这一行让出 CPU，1 秒后再回来
}
```

- `async fn` 编译后是一个**状态机**。`.await` 不是"调用一个会阻塞的函数"，
  而是"如果这个 future 还没就绪，就登记 waker 并把 CPU 还回去"。
- 执行器（executor）负责循环：谁能继续跑就轮询谁。本项目由 `#[esp_rtos::main]` 建好这个执行器。
- 所以 `.await` 期间，**别的任务能跑**；这就是为什么嵌入式 Rust 里到处是 `Timer::after(..).await` 而不是 `delay`。

## 4.2 执行器是谁、跑在哪

`esp-rtos` 的文档说得很清楚：打开 `embassy` feature、把 main 写成 `async`，
它会在**主线程上创建一个 thread-mode executor**。也就是说：

- 你有一个"RTOS 环境"（线程、调度、队列——Wi-Fi/BLE 驱动内部就靠它跑），
- 同时你的 `async main` 是这个 RTOS 里第一个线程上的第一个 embassy 任务。

```rust
#[esp_rtos::main]                         // 建执行器 + 生成真正的 main 符号
async fn main(spawner: Spawner) -> ! {    // Spawner 是执行器给你的"起任务手柄"
    ...
}
```

RTOS 层面还有 `esp_rtos::embassy::Executor`（自己再建一个线程模式执行器）和
`esp_rtos::embassy::InterruptExecutor<SWI>`（在某个中断优先级上跑执行器，用于"必须优先响应"的任务）。
新手先把主执行器用好就够了。

## 4.3 起一个任务：`#[task]` + `spawner.spawn(...)`

```rust
use embassy_executor::task;     // 就是 #[embassy_executor::task] 的短名字

#[task]                          // 把这个 async fn 变成"可以被 spawn 的静态任务"
async fn heartbeat() {           // 任务体：正常写 async 代码
    loop {
        rprintln!("tick");
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }
}

// 在 main 里：
spawner.spawn(heartbeat().unwrap());
```

三个必须知道的细节：

1. **0.10 版本 API 变了**：任务函数返回的是 `Result<SpawnToken<_>, SpawnError>`，
   而 `Spawner::spawn()` 本身返回 `()`。所以是 `spawner.spawn(task().unwrap())`。
   （改动原因是"在创建 token 时报错，而不是在 spawn 时报错"。）
   老教程里的 `spawner.spawn(task())?` 在这个版本**编译不过**，网上抄代码时注意版本。
2. **`SpawnToken` 不能丢**：`#[must_use]`，直接丢掉会 panic。必须交给某个 `spawner.spawn`。
3. **任务会一直占用自己的存储**：`SpawnToken` 里的 `SpawnError::Busy` 就是"同一任务还没结束又被 spawn"。
   需要同一函数有多个实例时写 `#[task(pool_size = 3)]`。

## 4.4 任务为什么要求 `'static`

任务的存储是 `static` 的，所以任务不能借用 `main` 的局部变量：

```rust
#[task]
async fn blinker(led: Output<'static>) {   // 参数可以，因为所有权被 move 进来
    loop {
        led.toggle();
        Timer::after(Duration::from_millis(500)).await;
    }
}

// main 里：peripherals.GPIO2 本身就是 'static 的，所以 Output<'static> 拿得到
spawner.spawn(blinker(Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default())).unwrap());
```

要传 `&mut` 之类的东西，就用 `StaticCell` 拿到 `&'static mut`（第 03 课手段 2），或者直接用
`AtomicX` / `Mutex` 这类本来就是 `'static` 的共享设施。

## 4.5 本地 Spawner 还是 SendSpawner

- `#[esp_rtos::main]` 给的 `Spawner` 是**本地** spawner：它不能跨线程传，但**可以 spawn 非 `Send` 的任务**
  （因为任务永远只在这个线程的执行器上跑）。多数场景用这个就够了。
- 需要从别的线程/中断里起任务：用 `SendSpawner`（`SendSpawner::for_current_executor()` 或
  `InterruptExecutor::start()` 的返回值），这时才要求 `S: Send`。

## 4.6 禁忌清单（踩了就很难查）

| 别做 | 为什么 | 该怎么做 |
| --- | --- | --- |
| `while x.elapsed() < d {}` 忙等 | 占住执行器线程，同线程其它任务全停 | `Timer::after(d).await` |
| 在 async 里 `Delay::delay_millis(1000)` | `Delay` 是纯忙等（源码就是循环查时间） | 同上；`Delay` 只留给微秒级驱动时序 |
| 在任务里 panic | `#[panic_handler]` 里是 `loop {}`，程序会**停在那里不重启** | 用 `Result` + 日志处理错误 |
| 在中断里 `.await` / 打印 | 中断上下文没有执行器、打印要临界区 | 中断里只置标志/发信号 |
| `spawn` 一个借用局部变量的任务 | 任务存储是 `'static`，编译器会拦你 | 传所有权 / `StaticCell` / `AtomicX` |

## 4.7 一个典型的任务拓扑

```text
main (async)
 ├── spawn  heartbeat_task()      日志心跳
 ├── spawn  led_task(Output)      LED 闪烁（拿到了 GPIO 所有权）
 ├── spawn  net_task(runner)      embassy-net 的 Runner（网络后台）
 └── spawn  wifi_task(controller) 连接管理/重连
```

主逻辑就写在 main 里（或者再 spawn 一个 `app_task`），通过 `AtomicX` / `Signal` / `Channel` 跟这些任务通信。

→ 下一课：时间相关的 API 细节（`Timer` / `Ticker` / `with_timeout` / `Delay`）。
