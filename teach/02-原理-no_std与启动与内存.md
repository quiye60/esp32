


# 02 · 原理：no_std、启动流程、内存

这一课解释"为什么这个文件必须长成那样"。理解了它，后面看任何 esp-hal 代码都会顺。

## 2.1 上电之后到底谁在跑你的代码

```text
上电 / 复位
  │
  ├─ ROM bootloader           芯片里固化的，不归你管；它决定从 flash 哪个地址启动
  │
  ├─ 二级 bootloader          esp-bootloader-esp-idf 烧在 flash 起始处
  │                            └─ 读你的固件头（esp_app_desc!() 生成的那个结构体）做校验
  │
  ├─ 跳进 Rust 入口           esp-hal 提供的启动代码：设栈指针、初始化 .data/.bss、设置时钟等
  │                           （这一步是 esp_rtos::main / esp_hal::main 宏安排的，你看不到源码）
  │
  └─ 你的 async main          esp_rtos 建好线程模式执行器后，把你的 main 当第一个任务跑
```

所以 `#![no_main]` 的含义是：**别生成 Rust 那个 `main` 符号**，由宏生成给 bootloader 看的入口。
`esp_app_desc!()` 之所以必须有，是因为二级 bootloader 要先读到合法的固件头才肯跳进来。

## 2.2 `#![no_std]` 意味着什么

`std` 假设下面有一个操作系统：有文件、有线程、有堆、有 `Instant::now()`。裸机上这些都不存在，于是：

| 没有 | 替代方案 |
| --- | --- |
| 线程 / 睡眠 | `embassy` 异步 + `esp-rtos`（RTOS 调度、线程、队列） |
| 网络 | `embassy-net`（TCP/UDP/DNS/DHCP）+ `esp-radio`（Wi-Fi） |
| 文件系统 | 自己接 `embassy-net`/flash 相关库，或用 `alloc` + 自定义存储 |
| 时间 | `embassy-time`（异步）/ `esp_hal::time`（同步、给驱动用） |
| `println!` | `rprintln!`（RTT，需 probe）/ `esp_println`（走串口） |
| 自动 panic 处理 | 必须自己写 `#[panic_handler]` |

但 `#![no_std]` **不等于没有堆**。`extern crate alloc;` + `esp_alloc::heap_allocator!` 之后，
`Vec`/`String`/`Box` 都能用，只是堆内存是你手动划出来的（下面 2.3）。

## 2.3 内存与堆：`heap_allocator!` 那两行在干什么

`esp_alloc::heap_allocator!` 注册全局分配器（`#[global_allocator]` 由宏展开出来），每调用一次就加入一块内存区域：

```rust
// 第一块：启动早期被 bootloader 用过、之后不再需要的 DRAM。
// esp-alloc 的文档原话是 "the memory region reclaimed from the bootloader, which is otherwise unused"
esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

// 第二块：普通 DRAM 里的 64 KiB。注释说这是给 Wi-Fi + BLE 共存（COEX）额外要的
esp_alloc::heap_allocator!(size: 64 * 1024);
```

实践要点：

- **分配失败/`unwrap()` panic 时，第一个该调的就是这两行的大小**。Wi-Fi + BLE 一起用、或者开大缓冲，都很吃 RAM。
- `#[esp_hal::ram(reclaimed)]` 是"白捡的内存"（bootloader 用完就空着），所以模板把它尽量用满了；
  哪块放什么、还剩多少，可以看链接后生成的 `.map` 文件。
- 栈和堆是两回事。栈大小由 esp-hal 的配置项管（见第 07 课的 `ESP_HAL_CONFIG_*`），
  栈溢出会踩栈守卫（`ESP_HAL_CONFIG_STACK_GUARD_MONITORING`），这也是为什么文件头 `deny(large_stack_frames)`。

## 2.4 时基和调度：`esp_rtos::start(...)` 是关键一步

```rust
let timg0 = TimerGroup::new(peripherals.TIMG0);                    // 拿一个硬件定时器
esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);         // 交给调度器当"心跳"
```

`esp-rtos` 的文档写得很直接：这个 crate 需要 "an esp-hal timer, as well as the `FROM_CPU0` software interrupt"，
启动之后就提供了线程、队列等能力；开启 `embassy` feature 并把 main 写成 `async`，
它会在主线程上创建一个 **thread-mode executor**（线程模式执行器）。

换句话说：

- 这一行之前：`Timer::after(...).await` 不会正常完成，Wi-Fi 驱动也起不来。
- 这一行之后：`embassy_time` 有了时基，`#![esp_rtos::main]` 的执行器开始轮询你的任务。

## 2.5 `esp_hal::Config` 在配什么

```rust
let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());  // S3：240 MHz
let peripherals = esp_hal::init(config);                                  // 只调用一次
```

- `with_cpu_clock`：跑多快。低功耗场景可以用更低频率（`CpuClock::_80MHz` 之类），代价是吞吐。
- 启动期的其它设置（看门狗、栈守卫等）一部分在 `Config`，一部分走编译期配置项（第 07 课）。
- 返回的 `peripherals` 是"所有外设的唯一钥匙串"，第 03 课专门讲它的所有权规则。

## 2.6 小结：一句话版本

> ROM → 二级 bootloader → esp-hal 启动代码 → `esp_rtos::main` 建执行器 → 你的 `async main`；
> 堆要自己划（`heap_allocator!`），时基要自己给（`esp_rtos::start`），panic 要自己处理（`#[panic_handler]`）。

下一个问题自然就是：`peripherals` 拿到手之后，怎么用、怎么分给别人？→ 第 03 课。
