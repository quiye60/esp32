// ============================================================================
// 14 · GPIO 输出：点亮 LED、闪烁
//
// 目标：掌握 Output 的创建/翻转/设置电平，并对比两种「等待」：
//         Delay  —— 阻塞忙等，精度稳，但纯占 CPU
//         Timer  —— 异步等待，期间让出 CPU，是 embassy 程序的常态
//
// 参考：官方 esp-hal/src/gpio/mod.rs 的文档示例、本项目 teach/05、08
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO2 ——[220Ω~1kΩ]—— LED 正极；LED 负极 —— GND（不接也能看打印）
// 运行：cargo run --release --bin 14_gpio_output
// 预期：先阻塞闪 3 次（300 ms 间隔），再异步每秒闪一次
// 坑：  要换引脚就改下面 Output::new 的第一个参数（约定见文件末尾注释）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型（本课不用它起任务）
use embassy_time::{Duration, Timer};           // 异步定时器
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::delay::Delay;                     // 阻塞式延时（忙等）
use esp_hal::gpio::{Level, Output, OutputConfig}; // GPIO 输出三件套
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use rtt_target::rprintln;                      // RTT 打印

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();             // 打开 RTT

    // 初始化芯片，拿到外设句柄
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 把 GPIO2 变成 LED 输出：
    //   Level::Low        = 初始低电平（LED 灭）
    //   OutputConfig::default() = 推挽输出、20 mA 驱动能力
    // 注意：这一行把 peripherals.GPIO2 的所有权拿走了，后面不能再取它
    let mut led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    // ---------------- 第一段：阻塞延时 ----------------
    // Delay 用 CPU 空转计时：不依赖调度器、精度稳，但期间啥也干不了
    let delay = Delay::new();

    for n in 1..=3u32 {
        led.toggle();                          // 翻转电平：低→高 或 高→低

        // is_set_high() 查询「我们当前设成什么电平」，顺便练格式化打印
        rprintln!(
            "[阻塞] 第 {} 次翻转，LED = {}",
            n,
            if led.is_set_high() { "亮" } else { "灭" }
        );

        delay.delay_millis(300);               // 忙等 300 毫秒
    }

    // ---------------- 第二段：异步延时 ----------------
    // 先给 embassy 装时基，Timer / Ticker / 任务才能工作
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 注意：Timer 必须在 esp_rtos::start 之后才能用
    let mut n: u32 = 0;
    loop {
        led.toggle();                          // 翻转 LED
        n += 1;

        rprintln!(
            "[异步] 第 {} 次翻转，LED = {}",
            n,
            if led.is_set_high() { "亮" } else { "灭" }
        );

        // await 期间执行器会把 CPU 让给别的任务（本课还没别的任务，先当省电）
        Timer::after(Duration::from_millis(1000)).await;
    }
}

// 引脚约定（本项目统一，改接线时两个地方一起改）：
//   LED   = GPIO2（高电平点亮）
//   按键  = GPIO9（内部上拉，按下为低）
//   电位器 = GPIO1（ADC1）
