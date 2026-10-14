// ============================================================================
// 13 · 最小可跑程序：RTT 打印
//
// 目标：跑通「编译 → probe-rs 烧录 → RTT 打印」这条链路。
//       文件结构照 src/bin/tasks.rs 抄（自带 panic 处理器、app_desc、
//       内联 esp_rtos::start），只把业务换成一个心跳循环。
//
// 参考：src/bin/tasks.rs（本项目已验证可跑的模板）
//       官方 reference/esp-hal-1.2.2/examples/hello_world（它走串口，本课改成 RTT）
//
// 接线：不需要任何外部器件
// 运行：cargo run --release --bin 13_hello_rtt
// 预期：每秒打印一次心跳，含「开机多少毫秒」和自增计数
// 坑：  ① 只插 USB 串口、没接调试器 → RTT 一行都看不到
//       ② 漏了 esp_rtos::start(...) → 一执行 Timer::after 就 panic
//          （报错：unwrap of `scheduler.time_driver.as_mut()` failed）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // #[esp_rtos::main] 要求 main 有这个形参
use embassy_time::{Duration, Instant, Timer};   // 异步定时器 + 开机时长
use esp_hal::clock::CpuClock;                  // CPU 频率枚举（下面取最高频 240 MHz）
use esp_hal::timer::timg::TimerGroup;          // TIMG 定时器组：给调度器提供时基
use rtt_target::rprintln;                      // RTT 打印：probe-rs 从内存环形缓冲里读

#[panic_handler]                               // no_std 程序必须自己提供 panic 处理器
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);              // 打印 panic 消息 + 出错位置
    loop {}                                    // 原地停住（不复位），方便接调试器看现场
}

esp_bootloader_esp_idf::esp_app_desc!();       // 固件头：bootloader 靠它认这个镜像

#[esp_rtos::main]                              // 生成真正的 main 符号 + 建线程模式执行器
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();             // 打开 RTT（全程序只能调一次，越早越好）

    // 初始化芯片：配好 CPU 频率，并拿到所有外设句柄（整个程序只能调一次）
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 第一行日志：能看到它，就说明烧录 + RTT + 芯片启动全都通了
    rprintln!("13 · hello, esp32s3!");

    // 启动 esp-rtos 调度器：它会把「时基驱动」装上，Timer / Ticker / 任务才能工作。
    // 少了这两行的症状很迷惑：Instant::now() 还正常（直接读硬件定时器），
    // 但 Timer::after 一执行就 panic。
    let timg0 = TimerGroup::new(peripherals.TIMG0);            // 拿走 TIMG0 这把钥匙
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);  // 时基 = TIMG0 timer0

    // 计数器：每次循环 +1，用来证明程序确实在跑
    let mut count: u32 = 0;

    loop {
        count += 1;                            // 每圈 +1

        // Instant::now() = 开机以来经过的时间，as_millis() 取毫秒
        rprintln!("第 {} 次心跳，开机 {} ms", count, Instant::now().as_millis());

        // Timer::after 在等待期间会把 CPU 让出去（对比第 14 课的阻塞延时）
        Timer::after(Duration::from_secs(1)).await;
    }
}
