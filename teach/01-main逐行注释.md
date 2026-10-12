# 01 · 逐行看懂 `src/bin/main.rs`

下面是 `src/bin/main.rs` 的逐行注释版（行号和真实文件一致，方便你对着编辑器看）。
为了排版，`reason = "..."` 之类的长字符串我用 `…` 省略了。

```rust
  1  #![no_std]                                    // 不用标准库（原因见第 02 课）
  2  #![no_main]                                   // 不用 Rust 默认入口；真正的入口由 #[esp_rtos::main] 生成
  3  #![deny(
  4      clippy::mem_forget,                       // 禁 std::mem::forget：esp-hal 的句柄常持有 DMA 缓冲区，
  5      reason = "…"                              //   把句柄 forget 掉＝让硬件永远占着那块内存，是真 bug
  6  )]
  7  #![deny(
  8      clippy::large_stack_frames,                // 禁超大栈帧：嵌入式栈只有几 KB，大数组要放 static/堆
  9  )]
 10  use bt_hci::controller::ExternalController;   // 把 esp-radio 的 BLE 控制器适配成 bt-hci 的 Controller
 11  use embassy_executor::Spawner;                // 用来 spawn 异步任务（下面 ++ 会讲到）
 12  use embassy_time::{Duration, Timer};          // 异步定时器 / 时长类型
 13  use esp_hal::clock::CpuClock;                 // CPU 频率枚举，max() = 该芯片最高频率（S3 = 240 MHz）
 14  use esp_hal::timer::timg::TimerGroup;         // TIMG 定时器组：用来给调度器提供"时基"
 15  use esp_radio::ble::controller::BleConnector; // esp-radio 的 BLE 控制器（HCI 传输层）
 16  use rtt_target::rprintln;                     // 通过 RTT 打印日志（必须接 probe 才能看到）
 17  use trouble_host::prelude::*;                 // BLE 协议栈（host/GATT）的常用类型
 18  
 19  #[panic_handler]                              // no_std 下编译器要求你提供 panic 处理器
 20  fn panic(panic_info: &core::panic::PanicInfo) -> ! {  // 参数里带 panic 消息和位置
 21      rprintln!("{}", panic_info);              // 打印到 RTT；没接调试器＝看不到，程序看起来"莫名其妙卡住"
 22      loop {}                                   // 不重启、不退栈：停在原地，方便你连调试器看现场
 23  }
 24  
 25  extern crate alloc;                           // 打开 alloc（Vec/String/Box）；能真正分配靠下面注册的堆
 26  
 27  const CONNECTIONS_MAX: usize = 1;             // BLE 同时连接数上限（编译期常量 → 决定静态缓冲区大小）
 28  const L2CAP_CHANNELS_MAX: usize = 1;          // 每个 BLE 连接的 L2CAP 通道数上限
 29  
 30  // 这段注释解释下面这个宏：ESP-IDF 的 app 描述结构体
 31  // https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html
 32  esp_bootloader_esp_idf::esp_app_desc!();      // 生成固件头（版本/项目名/IDF 版本），二级 bootloader 要读它
 33  
 34  #[allow(                                       // 给 main 开个后门：允许这里有比较大的栈帧
 35      clippy::large_stack_frames,
 36      reason = "…"                               // 原文：main 里放较大的 buffer 很正常
 37  )]
 38  #[esp_rtos::main]                              // 关键宏：创建"线程模式 executor"，把你的 async main 当主任务跑
 39  async fn main(spawner: Spawner) -> ! {         // 形参必须是 Spawner；返回 ! = 永不返回，所以 main 里要有 loop
 40      // generator version: 1.4.0                 // esp-generate 留下的生成信息，可以删
 41      // generator parameters: -o esp32s3 …       // 当时选的生成选项，留着方便以后"重新生成"时对齐
 42  
 43      rtt_target::rtt_init_print!();             // 初始化 RTT 打印通道：全程序只能调一次，越早越好
 44  
 45      let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());  // 启动配置：CPU 跑满频
 46      let peripherals = esp_hal::init(config);   // 真正初始化芯片（时钟、看门狗等），返回所有外设句柄
 47  
 48      // 下面注释说：GPIO0/3/45/46 是 strapping 脚（能用，但要查数据手册）；
 49      // GPIO27~32 被模组内部（flash/PSRAM）占用，别用。
 55      let _gpio27 = peripherals.GPIO27;          // 把被占用的引脚"取走"，防止误用；前缀 _ 表示故意不用它
 56      let _gpio28 = peripherals.GPIO28;
 57      let _gpio29 = peripherals.GPIO29;
 58      let _gpio30 = peripherals.GPIO30;
 59      let _gpio31 = peripherals.GPIO31;
 60      let _gpio32 = peripherals.GPIO32;
 61  
 62      esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);  // 用"启动后回收的 DRAM"做堆
 63      // COEX needs more RAM - so we've added some more                     // Wi-Fi+BLE 共存要额外内存
 64      esp_alloc::heap_allocator!(size: 64 * 1024);                        // 再要 64 KiB（普通 DRAM）做堆
 65  
 66      let timg0 = TimerGroup::new(peripherals.TIMG0);        // 取出 TIMG0 定时器组
 67      esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);  // 把 timer0 + 一个软中断交给调度器
 68                                                             // → 从此 Timer/Instant 和线程调度才有效
 69      rprintln!("Embassy initialized!");                     // 走到这里就该能在终端看到输出了（说明链路通了）
 70  
 71      let _wifi_controller =                                 // 初始化 Wi-Fi 驱动（射频 + 协议栈）
 72          esp_radio::wifi::WifiController::new(peripherals.WIFI, Default::default())
 73              .expect("Failed to initialize Wi-Fi controller");  // 失败就 panic（信息会通过 RTT 打出来）
 74      let _wifi_interface = esp_radio::wifi::Interface::station();  // station 模式接口，以后要交给 embassy-net
 75      // find more examples …                                  // 上游 BLE 示例链接（trouble 仓库）
 76      let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();  // 打开 BLE 控制器
 77      let ble_controller = ExternalController::<_, 1>::new(transport);  // 适配成 bt-hci 控制器（1 = 命令缓冲数）
 78      let mut resources: HostResources<_, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
 79          HostResources::new();                              // BLE host 的静态资源表（大小由上面的常量决定）
 80      let _stack = trouble_host::new(ble_controller, &mut resources).build();  // 建 BLE 协议栈
 81                                                             // 注意：它借用了 resources，所以两者都必须活着
 82      // TODO: Spawn some tasks                                // 以后在这里 spawner.spawn(...) 起任务
 83      let _ = spawner;                                       // 现在不用 spawner，写这行是为了避免 unused 警告
 84  
 85      loop {                                                 // 主任务永不退出（返回类型是 `!`，必须如此）
 86          rprintln!("Hello world!");                         // 每秒打印一次
 87          Timer::after(Duration::from_secs(1)).await;         // .await 期间让出 CPU：别的任务/驱动线程可以跑
 88      }
 89  
 90      // for inspiration …                                    // esp-hal 官方示例仓库链接
 91  }
```

