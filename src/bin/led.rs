
#![no_std]      // 不用标准库：裸机没有 OS
#![no_main]     // 不用 Rust 默认入口；真正的入口由 #[esp_rtos::main] 宏生成

use embassy_executor::Spawner;                      // main 宏要求的形参类型（本例用不到，但位置要留着）
use embassy_time::{Duration, Timer};                // 异步定时器：.await 期间不会占着 CPU
use esp_hal::clock::CpuClock;                       // CPU 频率枚举
use esp_hal::gpio::{Level, Output, OutputConfig};   // GPIO 输出：电平、句柄、配置
use esp_hal::timer::timg::TimerGroup;               // TIMG 定时器组，用来给调度器做时基
use rtt_target::rprintln;                           // 通过 RTT 打印日志（需要 probe 连接着）

#[panic_handler]                                    // no_std 必须自己提供 panic 处理器
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);                   // 打印 panic 的消息和位置
    loop {}                                         // 原地停住（不重启），方便接调试器看现场
}

esp_bootloader_esp_idf::esp_app_desc!();            // 生成固件头：二级 bootloader 读它才肯启动你

#[esp_rtos::main]                                   // 建线程模式执行器 + 生成真正的 main 符号
async fn main(_spawner: Spawner) -> ! {             // 返回 `!`：永不返回，所以必须有 loop
    rtt_target::rtt_init_print!();                  // 初始化 RTT 打印（全程序只能调一次，越早越好）

    // 启动配置：CPU 跑最高频（ESP32-S3 是 240 MHz）
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);        // 初始化芯片，拿到所有外设句柄（只能调一次）

    // 给调度器提供时基：把 TIMG0 的 timer0 和 FROM_CPU_INTR0 交出去
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 这个例子不需要堆：不用 Vec/String/Box，也不开 Wi-Fi/BLE，所以没有 heap_allocator!

    // 把 GPIO2 的所有权交给 Output；从这行之后 peripherals.GPIO2 就不存在了（move）
    let mut led = Output::new(
        peripherals.GPIO2,                          // 引脚
        Level::Low,                                 // 初始电平：低（LED 灭）
        OutputConfig::default(),                    // 默认输出配置：推挽、默认驱动能力
    );

    rprintln!("blink start");                       // 走到这里说明：烧录 + RTT + 芯片启动都通了

    loop {
        led.toggle();                               // 翻转电平
        // is_set_high() 查询当前输出电平；这里顺便练习一下格式化打印
        rprintln!("led = {}", if led.is_set_high() { "HIGH" } else { "LOW" });
        Timer::after(Duration::from_millis(500)).await;  // 等 500 ms；await 期间让出 CPU
    }
}