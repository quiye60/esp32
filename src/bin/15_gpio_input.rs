// ============================================================================
// 15 · GPIO 输入：按键
//
// 目标：掌握 Input 的创建（内部上拉）与两种读法：
//         轮询 is_low()           —— 简单，但一直占着 CPU，还可能漏掉短按
//         await 边沿 wait_for_*() —— 中断驱动，不占 CPU，按下瞬间就醒
//
// 参考：官方 esp-hal/src/gpio/mod.rs、esp-hal/src/gpio/asynch.rs 的文档示例
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO9 —— 按键 —— GND（按键不接时引脚被内部上拉成高电平 = 一直"松开"）
// 运行：cargo run --release --bin 15_gpio_input
// 预期：先每 200 ms 打印一次按键状态；之后按下/松开各有即时打印，LED 跟着亮灭
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器（去抖用）
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::delay::Delay;                     // 阻塞式延时（轮询用）
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull}; // GPIO 输入/输出
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

    // 按键：with_pull(Pull::Up) 打开芯片内部上拉电阻，省掉外部上拉
    // 于是「没按 = 高电平，按下（对地）= 低电平」
    let mut button = Input::new(
        peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );

    // LED：显示按键状态用
    let mut led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

        // ---------------- 第一段：轮询 ----------------
    let delay = Delay::new();

    for _ in 1..=10 {
        let pressed = button.is_low();         // 当前是不是低电平（= 按着）

        if pressed {
            led.set_high();                    // 按下 → 点亮
        } else {
            led.set_low();                     // 松开 → 熄灭
        }

        rprintln!("[轮询] 按键 = {}", if pressed { "按下" } else { "松开" });

        // 每 200 ms 看一次：这种写法会漏掉很短的按下（下一步用异步等待解决）
        delay.delay_millis(200);
    }

    // 时基：异步等待要用到
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- 第二段：异步等待边沿 ----------------
    loop {
        // 挂起，直到出现下降沿（= 按下）；等待期间 CPU 可以去干别的
        button.wait_for_falling_edge().await;
        rprintln!("[异步] 检测到按下");

        // 机械按键会抖动，等 50 ms 再确认一次电平 —— 最简单的软件去抖
        Timer::after(Duration::from_millis(50)).await;
        if button.is_low() {
            rprintln!("[异步] 确认按下（已去抖）");
            led.set_high();
        }

        // 再等上升沿（= 松开），把 LED 关掉
        button.wait_for_rising_edge().await;
        rprintln!("[异步] 松开");
        led.set_low();
    }
}