## 五个必须记住的点

**1. 三样东西全程序只能出现一次。** `rtt_target::rtt_init_print!()`、`esp_hal::init()`、
`esp_bootloader_esp_idf::esp_app_desc!()`。前两个重复调用会 panic/链接错误，最后一个重复会符号冲突。

**2. 初始化是有顺序的，不能乱。** `esp_hal::init` → 堆分配器 → `TimerGroup` + `esp_rtos::start` → 射频。
比如没有堆就没法给 Wi-Fi/BLE 分配缓冲；没有 `esp_rtos::start` 就没有时基和线程调度，Wi-Fi 驱动起不来。

**3. 带下划线的变量不是"没用"，是"保活"。** `_wifi_controller`、`_wifi_interface`、`_stack`
都是所有权句柄：一旦 drop，对应硬件/协议栈就被关掉。`esp-radio` 的文档明确写着
"dropping the controller deinitializes Wi-Fi"。

**4. `async fn main` 不是普通 main。** 它由 `esp_rtos` 建的执行器驱动（第 04 课细讲），
所以里面可以用 `.await`；也正因为执行器是协作式的，**不要在里面忙等**。

**5. 现在这个程序什么也没干。** 它只是"所有硬件都初始化好了 + 每秒打印一次"。
你后续写的代码，基本就是在第 82 行附近 `spawn` 任务，或把 `_wifi_*` / `_stack` 接进真正的业务。

> 下一步：看第 02 课搞懂"为什么必须 `no_std`、启动到底发生了什么"，然后直接跑第 08 课的点灯程序验证链路。
