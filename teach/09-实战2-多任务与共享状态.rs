// ============================================================================
// 09 · 实战 2：多任务 + 共享状态（逐行注释）
//
// 怎么用：复制成 src/bin/tasks.rs，然后 cargo run --bin tasks
//
// 做两件事：
//   main    —— 每 250 ms 翻转 LED，并统计翻转次数
//   heartbeat —— 独立任务，每 1 秒把共享计数器 +1 并打印
// 两者通过 static AtomicU32 交换信息：这是嵌入式里最简单、最安全的共享方式。
// ============================================================================

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};   // 原子类型：无锁，任何上下文（含中断）都能访问

use embassy_executor::task;                      // 属性宏：把 async fn 变成"可 spawn 的任务"
use embassy_executor::Spawner;                   // 起任务的手柄
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

static HEARTBEATS: AtomicU32 = AtomicU32::new(0);  // 共享计数器（static + 原子 = 可跨任务访问）

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

// #[task] 展开后会给这个函数生成一个静态任务存储池；
// 调用 heartbeat() 本身不执行任何代码，只是"申请一个任务实例"，返回 Result<SpawnToken, SpawnError>。
#[task]
async fn heartbeat() {
    loop {
        // fetch_add 返回"加之前的旧值"，所以 +1 才是当前次数
        let n = HEARTBEATS.fetch_add(1, Ordering::Relaxed) + 1;
        rprintln!("[heartbeat] total = {}", n);
        Timer::after(Duration::from_secs(1)).await;   // 睡 1 秒（期间 main 能继续跑）
    }
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {               // 这次真要用 spawner，所以不加下划线
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(
        esp_hal::Config::default().with_cpu_clock(CpuClock::max()),
    );

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);   // 时基先就位，后面 Timer 才有效

    // 起任务。注意 embassy-executor 0.10 的 API 变化：
    //   任务函数返回 Result<SpawnToken, SpawnError>（错误发生在"创建 token"时）
    //   Spawner::spawn() 本身返回 ()
    // 老教程里的 `spawner.spawn(heartbeat())?` 在这个版本编译不过。
    spawner.spawn(heartbeat().unwrap());

    let mut led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    let mut switches: u32 = 0;                        // main 自己的局部变量：只有 main 用，不需要原子

    loop {
        led.toggle();
        switches += 1;

        if switches % 8 == 0 {                        // 每 8 次 = 每 2 秒汇总打印一次
            rprintln!(
                "led switches = {}, heartbeats = {}",
                switches,
                HEARTBEATS.load(Ordering::Relaxed)
            );
        }

        Timer::after(Duration::from_millis(250)).await;  // 让出 CPU：heartbeat 才有机会被轮询
    }
}
